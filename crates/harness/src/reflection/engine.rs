//! LLM-driven reflection (spec §7): reads current memory + recent activity,
//! REPLACES the memory (compress, don't accumulate), preserving the full
//! prior entry (provenance and all) for facts that survive verbatim.
//! Returns a churn score the cadence policy consumes.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::agent::RunError;
use crate::error::HarnessError;
use crate::llm::{
    CompletionRequest, ContentBlock, LlmProvider, Message, ToolSpec, Usage,
};
use crate::memory::{FactSource, Memory, DEFAULT_WORD_CAP};

const WRITE_MEMORY: &str = "write_memory";

#[derive(Debug)]
pub struct ReflectionOutcome {
    pub memory: Memory,
    /// (added + removed) / (old_count + new_count); 0.0 when both are empty.
    /// Measured after word-cap clamping — actual-memory churn, not LLM-intent churn.
    pub churn: f32,
    pub usage: Usage,
}

pub struct ReflectionEngine {
    provider: Arc<dyn LlmProvider>,
    pub word_cap: usize,
    pub max_tokens: u32,
}

impl ReflectionEngine {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        ReflectionEngine { provider, word_cap: DEFAULT_WORD_CAP, max_tokens: 2048 }
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: WRITE_MEMORY.into(),
            description: "Write the complete updated memory. This REPLACES all sections."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "sections": {
                        "type": "object",
                        "description": "section name -> list of short fact strings",
                        "additionalProperties": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "required": ["sections"]
            }),
        }
    }

    fn system_prompt(&self) -> String {
        format!(
            "You maintain a compact long-term memory about one user for a field-work \
             assistant. Rewrite the ENTIRE memory: keep what stays true, integrate what \
             the recent activity shows, drop what is stale or disproven. Prefer fewer, \
             sharper facts. Hard limit: {} words total. \
             Keep facts that survive VERBATIM, character for character — do not paraphrase \
             them. When recent activity contradicts an existing fact, drop the stale fact \
             and write the corrected one; never merge the two into a blended claim. Facts \
             marked [corrected] are user corrections and outrank everything else — do not \
             drop or alter them. Typical sections: vocabulary, people, projects, \
             preferences. Call {} exactly once with the full result.",
            self.word_cap, WRITE_MEMORY
        )
    }

    /// Renders memory like `to_prompt`, but marks user-corrected facts with
    /// ` [corrected]` so the model can honor their precedence (see
    /// `Memory::render` for the marker-masquerading threat model note).
    fn memory_block(&self, memory: &Memory) -> String {
        if memory.sections.is_empty() {
            return "(empty)".to_string();
        }
        memory.render(true)
    }

    /// Runs one reflection. Must not overlap an active session: the caller
    /// swaps-and-persists the returned Memory, so an interleaved `update_memory`
    /// mutation would be silently discarded.
    ///
    /// On error, `RunError::usage` is zero when the provider call itself failed
    /// (network/auth — no tokens were burned). For post-completion failures
    /// (missing `write_memory`, malformed sections, empty-wipe guard), `usage`
    /// holds the tokens from the completed response so callers can log the cost.
    pub async fn reflect(
        &self,
        current: &Memory,
        activity: &[String],
        now: u64,
    ) -> Result<ReflectionOutcome, RunError> {
        let activity_block = if activity.is_empty() {
            "(none)".to_string()
        } else {
            activity
                .iter()
                .enumerate()
                .map(|(i, a)| format!("{}. {a}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let user = format!(
            "Current memory:\n{}\n\nRecent activity since last reflection:\n{activity_block}",
            self.memory_block(current)
        );

        let response = self
            .provider
            .complete(CompletionRequest {
                system: self.system_prompt(),
                messages: vec![Message::user_text(user)],
                tools: vec![self.tool_spec()],
                max_tokens: self.max_tokens,
                tool_choice: Some(WRITE_MEMORY.into()),
            })
            .await
            .map_err(|e| RunError { source: e, usage: Usage::default() })?;

        // Capture usage now: every post-completion error path below carries it
        // so the coordinator can log what was burned even on content failure.
        let response_usage = response.usage;

        let input = response
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::ToolUse { name, input, .. } if name == WRITE_MEMORY => Some(input),
                _ => None,
            })
            .ok_or_else(|| RunError {
                source: HarnessError::Provider(
                    "reflection response missing write_memory call".into(),
                ),
                usage: response_usage,
            })?;
        let sections = input
            .get("sections")
            .and_then(|s| s.as_object())
            .cloned()
            .ok_or_else(|| RunError {
                source: HarnessError::Provider(
                    "write_memory call had malformed sections".into(),
                ),
                usage: response_usage,
            })?;

        let mut memory = Memory::default();
        for (section, texts) in &sections {
            // Non-array section values drop the section; next reflection repopulates.
            let Some(texts) = texts.as_array() else { continue };
            for text in texts.iter().filter_map(|t| t.as_str()) {
                let prior = current
                    .sections
                    .get(section)
                    .and_then(|es| es.iter().find(|e| e.text == text))
                    .cloned();
                match prior {
                    Some(e) => {
                        memory.remember_from(section, text, e.last_touched, e.source, e.session)
                    }
                    None => memory.remember_from(section, text, now, FactSource::Inferred, None),
                }
            }
        }
        // A legit total wipe never happens; an empty write_memory result from a
        // confused model must not erase the user's memory. (Empty current with
        // an empty result stays OK — first-run case.)
        if !current.sections.is_empty() && memory.sections.is_empty() {
            return Err(RunError {
                source: HarnessError::Provider(
                    "reflection produced empty memory from non-empty input".into(),
                ),
                usage: response_usage,
            });
        }
        memory.clamp_to_cap(self.word_cap);

        let churn = churn_between(current, &memory);
        Ok(ReflectionOutcome { memory, churn, usage: response_usage })
    }
}

/// (added + removed) / (old_count + new_count), 0.0 when both sides are empty.
fn churn_between(old: &Memory, new: &Memory) -> f32 {
    let keys = |m: &Memory| -> BTreeSet<(String, String)> {
        m.sections
            .iter()
            .flat_map(|(s, es)| es.iter().map(move |e| (s.clone(), e.text.clone())))
            .collect()
    };
    let old_keys = keys(old);
    let new_keys = keys(new);
    let denominator = old_keys.len() + new_keys.len();
    if denominator == 0 {
        return 0.0;
    }
    let added = new_keys.difference(&old_keys).count();
    let removed = old_keys.difference(&new_keys).count();
    (added + removed) as f32 / denominator as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::*;
    use crate::memory::{FactSource, MemoryEntry};
    use crate::mock::MockProvider;
    use std::sync::Arc;

    fn write_memory_response(sections: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_memory".into(),
                input: serde_json::json!({ "sections": sections }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 100, output_tokens: 50 },
        }
    }

    fn current_memory() -> Memory {
        let mut m = Memory::default();
        m.remember_from("people", "Dev — framer", 111, FactSource::Corrected, Some("s1".into()));
        m.remember("people", "Dave — plumber", 222);
        m
    }

    #[tokio::test]
    async fn rebuilds_memory_preserving_full_prior_entries() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["Dev — framer", "Sara — electrician"] }),
        )]));
        let engine = ReflectionEngine::new(provider.clone());

        let out = engine
            .reflect(&current_memory(), &["walked the Johnson site".into()], 999)
            .await
            .unwrap();

        let people = &out.memory.sections["people"];
        assert_eq!(people.len(), 2);
        // survivor keeps its FULL prior entry: source, session, last_touched
        assert_eq!(
            people[0],
            MemoryEntry {
                text: "Dev — framer".into(),
                last_touched: 111,
                source: FactSource::Corrected,
                session: Some("s1".into()),
            }
        );
        // new fact: Inferred, no session, touched now
        assert_eq!(
            people[1],
            MemoryEntry {
                text: "Sara — electrician".into(),
                last_touched: 999,
                source: FactSource::Inferred,
                session: None,
            }
        );
        assert_eq!(out.usage, Usage { input_tokens: 100, output_tokens: 50 });

        // request shape: forced tool, memory + activity present, corrected marker rendered
        let reqs = provider.requests();
        assert_eq!(reqs[0].tool_choice.as_deref(), Some("write_memory"));
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text block")
        };
        assert!(text.contains("Dev — framer [corrected]"));
        assert!(text.contains("Dave — plumber"));
        assert!(!text.contains("Dave — plumber [corrected]"));
        assert!(text.contains("walked the Johnson site"));
    }

    #[tokio::test]
    async fn churn_measures_added_plus_removed() {
        // old: {Dev, Dave}; new: {Dev, Sara} → added 1, removed 1, sizes 2+2 → churn 0.5
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["Dev — framer", "Sara — electrician"] }),
        )]));
        let engine = ReflectionEngine::new(provider);
        let out = engine.reflect(&current_memory(), &[], 999).await.unwrap();
        assert!((out.churn - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn identical_rewrite_has_zero_churn() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["Dev — framer", "Dave — plumber"] }),
        )]));
        let engine = ReflectionEngine::new(provider);
        let out = engine.reflect(&current_memory(), &[], 999).await.unwrap();
        assert_eq!(out.churn, 0.0);
    }

    #[tokio::test]
    async fn result_is_clamped_to_word_cap() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "notes": ["one two three", "four five six seven"] }),
        )]));
        let mut engine = ReflectionEngine::new(provider);
        engine.word_cap = 4;
        let out = engine.reflect(&Memory::default(), &[], 999).await.unwrap();
        assert!(out.memory.word_count() <= 4);
    }

    #[tokio::test]
    async fn duplicate_texts_in_response_collapse_to_one() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["foo", "foo"] }),
        )]));
        let engine = ReflectionEngine::new(provider);
        let out = engine.reflect(&Memory::default(), &[], 999).await.unwrap();
        assert_eq!(out.memory.sections["people"].len(), 1);
    }

    #[tokio::test]
    async fn missing_tool_call_is_an_error() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::Text { text: "I decline".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        }]));
        let engine = ReflectionEngine::new(provider);
        let err = engine.reflect(&Memory::default(), &[], 999).await.unwrap_err();
        // provider failure before response → zero usage; post-completion → response usage
        assert!(
            matches!(&err.source, HarnessError::Provider(msg) if msg.contains("missing write_memory call"))
        );
        assert_eq!(err.usage, Usage::default());
    }

    #[tokio::test]
    async fn malformed_sections_is_a_distinct_error() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_memory".into(),
                input: serde_json::json!({ "sections": "not an object" }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 77, output_tokens: 11 },
        }]));
        let engine = ReflectionEngine::new(provider);
        let err = engine.reflect(&Memory::default(), &[], 999).await.unwrap_err();
        assert!(
            matches!(&err.source, HarnessError::Provider(msg) if msg.contains("malformed sections"))
        );
        // post-completion failure: usage from the completed response is preserved
        assert_eq!(err.usage, Usage { input_tokens: 77, output_tokens: 11 });
    }

    #[tokio::test]
    async fn empty_result_from_non_empty_memory_is_an_error() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({}),
        )]));
        let engine = ReflectionEngine::new(provider);
        let err = engine.reflect(&current_memory(), &[], 999).await.unwrap_err();
        assert!(
            matches!(&err.source, HarnessError::Provider(msg) if msg.contains("empty memory"))
        );
        // write_memory_response uses Usage { input_tokens: 100, output_tokens: 50 }
        assert_eq!(err.usage, Usage { input_tokens: 100, output_tokens: 50 });
    }

    #[tokio::test]
    async fn empty_result_from_empty_memory_is_ok() {
        // first-run case: nothing known, nothing learned
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({}),
        )]));
        let engine = ReflectionEngine::new(provider);
        let out = engine.reflect(&Memory::default(), &[], 999).await.unwrap();
        assert!(out.memory.sections.is_empty());
        assert_eq!(out.churn, 0.0);
    }
}

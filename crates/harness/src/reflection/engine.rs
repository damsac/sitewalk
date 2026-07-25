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
use crate::memory::{is_internal_section, FactSource, Memory, DEFAULT_WORD_CAP};

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
             preferences. Vocabulary terms are domain jargon that improve transcription \
             accuracy — preserve them verbatim and drop a vocabulary term only if it is \
             clearly a transcription artifact, not a real term. Call {} exactly once with \
             the full result.",
            self.word_cap, WRITE_MEMORY
        )
    }

    /// Renders memory like `to_prompt`, but marks user-corrected facts with
    /// ` [corrected]` so the model can honor their precedence (see
    /// `Memory::render` for the marker-masquerading threat model note).
    fn memory_block(&self, memory: &Memory) -> String {
        // "(empty)" keys off NON-internal content (Plan 15 D5-15): an
        // internal-only memory (just a `_seeds` marker) renders "(empty)"
        // rather than a spurious block, and real content always renders.
        if !has_non_internal_content(memory) {
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
                // Single-shot forced call — a cache write here would bill at
                // ~1.25x and never be read back.
                cache_prefix: false,
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
        // The FIFTH internal-section exclusion site (Plan 15 D5-15): the model
        // never saw internal sections (render skips them), so it can never echo
        // them back — carry them forward from `current` verbatim (full entries,
        // provenance and all) or every reflection would erase the `_seeds`
        // marker and user-deleted seeds would resurrect on the next re-seed.
        for (name, entries) in &current.sections {
            if is_internal_section(name) {
                memory.sections.insert(name.clone(), entries.clone());
            }
        }
        // A legit total wipe never happens; an empty write_memory result from a
        // confused model must not erase the user's memory. (Empty current with
        // an empty result stays OK — first-run case.) Compares NON-internal
        // sections only: a carried-over marker must not mask a genuine wipe,
        // and a legitimate `{}` over an internal-only memory must not error.
        if has_non_internal_content(current) && !has_non_internal_content(&memory) {
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

/// Whether `m` holds any non-internal, non-empty section — the "real content"
/// predicate shared by the empty-wipe guard and `memory_block` (Plan 15 D5-15).
fn has_non_internal_content(m: &Memory) -> bool {
    m.sections.iter().any(|(name, entries)| !is_internal_section(name) && !entries.is_empty())
}

/// (added + removed) / (old_count + new_count), 0.0 when both sides are empty.
/// Internal (`_`-prefixed) sections are excluded from both key sets so the
/// reflection carry-over never enters the churn signal (Plan 15 D5-15).
fn churn_between(old: &Memory, new: &Memory) -> f32 {
    let keys = |m: &Memory| -> BTreeSet<(String, String)> {
        m.sections
            .iter()
            .filter(|(s, _)| !is_internal_section(s))
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
            usage: Usage { input_tokens: 100, output_tokens: 50, ..Default::default() },
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
        assert_eq!(out.usage, Usage { input_tokens: 100, output_tokens: 50, ..Default::default() });

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
        // CONTENT failure (Text response, no write_memory) whose scripted usage
        // happens to be zero — not a provider failure
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
            usage: Usage { input_tokens: 77, output_tokens: 11, ..Default::default() },
        }]));
        let engine = ReflectionEngine::new(provider);
        let err = engine.reflect(&Memory::default(), &[], 999).await.unwrap_err();
        assert!(
            matches!(&err.source, HarnessError::Provider(msg) if msg.contains("malformed sections"))
        );
        // post-completion failure: usage from the completed response is preserved
        assert_eq!(err.usage, Usage { input_tokens: 77, output_tokens: 11, ..Default::default() });
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
        // write_memory_response uses Usage { input_tokens: 100, output_tokens: 50, ..Default::default() }
        assert_eq!(err.usage, Usage { input_tokens: 100, output_tokens: 50, ..Default::default() });
    }

    #[test]
    fn system_prompt_protects_vocabulary() {
        // Pins the D5 preservation GUIDANCE, not the mere word "vocabulary"
        // (which already appears in "Typical sections: ..." — review F1): the
        // distinctive phrase below exists only in the preserve-vocabulary
        // sentence, so this fails if that sentence is dropped.
        let engine = ReflectionEngine::new(std::sync::Arc::new(MockProvider::new(vec![])));
        let p = engine.system_prompt().to_lowercase();
        assert!(
            p.contains("drop a vocabulary term only if"),
            "reflection must carry the preserve-vocabulary guidance"
        );
    }

    // ---- Plan 15 D5-15: internal sections and reflection (the fifth site) ----

    fn seeded_current_memory() -> Memory {
        let mut m = Memory::default();
        m.add_vocabulary_term("french drain", 100, FactSource::Stated);
        m.remember("people", "Dev — framer", 200);
        m.mark_pack_seeded("landscape:1");
        m
    }

    #[tokio::test]
    async fn reflection_carries_internal_sections_forward() {
        // The model echoes only what it saw (vocabulary/people) — never _seeds.
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "vocabulary": ["french drain"], "people": ["Dev — framer"] }),
        )]));
        let engine = ReflectionEngine::new(provider.clone());
        let out = engine.reflect(&seeded_current_memory(), &[], 999).await.unwrap();

        // (a) marker preserved through the rebuild
        assert!(out.memory.is_pack_seeded("landscape:1"), "reflection must carry _seeds forward");
        // (b) the marker never leaks into the reflection prompt
        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text block")
        };
        assert!(!text.contains("_seeds"), "_seeds leaked into the reflection prompt");
        assert!(!text.contains("landscape:1"), "marker text leaked into the reflection prompt");
    }

    #[tokio::test]
    async fn internal_carry_over_does_not_distort_churn() {
        // old: {french drain, Dev}; new: {french drain, Sara} → churn 0.5 with
        // or without a _seeds marker present.
        let echo = serde_json::json!({
            "vocabulary": ["french drain"], "people": ["Sara — electrician"]
        });
        let with_marker = {
            let provider =
                Arc::new(MockProvider::new(vec![write_memory_response(echo.clone())]));
            let engine = ReflectionEngine::new(provider);
            engine.reflect(&seeded_current_memory(), &[], 999).await.unwrap().churn
        };
        let without_marker = {
            let mut current = seeded_current_memory();
            current.sections.remove("_seeds");
            let provider = Arc::new(MockProvider::new(vec![write_memory_response(echo)]));
            let engine = ReflectionEngine::new(provider);
            engine.reflect(&current, &[], 999).await.unwrap().churn
        };
        assert!((with_marker - without_marker).abs() < 1e-6, "carry-over distorted churn");
        assert!((with_marker - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn internal_only_memory_tolerates_empty_echo() {
        // An internal-only current (just _seeds) with a legitimate `{}` echo
        // must NOT trip the empty-wipe guard, and the marker must survive.
        let mut current = Memory::default();
        current.mark_pack_seeded("landscape:1");
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({}),
        )]));
        let engine = ReflectionEngine::new(provider.clone());
        let out = engine.reflect(&current, &[], 999).await.unwrap();
        assert!(out.memory.is_pack_seeded("landscape:1"));
        assert_eq!(out.churn, 0.0, "internal-only reflection has zero churn");
        // The prompt renders "(empty)" — not a spurious `## _seeds` block.
        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text block")
        };
        assert!(text.contains("(empty)"));
        assert!(!text.contains("_seeds"));
    }

    #[tokio::test]
    async fn empty_wipe_guard_still_fires_on_real_content_loss() {
        // The guard compares NON-internal sections: real content (people)
        // wiped to `{}` is still an error even though _seeds is carried over.
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({}),
        )]));
        let engine = ReflectionEngine::new(provider);
        let err = engine.reflect(&seeded_current_memory(), &[], 999).await.unwrap_err();
        assert!(
            matches!(&err.source, HarnessError::Provider(msg) if msg.contains("empty memory"))
        );
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

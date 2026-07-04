//! Synthetic site-walk corpus: paired fixture files on disk.
//!
//! Each scenario is two files sharing a stem in `fixtures/`:
//!   - `<id>.txt`  — the transcript, plain text (natural disfluency, trade jargon)
//!   - `<id>.json` — typed ground truth: what SHOULD be extracted, plus
//!     `distractors` (spans R6 says must NOT become items) and `expects_summary`.
//!
//! Ground truth is reviewed *with the plan* (see Task 5). Transcripts are text so
//! they read naturally; truth is JSON so it is typed and diffable.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::grade::MATCH_THRESHOLD;
use crate::normalize::{dice, token_set};

/// One expected item: the kind it should be filed under and its gist. Matching
/// is fuzzy (normalized-token Dice, see `normalize`/`grade`), so `text` is the
/// *canonical* phrasing — the grader tolerates STT/phrasing drift.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ExpectedItem {
    pub kind: String,
    pub text: String,
}

/// One expected contact. `trade` optional: absent means "any trade (or none) is
/// acceptable"; present means the model should have captured that role.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ExpectedContact {
    pub name: String,
    #[serde(default)]
    pub trade: Option<String>,
}

fn default_true() -> bool {
    true
}

/// The on-disk JSON shape (`<id>.json`). `description`/`tags` are metadata;
/// the rest is ground truth.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FixtureGroundTruth {
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub items: Vec<ExpectedItem>,
    #[serde(default)]
    pub contacts: Vec<ExpectedContact>,
    /// Content that R6 says must NOT become items (hedging, chatter, incomplete
    /// thoughts, social filler). Each string is matched against produced items;
    /// a hit is an R6 violation (a false positive on a distractor).
    #[serde(default)]
    pub distractors: Vec<String>,
    #[serde(default = "default_true")]
    pub expects_summary: bool,
}

/// A loaded scenario: transcript + ground truth, ready to run and grade.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub id: String,
    pub description: String,
    pub tags: Vec<String>,
    pub transcript: String,
    pub truth: GroundTruth,
}

/// Ground truth split out from fixture metadata for grading.
#[derive(Clone, Debug)]
pub struct GroundTruth {
    pub items: Vec<ExpectedItem>,
    pub contacts: Vec<ExpectedContact>,
    pub distractors: Vec<String>,
    pub expects_summary: bool,
}

/// Loads every `<id>.txt` + `<id>.json` pair from `dir`. A `.txt` with no
/// matching `.json` (or vice versa) is an error — a corpus authoring mistake
/// must be loud, not silently dropped. Order is sorted by id for stable,
/// comparable reports across runs.
pub fn load_corpus(dir: impl AsRef<Path>) -> io::Result<Vec<Scenario>> {
    let dir = dir.as_ref();
    // Collect stems and which extensions we saw.
    let mut txt: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    let mut json: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        match path.extension().and_then(|e| e.to_str()) {
            Some("txt") => {
                txt.insert(stem, path);
            }
            Some("json") => {
                json.insert(stem, path);
            }
            _ => {}
        }
    }
    let mut scenarios = Vec::new();
    for (stem, txt_path) in &txt {
        let json_path = json.remove(stem).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("fixture '{stem}' has {stem}.txt but no {stem}.json"),
            )
        })?;
        let transcript = fs::read_to_string(txt_path)?;
        let raw = fs::read_to_string(&json_path)?;
        let gt: FixtureGroundTruth = serde_json::from_str(&raw).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{stem}.json: {e}"))
        })?;
        let truth = GroundTruth {
            items: gt.items,
            contacts: gt.contacts,
            distractors: gt.distractors,
            expects_summary: gt.expects_summary,
        };
        validate_distractors_disjoint_from_items(stem, &truth)?;
        scenarios.push(Scenario {
            id: stem.clone(),
            description: gt.description,
            tags: gt.tags,
            transcript,
            truth,
        });
    }
    // Any leftover .json without a .txt is also an authoring error.
    if let Some((stem, _)) = json.into_iter().next() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("fixture '{stem}' has {stem}.json but no {stem}.txt"),
        ));
    }
    scenarios.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(scenarios)
}

/// Enforces the corpus invariant that no `distractor` Dice-matches (≥
/// `MATCH_THRESHOLD`) any expected item's text. `grade::grade` counts ANY
/// produced item that matches a distractor as an R6 false positive, with no
/// regard for whether that same item also matched (and scored as a true
/// positive for) an expected item — so an overlapping distractor would
/// silently penalize a model for correctly extracting a real item. Enforced
/// loudly at load time rather than left to corrupt scores quietly.
fn validate_distractors_disjoint_from_items(id: &str, truth: &GroundTruth) -> io::Result<()> {
    for d in &truth.distractors {
        let dt = token_set(d);
        for item in &truth.items {
            if dice(&dt, &token_set(&item.text)) >= MATCH_THRESHOLD {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "fixture '{id}': distractor {d:?} Dice-matches expected item {:?} — \
                         a correct extraction of that item would be wrongly counted as an R6 \
                         false positive",
                        item.text
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_truth_deserializes_with_defaults() {
        // contacts, distractors, tags default to empty; expects_summary defaults true
        let gt: FixtureGroundTruth =
            serde_json::from_str(r#"{"description":"x","items":[{"kind":"todo","text":"order lumber"}]}"#)
                .unwrap();
        assert_eq!(gt.items.len(), 1);
        assert!(gt.contacts.is_empty());
        assert!(gt.distractors.is_empty());
        assert!(gt.expects_summary, "defaults to true");
    }

    // These tests build synthetic fixtures in a temp dir — they exercise the
    // LOADER, not the real corpus (which Task 5 authors and asserts on). Keeping
    // Task 1 self-contained avoids a forward reference to files that don't exist
    // yet when the builder runs task-by-task.

    /// Writes a `<stem>.txt` + `<stem>.json` pair into `dir`.
    fn write_pair(dir: &std::path::Path, stem: &str, transcript: &str, json: &str) {
        std::fs::write(dir.join(format!("{stem}.txt")), transcript).unwrap();
        std::fs::write(dir.join(format!("{stem}.json")), json).unwrap();
    }

    fn fresh_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("evals-corpus-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_corpus_pairs_txt_and_json_by_stem() {
        let dir = fresh_dir("pairs");
        write_pair(&dir, "beta", "walking the back deck",
            r#"{"description":"d","items":[{"kind":"todo","text":"order lumber"}]}"#);
        write_pair(&dir, "alpha", "a short walk",
            r#"{"description":"d","items":[{"kind":"note","text":"soft joists"}]}"#);
        let scenarios = load_corpus(&dir).unwrap();
        let ids: Vec<&str> = scenarios.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "beta"], "sorted by id");
        // transcript is wired to its truth
        let beta = scenarios.iter().find(|s| s.id == "beta").unwrap();
        assert!(beta.transcript.contains("deck"));
        assert_eq!(beta.truth.items.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_corpus_is_deterministic_order() {
        let dir = fresh_dir("order");
        write_pair(&dir, "zulu", "t", r#"{"description":"d","items":[]}"#);
        write_pair(&dir, "alpha", "t", r#"{"description":"d","items":[]}"#);
        let a: Vec<String> = load_corpus(&dir).unwrap().into_iter().map(|s| s.id).collect();
        let b: Vec<String> = load_corpus(&dir).unwrap().into_iter().map(|s| s.id).collect();
        assert_eq!(a, b, "corpus load order must be stable for comparable reports");
        assert_eq!(a, vec!["alpha", "zulu"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_json_for_a_txt_is_an_error() {
        // a .txt with no matching .json is a corpus authoring bug, not silently skipped
        let dir = fresh_dir("orphan");
        std::fs::write(dir.join("orphan.txt"), "some transcript").unwrap();
        let err = load_corpus(&dir).unwrap_err();
        assert!(err.to_string().contains("orphan"), "names the offending fixture");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn distractor_overlapping_an_expected_item_is_a_load_error() {
        // A distractor that Dice-matches a real item would wrongly count a
        // correct extraction of that item as an R6 false positive — must be
        // rejected loudly at load time, not silently graded wrong.
        let dir = fresh_dir("overlapping-distractor");
        write_pair(
            &dir,
            "bad",
            "t",
            r#"{"description":"d","items":[{"kind":"todo","text":"order lumber for the deck"}],"distractors":["order some lumber"]}"#,
        );
        let err = load_corpus(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bad"), "names the offending fixture: {msg}");
        assert!(msg.contains("distractor"), "explains the invariant: {msg}");
        std::fs::remove_dir_all(&dir).ok();
    }
}

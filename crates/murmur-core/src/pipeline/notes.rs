//! Plan 14 D2/D5: the comprehensive-notes artifact. `NotesEntry` is the
//! core-side (plain serde) shape — `bucket` is a free-form string here
//! (tolerant of drift), mapped to the `NotesBucket` enum only at the FFI
//! boundary (`crates/ffi/src/notes.rs`). Persisted as a `kind="notes"`
//! artifact body `{"buckets":[…]}` (D5-14: no migration, no item columns).

/// The three entry buckets (D2-14). The top-level narrative `summary` is
/// NOT a bucket — it lives on `session.summary` (unchanged column).
pub const NOTE_BUCKETS: [&str; 3] = ["scope_of_work", "constraints", "conditions_and_issues"];

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NotesEntry {
    pub bucket: String,
    pub label: String,
    pub detail: String,
}

/// Tolerant parse of a notes artifact body (or a raw `notes` tool-call
/// value — both walk the same `{"buckets":[…]}`-shaped JSON). A missing or
/// unparseable body yields `[]`, never an error (R7: a garbled artifact
/// degrades to no notes, not a hard failure). Each candidate row is kept
/// only if it has all three required string fields AND `bucket` is one of
/// `NOTE_BUCKETS` — an unknown bucket string is dropped, not coerced (R6:
/// never fabricate a bucket).
pub fn parse_notes_artifact(body: &str) -> Vec<NotesEntry> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    parse_notes_value(&value)
}

/// Same tolerant walk as `parse_notes_artifact`, but starting from an
/// already-parsed `serde_json::Value` — shared by the artifact-body parser
/// and the `write_notes` tool-call `input.notes` parser (Task 2), so a
/// truncated/malformed tool response degrades identically to a garbled
/// stored artifact.
pub fn parse_notes_value(value: &serde_json::Value) -> Vec<NotesEntry> {
    let Some(buckets) = value.get("buckets").and_then(|b| b.as_array()) else {
        return Vec::new();
    };
    buckets
        .iter()
        .filter_map(|row| {
            let bucket = row.get("bucket")?.as_str()?;
            let label = row.get("label")?.as_str()?;
            let detail = row.get("detail")?.as_str()?;
            if !NOTE_BUCKETS.contains(&bucket) {
                return None;
            }
            Some(NotesEntry { bucket: bucket.to_string(), label: label.to_string(), detail: detail.to_string() })
        })
        .collect()
}

/// Serializes entries back to `{"buckets":[…]}` — round-trips through
/// `parse_notes_artifact`.
pub fn serialize_buckets(entries: &[NotesEntry]) -> String {
    serde_json::json!({ "buckets": entries }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(bucket: &str, label: &str, detail: &str) -> NotesEntry {
        NotesEntry { bucket: bucket.to_string(), label: label.to_string(), detail: detail.to_string() }
    }

    #[test]
    fn well_formed_body_round_trips() {
        let entries = vec![
            entry("scope_of_work", "Mulch", "Darker mulch than last year."),
            entry("constraints", "Budget", "Keep it under $1,200."),
        ];
        let body = serialize_buckets(&entries);
        assert_eq!(parse_notes_artifact(&body), entries);
    }

    #[test]
    fn unknown_bucket_is_dropped() {
        let body = serde_json::json!({
            "buckets": [
                {"bucket": "logistics", "label": "x", "detail": "y"},
                {"bucket": "scope_of_work", "label": "Mulch", "detail": "…"}
            ]
        })
        .to_string();
        let parsed = parse_notes_artifact(&body);
        assert_eq!(parsed, vec![entry("scope_of_work", "Mulch", "…")]);
    }

    #[test]
    fn malformed_entry_is_skipped() {
        let body = serde_json::json!({
            "buckets": [
                {"bucket": "constraints", "label": "Budget"},
                {"bucket": "constraints", "detail": "no label"},
                "not even an object",
                {"bucket": "scope_of_work", "label": "Mulch", "detail": "ok"}
            ]
        })
        .to_string();
        let parsed = parse_notes_artifact(&body);
        assert_eq!(parsed, vec![entry("scope_of_work", "Mulch", "ok")]);
    }

    #[test]
    fn empty_or_absent_body_is_empty() {
        assert_eq!(parse_notes_artifact(""), Vec::new());
        assert_eq!(parse_notes_artifact("not json"), Vec::new());
        assert_eq!(parse_notes_artifact("{}"), Vec::new());
        assert_eq!(parse_notes_artifact(r#"{"buckets":[]}"#), Vec::new());
    }
}

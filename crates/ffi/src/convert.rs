//! Domain -> FFI dictionary projections. Every mapping across the FFI
//! boundary lives here (plus the records themselves) — nowhere else.

use std::collections::HashMap;

use murmur_core::{Artifact, CapturedItem};

use crate::document::{DocField, DocLine, DocumentPayload};
use crate::events::BoardItem;
use crate::notes::{NotesBucket, NotesEntry};

/// `CapturedItem` -> `BoardItem`. `right` projects the core quantity/unit
/// string (`items.right_text`, Plan 16 D2-16) — `""` for un-edited items,
/// exactly the pre-Plan-16 stub behavior. `photo_count` is looked
/// up from a batched per-session map (`Store::count_live_photos_by_item_for_session`,
/// the photo_count fast-follow) — a missing key (no live photos attached to
/// this item) defaults to `0`. Callers load the map ONCE per snapshot, never
/// per item (no N+1 query per board item).
pub fn board_item(item: &CapturedItem, photo_counts: &HashMap<String, u32>) -> BoardItem {
    BoardItem {
        id: item.id.clone(),
        kind: item.kind.clone(),
        text: item.text.clone(),
        right: item.right.clone(),
        photo_count: photo_counts.get(&item.id).copied().unwrap_or(0),
    }
}

/// Plan 14 D2-14: core-side `NotesEntry` (string `bucket`) -> FFI
/// `NotesEntry` (enum `bucket`). A row whose `bucket` string isn't one of
/// the three known variants is DROPPED (R6: never fabricate/coerce a
/// bucket) — Swift only ever sees valid variants, so its exhaustive
/// `switch` is safe.
pub fn notes_entries(core: &[murmur_core::NotesEntry]) -> Vec<NotesEntry> {
    core.iter()
        .filter_map(|e| {
            let bucket = NotesBucket::from_wire(&e.bucket)?;
            Some(NotesEntry { bucket, label: e.label.clone(), detail: e.detail.clone() })
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("artifact body is not valid document JSON: {0}")]
    BadJson(String),
}

/// A `document`-kind `Artifact`'s JSON body -> `DocumentPayload` (Plan 07 D2:
/// the structured document is stored as an `Artifact` with `kind = "document"`
/// and a JSON body — no new domain type, no migration).
pub fn document_payload(artifact: &Artifact) -> Result<DocumentPayload, ConvertError> {
    let v: serde_json::Value =
        serde_json::from_str(&artifact.body).map_err(|e| ConvertError::BadJson(e.to_string()))?;

    let lines = v
        .get("lines")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .map(|line| DocLine {
                    id: line.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    title: line.get("title").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    detail: line.get("detail").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    qty: line.get("qty").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    amount_cents: line.get("amount_cents").and_then(|x| x.as_i64()),
                    section: line.get("section").and_then(|x| x.as_str()).map(str::to_string),
                    is_gap: line.get("is_gap").and_then(|x| x.as_bool()).unwrap_or(false),
                    item_id: line.get("item_id").and_then(|x| x.as_str()).map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();

    // Plan 19 additive keys, decoded tolerantly like everything else: a
    // pre-Plan-19 body has neither — `number_prefix: None`, `fields: []`.
    let fields = v
        .get("fields")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .map(|f| DocField {
                    section_key: f.get("section_key").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    key: f.get("key").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    label: f.get("label").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    kind: f.get("kind").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    fill: f.get("fill").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    value: f.get("value").and_then(|x| x.as_str()).map(str::to_string),
                    is_gap: f.get("is_gap").and_then(|x| x.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(DocumentPayload {
        doc_kind: v.get("doc_kind").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        doc_number: v.get("doc_number").and_then(|x| x.as_u64()).unwrap_or_default(),
        job_date_unix: v.get("job_date_unix").and_then(|x| x.as_u64()).unwrap_or_default(),
        total_kind: v.get("total_kind").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        total_label_key: v.get("total_label_key").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        static_total_cents: v.get("static_total_cents").and_then(|x| x.as_i64()),
        lines,
        queued: v.get("queued").and_then(|x| x.as_bool()).unwrap_or(false),
        number_prefix: v.get("number_prefix").and_then(|x| x.as_str()).map(str::to_string),
        fields,
    })
}

#[cfg(test)]
mod tests {
    use murmur_core::{ItemSource, Store};

    use super::*;

    #[test]
    fn captured_item_projects_to_board_item() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let item = store
            .add_item_with_source(&session.id, "todo", "order lumber", ItemSource::Live)
            .unwrap();
        let board = board_item(&item, &HashMap::new());
        assert_eq!(board.id, item.id);
        assert_eq!(board.kind, "todo");
        assert_eq!(board.text, "order lumber");
        assert_eq!(board.right, "", "an un-edited item projects the empty core right");
        assert_eq!(board.photo_count, 0, "no entry in the counts map defaults to 0");

        // Plan 16 D2-16: `right` now projects the core quantity string.
        store.update_item(&item.id, None, None, Some("3 CU YD")).unwrap();
        let item = store.get_item(&item.id).unwrap();
        assert_eq!(board_item(&item, &HashMap::new()).right, "3 CU YD");
    }

    #[test]
    fn board_item_photo_count_comes_from_the_batched_map() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let item = store
            .add_item_with_source(&session.id, "todo", "order lumber", ItemSource::Live)
            .unwrap();
        store.add_photo(&session.id, Some(&item.id), "a.jpg", None).unwrap();
        store.add_photo(&session.id, Some(&item.id), "b.jpg", None).unwrap();
        let counts = store.count_live_photos_by_item_for_session(&session.id).unwrap();
        let board = board_item(&item, &counts);
        assert_eq!(board.photo_count, 2);
    }

    #[test]
    fn document_artifact_parses_with_a_gap_line() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let body = serde_json::json!({
            "doc_kind": "estimate",
            "doc_number": 47,
            "job_date_unix": 1000,
            "total_kind": "sum",
            "total_label_key": "total",
            "static_total_cents": null,
            "lines": [
                {"id": "l1", "title": "Mulch", "detail": "", "qty": "3 CU YD", "amount_cents": 28500, "section": null, "is_gap": false},
                {"id": "l2", "title": "Haul", "detail": "", "qty": "", "amount_cents": null, "section": null, "is_gap": true}
            ],
            "queued": false,
        });
        let artifact = store
            .add_artifact(&session.id, "document", "estimate #47", &body.to_string())
            .unwrap();
        let payload = document_payload(&artifact).unwrap();
        assert_eq!(payload.doc_number, 47);
        assert_eq!(payload.lines.len(), 2);
        assert_eq!(payload.lines[1].amount_cents, None);
        assert!(payload.lines[1].is_gap);
    }

    #[test]
    fn document_line_carries_item_id_when_present_and_none_when_absent() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let body = serde_json::json!({
            "doc_kind":"estimate","doc_number":47,"job_date_unix":1000,
            "total_kind":"sum","total_label_key":"total","static_total_cents":null,
            "lines":[
                {"id":"l1","title":"Mulch","detail":"","qty":"","amount_cents":28500,"section":null,"is_gap":false,"item_id":"item-A1"},
                {"id":"l2","title":"Subtotal","detail":"","qty":"","amount_cents":90600,"section":null,"is_gap":false}
            ],
            "queued":false
        });
        let art = store.add_artifact(&session.id, "document", "estimate #47", &body.to_string()).unwrap();
        let payload = document_payload(&art).unwrap();
        assert_eq!(payload.lines[0].item_id.as_deref(), Some("item-A1"));
        assert_eq!(payload.lines[1].item_id, None, "a line with no item_id parses to None");
    }

    #[test]
    fn pre_plan12_document_body_parses_all_item_ids_as_none() {
        // A body written before Plan 12 (no item_id on any line) renders unchanged.
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let body = serde_json::json!({
            "doc_kind":"estimate","doc_number":1,"job_date_unix":0,
            "total_kind":"sum","total_label_key":"total","static_total_cents":null,
            "lines":[{"id":"l1","title":"Mulch","detail":"","qty":"","amount_cents":100,"section":null,"is_gap":false}],
            "queued":false
        });
        let art = store.add_artifact(&session.id, "document", "estimate #1", &body.to_string()).unwrap();
        assert_eq!(document_payload(&art).unwrap().lines[0].item_id, None);
    }

    #[test]
    fn document_payload_decodes_number_prefix_and_fields() {
        // Plan 19: the two additive body keys round-trip through the decoder.
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let body = serde_json::json!({
            "doc_kind":"hoa_addendum","doc_number":1,"job_date_unix":1000,
            "total_kind":"sum","total_label_key":"total","static_total_cents":null,
            "lines":[], "queued":false, "number_prefix":"HOA",
            "fields":[
                {"section_key":"approvals","key":"hoa_no","label":"HOA approval #",
                 "kind":"text","fill":"walk","value":"41827","is_gap":false},
                {"section_key":"approvals","key":"reviewed_by","label":"Reviewed by",
                 "kind":"text","fill":"walk","value":null,"is_gap":true}
            ]
        });
        let art = store.add_artifact(&session.id, "document", "hoa #1", &body.to_string()).unwrap();
        let payload = document_payload(&art).unwrap();
        assert_eq!(payload.number_prefix.as_deref(), Some("HOA"));
        assert_eq!(payload.fields.len(), 2);
        assert_eq!(payload.fields[0].value.as_deref(), Some("41827"));
        assert!(!payload.fields[0].is_gap);
        assert_eq!(payload.fields[1].value, None);
        assert!(payload.fields[1].is_gap, "the gap row survives the decode");
        assert_eq!(payload.fields[1].section_key, "approvals");
        assert_eq!(payload.fields[1].fill, "walk");
    }

    #[test]
    fn pre_plan19_body_defaults_number_prefix_none_and_fields_empty() {
        // A body written before Plan 19 (no number_prefix/fields) renders
        // unchanged — the additive-keys precedent (Plan 12 item_id).
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let body = serde_json::json!({
            "doc_kind":"estimate","doc_number":1,"job_date_unix":0,
            "total_kind":"sum","total_label_key":"total","static_total_cents":null,
            "lines":[], "queued":false
        });
        let art = store.add_artifact(&session.id, "document", "estimate #1", &body.to_string()).unwrap();
        let payload = document_payload(&art).unwrap();
        assert_eq!(payload.number_prefix, None);
        assert!(payload.fields.is_empty());
    }

    #[test]
    fn bad_json_body_is_an_error() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let artifact = store.add_artifact(&session.id, "document", "x", "not json").unwrap();
        assert!(document_payload(&artifact).is_err());
    }
}

//! `NotesPayload` (Plan 13 D2): the exact `finish()` record after the Stage 2
//! flip. A walk's finish output is items + summary — the document build moves
//! to the on-demand, engine-keyed `build_document(kind)` (Stage 1, additive).

use murmur_core::CapturedItem;

use crate::convert;
use crate::events::BoardItem;

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct NotesPayload {
    pub session_id: String,
    /// The template's DEFAULT kind (`doc_kind_for_template`) — advisory only,
    /// for button curation. Swift's button wiring keys off the client-known
    /// template (D2), never off this field.
    pub doc_kind: String,
    /// `session.summary`; `"(empty session)"` for a silent walk.
    pub summary: String,
    /// The authoritative+manual board post-swap, with batched photo_count
    /// (reuses `BoardItem` — no new item record).
    pub items: Vec<BoardItem>,
    /// `true` when `finish()` degraded offline (D9) — the session did NOT
    /// reach `Processed`; the client disables build-document buttons.
    pub queued: bool,
}

/// D3: builds a `NotesPayload` from a session's items — shared by the happy
/// path and every degrade branch (empty transcript, offline, double-finish).
pub(crate) fn notes_payload(
    session_id: &str,
    doc_kind: &str,
    summary: &str,
    items: &[CapturedItem],
    photo_counts: &std::collections::HashMap<String, u32>,
    queued: bool,
) -> NotesPayload {
    NotesPayload {
        session_id: session_id.to_string(),
        doc_kind: doc_kind.to_string(),
        summary: summary.to_string(),
        items: items.iter().map(|item| convert::board_item(item, photo_counts)).collect(),
        queued,
    }
}

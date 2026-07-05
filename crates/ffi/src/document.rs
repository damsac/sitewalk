//! FFI-facing document projections (Plan 07 D2). Display-copy-free structured
//! data: cents, unix seconds, integer doc number, label *keys* — the Swift
//! bridge (`MurmurEngine`) owns currency/date formatting and letterhead/board
//! chrome. Core never carries pre-formatted UI copy.

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DocLine {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub qty: String,
    pub amount_cents: Option<i64>,
    pub section: Option<String>,
    /// Template-aware (D2a) — set by the `build_document` tool, never derived
    /// by the FFI layer from `amount_cents == None`.
    pub is_gap: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DocumentPayload {
    pub doc_kind: String,
    pub doc_number: u64,
    pub job_date_unix: u64,
    pub total_kind: String,
    pub total_label_key: String,
    pub static_total_cents: Option<i64>,
    pub lines: Vec<DocLine>,
    /// True when `finish()` degraded offline (D9) — a partial document built
    /// from live items, all gaps, capture never lost.
    pub queued: bool,
}

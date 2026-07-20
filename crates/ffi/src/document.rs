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
    /// The core item this row was built from (Plan 12). `None` for
    /// total/rollup lines, or an old document body written before Plan 12.
    /// Additive; never derived by the FFI layer.
    pub item_id: Option<String>,
}

/// One authored `filled`/`static` schema field of a built document (Plan 19
/// Stage 5). `value: None, is_gap: true` is a truthful gap — either the
/// model declined the field (R6) or it is `manual` (operator completes at
/// review; always a gap in v1).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DocField {
    pub section_key: String,
    pub key: String,
    pub label: String,
    pub kind: String,
    pub fill: String,
    pub value: Option<String>,
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
    /// True when a model call this build needed didn't run to completion —
    /// originally the offline-degrade flag (D9), then the pricing degrade
    /// (Plan 13 D5), now also the fill-pass degrade (Plan 19). One meaning:
    /// regenerate to retry.
    pub queued: bool,
    /// The resolved schema row's numbering prefix ("EST", "HOA") — additive
    /// (Plan 19). `None` for a pre-Plan-19 document body. Swift's
    /// `docNumberLabel` still renders built-ins from its own switch today;
    /// consuming this field is sac's editor-milestone follow-up (WE-C).
    pub number_prefix: Option<String>,
    /// Authored schema fields — additive (Plan 19). Empty for built-ins and
    /// pre-Plan-19 bodies.
    pub fields: Vec<DocField>,
}

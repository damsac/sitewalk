//! Domain entities (spec §2, Rev 2 §3: Job is first-class; artifacts are a seam).
//! Plain serde data — these types cross the FFI boundary in Plan 07.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Done,
    Archived,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Active => "active",
            JobStatus::Done => "done",
            JobStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "active" => Ok(JobStatus::Active),
            "done" => Ok(JobStatus::Done),
            "archived" => Ok(JobStatus::Archived),
            other => Err(CoreError::Corrupt(format!("unknown job status: {other}"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Audio/transcript still coming in.
    Recording,
    /// Ended; queued for the processing pipeline (Plan 04). Offline-safe.
    AwaitingProcessing,
    /// Pipeline finished; summary and artifacts exist.
    Processed,
    /// Pipeline failed; retryable.
    Failed,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Recording => "recording",
            SessionStatus::AwaitingProcessing => "awaiting_processing",
            SessionStatus::Processed => "processed",
            SessionStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "recording" => Ok(SessionStatus::Recording),
            "awaiting_processing" => Ok(SessionStatus::AwaitingProcessing),
            "processed" => Ok(SessionStatus::Processed),
            "failed" => Ok(SessionStatus::Failed),
            other => Err(CoreError::Corrupt(format!("unknown session status: {other}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub client: Option<String>,
    pub site: Option<String>,
    /// Unix seconds; None = unscheduled/backlog.
    pub scheduled_at: Option<u64>,
    pub status: JobStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct NewJob {
    pub name: String,
    pub client: Option<String>,
    pub site: Option<String>,
    pub scheduled_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub job_id: Option<String>,
    /// Template key selecting extraction vocabulary + document layout
    /// (`landscape` | `property` | `inspection`). Persisted on the session
    /// (Plan 07 D4) so reprocessing stays template-consistent; `None` before
    /// `set_session_template` is called or for pre-migration sessions.
    pub template: Option<String>,
    pub status: SessionStatus,
    pub transcript: String,
    /// Filled by the processing pipeline (Plan 04); also feeds reflection activity.
    pub summary: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

/// Where a captured item came from. Drives the end-of-session swap
/// (`Store::finish_session_processed`): `live` items and *prior-run*
/// `authoritative` items are tombstoned when a new authoritative pass lands;
/// `manual` items are never swept by processing. Free of a migration for new
/// values would be nice, but the swap logic depends on the closed set — keep it
/// closed and parse defensively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemSource {
    /// Written by a live in-session pass (Plan 05). Provisional; swept on the
    /// next successful process().
    Live,
    /// Written by an end-of-session processing run (Plan 04). The source of
    /// truth once its run finishes.
    Authoritative,
    /// User-entered (story 10 parity) or a direct `add_item`. Never swept by
    /// processing; only a full session delete removes it.
    Manual,
}

impl ItemSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemSource::Live => "live",
            ItemSource::Authoritative => "authoritative",
            ItemSource::Manual => "manual",
        }
    }
    pub fn parse(raw: &str) -> Result<Self, crate::error::CoreError> {
        match raw {
            "live" => Ok(ItemSource::Live),
            "authoritative" => Ok(ItemSource::Authoritative),
            "manual" => Ok(ItemSource::Manual),
            other => Err(crate::error::CoreError::Corrupt(format!(
                "unknown item source: {other}"
            ))),
        }
    }
}

/// The item-kind allowlist (Plan 16 Task 2, the Plan 15 SEED_MAX
/// "don't fork the constant" discipline). Item `kind` stays a free `String`
/// at the domain/DB layer by design (new kinds must not require a
/// migration) — this const is the VALIDATION boundary, shared by the agent
/// `AddItemTool` (both its `execute` check and its advertised JSON-schema
/// enum) and the FFI edit seam (`update_item`/`add_item`), so the two can't
/// drift (R6: reject unknown kinds, never store them).
pub const VALID_ITEM_KINDS: [&str; 6] = ["todo", "decision", "note", "safety", "part", "price"];

/// A typed item extracted from (or manually added to) a session.
/// `kind` is a free string by design — conventions: "todo", "decision",
/// "note", "safety", "part", "price". New kinds must not require a migration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapturedItem {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub text: String,
    /// Quantity/unit display string ("3 CU YD", "× 4") — NOT price (pricing
    /// stays a document-build concern, keeper D-#2). Free-form by design
    /// (R6: coercing it would fabricate). Defaults `""` = "no quantity".
    /// Backed by the `items.right_text` column (Plan 16; `RIGHT` is a SQL
    /// keyword, hence the column rename — the field matches `BoardItem.right`).
    pub right: String,
    pub source: ItemSource,
    pub done: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub trade: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

/// The artifact seam (Rev 2 §1): generated documents of any kind hang off a
/// session. `kind` is a free string ("report", "estimate", …); generators
/// register in Plan 04. `body` is markdown (or JSON for structured kinds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

/// A user-captured photo attached to a session (spec: photos never leave the
/// device — only this METADATA row is sync-ready; the BYTES live in the shell's
/// Documents dir, local-only forever, Plan 11 D4). `item_id` is an optional
/// attachment to a specific captured item; it is demoted to `None` if that item
/// is swept (Plan 11 D3), so a live photo never references a tombstoned item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: String,
    pub session_id: String,
    pub item_id: Option<String>,
    /// Shell-owned, opaque to core: a relative filename in `<Documents>/photos/`.
    pub filename: String,
    pub captured_at: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

/// The section-kind allowlist for `DocumentSchema` sections (Plan 19 §3).
/// R6 validation boundary — shared by `Store::save_document_schema` and the
/// FFI seam so they can't drift. Exactly one `line_items` section per schema
/// in v1; `static`/`filled` carry authored fields.
pub const VALID_SECTION_KINDS: [&str; 3] = ["line_items", "static", "filled"];

/// The field-kind allowlist for `DocumentSchema` fields (Plan 19 §3).
pub const VALID_FIELD_KINDS: [&str; 7] =
    ["line_items", "text", "long_text", "currency", "quantity", "date", "static"];

/// The fill-kind allowlist for `DocumentSchema` fields (Plan 19 §3):
/// `walk` = the one focused build-time fill pass, `manual` = operator
/// completes at review (always a gap in v1), `static` = authored constant.
pub const VALID_FILL_KINDS: [&str; 3] = ["walk", "manual", "static"];

/// Fixed built-in schema ids (Plan 19 §3): UUIDv7-shaped constants (version
/// nibble 7, variant 8) so they sort FIRST in any UUIDv7-ordered list and
/// read as built-in. Identical on every device — together with the sentinel
/// `device_id` ("builtin") they form the stable sync merge key.
pub const BUILTIN_SCHEMA_ID_ESTIMATE: &str = "00000000-0000-7000-8000-000000000001";
pub const BUILTIN_SCHEMA_ID_INVOICE: &str = "00000000-0000-7000-8000-000000000002";
pub const BUILTIN_SCHEMA_ID_WORK_ORDER: &str = "00000000-0000-7000-8000-000000000003";
pub const BUILTIN_SCHEMA_ID_CONDITION: &str = "00000000-0000-7000-8000-000000000004";
pub const BUILTIN_SCHEMA_ID_MOVE_OUT: &str = "00000000-0000-7000-8000-000000000005";
pub const BUILTIN_SCHEMA_ID_INSPECTION: &str = "00000000-0000-7000-8000-000000000006";
pub const BUILTIN_SCHEMA_ID_REPORT: &str = "00000000-0000-7000-8000-000000000007";

/// Sentinel `device_id` on every seeded built-in row — identical on every
/// device so two devices converge on "the same built-in" instead of
/// duplicating it (Plan 19 Stage 1).
pub const BUILTIN_SCHEMA_DEVICE_ID: &str = "builtin";

/// One field of a document-schema section (Plan 19 §3). `kind`/`fill` are
/// free `String`s at the domain layer by design (new kinds must not require
/// a migration); the `VALID_*` consts are the validation boundary (R6:
/// reject unknowns at save, never coerce at build).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SchemaField {
    pub key: String,
    /// One of `VALID_FIELD_KINDS`.
    pub kind: String,
    pub label: String,
    /// One of `VALID_FILL_KINDS`.
    pub fill: String,
    /// The authored constant for `fill == "static"` fields; `None` otherwise.
    #[serde(default)]
    pub static_value: Option<String>,
}

/// One ordered section of a document schema (Plan 19 §3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SchemaSection {
    pub key: String,
    /// One of `VALID_SECTION_KINDS`.
    pub kind: String,
    pub label: String,
    /// `line_items` sections only: whether the build runs the pricing pass.
    #[serde(default)]
    pub priced: bool,
    #[serde(default)]
    pub fields: Vec<SchemaField>,
}

/// A document type as DATA (Plan 19): an ordered list of named sections plus
/// the total shape and numbering prefix. Row-shaped exactly like `items`
/// (sync-ready: created_at/updated_at/device_id, tombstones); the structural
/// part persists as a JSON envelope column (`document_schemas.sections`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentSchema {
    /// UUIDv7 (custom) or a FIXED built-in id (`BUILTIN_SCHEMA_ID_*`).
    pub id: String,
    /// "estimate" | "hoa_addendum" | …
    pub kind: String,
    /// "Estimate", "HOA Addendum" — display copy seed (Swift owns final copy).
    pub label: String,
    /// "EST", "HOA" — core mints `<prefix>-NNNN` (the integer; Swift renders).
    pub number_prefix: String,
    /// "landscape" | "property" | "inspection" | `None` (template-agnostic).
    pub trade_key: Option<String>,
    /// "sum" | "static" — envelope `total_kind`.
    pub total_kind: String,
    /// Free label key ("total", "findings") — Swift owns the copy.
    pub total_label_key: String,
    pub sections: Vec<SchemaSection>,
    /// Shape version of the `sections` JSON envelope (starts at 1).
    pub schema_version: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

fn builtin_schema(
    id: &str,
    kind: &str,
    label: &str,
    number_prefix: &str,
    trade_key: Option<&str>,
    priced: bool,
    (total_kind, total_label_key): (&str, &str),
) -> DocumentSchema {
    DocumentSchema {
        id: id.to_string(),
        kind: kind.to_string(),
        label: label.to_string(),
        number_prefix: number_prefix.to_string(),
        trade_key: trade_key.map(str::to_string),
        total_kind: total_kind.to_string(),
        total_label_key: total_label_key.to_string(),
        sections: vec![SchemaSection {
            key: "line_items".into(),
            kind: "line_items".into(),
            label: "Items".into(),
            priced,
            fields: vec![],
        }],
        schema_version: 1,
        // Fixed literals (Plan 19 Stage 1): a seeded row is byte-identical
        // on every device.
        created_at: 0,
        updated_at: 0,
        device_id: BUILTIN_SCHEMA_DEVICE_ID.to_string(),
    }
}

/// The ONE source of truth for the seeded built-ins (Plan 19 §3's table).
/// `seed_builtin_schemas` iterates this — never an inline-SQL duplicate.
/// `priced`/`total_*` per built-in reproduce `is_pricing_kind`/`total_shape`
/// EXACTLY (pinned by `builtin_schemas_reproduce_todays_pricing_and_total_shape`).
pub fn builtin_schemas() -> Vec<DocumentSchema> {
    vec![
        builtin_schema(BUILTIN_SCHEMA_ID_ESTIMATE, "estimate", "Estimate", "EST", Some("landscape"), true, ("sum", "total")),
        builtin_schema(BUILTIN_SCHEMA_ID_INVOICE, "invoice", "Invoice", "INV", Some("landscape"), true, ("sum", "total")),
        builtin_schema(BUILTIN_SCHEMA_ID_WORK_ORDER, "work_order", "Work Order", "WO", Some("landscape"), false, ("sum", "total")),
        builtin_schema(BUILTIN_SCHEMA_ID_CONDITION, "condition", "Condition Report", "COND", Some("property"), false, ("sum", "total")),
        builtin_schema(BUILTIN_SCHEMA_ID_MOVE_OUT, "move_out", "Move-Out Report", "MO", Some("property"), false, ("sum", "total")),
        builtin_schema(BUILTIN_SCHEMA_ID_INSPECTION, "inspection", "Inspection Report", "IR", Some("inspection"), false, ("static", "findings")),
        builtin_schema(BUILTIN_SCHEMA_ID_REPORT, "report", "Report", "DOC", None, false, ("sum", "total")),
    ]
}

/// One LLM call's cost record (R9). Append-only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmUsageRow {
    pub id: String,
    pub session_id: Option<String>,
    /// What the tokens bought: "processing" (extraction agent + summary call are
    /// folded into a single row per session by design), "reflection", or future
    /// pipeline phases. "summary" never appears as a standalone purpose.
    pub purpose: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub created_at: u64,
    pub device_id: String,
}

/// Transcript-free projection for lists and queue polling (Plan 03 review:
/// full `Session` structs carry 50-100KB transcripts; lists must not).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub job_id: Option<String>,
    pub status: SessionStatus,
    pub summary: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub transcript_chars: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_source_round_trips_through_str() {
        for s in [ItemSource::Live, ItemSource::Authoritative, ItemSource::Manual] {
            assert_eq!(ItemSource::parse(s.as_str()).unwrap(), s);
        }
        assert!(ItemSource::parse("bogus").is_err());
    }
}

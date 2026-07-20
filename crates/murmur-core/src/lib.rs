pub mod coordinator;
pub mod domain;
pub mod error;
pub mod ids;
pub mod pipeline;
pub mod reflection;
pub mod store;

pub use coordinator::ReflectionCoordinator;
pub use domain::{
    builtin_schemas, Artifact, CapturedItem, Contact, DocumentSchema, Job, JobStatus, ItemSource,
    LlmUsageRow, NewJob, Photo, SchemaField, SchemaSection, Session, SessionStatus,
    SessionSummary, BUILTIN_SCHEMA_DEVICE_ID, BUILTIN_SCHEMA_ID_CONDITION,
    BUILTIN_SCHEMA_ID_ESTIMATE, BUILTIN_SCHEMA_ID_INSPECTION, BUILTIN_SCHEMA_ID_INVOICE,
    BUILTIN_SCHEMA_ID_MOVE_OUT, BUILTIN_SCHEMA_ID_REPORT, BUILTIN_SCHEMA_ID_WORK_ORDER,
    VALID_FIELD_KINDS, VALID_FILL_KINDS, VALID_ITEM_KINDS, VALID_SECTION_KINDS,
};
pub use error::CoreError;
pub use ids::new_id;
pub use pipeline::document::{BuildDocumentOutcome, DocumentBuilder};
pub use pipeline::live::{LiveExtractOutcome, LiveExtractor};
pub use pipeline::notes::{parse_notes_artifact, NotesEntry};
pub use pipeline::{
    doc_kind_for_template, doc_kinds_for_template, is_pricing_kind, total_shape, ProcessOutcome,
    SessionProcessor,
};
pub use pipeline::tools::{AddItemTool, BuildDocumentTool, UpsertContactTool, WriteReportTool};
pub use store::Store;

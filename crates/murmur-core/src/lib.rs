pub mod coordinator;
pub mod domain;
pub mod error;
pub mod ids;
pub mod pipeline;
pub mod reflection;
pub mod store;

pub use coordinator::ReflectionCoordinator;
pub use domain::{
    Artifact, CapturedItem, Contact, Job, JobStatus, ItemSource, LlmUsageRow, NewJob, Photo,
    Session, SessionStatus, SessionSummary,
};
pub use error::CoreError;
pub use ids::new_id;
pub use pipeline::document::{BuildDocumentOutcome, DocumentBuilder};
pub use pipeline::live::{LiveExtractOutcome, LiveExtractor};
pub use pipeline::notes::{parse_notes_artifact, NotesEntry};
pub use pipeline::{
    doc_kind_for_template, doc_kinds_for_template, is_pricing_kind, ProcessOutcome,
    SessionProcessor,
};
pub use pipeline::tools::{AddItemTool, BuildDocumentTool, UpsertContactTool, WriteReportTool};
pub use store::Store;

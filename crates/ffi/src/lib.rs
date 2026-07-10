//! The FFI bridge (Plan 07): a UniFFI-facing crate wrapping `murmur-core`
//! behind the `WalkEngine` contract sac's iOS app expects. `murmur-core`
//! stays UniFFI-free (D1) — every binding-generator dependency lives here.
//! Proc-macro mode only: no build.rs, no UDL.

uniffi::setup_scaffolding!();

pub mod convert;
pub mod document;
pub mod document_build;
pub mod engine;
pub mod events;
pub mod notes;
pub mod photos;
pub mod session;
pub mod vocabulary;

pub use convert::{document_payload, partial_document_from_items};
pub use document::{DocLine, DocumentPayload};
pub use engine::{EngineConfig, EngineError, MurmurEngine, Providers};
pub use events::{BoardItem, WalkEvent, WalkEventListener};
pub use notes::NotesPayload;
pub use photos::PhotoRef;
pub use session::WalkSession;

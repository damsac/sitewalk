use thiserror::Error;

/// Errors from the murmur-core domain layer.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("corrupt row: {0}")]
    Corrupt(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("agent error: {0}")]
    Agent(#[from] harness::HarnessError),
}

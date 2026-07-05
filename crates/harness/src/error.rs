#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    /// Provider errors are stringified at the boundary deliberately: these
    /// errors will cross an FFI boundary later, where source chains don't travel.
    #[error("provider error: {0}")]
    Provider(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool '{name}' failed: {message}")]
    Tool { name: String, message: String },
    #[error("agent exceeded max turns ({0})")]
    MaxTurns(usize),
}

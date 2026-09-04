//! Error types for the intel adapter.

use thiserror::Error;

/// Adapter-level errors.
#[derive(Debug, Error)]
pub enum AdapterError {
    /// Configuration is missing or invalid.
    #[error("config: {0}")]
    Config(String),

    /// JSON-RPC / ACP protocol error.
    #[error("protocol: {0}")]
    Protocol(String),

    /// Intelligence gateway HTTP or SSE error.
    #[error("intel: {0}")]
    Intel(String),

    /// Intel gateway rejected credentials.
    #[error("intel auth: {0}")]
    IntelAuth(String),

    /// Session expired or stopped on the intel side (404/409).
    #[error("intel session gone: {0}")]
    IntelSessionGone(String),

    /// Buzz relay publish / query error.
    #[error("relay: {0}")]
    Relay(String),

    /// I/O error (state file, stdin, etc.).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// JSON (de)serialization failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Turn was cancelled via session/cancel.
    #[error("cancelled")]
    Cancelled,
}

impl AdapterError {
    /// JSON-RPC error code for ACP responses.
    pub fn json_rpc_code(&self) -> i32 {
        match self {
            Self::Config(_) | Self::Protocol(_) => -32602,
            Self::Cancelled => -32000,
            _ => -32000,
        }
    }
}

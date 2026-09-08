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

    /// Intel gateway is rate-limiting this adapter (HTTP 429 Too Many Requests).
    ///
    /// This is distinct from [`crate::quota::QuotaDecision::Deny`]: the quota
    /// module is *this adapter* refusing a user before ever calling the
    /// gateway (an outbound cost control); `IntelRateLimited` is the
    /// *gateway* refusing a call this adapter already made (upstream
    /// backpressure). Keep the two separate — conflating them would blur
    /// logs and point operators at the wrong remediation (outbound-quota
    /// tuning vs. gateway capacity/backoff).
    #[error("intel rate limited: {message}")]
    IntelRateLimited {
        /// Seconds to wait before retrying, parsed from the gateway's
        /// `Retry-After` response header when present (delay-seconds form
        /// only — see `intel::parse_retry_after_secs`). `None` when the
        /// header was absent or not in a form this adapter parses.
        retry_after_secs: Option<u64>,
        /// Status/body/request-id detail for logs, same shape as the other
        /// `Intel*` variants.
        message: String,
    },

    /// Buzz relay publish / query error.
    #[error("relay: {0}")]
    Relay(String),

    /// I/O error (state file, stdin, etc.).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// JSON (de)serialization failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Turn was cancelled — via `session/cancel`, a Steer supersede, or
    /// because the process is shutting down. This variant itself carries no
    /// cause; callers that need to distinguish *why* read the session's
    /// [`CancelCause`] (the `watch` channel value at the time this error was
    /// produced) rather than matching on this variant alone.
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

/// Cause of a cancellation signalled to an in-flight turn over the session's
/// `watch` channel.
///
/// The correct response differs by cause: a [`User`](CancelCause::User)
/// cancel (`session/cancel`, or a Steer supersede) means the asker withdrew
/// the question, or a merged turn will answer instead — staying silent is
/// correct. A [`Shutdown`](CancelCause::Shutdown) cancel means the *adapter*
/// dropped the question mid-flight (e.g. a deploy restart) while the asker
/// is still waiting on an answer that will otherwise never come — that case
/// should post a best-effort notice. See `buzz-intel-agent/src/acp.rs`
/// (`graceful_shutdown`, `cancel_session`, `run_turn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelCause {
    /// No cancellation in effect (initial state, or reset at the start of a
    /// new turn).
    None,
    /// User/client-initiated (`session/cancel`) or a Steer supersede.
    User,
    /// Process shutdown (SIGTERM / stdin EOF) — the turn was dropped, not
    /// declined; the asker should be told to retry.
    Shutdown,
}

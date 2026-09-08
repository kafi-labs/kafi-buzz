//! Intelligence Platform HTTP + SSE client.

use std::fmt;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::config::Config;
use crate::error::{AdapterError, CancelCause};

/// Parsed SSE / message event kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameKind {
    /// Turn-start placeholder (no text content).
    Thinking,
    /// Tool invocation started.
    ToolCall,
    /// Tool invocation finished.
    ToolResult,
    /// Final assistant response (single-shot).
    Response,
    /// Error frame from the runtime.
    Error,
    /// Terminal stream marker.
    Done,
    /// Unrecognized event type (ignored for progress, logged).
    Other(String),
}

/// One SSE frame after JSON parse.
#[derive(Debug, Clone)]
pub struct SseFrame {
    /// Event kind.
    pub kind: FrameKind,
    /// Raw JSON payload (empty for bare `event: done`).
    pub payload: Value,
    /// Accumulated response text when kind is Response.
    pub response_text: Option<String>,
    /// Error message when kind is Error.
    pub error_message: Option<String>,
    /// Error code when present.
    pub error_code: Option<String>,
    /// Tool call id if present.
    pub tool_id: Option<String>,
    /// Tool title/name if present.
    pub tool_title: Option<String>,
    /// Tool result content if present.
    pub tool_content: Option<String>,
}

/// Result of consuming a full turn stream.
#[derive(Debug, Default)]
pub struct TurnStreamResult {
    /// Accumulated assistant text from RESPONSE frames.
    pub response_text: String,
    /// Error from ERROR frame, if any.
    pub stream_error: Option<(Option<String>, String)>,
    /// Whether any SSE data frame was received (for retry idempotency).
    pub received_frame: bool,
    /// Whether the stream delivered a terminal Done or Error frame.
    pub terminal_received: bool,
    /// Request id from response headers when available.
    pub request_id: Option<String>,
}

/// Intel gateway client.
#[derive(Clone)]
pub struct IntelClient {
    http: reqwest::Client,
    base: String,
    api_key: String,
    org_id: Option<String>,
    sse_idle: Duration,
    response_headers_timeout: Duration,
}

impl fmt::Debug for IntelClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntelClient")
            // Avoid delegating to reqwest's Debug: future client defaults may
            // carry secret headers or proxy credentials.
            .field("http", &"<configured>")
            .field("base", &self.base)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<unset>"
                } else {
                    "<redacted>"
                },
            )
            .field("org_id", &self.org_id)
            .field("sse_idle", &self.sse_idle)
            .field("response_headers_timeout", &self.response_headers_timeout)
            .finish()
    }
}

impl IntelClient {
    /// Build an HTTP client from config.
    pub fn new(cfg: &Config) -> Result<Self, AdapterError> {
        // No whole-response timeout on the `reqwest::Client` itself: SSE
        // streams are long-lived, and `ClientBuilder::timeout()` would abort
        // an in-progress stream the moment the clock runs out, which is
        // exactly what we don't want. Once the body starts streaming,
        // `sse_idle` bounds per-frame gaps in the read loop instead.
        //
        // That reasoning covers the *body*, but "whole response" and "time
        // to first byte" are different things. `connect_timeout` only bounds
        // establishing the TCP/TLS connection; once connected, `.send().await`
        // resolves as soon as response **headers** arrive, and nothing here
        // bounds that wait. A gateway that accepts the connection and then
        // stalls before sending headers hangs `.send().await` indefinitely —
        // observed live: a gateway stalled ~300s before returning 503, and
        // the adapter waited the whole time (worst case, up to the 3300s
        // whole-turn bound). `response_headers_timeout` closes that gap: every
        // `.send().await` call below is wrapped in `tokio::time::timeout`
        // against it, bounding only the headers phase — the body is still
        // unbounded here and governed by `sse_idle` once streaming starts.
        let http = reqwest::Client::builder()
            .connect_timeout(cfg.connect_timeout)
            .build()
            .map_err(|e| AdapterError::Intel(format!("http client build: {e}")))?;
        Ok(Self {
            http,
            base: cfg.gateway_url.clone(),
            api_key: cfg.api_key.clone(),
            org_id: cfg.org_id.clone(),
            sse_idle: cfg.sse_idle_timeout,
            response_headers_timeout: cfg.response_headers_timeout,
        })
    }

    /// Send a request, bounding only the time to receive response **headers**
    /// (connection already established, request already written) — not the
    /// whole response. See the doc comment on [`IntelClient::new`] for why
    /// this is a separate, narrower bound than `sse_idle`.
    ///
    /// On expiry this returns a retryable [`AdapterError::Intel`] (the same
    /// bucket the SSE idle-timeout error uses), consistent with how
    /// `crate::acp` treats transient intel errors — see the "retry once if no
    /// SSE frame was received" handling there.
    async fn send_bounded(
        &self,
        req: reqwest::RequestBuilder,
        context: &str,
    ) -> Result<reqwest::Response, AdapterError> {
        match tokio::time::timeout(self.response_headers_timeout, req.send()).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(AdapterError::Intel(format!("{context} connect: {e}"))),
            Err(_) => Err(AdapterError::Intel(format!(
                "{context}: timed out waiting for response headers after {}s",
                self.response_headers_timeout.as_secs()
            ))),
        }
    }

    fn headers(&self) -> Result<HeaderMap, AdapterError> {
        let mut h = HeaderMap::new();
        let auth = format!("Bearer {}", self.api_key);
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth)
                .map_err(|e| AdapterError::Config(format!("invalid API key header: {e}")))?,
        );
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(ref org) = self.org_id {
            h.insert(
                "X-Org-Id",
                HeaderValue::from_str(org)
                    .map_err(|e| AdapterError::Config(format!("invalid X-Org-Id: {e}")))?,
            );
        }
        Ok(h)
    }

    fn request_id(resp: &reqwest::Response) -> Option<String> {
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    /// Parse the gateway's `Retry-After` response header, when present.
    ///
    /// Only the delay-seconds form is supported (see [`parse_retry_after_secs`]);
    /// a missing or unparseable header yields `None`, never a panic or a bogus `0`.
    fn retry_after_secs(resp: &reqwest::Response) -> Option<u64> {
        resp.headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after_secs)
    }

    /// Probe `GET /v1/whoami`.
    pub async fn whoami(&self) -> Result<Value, AdapterError> {
        let url = format!("{}/v1/whoami", self.base);
        let resp = self
            .send_bounded(self.http.get(&url).headers(self.headers()?), "whoami")
            .await?;
        let rid = Self::request_id(&resp);
        let retry_after = Self::retry_after_secs(&resp);
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AdapterError::Intel(format!("whoami body: {e}")))?;
        if !status.is_success() {
            return Err(map_http_error(status, &body, rid.as_deref(), retry_after));
        }
        serde_json::from_str(&body).map_err(|e| AdapterError::Intel(format!("whoami json: {e}")))
    }

    /// `GET /v1/agents` → full JSON list.
    pub async fn list_agents(&self) -> Result<Value, AdapterError> {
        let url = format!("{}/v1/agents", self.base);
        let resp = self
            .send_bounded(self.http.get(&url).headers(self.headers()?), "list agents")
            .await?;
        let rid = Self::request_id(&resp);
        let retry_after = Self::retry_after_secs(&resp);
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AdapterError::Intel(format!("list agents body: {e}")))?;
        if !status.is_success() {
            return Err(map_http_error(status, &body, rid.as_deref(), retry_after));
        }
        serde_json::from_str(&body)
            .map_err(|e| AdapterError::Intel(format!("list agents json: {e}")))
    }

    /// Resolve agent name or UUID to `agent_id`.
    pub async fn resolve_agent_id(&self, name_or_id: &str) -> Result<String, AdapterError> {
        // If it already looks like a UUID, still verify via list (and accept
        // direct match on agent_id).
        let agents = self.list_agents().await?;
        let list = agents_as_array(&agents)?;
        let needle = name_or_id.trim();

        for a in list {
            let id = a
                .get("agent_id")
                .or_else(|| a.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let name = a.get("name").and_then(Value::as_str).unwrap_or("");
            if id.eq_ignore_ascii_case(needle) || name == needle {
                if id.is_empty() {
                    continue;
                }
                return Ok(id.to_owned());
            }
        }
        Err(AdapterError::Config(format!(
            "INTEL_AGENT {needle:?} not found in GET /v1/agents"
        )))
    }

    /// `POST /v1/sessions` → session_id.
    pub async fn create_session(
        &self,
        agent_id: &str,
        entity_id: &str,
    ) -> Result<CreateSessionResponse, AdapterError> {
        let url = format!("{}/v1/sessions", self.base);
        let body = json!({
            "agent_id": agent_id,
            "metadata": { "entity_id": entity_id }
        });
        let resp = self
            .send_bounded(
                self.http.post(&url).headers(self.headers()?).json(&body),
                "create session",
            )
            .await?;
        let rid = Self::request_id(&resp);
        let retry_after = Self::retry_after_secs(&resp);
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::Intel(format!("create session body: {e}")))?;
        if !status.is_success() {
            return Err(map_http_error(status, &text, rid.as_deref(), retry_after));
        }
        let parsed: CreateSessionResponse = serde_json::from_str(&text)
            .map_err(|e| AdapterError::Intel(format!("create session json: {e}")))?;
        if parsed.session_id.is_empty() {
            return Err(AdapterError::Intel(
                "create session: missing session_id".into(),
            ));
        }
        Ok(parsed)
    }

    /// `POST /v1/sessions/{id}/messages` and consume the SSE stream.
    ///
    /// `on_frame` is invoked for every parsed frame (for ACP session/update emission).
    /// It may be async so callers can await-send wire updates with a short timeout.
    /// `cancel` aborts the read loop when set to any cause other than
    /// [`CancelCause::None`].
    pub async fn send_message_stream<F, Fut>(
        &self,
        session_id: &str,
        message: &str,
        metadata: Value,
        cancel: &watch::Receiver<CancelCause>,
        mut on_frame: F,
    ) -> Result<TurnStreamResult, AdapterError>
    where
        F: FnMut(&SseFrame) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let url = format!("{}/v1/sessions/{session_id}/messages", self.base);
        let body = json!({
            "message": message,
            "metadata": metadata,
        });

        let resp = self
            .send_bounded(
                self.http.post(&url).headers(self.headers()?).json(&body),
                "send message",
            )
            .await?;

        let rid = Self::request_id(&resp);
        let retry_after = Self::retry_after_secs(&resp);
        let status = resp.status();

        if status == StatusCode::NOT_FOUND || status == StatusCode::CONFLICT {
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::IntelSessionGone(format!(
                "status {status}: {}{}",
                summarize_error_body(&text),
                rid.as_deref()
                    .map(|r| format!(" x-request-id={r}"))
                    .unwrap_or_default()
            )));
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::IntelAuth(format!(
                "status {}: {}{}",
                status.as_u16(),
                summarize_error_body(&text),
                rid.as_deref()
                    .map(|r| format!(" x-request-id={r}"))
                    .unwrap_or_default()
            )));
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::IntelRateLimited {
                retry_after_secs: retry_after,
                message: format!(
                    "status {}: {}{}",
                    status.as_u16(),
                    summarize_error_body(&text),
                    rid.as_deref()
                        .map(|r| format!(" x-request-id={r}"))
                        .unwrap_or_default()
                ),
            });
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status, &text, rid.as_deref(), retry_after));
        }

        let mut result = TurnStreamResult {
            request_id: rid,
            ..Default::default()
        };

        let mut stream = resp.bytes_stream();
        let mut parser = SseByteParser::new();
        let mut cancel = cancel.clone();

        loop {
            if *cancel.borrow() != CancelCause::None {
                return Err(AdapterError::Cancelled);
            }

            let next = tokio::select! {
                next = tokio::time::timeout(self.sse_idle, stream.next()) => next,
                _ = wait_for_cancel(&mut cancel) => {
                    // The gateway has no turn-abort API. Dropping our read improves
                    // user-visible latency and releases local resources, but the
                    // gateway may keep generating and billing the request.
                    return Err(AdapterError::Cancelled);
                }
            };
            let chunk = match next {
                Ok(Some(Ok(bytes))) => bytes,
                Ok(Some(Err(e))) => {
                    return Err(AdapterError::Intel(format!(
                        "sse read: {e}{}",
                        result
                            .request_id
                            .as_deref()
                            .map(|r| format!(" x-request-id={r}"))
                            .unwrap_or_default()
                    )));
                }
                Ok(None) => break,
                Err(_) => {
                    return Err(AdapterError::Intel(format!(
                        "sse idle timeout after {}s{}",
                        self.sse_idle.as_secs(),
                        result
                            .request_id
                            .as_deref()
                            .map(|r| format!(" x-request-id={r}"))
                            .unwrap_or_default()
                    )));
                }
            };

            let frames = match parser.push(&chunk) {
                Ok(f) => f,
                Err(e) => return Err(e),
            };
            for frame in frames {
                if apply_frame(&mut result, &frame, &mut on_frame).await {
                    return Ok(result);
                }
            }
        }

        // Flush trailing event without blank line.
        match parser.finish() {
            Ok(Some(frame)) => {
                let _ = apply_frame(&mut result, &frame, &mut on_frame).await;
            }
            Ok(None) => {}
            Err(e) => return Err(e),
        }

        Ok(result)
    }
}

async fn wait_for_cancel(cancel: &mut watch::Receiver<CancelCause>) {
    loop {
        if *cancel.borrow() != CancelCause::None {
            return;
        }
        if cancel.changed().await.is_err() {
            // A closed sender left at `None` is not a cancellation signal.
            // Stay pending so the stream read or its idle timeout remains active.
            std::future::pending::<()>().await;
        }
    }
}

/// Apply one frame to the turn accumulator; returns true when the stream should stop.
async fn apply_frame<F, Fut>(
    result: &mut TurnStreamResult,
    frame: &SseFrame,
    on_frame: &mut F,
) -> bool
where
    F: FnMut(&SseFrame) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    result.received_frame = true;
    if let Some(ref t) = frame.response_text {
        result.response_text.push_str(t);
    }
    if frame.kind == FrameKind::Error {
        result.stream_error = Some((
            frame.error_code.clone(),
            frame
                .error_message
                .clone()
                .unwrap_or_else(|| "unknown error".into()),
        ));
    }
    let stop = frame.kind == FrameKind::Done || frame.kind == FrameKind::Error;
    if stop {
        result.terminal_received = true;
    }
    on_frame(frame).await;
    stop
}

/// Hard cap on accounted SSE memory: incomplete line bytes, retained data
/// contents, and one [`String`] metadata slot per retained data line.
///
/// This includes the dominant per-line overhead; allocator bookkeeping and
/// unused `Vec` capacity are not counted. The cap mirrors the ACP NDJSON max
/// line size to bound memory under adversarial streams.
pub const SSE_BUFFER_CAP: usize = 8 * 1024 * 1024;

/// Incremental SSE parser that tolerates frames split across arbitrary **byte** chunks.
///
/// Incomplete UTF-8 sequences at chunk boundaries stay in the byte buffer until a
/// complete line (`\n`) arrives; only then is the line decoded. This avoids
/// `from_utf8_lossy` corruption of multi-byte codepoints split by the network.
#[derive(Debug, Default)]
pub struct SseByteParser {
    buf: Vec<u8>,
    data_lines: Vec<String>,
    /// Retained data content plus `size_of::<String>()` per line.
    data_bytes: usize,
    event_name: Option<String>,
}

impl SseByteParser {
    /// Create an empty parser.
    pub fn new() -> Self {
        Self::default()
    }

    fn buffered_total(&self) -> usize {
        self.buf.len().saturating_add(self.data_bytes)
    }

    /// Push a raw byte chunk and return any complete frames.
    ///
    /// Returns [`AdapterError::Intel`] when the buffer would exceed
    /// [`SSE_BUFFER_CAP`] or a complete line is not valid UTF-8.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>, AdapterError> {
        if self
            .buffered_total()
            .saturating_add(chunk.len())
            .saturating_add(1)
            > SSE_BUFFER_CAP
        {
            return Err(AdapterError::Intel(format!(
                "sse buffer exceeded {SSE_BUFFER_CAP} bytes (possible runaway stream)"
            )));
        }
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some(nl) = self.buf.iter().position(|&b| b == b'\n') else {
                // Incomplete line still in buf — enforce cap on growth.
                if self.buffered_total() > SSE_BUFFER_CAP {
                    return Err(AdapterError::Intel(format!(
                        "sse buffer exceeded {SSE_BUFFER_CAP} bytes (possible runaway stream)"
                    )));
                }
                break;
            };
            let mut line_bytes = self.buf.drain(..=nl).collect::<Vec<u8>>();
            // Drop trailing \n (and optional \r).
            if line_bytes.last() == Some(&b'\n') {
                line_bytes.pop();
            }
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }

            let line = match String::from_utf8(line_bytes) {
                Ok(s) => s,
                Err(e) => {
                    return Err(AdapterError::Intel(format!(
                        "sse line is not valid UTF-8: {e}"
                    )));
                }
            };

            if line.is_empty() {
                if let Some(frame) = dispatch_sse_event(self.event_name.take(), &self.data_lines) {
                    out.push(frame);
                }
                self.data_lines.clear();
                self.data_bytes = 0;
                continue;
            }

            if let Some(rest) = line.strip_prefix("event:") {
                self.event_name = Some(rest.trim().to_owned());
            } else if let Some(rest) = line.strip_prefix("data:") {
                // Spec: single space after colon is conventional; strip one.
                let data = rest.strip_prefix(' ').unwrap_or(rest);
                let add = data.len().saturating_add(std::mem::size_of::<String>());
                if self.data_bytes.saturating_add(add) > SSE_BUFFER_CAP {
                    return Err(AdapterError::Intel(format!(
                        "sse data_lines exceeded {SSE_BUFFER_CAP} bytes (possible runaway stream)"
                    )));
                }
                self.data_bytes = self.data_bytes.saturating_add(add);
                self.data_lines.push(data.to_owned());
            }
            // ignore comments / id: / retry:
        }
        Ok(out)
    }

    /// Flush a trailing event that was not terminated by a blank line.
    pub fn finish(&mut self) -> Result<Option<SseFrame>, AdapterError> {
        // Any residual incomplete line without `\n` is discarded (incomplete SSE).
        self.buf.clear();
        if self.event_name.is_none() && self.data_lines.is_empty() {
            return Ok(None);
        }
        let frame = dispatch_sse_event(self.event_name.take(), &self.data_lines);
        self.data_lines.clear();
        self.data_bytes = 0;
        Ok(frame)
    }
}

/// Response from `POST /v1/sessions`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateSessionResponse {
    /// New session id.
    pub session_id: String,
    /// Creation timestamp (optional).
    #[serde(default)]
    pub created_at: Option<String>,
}

fn agents_as_array(agents: &Value) -> Result<&Vec<Value>, AdapterError> {
    if let Some(arr) = agents.as_array() {
        return Ok(arr);
    }
    if let Some(arr) = agents.get("agents").and_then(Value::as_array) {
        return Ok(arr);
    }
    if let Some(arr) = agents.get("items").and_then(Value::as_array) {
        return Ok(arr);
    }
    if let Some(arr) = agents.get("data").and_then(Value::as_array) {
        return Ok(arr);
    }
    Err(AdapterError::Intel(
        "GET /v1/agents: expected array or {agents|items|data:[]}".into(),
    ))
}

/// Map HTTP status + body into AdapterError (handles both error envelopes).
///
/// `retry_after_secs` is the already-parsed `Retry-After` header (see
/// [`parse_retry_after_secs`]); pass `None` when the header was absent, not
/// present in the delay-seconds form, or not applicable to the call site.
/// It is only used when `status` is 429.
pub fn map_http_error(
    status: StatusCode,
    body: &str,
    request_id: Option<&str>,
    retry_after_secs: Option<u64>,
) -> AdapterError {
    let summary = summarize_error_body(body);
    let rid = request_id
        .map(|r| format!(" x-request-id={r}"))
        .unwrap_or_default();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return AdapterError::IntelAuth(format!("status {}: {summary}{rid}", status.as_u16()));
    }
    if status == StatusCode::NOT_FOUND || status == StatusCode::CONFLICT {
        return AdapterError::IntelSessionGone(format!(
            "status {}: {summary}{rid}",
            status.as_u16()
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return AdapterError::IntelRateLimited {
            retry_after_secs,
            message: format!("status {}: {summary}{rid}", status.as_u16()),
        };
    }
    AdapterError::Intel(format!("status {}: {summary}{rid}", status.as_u16()))
}

/// Parse a `Retry-After` header value in the **delay-seconds** form (e.g.
/// `"120"`), per RFC 9110 §10.2.3.
///
/// The HTTP-date form (e.g. `"Wed, 21 Oct 2026 07:28:00 GMT"`) is also legal
/// under the RFC but is treated as absent here rather than parsed: adding a
/// date/time parser (with its timezone and clock-skew edge cases) is not
/// worth the risk for a single advisory header on a best-effort retry hint,
/// and the intel gateway's own delay-seconds usage is what this adapter
/// needs to act on. Any value that is not a bare non-negative integer —
/// including an HTTP-date, empty string, or garbage — yields `None`, never
/// a panic or a bogus `0`.
pub fn parse_retry_after_secs(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

/// Owner-visible message posted to the channel when the intel gateway itself
/// refuses a turn with HTTP 429 (upstream backpressure).
///
/// Mirrors the tone/shape of [`crate::quota::quota_exceeded_message`] but
/// names a different cause: that message is *this adapter* refusing a user
/// against its own outbound turn budget; this one is the *gateway* refusing
/// a call this adapter already made. Distinguishing the two in the copy
/// matters — one is a local config knob, the other is upstream capacity.
pub fn intel_rate_limited_message(retry_after_secs: Option<u64>) -> String {
    match retry_after_secs {
        Some(secs) => {
            format!("⏳ The AI service is busy (rate-limited). Try again in about {secs}s.")
        }
        None => "⏳ The AI service is busy (rate-limited). Try again shortly.".to_owned(),
    }
}

/// Parse either `{"error":{...}}` or `{"detail":[...]}` (or plain text).
pub fn summarize_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty body".into();
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if let Some(err) = v.get("error") {
            if let Some(msg) = err.get("message").and_then(Value::as_str) {
                let code = err.get("code").and_then(Value::as_str).unwrap_or("");
                return if code.is_empty() {
                    msg.to_owned()
                } else {
                    format!("{code}: {msg}")
                };
            }
            if let Some(s) = err.as_str() {
                return s.to_owned();
            }
        }
        if let Some(detail) = v.get("detail") {
            if let Some(arr) = detail.as_array() {
                let parts: Vec<String> = arr
                    .iter()
                    .map(|d| {
                        d.get("msg")
                            .and_then(Value::as_str)
                            .or_else(|| d.as_str())
                            .unwrap_or("validation error")
                            .to_owned()
                    })
                    .collect();
                if !parts.is_empty() {
                    return parts.join("; ");
                }
            }
            if let Some(s) = detail.as_str() {
                return s.to_owned();
            }
        }
        if let Some(msg) = v.get("message").and_then(Value::as_str) {
            return msg.to_owned();
        }
    }
    // Truncate raw body for logs / user messages (char-safe — never panic on
    // multi-byte UTF-8 straddling a byte index).
    truncate_chars(trimmed, 200)
}

/// Take at most `max` Unicode scalars; append an ellipsis when truncated.
pub fn truncate_chars(s: &str, max: usize) -> String {
    let mut iter = s.chars();
    let taken: String = iter.by_ref().take(max).collect();
    if iter.next().is_some() {
        format!("{taken}…")
    } else {
        taken
    }
}

fn dispatch_sse_event(event_name: Option<String>, data_lines: &[String]) -> Option<SseFrame> {
    if let Some(name) = event_name.as_deref() {
        if name.eq_ignore_ascii_case("done") {
            return Some(SseFrame {
                kind: FrameKind::Done,
                payload: Value::Null,
                response_text: None,
                error_message: None,
                error_code: None,
                tool_id: None,
                tool_title: None,
                tool_content: None,
            });
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let data = data_lines.join("\n");
    // Terminal done can also arrive as data.
    if data.trim() == "done" || data.trim() == "[DONE]" {
        return Some(SseFrame {
            kind: FrameKind::Done,
            payload: Value::Null,
            response_text: None,
            error_message: None,
            error_code: None,
            tool_id: None,
            tool_title: None,
            tool_content: None,
        });
    }

    let payload: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("sse: non-json data frame: {e}");
            return None;
        }
    };

    Some(classify_payload(payload))
}

/// Classify a MessageEvent JSON payload into an SseFrame.
pub fn classify_payload(payload: Value) -> SseFrame {
    let event_type = payload
        .get("event_type")
        .or_else(|| payload.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let kind = classify_event_type(event_type);

    let mut frame = SseFrame {
        kind: kind.clone(),
        payload: payload.clone(),
        response_text: None,
        error_message: None,
        error_code: None,
        tool_id: None,
        tool_title: None,
        tool_content: None,
    };

    match kind {
        FrameKind::Response => {
            frame.response_text = extract_response_text(&payload);
        }
        FrameKind::Error => {
            frame.error_message = extract_error_message(&payload);
            frame.error_code = payload
                .get("code")
                .or_else(|| payload.pointer("/error/code"))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        FrameKind::ToolCall => {
            frame.tool_id = payload
                .get("id")
                .or_else(|| payload.get("tool_call_id"))
                .or_else(|| payload.get("toolCallId"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            frame.tool_title = payload
                .get("title")
                .or_else(|| payload.get("name"))
                .or_else(|| payload.get("tool_name"))
                .or_else(|| payload.get("toolName"))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        FrameKind::ToolResult => {
            frame.tool_id = payload
                .get("id")
                .or_else(|| payload.get("tool_call_id"))
                .or_else(|| payload.get("toolCallId"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            frame.tool_content = payload
                .get("content")
                .or_else(|| payload.get("result"))
                .or_else(|| payload.get("output"))
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_owned()
                    } else {
                        v.to_string()
                    }
                });
        }
        _ => {}
    }

    frame
}

fn classify_event_type(t: &str) -> FrameKind {
    let u = t.to_ascii_uppercase();
    if u.contains("THINKING") || u == "TURN_START" {
        FrameKind::Thinking
    } else if u.contains("TOOL_CALL") || u.contains("TOOL_EXECUTION_START") {
        FrameKind::ToolCall
    } else if u.contains("TOOL_RESULT") || u.contains("TOOL_EXECUTION_END") {
        FrameKind::ToolResult
    } else if u.contains("RESPONSE") || u == "MESSAGE_END" {
        FrameKind::Response
    } else if u.contains("ERROR") {
        FrameKind::Error
    } else if u == "DONE" {
        FrameKind::Done
    } else if t.is_empty() {
        // Some streams omit type and only send response object.
        FrameKind::Other("unknown".into())
    } else {
        FrameKind::Other(t.to_owned())
    }
}

/// RESPONSE text fallback chain: response.response → .content → .text → .output.
pub fn extract_response_text(payload: &Value) -> Option<String> {
    let candidates = [
        payload.pointer("/response/response"),
        payload.pointer("/response/content"),
        payload.pointer("/response/text"),
        payload.pointer("/response/output"),
        payload.get("response"),
        payload.get("content"),
        payload.get("text"),
        payload.get("output"),
        payload.get("message"),
    ];
    for v in candidates.into_iter().flatten() {
        if let Some(s) = v.as_str() {
            if !s.is_empty() {
                return Some(s.to_owned());
            }
        }
        // Nested {response: "…"} already handled; if response is object, dig.
        if let Some(s) = v.get("response").and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_owned());
            }
        }
        if let Some(s) = v.get("content").and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_owned());
            }
        }
        if let Some(s) = v.get("text").and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_owned());
            }
        }
        if let Some(s) = v.get("output").and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_owned());
            }
        }
    }
    None
}

fn extract_error_message(payload: &Value) -> Option<String> {
    payload
        .get("message")
        .or_else(|| payload.get("detail"))
        .or_else(|| payload.pointer("/error/message"))
        .and_then(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_owned())
            } else if let Some(arr) = v.as_array() {
                let parts: Vec<&str> = arr.iter().filter_map(Value::as_str).collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("; "))
                }
            } else {
                Some(v.to_string())
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_client_secret_but_keeps_context() {
        let secret = "intel_client_debug_secret";
        let client = IntelClient {
            http: reqwest::Client::new(),
            base: "https://debug-gateway.example.test".to_string(),
            api_key: secret.to_string(),
            org_id: Some("debug-org".to_string()),
            sse_idle: Duration::from_secs(42),
            response_headers_timeout: Duration::from_secs(30),
        };

        let debug = format!("{client:?}");

        assert!(!debug.contains(secret));
        assert!(debug.contains("api_key: \"<redacted>\""));
        assert!(debug.contains("https://debug-gateway.example.test"));
        assert!(debug.contains("debug-org"));
        assert!(debug.contains("42s"));
    }

    #[test]
    fn error_envelope_native() {
        let body = r#"{"error":{"message":"nope","code":"UNAUTHENTICATED","status":401}}"#;
        assert!(summarize_error_body(body).contains("UNAUTHENTICATED"));
        assert!(summarize_error_body(body).contains("nope"));
    }

    #[test]
    fn error_envelope_fastapi() {
        let body = r#"{"detail":[{"msg":"field required"},{"msg":"bad type"}]}"#;
        let s = summarize_error_body(body);
        assert!(s.contains("field required"));
        assert!(s.contains("bad type"));
    }

    #[test]
    fn error_envelope_fastapi_string_detail() {
        let body = r#"{"detail":"session stopped"}"#;
        assert_eq!(summarize_error_body(body), "session stopped");
    }

    #[test]
    fn error_envelope_empty_and_plain() {
        assert_eq!(summarize_error_body(""), "empty body");
        assert_eq!(summarize_error_body("  not-json  "), "not-json");
    }

    #[test]
    fn response_text_fallbacks() {
        let v = json!({"event_type":"MESSAGE_EVENT_TYPE_RESPONSE","response":{"response":"hi"}});
        assert_eq!(extract_response_text(&v).as_deref(), Some("hi"));

        let v = json!({"type":"RESPONSE","content":"yo"});
        assert_eq!(extract_response_text(&v).as_deref(), Some("yo"));

        let v = json!({"text":"plain"});
        assert_eq!(extract_response_text(&v).as_deref(), Some("plain"));

        let v = json!({"output":"out"});
        assert_eq!(extract_response_text(&v).as_deref(), Some("out"));

        // Nested response object content field.
        let v = json!({"response":{"content":"nested-content"}});
        assert_eq!(extract_response_text(&v).as_deref(), Some("nested-content"));
    }

    #[test]
    fn classify_thinking() {
        let f = classify_payload(json!({"event_type":"MESSAGE_EVENT_TYPE_THINKING"}));
        assert_eq!(f.kind, FrameKind::Thinking);
    }

    #[test]
    fn classify_error_frame() {
        let f = classify_payload(json!({
            "event_type": "MESSAGE_EVENT_TYPE_ERROR",
            "code": "TURN_FAILED",
            "message": "boom"
        }));
        assert_eq!(f.kind, FrameKind::Error);
        assert_eq!(f.error_code.as_deref(), Some("TURN_FAILED"));
        assert_eq!(f.error_message.as_deref(), Some("boom"));
    }

    #[test]
    fn classify_tool_call_and_result() {
        let call = classify_payload(json!({
            "event_type": "MESSAGE_EVENT_TYPE_TOOL_CALL",
            "id": "tc-1",
            "name": "search"
        }));
        assert_eq!(call.kind, FrameKind::ToolCall);
        assert_eq!(call.tool_id.as_deref(), Some("tc-1"));
        assert_eq!(call.tool_title.as_deref(), Some("search"));

        let result = classify_payload(json!({
            "event_type": "MESSAGE_EVENT_TYPE_TOOL_RESULT",
            "tool_call_id": "tc-1",
            "content": "found it"
        }));
        assert_eq!(result.kind, FrameKind::ToolResult);
        assert_eq!(result.tool_id.as_deref(), Some("tc-1"));
        assert_eq!(result.tool_content.as_deref(), Some("found it"));
    }

    #[test]
    fn sse_event_done_terminal() {
        let mut p = SseByteParser::new();
        let frames = p.push(b"event: done\n\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, FrameKind::Done);
    }

    #[test]
    fn sse_data_done_variants() {
        let mut p = SseByteParser::new();
        let frames = p.push(b"data: [DONE]\n\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, FrameKind::Done);
    }

    #[test]
    fn sse_frames_split_across_chunk_boundaries() {
        // Multi-frame stream with awkward mid-line and mid-event splits.
        let full = concat!(
            "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_THINKING\"}\n\n",
            "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_TOOL_CALL\",\"id\":\"t1\",\"title\":\"lookup\"}\n\n",
            "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_TOOL_RESULT\",\"id\":\"t1\",\"content\":\"ok\"}\n\n",
            "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_RESPONSE\",\"response\":{\"response\":\"final answer\"}}\n\n",
            "event: done\n\n",
        );
        // Split into awkward chunks: mid-line, mid-JSON, 1-byte slices.
        let mut cuts = vec![0usize];
        let mut i = 1usize;
        while i < full.len() {
            // Vary step size so frames cross chunk boundaries.
            let step = match i % 5 {
                0 => 1,
                1 => 3,
                2 => 7,
                3 => 13,
                _ => 17,
            };
            i = (i + step).min(full.len());
            cuts.push(i);
        }
        if *cuts.last().unwrap() != full.len() {
            cuts.push(full.len());
        }
        let mut p = SseByteParser::new();
        let mut frames = Vec::new();
        for w in cuts.windows(2) {
            frames.extend(p.push(&full.as_bytes()[w[0]..w[1]]).unwrap());
        }
        if let Some(f) = p.finish().unwrap() {
            frames.push(f);
        }
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[0].kind, FrameKind::Thinking);
        assert_eq!(frames[1].kind, FrameKind::ToolCall);
        assert_eq!(frames[1].tool_id.as_deref(), Some("t1"));
        assert_eq!(frames[2].kind, FrameKind::ToolResult);
        assert_eq!(frames[3].kind, FrameKind::Response);
        assert_eq!(frames[3].response_text.as_deref(), Some("final answer"));
        assert_eq!(frames[4].kind, FrameKind::Done);
    }

    /// Multi-byte payload deliberately split mid-codepoint across push() calls
    /// must reassemble losslessly (no U+FFFD).
    #[test]
    fn sse_multibyte_split_mid_codepoint_is_lossless() {
        // 🔥 = F0 9F 94 A5; Indonesian + emoji reply text.
        let text = "Jawaban: baik 🔥 terima kasih 测试";
        let payload = format!(
            "{{\"event_type\":\"MESSAGE_EVENT_TYPE_RESPONSE\",\"response\":{{\"response\":{}}}}}",
            serde_json::to_string(text).unwrap()
        );
        let line = format!("data: {payload}\n\n");
        let bytes = line.as_bytes();

        // Find the emoji byte offset inside the full line and split mid-codepoint.
        let emoji_at = line.find('🔥').expect("emoji present");
        assert_eq!(
            &line.as_bytes()[emoji_at..emoji_at + 4],
            [0xF0, 0x9F, 0x94, 0xA5]
        );
        let mid = emoji_at + 2; // middle of the 4-byte sequence

        let mut p = SseByteParser::new();
        let mut frames = p.push(&bytes[..mid]).unwrap();
        frames.extend(p.push(&bytes[mid..]).unwrap());
        if let Some(f) = p.finish().unwrap() {
            frames.push(f);
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, FrameKind::Response);
        assert_eq!(frames[0].response_text.as_deref(), Some(text));
        assert!(!frames[0]
            .response_text
            .as_deref()
            .unwrap_or("")
            .contains('\u{FFFD}'));
    }

    #[tokio::test]
    async fn sse_cancel_interrupts_quiet_stream_without_waiting_for_idle_timeout() {
        use std::convert::Infallible;
        use std::sync::Arc;

        use axum::body::{Body, Bytes};
        use axum::http::{header, Response};
        use axum::routing::post;
        use axum::Router;
        use tokio::sync::Notify;

        let app = Router::new().route(
            "/v1/sessions/{id}/messages",
            post(|| async {
                let chunks = futures_util::stream::once(async {
                    Ok::<_, Infallible>(Bytes::from_static(
                        b"data: {\"event_type\":\"MESSAGE_EVENT_TYPE_THINKING\"}\n\n",
                    ))
                })
                .chain(futures_util::stream::pending::<Result<Bytes, Infallible>>());
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(chunks))
                    .expect("quiet SSE response")
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind quiet SSE server");
        let addr = listener.local_addr().expect("quiet SSE server address");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = IntelClient {
            http: reqwest::Client::new(),
            base: format!("http://{addr}"),
            api_key: "intel_test_key".to_owned(),
            org_id: None,
            sse_idle: Duration::from_secs(5),
            response_headers_timeout: Duration::from_secs(5),
        };
        let (cancel_tx, cancel_rx) = watch::channel(CancelCause::None);
        let first_frame = Arc::new(Notify::new());
        let frame_seen = Arc::clone(&first_frame);
        let turn = tokio::spawn(async move {
            client
                .send_message_stream(
                    "quiet-session",
                    "wait quietly",
                    json!({}),
                    &cancel_rx,
                    move |_| {
                        let frame_seen = Arc::clone(&frame_seen);
                        async move {
                            frame_seen.notify_one();
                        }
                    },
                )
                .await
        });

        tokio::time::timeout(Duration::from_secs(1), first_frame.notified())
            .await
            .expect("initial SSE frame was not observed");
        // Let the read loop re-enter stream.next() after processing the frame.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let cancel_started = tokio::time::Instant::now();
        cancel_tx
            .send(CancelCause::User)
            .expect("send cancellation");
        let result = tokio::time::timeout(Duration::from_millis(500), turn)
            .await
            .expect("cancellation waited for the SSE idle timeout")
            .expect("turn task panicked");
        assert!(matches!(result, Err(AdapterError::Cancelled)));
        assert!(cancel_started.elapsed() < Duration::from_millis(500));

        server.abort();
    }

    #[test]
    fn sse_buffer_cap_errors_on_runaway_line() {
        let mut p = SseByteParser::new();
        // One huge line without newline — must error instead of growing unbounded.
        let big = vec![b'a'; SSE_BUFFER_CAP + 1];
        let err = p.push(&big).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("sse buffer exceeded") || msg.contains("runaway"),
            "unexpected err: {msg}"
        );
    }

    #[test]
    fn sse_buffer_cap_counts_per_line_overhead() {
        let mut p = SseByteParser::new();
        let retained_per_line = 1 + std::mem::size_of::<String>();
        let attempts = SSE_BUFFER_CAP / retained_per_line + 1;
        let mut error = None;

        for _ in 0..attempts {
            if let Err(err) = p.push(b"data: x\n") {
                error = Some(err);
                break;
            }
        }

        let err = error.expect("per-line metadata must count toward the SSE buffer cap");
        assert!(
            err.to_string().contains("sse buffer exceeded")
                || err.to_string().contains("sse data_lines exceeded"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn sse_error_frame_stops_with_message() {
        let mut p = SseByteParser::new();
        let frames = p
            .push(b"data: {\"event_type\":\"ERROR\",\"code\":\"X\",\"message\":\"nope\"}\n\n")
            .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, FrameKind::Error);
        assert_eq!(frames[0].error_code.as_deref(), Some("X"));
        assert_eq!(frames[0].error_message.as_deref(), Some("nope"));
    }

    #[test]
    fn map_http_error_status_classes() {
        let e = map_http_error(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"code":"UNAUTHENTICATED","message":"bad key"}}"#,
            Some("rid-1"),
            None,
        );
        match e {
            AdapterError::IntelAuth(s) => {
                assert!(s.contains("401"));
                assert!(s.contains("rid-1"));
            }
            other => panic!("expected IntelAuth, got {other:?}"),
        }

        let e = map_http_error(StatusCode::CONFLICT, r#"{"detail":"stopped"}"#, None, None);
        assert!(matches!(e, AdapterError::IntelSessionGone(_)));

        let e = map_http_error(StatusCode::INTERNAL_SERVER_ERROR, "boom", None, None);
        assert!(matches!(e, AdapterError::Intel(_)));
    }

    #[test]
    fn map_http_error_429_with_retry_after_is_rate_limited() {
        let e = map_http_error(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"code":"RESOURCE_EXHAUSTED","message":"slow down"}}"#,
            Some("rid-429"),
            Some(120),
        );
        match e {
            AdapterError::IntelRateLimited {
                retry_after_secs,
                message,
            } => {
                assert_eq!(retry_after_secs, Some(120));
                assert!(message.contains("429"));
                assert!(message.contains("rid-429"));
                assert!(message.contains("slow down"));
            }
            other => panic!("expected IntelRateLimited, got {other:?}"),
        }
    }

    #[test]
    fn map_http_error_429_without_retry_after_is_none_not_zero() {
        let e = map_http_error(StatusCode::TOO_MANY_REQUESTS, "busy", None, None);
        match e {
            AdapterError::IntelRateLimited {
                retry_after_secs, ..
            } => {
                assert_eq!(
                    retry_after_secs, None,
                    "missing Retry-After must map to None, never a bogus 0"
                );
            }
            other => panic!("expected IntelRateLimited, got {other:?}"),
        }
    }

    #[test]
    fn parse_retry_after_secs_delay_seconds_form() {
        assert_eq!(parse_retry_after_secs("120"), Some(120));
        assert_eq!(parse_retry_after_secs(" 45 "), Some(45));
        assert_eq!(parse_retry_after_secs("0"), Some(0));
    }

    #[test]
    fn parse_retry_after_secs_unparseable_is_none() {
        // Garbage.
        assert_eq!(parse_retry_after_secs("not-a-number"), None);
        // Empty.
        assert_eq!(parse_retry_after_secs(""), None);
        // Negative is not a valid delay-seconds value.
        assert_eq!(parse_retry_after_secs("-5"), None);
        // HTTP-date form is legal per RFC 9110 but deliberately not parsed —
        // treated as absent, per the documented decision on `parse_retry_after_secs`.
        assert_eq!(
            parse_retry_after_secs("Wed, 21 Oct 2026 07:28:00 GMT"),
            None
        );
    }

    #[test]
    fn intel_rate_limited_message_states_wait_when_known() {
        let m = intel_rate_limited_message(Some(30));
        assert!(m.contains("30"));
        assert!(m.to_lowercase().contains("rate-limited") || m.to_lowercase().contains("busy"));
    }

    #[test]
    fn intel_rate_limited_message_is_sane_when_unknown() {
        let m = intel_rate_limited_message(None);
        assert!(!m.is_empty());
        assert!(m.to_lowercase().contains("rate-limited") || m.to_lowercase().contains("busy"));
    }

    /// Build a client that talks to `base` with a short (test-fast)
    /// `response_headers_timeout` and the given `sse_idle` — bypasses
    /// `IntelClient::new`/`Config` (private-field construction is legal here
    /// since `tests` is a submodule of `intel`) so the test doesn't need a
    /// full `Config` just to override one duration.
    fn test_client(base: String, response_headers_timeout: Duration) -> IntelClient {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        IntelClient {
            http,
            base,
            api_key: "test-key".into(),
            org_id: None,
            sse_idle: Duration::from_secs(5),
            response_headers_timeout,
        }
    }

    /// Axum handler that never responds within any sane test timeout —
    /// simulates a gateway that accepted the TCP connection but stalled
    /// before sending response headers. This is the exact failure mode from
    /// the live dev-VM incident this timeout guards against: the gateway
    /// held the connection open for ~300s before eventually returning a 503,
    /// and `.send().await` — bounded only by (unset) whole-response timeout —
    /// waited the entire time.
    async fn hang_forever() -> axum::http::StatusCode {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        axum::http::StatusCode::OK
    }

    /// A stalled headers phase on a simple GET call site (`whoami`) must
    /// time out quickly — bounded by `response_headers_timeout`, not left to
    /// hang for the whole-turn bound — and land in the `AdapterError::Intel`
    /// bucket, which `crate::acp`'s turn loop treats as retryable (one
    /// jittered retry when no SSE frame has been received yet; see the
    /// "transient intel error … retrying once" handling there). Everything
    /// else in that loop is special-cased out (Cancelled, IntelSessionGone,
    /// IntelAuth, IntelRateLimited), so asserting the `Intel` variant here
    /// *is* asserting retryability.
    #[tokio::test]
    async fn whoami_headers_timeout_is_retryable() {
        let app = axum::Router::new().route("/v1/whoami", axum::routing::get(hang_forever));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock gateway");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // Kept small so the test stays fast; the production default (570s)
        // is sized for headroom over the longest legitimate generation
        // (headers are withheld until generation starts on this gateway),
        // not for speed.
        let headers_timeout = Duration::from_millis(150);
        let client = test_client(format!("http://{addr}"), headers_timeout);

        let started = std::time::Instant::now();
        let err = client
            .whoami()
            .await
            .expect_err("headers phase must time out");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(2),
            "expected the {headers_timeout:?} headers timeout to fire quickly, took {elapsed:?}"
        );

        match err {
            AdapterError::Intel(msg) => {
                let lower = msg.to_lowercase();
                assert!(
                    lower.contains("timed out") || lower.contains("timeout"),
                    "expected a timeout-flavored message, got {msg:?}"
                );
            }
            other => panic!(
                "expected AdapterError::Intel (the retryable bucket per crate::acp's turn \
                 loop), got {other:?}"
            ),
        }
    }

    /// Same failure mode as [`whoami_headers_timeout_is_retryable`] but on
    /// `send_message_stream` — the actual call site involved in the live
    /// incident (POST `/v1/sessions/{id}/messages`, headers stalled before
    /// any SSE bytes arrived). Proves the fix isn't limited to the simple
    /// GET sites: the same `tokio::time::timeout` wrap and retryable
    /// `AdapterError::Intel` classification applies to the streaming POST.
    #[tokio::test]
    async fn send_message_stream_headers_timeout_is_retryable() {
        let app = axum::Router::new().route(
            "/v1/sessions/{id}/messages",
            axum::routing::post(hang_forever),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock gateway");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let headers_timeout = Duration::from_millis(150);
        let client = test_client(format!("http://{addr}"), headers_timeout);
        let (_cancel_tx, cancel_rx) = watch::channel(CancelCause::None);

        let started = std::time::Instant::now();
        let err = client
            .send_message_stream(
                "sess-1",
                "hello",
                json!({}),
                &cancel_rx,
                |_frame: &SseFrame| async {},
            )
            .await
            .expect_err("headers phase must time out before any SSE bytes arrive");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(2),
            "expected the {headers_timeout:?} headers timeout to fire quickly, took {elapsed:?}"
        );

        match err {
            AdapterError::Intel(msg) => {
                let lower = msg.to_lowercase();
                assert!(
                    lower.contains("timed out") || lower.contains("timeout"),
                    "expected a timeout-flavored message, got {msg:?}"
                );
            }
            other => panic!(
                "expected AdapterError::Intel (the retryable bucket per crate::acp's turn \
                 loop), got {other:?}"
            ),
        }
    }

    #[test]
    fn summarize_error_body_truncates_multibyte_safely() {
        // Build a body where byte index 200 lands mid multi-byte char.
        // "测" is 3 bytes; pad with ASCII then many CJK so char-truncation is exercised.
        let mut body = "x".repeat(199);
        body.push('测'); // if we sliced at byte 200 we'd panic mid-char
        body.push_str(&"测".repeat(50));
        let out = summarize_error_body(&body);
        // Must not panic; must be truncated with ellipsis; must be valid UTF-8.
        assert!(out.ends_with('…'), "expected ellipsis, got {out:?}");
        assert!(!out.contains('\u{FFFD}'));
        // Char-safe: every char is complete.
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn truncate_chars_mid_emoji() {
        let s = format!("{}🔥", "a".repeat(10));
        // 11 scalars = 10 'a's + full emoji — no truncation.
        let t = truncate_chars(&s, 11);
        assert_eq!(t, format!("{}🔥", "a".repeat(10)));
        // 10 scalars leaves the emoji out → truncated with ellipsis.
        let t2 = truncate_chars(&s, 10);
        assert_eq!(t2, format!("{}…", "a".repeat(10)));
        let t3 = truncate_chars(&format!("{}🔥extra", "a".repeat(10)), 11);
        assert!(t3.ends_with('…'));
        assert!(t3.contains('🔥'));
    }
}

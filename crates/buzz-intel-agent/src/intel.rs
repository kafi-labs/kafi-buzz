//! Intelligence Platform HTTP + SSE client.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::config::Config;
use crate::error::AdapterError;

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
    /// Request id from response headers when available.
    pub request_id: Option<String>,
}

/// Intel gateway client.
#[derive(Debug, Clone)]
pub struct IntelClient {
    http: reqwest::Client,
    base: String,
    api_key: String,
    org_id: Option<String>,
    sse_idle: Duration,
}

impl IntelClient {
    /// Build an HTTP client from config.
    pub fn new(cfg: &Config) -> Result<Self, AdapterError> {
        // No whole-response timeout: SSE streams are long-lived; per-frame idle
        // is enforced while reading the body.
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
        })
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

    /// Probe `GET /v1/whoami`.
    pub async fn whoami(&self) -> Result<Value, AdapterError> {
        let url = format!("{}/v1/whoami", self.base);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| AdapterError::Intel(format!("whoami connect: {e}")))?;
        let rid = Self::request_id(&resp);
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AdapterError::Intel(format!("whoami body: {e}")))?;
        if !status.is_success() {
            return Err(map_http_error(status, &body, rid.as_deref()));
        }
        serde_json::from_str(&body).map_err(|e| AdapterError::Intel(format!("whoami json: {e}")))
    }

    /// `GET /v1/agents` → full JSON list.
    pub async fn list_agents(&self) -> Result<Value, AdapterError> {
        let url = format!("{}/v1/agents", self.base);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| AdapterError::Intel(format!("list agents connect: {e}")))?;
        let rid = Self::request_id(&resp);
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AdapterError::Intel(format!("list agents body: {e}")))?;
        if !status.is_success() {
            return Err(map_http_error(status, &body, rid.as_deref()));
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
            .http
            .post(&url)
            .headers(self.headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::Intel(format!("create session connect: {e}")))?;
        let rid = Self::request_id(&resp);
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::Intel(format!("create session body: {e}")))?;
        if !status.is_success() {
            return Err(map_http_error(status, &text, rid.as_deref()));
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
    /// `cancel` aborts the read loop when set to true.
    pub async fn send_message_stream<F>(
        &self,
        session_id: &str,
        message: &str,
        metadata: Value,
        cancel: &watch::Receiver<bool>,
        mut on_frame: F,
    ) -> Result<TurnStreamResult, AdapterError>
    where
        F: FnMut(&SseFrame),
    {
        let url = format!("{}/v1/sessions/{session_id}/messages", self.base);
        let body = json!({
            "message": message,
            "metadata": metadata,
        });

        let resp = self
            .http
            .post(&url)
            .headers(self.headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::Intel(format!("send message connect: {e}")))?;

        let rid = Self::request_id(&resp);
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
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status, &text, rid.as_deref()));
        }

        let mut result = TurnStreamResult {
            request_id: rid,
            ..Default::default()
        };

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut data_lines: Vec<String> = Vec::new();
        let mut event_name: Option<String> = None;

        loop {
            if *cancel.borrow() {
                return Err(AdapterError::Cancelled);
            }

            let next = tokio::time::timeout(self.sse_idle, stream.next()).await;
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

            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(nl) = buf.find('\n') {
                let mut line = buf[..nl].to_owned();
                buf.drain(..=nl);
                if line.ends_with('\r') {
                    line.pop();
                }

                if line.is_empty() {
                    // Dispatch accumulated event.
                    if let Some(frame) = dispatch_sse_event(event_name.take(), &data_lines) {
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
                        let is_done = frame.kind == FrameKind::Done;
                        on_frame(&frame);
                        if is_done || frame.kind == FrameKind::Error {
                            return Ok(result);
                        }
                    }
                    data_lines.clear();
                    continue;
                }

                if let Some(rest) = line.strip_prefix("event:") {
                    event_name = Some(rest.trim().to_owned());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    // Spec: single space after colon is conventional; strip one.
                    let data = if let Some(s) = rest.strip_prefix(' ') {
                        s
                    } else {
                        rest
                    };
                    data_lines.push(data.to_owned());
                }
                // ignore comments / id: / retry:
            }
        }

        // Flush trailing event without blank line.
        if let Some(frame) = dispatch_sse_event(event_name, &data_lines) {
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
            on_frame(&frame);
        }

        Ok(result)
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
pub fn map_http_error(status: StatusCode, body: &str, request_id: Option<&str>) -> AdapterError {
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
    AdapterError::Intel(format!("status {}: {summary}{rid}", status.as_u16()))
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
    // Truncate raw body for logs / user messages.
    let max = 200;
    if trimmed.len() > max {
        format!("{}…", &trimmed[..max])
    } else {
        trimmed.to_owned()
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
    fn response_text_fallbacks() {
        let v = json!({"event_type":"MESSAGE_EVENT_TYPE_RESPONSE","response":{"response":"hi"}});
        assert_eq!(extract_response_text(&v).as_deref(), Some("hi"));

        let v = json!({"type":"RESPONSE","content":"yo"});
        assert_eq!(extract_response_text(&v).as_deref(), Some("yo"));

        let v = json!({"text":"plain"});
        assert_eq!(extract_response_text(&v).as_deref(), Some("plain"));

        let v = json!({"output":"out"});
        assert_eq!(extract_response_text(&v).as_deref(), Some("out"));
    }

    #[test]
    fn classify_thinking() {
        let f = classify_payload(json!({"event_type":"MESSAGE_EVENT_TYPE_THINKING"}));
        assert_eq!(f.kind, FrameKind::Thinking);
    }
}

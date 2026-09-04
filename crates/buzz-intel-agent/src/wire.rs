//! NDJSON JSON-RPC 2.0 wire helpers (ACP server surface).

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// JSON-RPC parse error.
pub const PARSE_ERROR: i32 = -32700;
/// JSON-RPC invalid request.
pub const INVALID_REQUEST: i32 = -32600;
/// JSON-RPC method not found.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC invalid params.
pub const INVALID_PARAMS: i32 = -32602;

/// Outbound wire message.
pub enum WireMsg {
    /// Serialize and write a JSON value as one NDJSON line.
    Notify(Value),
}

/// Channel used by handlers to send protocol messages to stdout.
pub type WireSender = mpsc::Sender<WireMsg>;

/// Classified inbound frame.
#[derive(Debug)]
pub enum Inbound {
    /// JSON-RPC request (has id).
    Request {
        /// Request id.
        id: Value,
        /// Method name.
        method: String,
        /// Params object.
        params: Value,
    },
    /// JSON-RPC notification (no id).
    Notification {
        /// Method name.
        method: String,
        /// Params object.
        params: Value,
    },
    /// Bare response or other noise — ignore.
    Ignored,
    /// Malformed frame that still has an id to reject.
    Invalid {
        /// Request id if present.
        id: Value,
        /// Error code.
        code: i32,
        /// Error message.
        message: String,
    },
}

/// `initialize` params.
#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    /// Client protocol version.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    /// Client capabilities (accepted, unused).
    #[serde(default, rename = "clientCapabilities")]
    pub _client_capabilities: Value,
}

/// ACP content block.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text.
    Text {
        /// Text body.
        text: String,
    },
    /// Resource link (treated as a URI string).
    ResourceLink {
        /// Resource URI.
        uri: String,
    },
    /// Unknown block types are ignored.
    #[serde(other)]
    Unsupported,
}

/// `session/new` params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewParams {
    /// Working directory (absolute).
    pub cwd: String,
    /// MCP servers (accepted and ignored in MVP).
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    /// Optional system prompt from the harness.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

/// `session/prompt` params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptParams {
    /// Local ACP session id.
    pub session_id: String,
    /// Prompt content blocks.
    pub prompt: Vec<ContentBlock>,
}

/// `session/cancel` params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCancelParams {
    /// Local ACP session id.
    pub session_id: String,
}

/// Classify a parsed JSON value as an inbound message.
pub fn classify(msg: &Value) -> Inbound {
    if !msg.is_object() || msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Inbound::Invalid {
            id: msg.get("id").cloned().unwrap_or(Value::Null),
            code: INVALID_REQUEST,
            message: "jsonrpc: missing or invalid version".into(),
        };
    }
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    match (method, id) {
        (Some(m), Some(id)) => Inbound::Request {
            id,
            method: m,
            params,
        },
        (Some(m), None) => Inbound::Notification { method: m, params },
        (None, Some(_)) => Inbound::Ignored,
        (None, None) => Inbound::Invalid {
            id: Value::Null,
            code: INVALID_REQUEST,
            message: "jsonrpc: missing method and id".into(),
        },
    }
}

/// Build a successful JSON-RPC result.
pub fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Build a JSON-RPC error response.
pub fn err(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Build a `session/update` notification.
pub fn session_update(sid: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": sid, "update": update },
    })
}

/// Send a wire message (best-effort; drops if the channel is closed).
pub async fn send(wire: &WireSender, msg: Value) {
    let _ = wire.send(WireMsg::Notify(msg)).await;
}

/// Read one newline-delimited frame with a max byte bound.
pub async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    stdin: &mut R,
    max: usize,
) -> std::io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = stdin.fill_buf().await?;
        if chunk.is_empty() {
            if !buf.is_empty() {
                tracing::error!(
                    "io: unterminated frame at EOF ({} bytes dropped)",
                    buf.len()
                );
            }
            return Ok(None);
        }
        let take = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map_or(chunk.len(), |i| i + 1);
        if buf.len().saturating_add(take) > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("io: line exceeds max ({max} bytes)"),
            ));
        }
        buf.extend_from_slice(&chunk[..take]);
        stdin.consume(take);
        if buf.ends_with(b"\n") {
            buf.pop();
            if buf.ends_with(b"\r") {
                buf.pop();
            }
            match String::from_utf8(buf) {
                Ok(s) => return Ok(Some(s)),
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "io: frame contains invalid UTF-8",
                    ))
                }
            }
        }
    }
}

/// Writer task: serializes outbound messages to stdout.
pub async fn writer_task(mut rx: mpsc::Receiver<WireMsg>) {
    let mut stdout = tokio::io::stdout();
    while let Some(msg) = rx.recv().await {
        let WireMsg::Notify(v) = msg;
        let mut s = match serde_json::to_string(&v) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("io: serialize: {e}");
                continue;
            }
        };
        s.push('\n');
        if stdout.write_all(s.as_bytes()).await.is_err() {
            return;
        }
        let _ = stdout.flush().await;
    }
}

/// Concatenate content blocks into a single prompt string.
pub fn prompt_to_text(prompt: Vec<ContentBlock>) -> String {
    let mut parts = Vec::new();
    for block in prompt {
        match block {
            ContentBlock::Text { text } => parts.push(text),
            ContentBlock::ResourceLink { uri } => parts.push(format!("[resource: {uri}]")),
            ContentBlock::Unsupported => {}
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_request() {
        let msg = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        match classify(&msg) {
            Inbound::Request { method, .. } => assert_eq!(method, "initialize"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn prompt_to_text_joins() {
        let blocks = vec![
            ContentBlock::Text {
                text: "hello".into(),
            },
            ContentBlock::Text {
                text: "world".into(),
            },
        ];
        assert_eq!(prompt_to_text(blocks), "hello\nworld");
    }
}

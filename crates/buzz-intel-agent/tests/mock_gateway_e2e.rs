//! Mock-gateway e2e: drive `buzz-intel-agent` over ACP stdio against an axum
//! intel stub (+ optional relay stub for POST /events).

use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

const AGENT_ID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const AGENT_NAME: &str = "demo-agent";
const CHANNEL_ID: &str = "11111111-2222-3333-4444-555555555555";
const EVENT_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Well-known test secret (secp256k1).
const TEST_SK: &str = "0000000000000000000000000000000000000000000000000000000000000001";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    /// THINKING → TOOL_CALL → TOOL_RESULT → RESPONSE → event:done
    HappyMultiFrame,
    /// RESPONSE followed by a clean body EOF, with no terminal event.
    CleanEofAfterPartialResponse,
    /// Empty RESPONSE followed by event:done.
    DoneWithEmptyResponse,
    /// Whitespace-only RESPONSE followed by event:done.
    DoneWithWhitespaceResponse,
    /// POST messages → 401
    Unauthorized,
    /// First messages call → 409; after recreate → happy stream
    ConflictThenOk,
    /// SSE ERROR frame mid-stream
    SseError,
    /// THINKING + TOOL_CALL, then a rate-limit error signal.
    MidTurnTooManyRequests,
    /// THINKING + TOOL_CALL, then an upstream-unavailable error signal.
    MidTurnServiceUnavailable,
    /// THINKING + TOOL_CALL, then a credentials-revoked error signal.
    MidTurnUnauthorized,
}

#[derive(Clone)]
struct MockState {
    scenario: Scenario,
    sessions_created: Arc<AtomicUsize>,
    messages_hits: Arc<AtomicUsize>,
    /// session_id → message attempt count for that id
    per_session_msgs: Arc<Mutex<std::collections::HashMap<String, usize>>>,
    relay_posts: Arc<AtomicUsize>,
    relay_bodies: Arc<Mutex<Vec<Value>>>,
}

impl MockState {
    fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            sessions_created: Arc::new(AtomicUsize::new(0)),
            messages_hits: Arc::new(AtomicUsize::new(0)),
            per_session_msgs: Arc::new(Mutex::new(std::collections::HashMap::new())),
            relay_posts: Arc::new(AtomicUsize::new(0)),
            relay_bodies: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

async fn get_agents() -> impl IntoResponse {
    Json(json!([
        { "agent_id": AGENT_ID, "name": AGENT_NAME }
    ]))
}

async fn create_session(State(st): State<MockState>) -> impl IntoResponse {
    let n = st.sessions_created.fetch_add(1, Ordering::SeqCst) + 1;
    let session_id = format!("intel-sess-{n}");
    Json(json!({
        "session_id": session_id,
        "created_at": "2026-07-24T00:00:00Z"
    }))
}

fn happy_sse() -> String {
    concat!(
        "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_THINKING\"}\n\n",
        "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_TOOL_CALL\",\"id\":\"tc-1\",\"title\":\"lookup\"}\n\n",
        "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_TOOL_RESULT\",\"id\":\"tc-1\",\"content\":\"ok\"}\n\n",
        "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_RESPONSE\",\"response\":{\"response\":\"hello from intel\"}}\n\n",
        "event: done\n\n",
    )
    .to_owned()
}

fn error_sse() -> String {
    concat!(
        "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_THINKING\"}\n\n",
        "data: {\"event_type\":\"MESSAGE_EVENT_TYPE_ERROR\",\"code\":\"TURN_FAILED\",\"message\":\"model exploded\"}\n\n",
    )
    .to_owned()
}

fn response_sse(text: &str, include_done: bool) -> String {
    let mut body = format!(
        "data: {}\n\n",
        json!({
            "event_type": "MESSAGE_EVENT_TYPE_RESPONSE",
            "response": { "response": text },
        })
    );
    if include_done {
        body.push_str("event: done\n\n");
    }
    body
}

fn midturn_error_sse(code: &str, internal_message: &str) -> String {
    format!(
        concat!(
            "data: {{\"event_type\":\"MESSAGE_EVENT_TYPE_THINKING\"}}\n\n",
            "data: {{\"event_type\":\"MESSAGE_EVENT_TYPE_TOOL_CALL\",",
            "\"id\":\"tc-before-failure\",\"title\":\"started-before-failure\"}}\n\n",
            "data: {{\"event_type\":\"MESSAGE_EVENT_TYPE_ERROR\",",
            "\"code\":{code},\"message\":{message}}}\n\n",
        ),
        code = serde_json::to_string(code).unwrap(),
        message = serde_json::to_string(internal_message).unwrap(),
    )
}

fn sse_response(body: String) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header("x-request-id", "test-rid")
        .body(Body::from(body))
        .unwrap()
}

/// Once a 200 SSE response has begun, HTTP cannot send a second status line.
/// The gateway's mid-turn status is therefore represented by its ERROR frame;
/// response metadata such as Retry-After and x-request-id remains in headers.
fn midturn_error_response(
    status_code: &str,
    internal_message: &str,
    request_id: &str,
    retry_after: Option<&str>,
) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header("x-request-id", request_id);
    if let Some(value) = retry_after {
        builder = builder.header(header::RETRY_AFTER, value);
    }
    builder
        .body(Body::from(midturn_error_sse(status_code, internal_message)))
        .unwrap()
}

async fn post_message(
    State(st): State<MockState>,
    Path(session_id): Path<String>,
    Json(_body): Json<Value>,
) -> Response {
    st.messages_hits.fetch_add(1, Ordering::SeqCst);
    let attempt = {
        let mut map = st.per_session_msgs.lock().await;
        let e = map.entry(session_id.clone()).or_insert(0);
        *e += 1;
        *e
    };

    match st.scenario {
        Scenario::Unauthorized => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-request-id", "auth-fail")
            .body(Body::from(
                r#"{"error":{"code":"UNAUTHENTICATED","message":"bad key"}}"#,
            ))
            .unwrap(),
        Scenario::ConflictThenOk => {
            // First messages hit on the first session → 409; subsequent sessions ok.
            if attempt == 1 && session_id == "intel-sess-1" {
                Response::builder()
                    .status(StatusCode::CONFLICT)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"detail":"session stopped"}"#))
                    .unwrap()
            } else {
                sse_response(happy_sse())
            }
        }
        Scenario::SseError => sse_response(error_sse()),
        Scenario::MidTurnTooManyRequests => midturn_error_response(
            "HTTP_429",
            "rate limiter shard=private-red retry ledger internals",
            "midturn-429-rid",
            Some("17"),
        ),
        Scenario::MidTurnServiceUnavailable => midturn_error_response(
            "HTTP_503",
            "upstream pool=secret-blue stack=private-handler",
            "midturn-503-rid",
            None,
        ),
        Scenario::MidTurnUnauthorized => midturn_error_response(
            "HTTP_401",
            "credential fingerprint=private-fingerprint was revoked",
            "midturn-401-rid",
            None,
        ),
        Scenario::CleanEofAfterPartialResponse => {
            sse_response(response_sse("PARTIAL-CFO-NUMBER-742", false))
        }
        Scenario::DoneWithEmptyResponse => sse_response(response_sse("", true)),
        Scenario::DoneWithWhitespaceResponse => sse_response(response_sse(" \t  ", true)),
        Scenario::HappyMultiFrame => sse_response(happy_sse()),
    }
}

async fn relay_events(State(st): State<MockState>, Json(body): Json<Value>) -> impl IntoResponse {
    st.relay_posts.fetch_add(1, Ordering::SeqCst);
    st.relay_bodies.lock().await.push(body);
    (StatusCode::OK, Json(json!({"ok": true})))
}

async fn relay_query() -> impl IntoResponse {
    // Empty array → adapter falls back to parent-as-root.
    (StatusCode::OK, Json(json!([])))
}

async fn spawn_gateway(scenario: Scenario) -> (String, MockState) {
    let st = MockState::new(scenario);
    let app = Router::new()
        .route("/v1/agents", get(get_agents))
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/{id}/messages", post(post_message))
        // Relay stubs on the same listener so one URL works for both.
        .route("/events", post(relay_events))
        .route("/query", post(relay_query))
        .with_state(st.clone());

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), st)
}

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
    _state_dir: TempDir,
}

impl Harness {
    async fn spawn(gateway: &str) -> Self {
        Self::spawn_with_env(gateway, &[]).await
    }

    /// Spawn with extra environment, for tests that need non-default config.
    async fn spawn_with_env(gateway: &str, extra_env: &[(&str, &str)]) -> Self {
        let state_dir = TempDir::new().unwrap();
        let bin = env!("CARGO_BIN_EXE_buzz-intel-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("INTEL_GATEWAY_URL", gateway)
            .env("INTEL_API_KEY", "intel_test_key")
            .env("INTEL_AGENT", AGENT_NAME)
            .env("INTEL_STATE_DIR", state_dir.path())
            .env("INTEL_ERROR_REPLIES", "true")
            .env("INTEL_FORWARD_SYSTEM_PROMPT", "never")
            .env("INTEL_CONNECT_TIMEOUT_SECS", "5")
            .env("INTEL_SSE_IDLE_TIMEOUT_SECS", "10")
            .env("INTEL_TURN_TIMEOUT_SECS", "30")
            .env("INTEL_KEEPALIVE_SECS", "3600") // effectively disable keepalive noise
            .env("BUZZ_RELAY_URL", gateway)
            .env("BUZZ_PRIVATE_KEY", TEST_SK)
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn buzz-intel-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            _state_dir: state_dir,
        }
    }

    async fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write_json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await;
        id
    }

    async fn write_json(&mut self, msg: Value) {
        let mut s = serde_json::to_string(&msg).unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(15), self.stdout.read_line(&mut line))
            .await
            .expect("recv timeout")
            .expect("read line");
        assert!(n > 0, "adapter EOF");
        serde_json::from_str(&line).unwrap_or_else(|e| {
            panic!("non-JSON line from adapter: {e}; line={line:?}");
        })
    }

    async fn recv_for_id(&mut self, id: i64) -> (Value, Vec<Value>) {
        let mut notifications = Vec::new();
        loop {
            let v = self.recv().await;
            if v.get("id") == Some(&json!(id)) {
                return (v, notifications);
            }
            if v.get("method").and_then(Value::as_str) == Some("session/update") {
                notifications.push(v);
            }
        }
    }

    async fn initialize(&mut self) {
        let id = self
            .send(
                "initialize",
                json!({
                    "protocolVersion": 2,
                    "clientCapabilities": {}
                }),
            )
            .await;
        let (resp, _) = self.recv_for_id(id).await;
        assert!(resp.get("result").is_some(), "initialize failed: {resp}");
        assert_eq!(resp["result"]["protocolVersion"], 2);
    }

    async fn session_new(&mut self) -> String {
        let id = self
            .send(
                "session/new",
                json!({
                    "cwd": "/tmp",
                    "mcpServers": [],
                    "systemPrompt": "you are a test agent"
                }),
            )
            .await;
        let (resp, _) = self.recv_for_id(id).await;
        resp["result"]["sessionId"]
            .as_str()
            .expect("sessionId")
            .to_owned()
    }

    async fn prompt(&mut self, session_id: &str, text: &str) -> (String, Vec<Value>) {
        let id = self
            .send(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await;
        let (resp, updates) = self.recv_for_id(id).await;
        let reason = resp["result"]["stopReason"]
            .as_str()
            .unwrap_or_else(|| panic!("missing stopReason: {resp}"))
            .to_owned();
        (reason, updates)
    }

    async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.start_kill();
    }
}

fn harness_prompt_named() -> String {
    format!(
        "Event ID: {EVENT_ID}\n\
         Channel: general (#{CHANNEL_ID})\n\
         Kind: 9\n\
         From: Alice (npub: npub1test, hex: deadbeef)\n\
         Time: 2026-07-24T00:00:00Z\n\
         Content: @agent hello\n\
         Tags: [[\"h\",\"{CHANNEL_ID}\"]]\n\
\n\
IMPORTANT: For ordinary replies in this turn, use `--reply-to {EVENT_ID}` \
on `buzz messages send` so the conversation stays threaded. \
If the human explicitly asks for a channel-root, top-level, \
or broadcast post, send that message without `--reply-to`. \
If the requested destination is ambiguous, ask before sending."
    )
}

fn update_kinds(updates: &[Value]) -> Vec<String> {
    updates
        .iter()
        .filter_map(|u| {
            u.pointer("/params/update/sessionUpdate")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

async fn assert_midturn_gateway_failure(
    scenario: Scenario,
    expected_request_id: &str,
    forbidden_gateway_details: &[&str],
) {
    let (gateway, st) = spawn_gateway(scenario).await;
    let mut h = Harness::spawn_with_env(
        &gateway,
        &[
            ("INTEL_SSE_IDLE_TIMEOUT_SECS", "2"),
            ("INTEL_TURN_TIMEOUT_SECS", "5"),
        ],
    )
    .await;
    h.initialize().await;
    let sid = h.session_new().await;

    let (reason, updates) = tokio::time::timeout(
        Duration::from_secs(6),
        h.prompt(&sid, &harness_prompt_named()),
    )
    .await
    .unwrap_or_else(|_| panic!("{scenario:?} hung past the configured turn bound"));

    assert_eq!(
        reason, "end_turn",
        "{scenario:?} should terminate the failed stream cleanly"
    );

    let kinds = update_kinds(&updates);
    assert!(
        kinds.iter().any(|kind| kind == "agent_thought_chunk"),
        "{scenario:?} must deliver a frame before failing, got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "tool_call"),
        "{scenario:?} must fail after the stream has made progress, got {kinds:?}"
    );

    assert_eq!(
        st.relay_posts.load(Ordering::SeqCst),
        1,
        "{scenario:?} must post exactly one owner-visible error"
    );
    let relay_bodies = st.relay_bodies.lock().await;
    let owner_text = relay_bodies
        .iter()
        .find_map(|body| body.get("content").and_then(Value::as_str))
        .unwrap_or_else(|| panic!("{scenario:?} relay post had no event content"));
    assert!(
        owner_text.starts_with("⚠️ Intel platform error"),
        "{scenario:?} owner-visible message was not a safe platform error: {owner_text:?}"
    );
    assert!(
        owner_text.contains(expected_request_id),
        "{scenario:?} lost gateway request id {expected_request_id:?}: {owner_text:?}"
    );
    for forbidden in forbidden_gateway_details {
        assert!(
            !owner_text.contains(forbidden),
            "{scenario:?} leaked gateway detail {forbidden:?}: {owner_text:?}"
        );
    }

    drop(relay_bodies);
    h.shutdown().await;
}

async fn assert_incomplete_answer_is_visible_error(
    scenario: Scenario,
    forbidden_answer_marker: Option<&str>,
) {
    let (gateway, st) = spawn_gateway(scenario).await;
    let mut h = Harness::spawn_with_env(
        &gateway,
        &[
            ("INTEL_SSE_IDLE_TIMEOUT_SECS", "2"),
            ("INTEL_TURN_TIMEOUT_SECS", "5"),
        ],
    )
    .await;
    h.initialize().await;
    let sid = h.session_new().await;

    let (reason, updates) = tokio::time::timeout(
        Duration::from_secs(6),
        h.prompt(&sid, &harness_prompt_named()),
    )
    .await
    .unwrap_or_else(|_| panic!("{scenario:?} hung past the configured turn bound"));
    assert_eq!(reason, "end_turn");

    if let Some(marker) = forbidden_answer_marker {
        assert!(
            !updates.iter().any(|update| {
                update
                    .pointer("/params/update/content/text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains(marker))
            }),
            "{scenario:?} exposed incomplete answer marker {marker:?} in ACP updates: {updates:?}"
        );
    }

    assert_eq!(
        st.relay_posts.load(Ordering::SeqCst),
        1,
        "{scenario:?} must post exactly one owner-visible error"
    );
    let relay_bodies = st.relay_bodies.lock().await;
    let owner_text = relay_bodies
        .iter()
        .find_map(|body| body.get("content").and_then(Value::as_str))
        .unwrap_or_else(|| panic!("{scenario:?} relay post had no event content"));
    assert!(
        owner_text.starts_with("⚠️ Intel platform error"),
        "{scenario:?} owner-visible message was not a safe platform error: {owner_text:?}"
    );
    assert!(
        owner_text.contains("test-rid"),
        "{scenario:?} lost gateway request id: {owner_text:?}"
    );
    if let Some(marker) = forbidden_answer_marker {
        assert!(
            !owner_text.contains(marker),
            "{scenario:?} published incomplete answer marker {marker:?}: {owner_text:?}"
        );
    }

    drop(relay_bodies);
    h.shutdown().await;
}

#[tokio::test]
async fn e2e_clean_eof_without_terminal_does_not_publish_partial_response() {
    assert_incomplete_answer_is_visible_error(
        Scenario::CleanEofAfterPartialResponse,
        Some("PARTIAL-CFO-NUMBER-742"),
    )
    .await;
}

#[tokio::test]
async fn e2e_done_with_empty_or_whitespace_response_posts_safe_error() {
    for scenario in [
        Scenario::DoneWithEmptyResponse,
        Scenario::DoneWithWhitespaceResponse,
    ] {
        assert_incomplete_answer_is_visible_error(scenario, None).await;
    }
}

#[tokio::test]
async fn e2e_happy_path_multi_frame_and_one_session_per_channel() {
    let (gateway, st) = spawn_gateway(Scenario::HappyMultiFrame).await;
    let mut h = Harness::spawn(&gateway).await;
    h.initialize().await;
    let sid = h.session_new().await;

    let prompt = harness_prompt_named();
    let (reason1, updates1) = h.prompt(&sid, &prompt).await;
    assert_eq!(reason1, "end_turn");

    let kinds = update_kinds(&updates1);
    // Stream frames + final agent_message_chunk
    assert!(
        kinds.iter().any(|k| k == "agent_thought_chunk"),
        "expected thought chunk, got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "tool_call"),
        "expected tool_call, got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "tool_call_update"),
        "expected tool_call_update, got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "agent_message_chunk"),
        "expected final agent_message_chunk, got {kinds:?}"
    );

    // Final message text
    let final_text = updates1.iter().rev().find_map(|u| {
        if u.pointer("/params/update/sessionUpdate")
            .and_then(Value::as_str)
            == Some("agent_message_chunk")
        {
            u.pointer("/params/update/content/text")
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            None
        }
    });
    assert_eq!(final_text.as_deref(), Some("hello from intel"));

    // Second prompt same channel → must reuse intel session.
    let (reason2, _) = h.prompt(&sid, &prompt).await;
    assert_eq!(reason2, "end_turn");

    assert_eq!(
        st.sessions_created.load(Ordering::SeqCst),
        1,
        "exactly one intel session per channel across two prompts"
    );
    assert!(
        st.messages_hits.load(Ordering::SeqCst) >= 2,
        "two message posts expected"
    );
    // Reply published to relay stub (once per successful non-empty response).
    assert!(
        st.relay_posts.load(Ordering::SeqCst) >= 2,
        "expected relay posts for replies"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn e2e_unauthorized_returns_refusal() {
    let (gateway, st) = spawn_gateway(Scenario::Unauthorized).await;
    let mut h = Harness::spawn(&gateway).await;
    h.initialize().await;
    let sid = h.session_new().await;
    let (reason, _updates) = h.prompt(&sid, &harness_prompt_named()).await;
    assert_eq!(reason, "refusal");
    assert_eq!(st.sessions_created.load(Ordering::SeqCst), 1);
    h.shutdown().await;
}

/// The quota must refuse the second turn *without spending a gateway call*.
///
/// Asserting on the mock's hit counters is the point: a quota that only changed
/// the stop reason while still calling the gateway would pass a reply-shaped
/// assertion but save no money.
#[tokio::test]
async fn e2e_turn_quota_refuses_without_calling_gateway() {
    let (gateway, st) = spawn_gateway(Scenario::HappyMultiFrame).await;
    let mut h = Harness::spawn_with_env(
        &gateway,
        &[
            ("INTEL_MAX_TURNS_PER_WINDOW", "1"),
            ("INTEL_QUOTA_WINDOW_SECS", "3600"),
        ],
    )
    .await;
    h.initialize().await;
    let sid = h.session_new().await;

    let (first, _) = h.prompt(&sid, &harness_prompt_named()).await;
    assert_eq!(first, "end_turn", "first turn is within quota");
    let messages_after_first = st.messages_hits.load(Ordering::SeqCst);
    assert_eq!(messages_after_first, 1, "first turn calls the gateway once");

    let (second, _) = h.prompt(&sid, &harness_prompt_named()).await;
    assert_eq!(second, "refusal", "second turn is over quota");
    assert_eq!(
        st.messages_hits.load(Ordering::SeqCst),
        messages_after_first,
        "a throttled turn must not reach the gateway"
    );
    assert_eq!(
        st.sessions_created.load(Ordering::SeqCst),
        1,
        "a throttled turn must not create an intel session either"
    );
    assert!(
        st.relay_posts.load(Ordering::SeqCst) >= 2,
        "the refusal must be visible in the channel, not silent"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn e2e_missing_channel_cannot_bypass_quota_with_new_acp_session() {
    let (gateway, st) = spawn_gateway(Scenario::HappyMultiFrame).await;
    let mut h = Harness::spawn_with_env(
        &gateway,
        &[
            ("INTEL_MAX_TURNS_PER_WINDOW", "1"),
            ("INTEL_QUOTA_WINDOW_SECS", "3600"),
        ],
    )
    .await;
    h.initialize().await;
    let first_sid = h.session_new().await;
    let second_sid = h.session_new().await;
    let channel_less_prompt = "direct ACP prompt without Buzz channel metadata";

    let (first, _) = h.prompt(&first_sid, channel_less_prompt).await;
    assert_eq!(first, "end_turn", "first turn is within quota");
    let messages_after_first = st.messages_hits.load(Ordering::SeqCst);
    let sessions_after_first = st.sessions_created.load(Ordering::SeqCst);
    assert_eq!(messages_after_first, 1, "first turn calls the gateway once");
    assert_eq!(
        sessions_after_first, 1,
        "first turn creates one intel session"
    );

    let (second, _) = h.prompt(&second_sid, channel_less_prompt).await;
    assert_eq!(
        second, "refusal",
        "a new ACP session must not reset the channel-less quota budget"
    );
    assert_eq!(
        st.messages_hits.load(Ordering::SeqCst),
        messages_after_first,
        "the refused second turn must not send another gateway message"
    );
    assert_eq!(
        st.sessions_created.load(Ordering::SeqCst),
        sessions_after_first,
        "the refused second turn must not create another intel session"
    );

    h.shutdown().await;
}

/// Setting the limit to 0 disables enforcement rather than blocking everything.
#[tokio::test]
async fn e2e_turn_quota_zero_is_disabled() {
    let (gateway, st) = spawn_gateway(Scenario::HappyMultiFrame).await;
    let mut h = Harness::spawn_with_env(&gateway, &[("INTEL_MAX_TURNS_PER_WINDOW", "0")]).await;
    h.initialize().await;
    let sid = h.session_new().await;

    for _ in 0..3 {
        let (reason, _) = h.prompt(&sid, &harness_prompt_named()).await;
        assert_eq!(reason, "end_turn", "quota disabled must not refuse");
    }
    assert_eq!(
        st.messages_hits.load(Ordering::SeqCst),
        3,
        "all three turns reach the gateway when the quota is off"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn e2e_conflict_recreates_session_once() {
    let (gateway, st) = spawn_gateway(Scenario::ConflictThenOk).await;
    let mut h = Harness::spawn(&gateway).await;
    h.initialize().await;
    let sid = h.session_new().await;
    let (reason, updates) = h.prompt(&sid, &harness_prompt_named()).await;
    assert_eq!(reason, "end_turn");
    assert_eq!(
        st.sessions_created.load(Ordering::SeqCst),
        2,
        "409 should recreate session once"
    );
    assert!(
        st.messages_hits.load(Ordering::SeqCst) >= 2,
        "original + retry message posts"
    );
    let kinds = update_kinds(&updates);
    assert!(
        kinds.iter().any(|k| k == "agent_message_chunk"),
        "expected success after recreate, got {kinds:?}"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn e2e_sse_error_frame_end_turn() {
    let (gateway, st) = spawn_gateway(Scenario::SseError).await;
    let mut h = Harness::spawn(&gateway).await;
    h.initialize().await;
    let sid = h.session_new().await;
    let (reason, updates) = h.prompt(&sid, &harness_prompt_named()).await;
    assert_eq!(reason, "end_turn");
    // Thought from THINKING before ERROR; no successful agent_message_chunk with intel text.
    let kinds = update_kinds(&updates);
    assert!(
        kinds.iter().any(|k| k == "agent_thought_chunk"),
        "expected thinking before error, got {kinds:?}"
    );
    // ERROR path posts owner-visible ⚠️ reply when INTEL_ERROR_REPLIES=true.
    assert!(
        st.relay_posts.load(Ordering::SeqCst) >= 1,
        "error reply should be posted"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn e2e_midturn_429_retry_after_posts_safe_owner_error() {
    assert_midturn_gateway_failure(
        Scenario::MidTurnTooManyRequests,
        "midturn-429-rid",
        &["private-red", "retry ledger", "shard="],
    )
    .await;
}

#[tokio::test]
async fn e2e_midturn_503_after_frames_posts_safe_owner_error() {
    assert_midturn_gateway_failure(
        Scenario::MidTurnServiceUnavailable,
        "midturn-503-rid",
        &["secret-blue", "private-handler", "stack="],
    )
    .await;
}

#[tokio::test]
async fn e2e_midturn_401_revoked_credentials_posts_safe_owner_error() {
    assert_midturn_gateway_failure(
        Scenario::MidTurnUnauthorized,
        "midturn-401-rid",
        &["private-fingerprint", "credential fingerprint", "revoked"],
    )
    .await;
}

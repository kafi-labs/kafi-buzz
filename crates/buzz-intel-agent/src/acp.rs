//! ACP server loop: initialize / session/new / session/prompt / session/cancel.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use rand::RngExt;
use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::sync::{mpsc, watch, Mutex};
use uuid::Uuid;

use crate::config::{Config, EntityMode, ForwardSystemPrompt, SessionMode, PROTOCOL_VERSION};
use crate::error::AdapterError;
use crate::intel::{FrameKind, IntelClient, SseFrame};
use crate::prompt::parse_prompt;
use crate::quota::TurnQuota;
use crate::reply::RelayPublisher;
use crate::session_ensure::{get_or_create_intel_session, CreateLockMap};
use crate::state::StateStore;
use crate::wire::{
    self, classify, prompt_to_text, Inbound, InitializeParams, SessionCancelParams,
    SessionNewParams, SessionPromptParams, WireMsg, WireSender, INVALID_PARAMS, METHOD_NOT_FOUND,
    PARSE_ERROR,
};

/// Max wait when pushing a session/update under stdout backpressure.
const SESSION_UPDATE_SEND_TIMEOUT: Duration = Duration::from_millis(500);

/// Bound for the outbound wire channel (session/update + responses).
const WIRE_CHANNEL_CAP: usize = 256;

/// Local ACP session state.
struct AcpSession {
    system_prompt: Option<String>,
    cancel_tx: watch::Sender<bool>,
    busy: bool,
    /// Turn epoch — incremented on cancel so late replies are suppressed.
    epoch: u64,
}

/// Process-wide app state.
struct App {
    cfg: Config,
    intel: IntelClient,
    state: Mutex<StateStore>,
    agent_id: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, AcpSession>>,
    /// Single-flight locks for intel session creation per mapping key.
    create_locks: CreateLockMap,
    relay: Option<RelayPublisher>,
    /// Per-scope LLM turn quota, checked before any paid gateway call.
    quota: Mutex<TurnQuota>,
}

/// Run the ACP NDJSON server until stdin EOF or SIGTERM.
pub async fn run_server(cfg: Config) -> Result<(), AdapterError> {
    let intel = IntelClient::new(&cfg)?;
    let state = StateStore::load(&cfg.state_path)?;

    let relay = match (&cfg.relay_url, &cfg.private_key) {
        (Some(url), Some(key)) => match RelayPublisher::new(url, key, cfg.auth_tag.as_deref()) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("relay publisher unavailable at startup: {e}");
                None
            }
        },
        _ => {
            tracing::warn!(
                "BUZZ_RELAY_URL / BUZZ_PRIVATE_KEY not set — replies will not be published"
            );
            None
        }
    };

    let cached_agent = state.agent_id().map(str::to_owned);
    let app = Arc::new(App {
        cfg: cfg.clone(),
        intel,
        state: Mutex::new(state),
        agent_id: Mutex::new(cached_agent),
        sessions: Mutex::new(HashMap::new()),
        create_locks: Mutex::new(HashMap::new()),
        relay,
        quota: Mutex::new(TurnQuota::new(cfg.quota)),
    });

    let (wire_tx, wire_rx) = mpsc::channel::<WireMsg>(WIRE_CHANNEL_CAP);
    let writer = tokio::spawn(wire::writer_task(wire_rx));

    let max_line = cfg.max_line_bytes;
    let reader_app = app.clone();
    let reader_tx = wire_tx.clone();
    let read_fut = async {
        if let Err(e) = read_loop(
            BufReader::new(tokio::io::stdin()),
            reader_app,
            reader_tx,
            max_line,
        )
        .await
        {
            tracing::error!("io: reader: {e}");
        }
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).map_err(|e| {
            AdapterError::Io(std::io::Error::other(format!("sigterm handler: {e}")))
        })?;
        tokio::select! {
            _ = read_fut => {
                tracing::info!("stdin EOF — shutting down");
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM — shutting down");
            }
        }
    }
    #[cfg(not(unix))]
    {
        read_fut.await;
    }

    // Graceful shutdown: cancel in-flight turns and flush state.
    graceful_shutdown(&app).await;

    drop(wire_tx);
    let _ = writer.await;
    Ok(())
}

async fn graceful_shutdown(app: &App) {
    {
        let sessions = app.sessions.lock().await;
        for s in sessions.values() {
            let _ = s.cancel_tx.send(true);
        }
    }
    if let Err(e) = app.state.lock().await.flush() {
        tracing::warn!("state flush on shutdown: {e}");
    }
}

async fn read_loop<R: tokio::io::AsyncBufRead + Unpin>(
    mut stdin: R,
    app: Arc<App>,
    wire_tx: WireSender,
    max_line: usize,
) -> std::io::Result<()> {
    while let Some(line) = wire::read_bounded_line(&mut stdin, max_line).await? {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(msg) => dispatch(&app, msg, &wire_tx).await,
            Err(e) => {
                wire::send(
                    &wire_tx,
                    wire::err(Value::Null, PARSE_ERROR, &format!("jsonrpc: parse: {e}")),
                )
                .await;
            }
        }
    }
    Ok(())
}

async fn dispatch(app: &Arc<App>, msg: Value, wire_tx: &WireSender) {
    match classify(&msg) {
        Inbound::Request { id, method, params } => {
            handle_request(app, id, method, params, wire_tx).await
        }
        Inbound::Notification { method, params } => {
            if method == "session/cancel" {
                cancel_session(app, params).await;
            }
        }
        Inbound::Ignored => {}
        Inbound::Invalid { id, code, message } => {
            wire::send(wire_tx, wire::err(id, code, &message)).await
        }
    }
}

async fn handle_request(
    app: &Arc<App>,
    id: Value,
    method: String,
    params: Value,
    wire_tx: &WireSender,
) {
    match method.as_str() {
        "initialize" => initialize(app, id, params, wire_tx).await,
        "session/new" => session_new(app, id, params, wire_tx).await,
        "session/prompt" => {
            let app = app.clone();
            let wire_tx = wire_tx.clone();
            tokio::spawn(async move {
                // Contain panics so session.busy is cleared and the client
                // receives a JSON-RPC error instead of hanging forever.
                let session_id_hint = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let id_for_err = id.clone();
                let outcome = AssertUnwindSafe(session_prompt(&app, id, params, &wire_tx))
                    .catch_unwind()
                    .await;
                if let Err(panic) = outcome {
                    let detail = panic_message(&panic);
                    tracing::error!("session/prompt panicked: {detail}");
                    if let Some(ref sid) = session_id_hint {
                        let mut sessions = app.sessions.lock().await;
                        if let Some(s) = sessions.get_mut(sid) {
                            s.busy = false;
                            let _ = s.cancel_tx.send(true);
                        }
                    }
                    wire::send(
                        &wire_tx,
                        wire::err(
                            id_for_err,
                            -32000,
                            &format!("session/prompt internal error (panic contained): {detail}"),
                        ),
                    )
                    .await;
                }
            });
        }
        "session/cancel" => {
            cancel_session(app, params).await;
            wire::send(wire_tx, wire::ok(id, Value::Null)).await;
        }
        _ => {
            wire::send(
                wire_tx,
                wire::err(
                    id,
                    METHOD_NOT_FOUND,
                    &format!("jsonrpc: method not found: {method}"),
                ),
            )
            .await
        }
    }
}

async fn initialize(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: InitializeParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                &format!("initialize: invalid params: {e}"),
            )
            .await;
        }
    };

    // Fail-fast config: missing credentials surface as a clear JSON-RPC error
    // (process may start without them so the harness can display the message).
    if let Err(e) = app.cfg.require_acp_runtime_config() {
        tracing::error!("initialize failed (config): {e}");
        return reject(
            wire_tx,
            id,
            e.json_rpc_code(),
            &format!("initialize failed — configuration error (respawn will not fix this): {e}"),
        )
        .await;
    }

    // Fail-fast: probe gateway and resolve agent id.
    let agent_id = match app.intel.resolve_agent_id(&app.cfg.agent).await {
        Ok(aid) => aid,
        Err(e) => {
            tracing::error!("initialize failed: {e}");
            return reject(
                wire_tx,
                id,
                e.json_rpc_code(),
                &format!(
                    "initialize failed — check INTEL_GATEWAY_URL / INTEL_API_KEY / INTEL_AGENT: {e}"
                ),
            )
            .await;
        }
    };

    {
        let mut cached = app.agent_id.lock().await;
        *cached = Some(agent_id.clone());
    }
    if let Err(e) = app.state.lock().await.set_agent_id(agent_id.clone()) {
        tracing::warn!("state: could not persist agent_id: {e}");
    }

    let negotiated = p.protocol_version.min(PROTOCOL_VERSION);
    wire::send(
        wire_tx,
        wire::ok(
            id,
            json!({
                "protocolVersion": negotiated,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": {
                        "image": false,
                        "audio": false,
                        "embeddedContext": false
                    },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                "agentInfo": {
                    "name": "buzz-intel-agent",
                    "version": env!("CARGO_PKG_VERSION"),
                    "intelAgentId": agent_id,
                    "intelAgent": app.cfg.agent,
                },
            }),
        ),
    )
    .await;
}

async fn session_new(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionNewParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                &format!("session/new: invalid params: {e}"),
            )
            .await;
        }
    };
    if p.cwd.is_empty() || !Path::new(&p.cwd).is_absolute() {
        return reject(
            wire_tx,
            id,
            INVALID_PARAMS,
            "session/new: cwd must be an absolute path",
        )
        .await;
    }

    // mcpServers accepted and ignored (MVP).
    let _ = p.mcp_servers;

    let session_id = format!("ses_{}", Uuid::new_v4());
    let (cancel_tx, _) = watch::channel(false);
    let session = AcpSession {
        system_prompt: p.system_prompt.filter(|s| !s.trim().is_empty()),
        cancel_tx,
        busy: false,
        epoch: 0,
    };
    app.sessions
        .lock()
        .await
        .insert(session_id.clone(), session);

    wire::send(wire_tx, wire::ok(id, json!({ "sessionId": session_id }))).await;
}

async fn cancel_session(app: &Arc<App>, params: Value) {
    let p: SessionCancelParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("session/cancel: bad params: {e}");
            return;
        }
    };
    let mut sessions = app.sessions.lock().await;
    if let Some(s) = sessions.get_mut(&p.session_id) {
        s.epoch = s.epoch.saturating_add(1);
        let _ = s.cancel_tx.send(true);
        tracing::info!(
            session_id = %p.session_id,
            epoch = s.epoch,
            "session/cancel: aborting in-flight turn"
        );
    }
}

async fn session_prompt(app: &Arc<App>, id: Value, params: Value, wire_tx: &WireSender) {
    let p: SessionPromptParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                &format!("session/prompt: invalid params: {e}"),
            )
            .await;
        }
    };

    // Mark busy + grab cancel receiver + epoch + system prompt.
    let (mut cancel_rx, epoch, system_prompt) = {
        let mut sessions = app.sessions.lock().await;
        let Some(s) = sessions.get_mut(&p.session_id) else {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                &format!("session/prompt: unknown session {}", p.session_id),
            )
            .await;
        };
        if s.busy {
            return reject(
                wire_tx,
                id,
                INVALID_PARAMS,
                "session/prompt: session already has an in-flight prompt",
            )
            .await;
        }
        // Reset cancel flag for this turn.
        let _ = s.cancel_tx.send(false);
        s.busy = true;
        let rx = s.cancel_tx.subscribe();
        (rx, s.epoch, s.system_prompt.clone())
    };

    let prompt_text = prompt_to_text(p.prompt);
    let parsed = parse_prompt(&prompt_text);

    let turn_result = tokio::time::timeout(
        app.cfg.turn_timeout,
        run_turn(
            app,
            &p.session_id,
            &prompt_text,
            &parsed,
            system_prompt.as_deref(),
            epoch,
            &mut cancel_rx,
            wire_tx,
        ),
    )
    .await;

    // Clear busy.
    {
        let mut sessions = app.sessions.lock().await;
        if let Some(s) = sessions.get_mut(&p.session_id) {
            s.busy = false;
        }
    }

    // If epoch advanced (cancel), always report cancelled.
    let current_epoch = {
        let sessions = app.sessions.lock().await;
        sessions
            .get(&p.session_id)
            .map(|s| s.epoch)
            .unwrap_or(epoch)
    };
    if current_epoch != epoch {
        wire::send(wire_tx, wire::ok(id, json!({ "stopReason": "cancelled" }))).await;
        return;
    }

    let stop_reason = match turn_result {
        Ok(Ok(reason)) => reason,
        Ok(Err(AdapterError::Cancelled)) => "cancelled".to_owned(),
        Ok(Err(AdapterError::IntelAuth(msg))) => {
            tracing::error!("turn auth failure: {msg}");
            "refusal".to_owned()
        }
        Ok(Err(e)) => {
            tracing::error!("turn failed: {e}");
            "end_turn".to_owned()
        }
        Err(_) => {
            tracing::error!(
                "turn exceeded INTEL_TURN_TIMEOUT_SECS ({})",
                app.cfg.turn_timeout.as_secs()
            );
            if app.cfg.error_replies {
                let _ = post_error_reply(
                    app,
                    parsed.channel_id,
                    parsed.reply_to_event_id.as_deref(),
                    "⚠️ Intel turn timed out",
                    epoch,
                    &p.session_id,
                )
                .await;
            }
            "end_turn".to_owned()
        }
    };

    wire::send(wire_tx, wire::ok(id, json!({ "stopReason": stop_reason }))).await;
}

#[allow(clippy::too_many_arguments)]
async fn run_turn(
    app: &Arc<App>,
    acp_session_id: &str,
    prompt_text: &str,
    parsed: &crate::prompt::ParsedPrompt,
    system_prompt: Option<&str>,
    epoch: u64,
    cancel_rx: &mut watch::Receiver<bool>,
    wire_tx: &WireSender,
) -> Result<String, AdapterError> {
    if *cancel_rx.borrow() {
        return Err(AdapterError::Cancelled);
    }

    // Cost gate. This must sit ahead of `ensure_and_run`, because that path can
    // create an intel session as well as send the message — both are paid
    // gateway calls. Refusing here means a throttled turn costs nothing.
    if let Some(reason) = enforce_turn_quota(app, acp_session_id, parsed, epoch).await {
        return Ok(reason);
    }

    let agent_id = app.agent_id.lock().await.clone().ok_or_else(|| {
        AdapterError::Config("agent_id not resolved; call initialize first".into())
    })?;

    let mapping_key = session_mapping_key(app, acp_session_id, parsed)?;
    let entity_id = build_entity_id(app, parsed)?;

    // Ensure intel session (with one recreate-on-gone).
    let mut attempt = 0u8;
    loop {
        attempt += 1;
        match ensure_and_run(
            app,
            acp_session_id,
            &agent_id,
            &mapping_key,
            &entity_id,
            prompt_text,
            system_prompt,
            parsed,
            epoch,
            cancel_rx,
            wire_tx,
            attempt > 1,
        )
        .await
        {
            Ok(reason) => return Ok(reason),
            Err(AdapterError::IntelSessionGone(msg)) if attempt < 2 => {
                tracing::warn!("intel session gone ({msg}); recreating and retrying once");
                let mut state = app.state.lock().await;
                let _ = state.remove_session(&mapping_key);
                continue;
            }
            Err(AdapterError::IntelAuth(msg)) => {
                tracing::error!(error = %msg, "intel auth failure");
                if app.cfg.error_replies {
                    let _ = post_error_reply(
                        app,
                        parsed.channel_id,
                        parsed.reply_to_event_id.as_deref(),
                        &owner_visible_error(
                            "credentials rejected (401/403)",
                            extract_request_id(&msg),
                        ),
                        epoch,
                        acp_session_id,
                    )
                    .await;
                }
                return Err(AdapterError::IntelAuth(msg));
            }
            Err(AdapterError::Cancelled) => return Err(AdapterError::Cancelled),
            Err(e) => {
                // Transient: one jittered retry only if no SSE frame was received
                // is handled inside ensure_and_run; here we post error reply.
                tracing::error!(error = %e, "intel turn failed");
                if app.cfg.error_replies {
                    let (category, rid) = classify_owner_error(&e);
                    let _ = post_error_reply(
                        app,
                        parsed.channel_id,
                        parsed.reply_to_event_id.as_deref(),
                        &owner_visible_error(&category, rid.as_deref()),
                        epoch,
                        acp_session_id,
                    )
                    .await;
                }
                return Err(e);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn ensure_and_run(
    app: &Arc<App>,
    acp_session_id: &str,
    agent_id: &str,
    mapping_key: &str,
    entity_id: &str,
    prompt_text: &str,
    system_prompt: Option<&str>,
    parsed: &crate::prompt::ParsedPrompt,
    epoch: u64,
    cancel_rx: &mut watch::Receiver<bool>,
    wire_tx: &WireSender,
    force_new: bool,
) -> Result<String, AdapterError> {
    let (intel_session_id, is_new, already_forwarded) = get_or_create_intel_session(
        &app.state,
        &app.create_locks,
        mapping_key,
        force_new,
        entity_id,
        || app.intel.create_session(agent_id, entity_id),
    )
    .await?;

    let message = build_outbound_message(
        app.cfg.forward_system_prompt,
        is_new,
        already_forwarded,
        system_prompt,
        prompt_text,
    );

    let metadata = json!({
        "source": "buzz",
        "channel_id": parsed.channel_id.map(|u| u.to_string()),
        "reply_to": parsed.reply_to_event_id,
        "acp_session_id": acp_session_id,
    });

    // Keepalive task while SSE is quiet.
    let (activity_tx, mut activity_rx) = mpsc::channel::<()>(8);
    let keepalive = app.cfg.keepalive;
    let sid = acp_session_id.to_owned();
    let wire_ka = wire_tx.clone();
    let ka_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(keepalive) => {
                    wire::send(
                        &wire_ka,
                        wire::session_update(
                            &sid,
                            json!({
                                "sessionUpdate": "agent_thought_chunk",
                                "content": { "type": "text", "text": "…" }
                            }),
                        ),
                    )
                    .await;
                }
                msg = activity_rx.recv() => {
                    if msg.is_none() {
                        break;
                    }
                    // Activity resets the timer by looping.
                }
            }
        }
    });

    let mut received_any_frame = false;
    let mut first_try = true;
    let stream_result = loop {
        let activity_tx = activity_tx.clone();
        let result = app
            .intel
            .send_message_stream(
                &intel_session_id,
                &message,
                metadata.clone(),
                cancel_rx,
                |frame: &SseFrame| {
                    received_any_frame = true;
                    let _ = activity_tx.try_send(());
                    // Emit is async with a short timeout so thought/tool updates
                    // are not silently dropped under mild backpressure, yet the
                    // SSE loop never blocks indefinitely on a stuck stdout.
                    let wire_tx = wire_tx.clone();
                    let sid = acp_session_id.to_owned();
                    let frame = frame.clone();
                    async move {
                        emit_acp_frame(&wire_tx, &sid, &frame).await;
                    }
                },
            )
            .await;

        match result {
            Ok(r) => break Ok(r),
            Err(AdapterError::Cancelled) => break Err(AdapterError::Cancelled),
            Err(AdapterError::IntelSessionGone(e)) => break Err(AdapterError::IntelSessionGone(e)),
            Err(AdapterError::IntelAuth(e)) => break Err(AdapterError::IntelAuth(e)),
            Err(e) if first_try && !received_any_frame => {
                // One jittered retry only if no SSE frame was received.
                first_try = false;
                let delay = jitter_backoff();
                tracing::warn!("transient intel error ({e}); retrying once after {delay:?}");
                tokio::time::sleep(delay).await;
                if *cancel_rx.borrow() {
                    break Err(AdapterError::Cancelled);
                }
                continue;
            }
            Err(e) => break Err(e),
        }
    };

    drop(activity_tx);
    let _ = ka_handle.await;

    let stream = stream_result?;

    // Epoch check before posting.
    if !epoch_is_current(app, acp_session_id, epoch).await {
        return Err(AdapterError::Cancelled);
    }

    if !stream.terminal_received {
        tracing::error!(
            request_id = ?stream.request_id,
            "intel SSE ended without a terminal frame; discarding incomplete response"
        );
        if app.cfg.error_replies {
            // This unattended agent can answer money-adjacent questions. A partial
            // number presented as complete is more dangerous than a visible error.
            let text = owner_visible_error("incomplete response", stream.request_id.as_deref());
            notify_incomplete_answer(
                app,
                wire_tx,
                parsed.channel_id,
                parsed.reply_to_event_id.as_deref(),
                &text,
                epoch,
                acp_session_id,
            )
            .await;
        }
        return Ok("end_turn".into());
    }

    if let Some((code, msg)) = stream.stream_error {
        tracing::error!(
            code = ?code,
            error = %msg,
            request_id = ?stream.request_id,
            "intel SSE ERROR frame"
        );
        if app.cfg.error_replies {
            let category = match code.as_deref() {
                Some(c) => format!("runtime error [{c}]"),
                None => "runtime error".into(),
            };
            let _ = post_error_reply(
                app,
                parsed.channel_id,
                parsed.reply_to_event_id.as_deref(),
                &owner_visible_error(&category, stream.request_id.as_deref()),
                epoch,
                acp_session_id,
            )
            .await;
        }
        return Ok("end_turn".into());
    }

    let reply_text = stream.response_text.trim().to_owned();

    // Mark system prompt forwarded / touch LRU.
    {
        let mut state = app.state.lock().await;
        let mark = is_new
            || matches!(
                app.cfg.forward_system_prompt,
                ForwardSystemPrompt::FirstMessage
            ) && !already_forwarded
                && system_prompt.is_some();
        let _ = state.touch_session(mapping_key, mark || already_forwarded || is_new);
    }

    if reply_text.is_empty() {
        tracing::error!(
            request_id = ?stream.request_id,
            "intel SSE completed without a non-empty response"
        );
        if app.cfg.error_replies {
            // Do not make an unattended empty answer look like a successful turn.
            let text = owner_visible_error("empty response", stream.request_id.as_deref());
            notify_incomplete_answer(
                app,
                wire_tx,
                parsed.channel_id,
                parsed.reply_to_event_id.as_deref(),
                &text,
                epoch,
                acp_session_id,
            )
            .await;
        }
        return Ok("end_turn".into());
    }

    if let Some(channel) = parsed.channel_id {
        if let Some(ref relay) = app.relay {
            if epoch_is_current(app, acp_session_id, epoch).await {
                match relay
                    .post_message(channel, &reply_text, parsed.reply_to_event_id.as_deref())
                    .await
                {
                    Ok(Some(eid)) => {
                        tracing::info!(event_id = %eid, "posted reply to buzz");
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("failed to post reply: {e}");
                    }
                }
            }
        } else {
            tracing::warn!("no relay publisher; skipping buzz reply post");
        }
    } else {
        tracing::warn!("no channel_id in prompt; skipping buzz reply post");
    }

    // Final agent_message_chunk for desktop transcript coherence.
    if epoch_is_current(app, acp_session_id, epoch).await {
        wire::send(
            wire_tx,
            wire::session_update(
                acp_session_id,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": reply_text }
                }),
            ),
        )
        .await;
    }

    Ok("end_turn".into())
}

async fn emit_acp_frame(wire_tx: &WireSender, sid: &str, frame: &SseFrame) {
    let update = match frame.kind {
        FrameKind::Thinking => {
            // THINKING frames carry no text — emit generic placeholder.
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "thinking…" }
            })
        }
        FrameKind::ToolCall => {
            let id = frame
                .tool_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let title = frame
                .tool_title
                .clone()
                .unwrap_or_else(|| "tool".to_owned());
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": title,
                "status": "in_progress"
            })
        }
        FrameKind::ToolResult => {
            let id = frame
                .tool_id
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());
            let content = frame.tool_content.clone().unwrap_or_default();
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text", "text": content } }]
            })
        }
        FrameKind::Response => {
            // Accumulated at end; final agent_message_chunk is emitted after post.
            return;
        }
        FrameKind::Error | FrameKind::Done | FrameKind::Other(_) => return,
    };

    let msg = WireMsg::Notify(wire::session_update(sid, update));
    match tokio::time::timeout(SESSION_UPDATE_SEND_TIMEOUT, wire_tx.send(msg)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            tracing::warn!("session/update dropped: wire channel closed");
        }
        Err(_) => {
            tracing::warn!(
                "session/update dropped under backpressure after {}ms",
                SESSION_UPDATE_SEND_TIMEOUT.as_millis()
            );
        }
    }
}

/// Channel-visible error text: category + optional request id only (no gateway internals).
fn owner_visible_error(category: &str, request_id: Option<&str>) -> String {
    match request_id {
        Some(rid) if !rid.is_empty() => {
            format!("⚠️ Intel platform error ({category}; request id: {rid})")
        }
        _ => format!("⚠️ Intel platform error ({category})"),
    }
}

fn extract_request_id(msg: &str) -> Option<&str> {
    // Errors append ` x-request-id=<id>` (see intel::map_http_error).
    msg.split("x-request-id=")
        .nth(1)
        .map(|s| s.split_whitespace().next().unwrap_or(s).trim())
        .filter(|s| !s.is_empty())
}

fn classify_owner_error(e: &AdapterError) -> (String, Option<String>) {
    let full = e.to_string();
    let rid = extract_request_id(&full).map(str::to_owned);
    let category = match e {
        AdapterError::Intel(m)
            if m.contains("unreachable")
                || m.contains("connect")
                || m.contains("timeout")
                || m.contains("status 5") =>
        {
            "platform unreachable"
        }
        AdapterError::IntelAuth(_) => "credentials rejected",
        AdapterError::IntelSessionGone(_) => "session expired",
        _ => "turn failed",
    };
    (category.to_owned(), rid)
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".into()
    }
}

fn build_outbound_message(
    policy: ForwardSystemPrompt,
    is_new: bool,
    already_forwarded: bool,
    system_prompt: Option<&str>,
    prompt_text: &str,
) -> String {
    let should_forward = matches!(policy, ForwardSystemPrompt::FirstMessage)
        && is_new
        && !already_forwarded
        && system_prompt.map(|s| !s.trim().is_empty()).unwrap_or(false);

    if should_forward {
        if let Some(sp) = system_prompt {
            return format!("[Buzz harness context]\n```\n{sp}\n```\n\n{prompt_text}");
        }
    }
    prompt_text.to_owned()
}

/// Community component of the quota scope: the relay host, never the full URL
/// (which can carry a token in some deployments) and never the key.
fn quota_community(cfg: &Config) -> Option<String> {
    let raw = cfg.relay_url.as_deref()?;
    let without_scheme = raw.split_once("://").map(|(_, rest)| rest).unwrap_or(raw);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    // Strip any userinfo so credentials can never reach a log line.
    let host = host.rsplit('@').next().unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Build the quota scope key for this turn.
fn quota_scope_key(cfg: &Config, parsed: &crate::prompt::ParsedPrompt) -> String {
    let community = quota_community(cfg);
    let channel = parsed.channel_id.map(|c| c.to_string());
    // Channel-less ACP traffic intentionally shares one stable `nochannel`
    // budget per community+agent. One noisy direct-ACP client can exhaust it
    // for the others, but a cost control must fail toward refusal rather than
    // grant a fresh budget whenever a client creates a new ACP session.
    crate::quota::scope_key(community.as_deref(), channel.as_deref(), &cfg.agent)
}

/// Enforce the per-scope turn quota.
///
/// Returns `Some(stop_reason)` when the turn must not proceed. The caller
/// returns that reason as a normal ACP result — a throttled turn is a refusal,
/// not an adapter error, so the harness does not treat it as a crash and retry.
async fn enforce_turn_quota(
    app: &Arc<App>,
    acp_session_id: &str,
    parsed: &crate::prompt::ParsedPrompt,
    epoch: u64,
) -> Option<String> {
    if !app.cfg.quota.is_enabled() {
        return None;
    }

    let key = quota_scope_key(&app.cfg, parsed);
    let decision = {
        let mut quota = app.quota.lock().await;
        quota.check_and_record_at(&key, std::time::Instant::now())
    };

    match decision {
        crate::quota::QuotaDecision::Allow { remaining } => {
            tracing::debug!(scope = %key, remaining, "turn quota ok");
            None
        }
        crate::quota::QuotaDecision::Deny {
            limit,
            retry_after_secs,
        } => {
            let window_secs = app.cfg.quota.window.as_secs();
            // `key` is safe to log: host, channel uuid, agent name — no secrets.
            tracing::warn!(
                scope = %key,
                limit,
                window_secs,
                retry_after_secs,
                "turn quota exceeded; refusing turn without calling the gateway"
            );
            let text = crate::quota::quota_exceeded_message(limit, window_secs, retry_after_secs);
            let _ = post_error_reply(
                app,
                parsed.channel_id,
                parsed.reply_to_event_id.as_deref(),
                &text,
                epoch,
                acp_session_id,
            )
            .await;
            Some("refusal".to_owned())
        }
    }
}

fn session_mapping_key(
    app: &App,
    acp_session_id: &str,
    parsed: &crate::prompt::ParsedPrompt,
) -> Result<String, AdapterError> {
    match app.cfg.session_mode {
        SessionMode::Acp => Ok(format!("acp:{acp_session_id}")),
        SessionMode::Channel => {
            if let Some(ch) = parsed.channel_id {
                Ok(ch.to_string())
            } else {
                // Fall back to ACP session so we still get multi-turn within
                // one harness session when channel can't be parsed.
                Ok(format!("acp:{acp_session_id}"))
            }
        }
    }
}

/// Strip non-alphanumeric characters and lowercase so OpenViking accepts the
/// value as `X-OpenViking-User` (must be `[a-z0-9]+`).
fn alphanumeric_entity_suffix(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

/// Build a stable, alphanumeric-only `entity_id` for intel session metadata.
///
/// Formats (per `INTEL_ENTITY_MODE`):
/// - channel → `buzzchannel{uuid_hex}`
/// - owner → `buzzowner{pubkey_hex}`
/// - agent → `buzzagent{pubkey_hex}`
fn format_entity_id(mode: EntityMode, raw: &str) -> String {
    let suffix = alphanumeric_entity_suffix(raw);
    match mode {
        EntityMode::Channel => format!("buzzchannel{suffix}"),
        EntityMode::Owner => format!("buzzowner{suffix}"),
        EntityMode::Agent => format!("buzzagent{suffix}"),
    }
}

fn build_entity_id(
    app: &App,
    parsed: &crate::prompt::ParsedPrompt,
) -> Result<String, AdapterError> {
    match app.cfg.entity_mode {
        EntityMode::Channel => {
            let raw = parsed
                .channel_id
                .map(|ch| ch.to_string())
                .unwrap_or_else(|| "unknown".into());
            Ok(format_entity_id(EntityMode::Channel, &raw))
        }
        EntityMode::Owner => {
            let owner = app
                .relay
                .as_ref()
                .and_then(|r| r.owner_pubkey_hex())
                .unwrap_or_else(|| "unknown".into());
            Ok(format_entity_id(EntityMode::Owner, &owner))
        }
        EntityMode::Agent => {
            let agent = app
                .relay
                .as_ref()
                .map(|r| r.agent_pubkey_hex())
                .unwrap_or_else(|| "unknown".into());
            Ok(format_entity_id(EntityMode::Agent, &agent))
        }
    }
}

async fn epoch_is_current(app: &App, acp_session_id: &str, epoch: u64) -> bool {
    let sessions = app.sessions.lock().await;
    sessions
        .get(acp_session_id)
        .map(|s| s.epoch == epoch)
        .unwrap_or(false)
}

async fn post_error_reply(
    app: &App,
    channel_id: Option<Uuid>,
    reply_to: Option<&str>,
    text: &str,
    epoch: u64,
    acp_session_id: &str,
) -> Result<(), AdapterError> {
    if !epoch_is_current(app, acp_session_id, epoch).await {
        return Ok(());
    }
    let Some(channel) = channel_id else {
        return Ok(());
    };
    let Some(ref relay) = app.relay else {
        return Ok(());
    };
    let _ = relay.post_message(channel, text, reply_to).await?;
    Ok(())
}

async fn notify_incomplete_answer(
    app: &App,
    wire_tx: &WireSender,
    channel_id: Option<Uuid>,
    reply_to: Option<&str>,
    text: &str,
    epoch: u64,
    acp_session_id: &str,
) {
    // ACP transcript and Buzz channel are separate audiences. Always notify
    // ACP, and additionally publish the same safe text when a channel exists.
    if epoch_is_current(app, acp_session_id, epoch).await {
        wire::send(
            wire_tx,
            wire::session_update(
                acp_session_id,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text }
                }),
            ),
        )
        .await;
    }

    let _ = post_error_reply(app, channel_id, reply_to, text, epoch, acp_session_id).await;
}

fn jitter_backoff() -> Duration {
    let base_ms = 500u64;
    let jitter = rand::rng().random_range(0..1000u64);
    Duration::from_millis(base_ms + jitter)
}

async fn reject(wire_tx: &WireSender, id: Value, code: i32, message: &str) {
    wire::send(wire_tx, wire::err(id, code, message)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_visible_error_omits_gateway_internals() {
        let s = owner_visible_error("platform unreachable", Some("abc-123"));
        assert!(s.contains("abc-123"));
        assert!(s.contains("platform unreachable"));
        assert!(!s.contains("stack"));
        assert!(!s.contains("internal"));
    }

    #[test]
    fn extract_request_id_from_error_suffix() {
        let msg = "status 500: boom x-request-id=req-99 extra";
        assert_eq!(extract_request_id(msg), Some("req-99"));
        assert_eq!(extract_request_id("no rid here"), None);
    }

    fn assert_alphanumeric_entity_id(id: &str) {
        assert!(
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "entity_id must match [a-z0-9]+, got {id:?}"
        );
    }

    #[test]
    fn entity_id_channel_is_alphanumeric_and_strips_uuid_dashes() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let id = format_entity_id(EntityMode::Channel, uuid);
        assert_eq!(id, "buzzchannel550e8400e29b41d4a716446655440000");
        assert_alphanumeric_entity_id(&id);
    }

    #[test]
    fn entity_id_owner_strips_colons_and_dashes() {
        let pubkey = "aa:bb-cc:DD";
        let id = format_entity_id(EntityMode::Owner, pubkey);
        assert_eq!(id, "buzzowneraabbccdd");
        assert_alphanumeric_entity_id(&id);
    }

    #[test]
    fn entity_id_agent_is_alphanumeric() {
        let pubkey = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let id = format_entity_id(EntityMode::Agent, pubkey);
        assert_eq!(id, format!("buzzagent{pubkey}"));
        assert_alphanumeric_entity_id(&id);
    }

    #[test]
    fn entity_id_unknown_fallback_is_alphanumeric() {
        for mode in [EntityMode::Channel, EntityMode::Owner, EntityMode::Agent] {
            let id = format_entity_id(mode, "unknown");
            assert_alphanumeric_entity_id(&id);
            assert!(id.ends_with("unknown"), "got {id}");
        }
        // Empty / punctuation-only collapses to "unknown" suffix.
        let id = format_entity_id(EntityMode::Channel, ":::---");
        assert_eq!(id, "buzzchannelunknown");
        assert_alphanumeric_entity_id(&id);
    }

    #[test]
    fn entity_id_rejects_legacy_colon_form() {
        // Regression: OpenViking rejects non-alphanumeric X-OpenViking-User.
        let legacy = format!("buzz:channel:{}", "550e8400-e29b-41d4-a716-446655440000");
        assert!(
            legacy.chars().any(|c| !c.is_ascii_alphanumeric()),
            "sanity: legacy form has non-alnum"
        );
        let id = format_entity_id(EntityMode::Channel, "550e8400-e29b-41d4-a716-446655440000");
        assert!(!id.contains(':'));
        assert!(!id.contains('-'));
        assert_alphanumeric_entity_id(&id);
    }
}

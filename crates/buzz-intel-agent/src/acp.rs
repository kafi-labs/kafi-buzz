//! ACP server loop: initialize / session/new / session/prompt / session/cancel.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rand::RngExt;
use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::sync::{mpsc, watch, Mutex};
use uuid::Uuid;

use crate::config::{Config, EntityMode, ForwardSystemPrompt, SessionMode, PROTOCOL_VERSION};
use crate::error::AdapterError;
use crate::intel::{FrameKind, IntelClient, SseFrame};
use crate::prompt::parse_prompt;
use crate::reply::RelayPublisher;
use crate::state::{SessionEntry, StateStore};
use crate::wire::{
    self, classify, prompt_to_text, Inbound, InitializeParams, SessionCancelParams,
    SessionNewParams, SessionPromptParams, WireMsg, WireSender, INVALID_PARAMS, METHOD_NOT_FOUND,
    PARSE_ERROR,
};

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
    relay: Option<RelayPublisher>,
}

/// Run the ACP NDJSON server until stdin EOF.
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
        relay,
    });

    let (wire_tx, wire_rx) = mpsc::channel::<WireMsg>(64);
    let writer = tokio::spawn(wire::writer_task(wire_rx));

    let max_line = cfg.max_line_bytes;
    if let Err(e) = read_loop(
        BufReader::new(tokio::io::stdin()),
        app.clone(),
        wire_tx,
        max_line,
    )
    .await
    {
        tracing::error!("io: reader: {e}");
    }

    // Graceful shutdown: cancel in-flight turns and flush state.
    {
        let sessions = app.sessions.lock().await;
        for s in sessions.values() {
            let _ = s.cancel_tx.send(true);
        }
    }
    if let Err(e) = app.state.lock().await.flush() {
        tracing::warn!("state flush on shutdown: {e}");
    }

    let _ = writer.await;
    Ok(())
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
            tokio::spawn(async move { session_prompt(&app, id, params, &wire_tx).await });
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
                if app.cfg.error_replies {
                    let _ = post_error_reply(
                        app,
                        parsed.channel_id,
                        parsed.reply_to_event_id.as_deref(),
                        &format!("⚠️ Intel gateway rejected credentials (401/403). {msg}"),
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
                if app.cfg.error_replies {
                    let user_msg = match &e {
                        AdapterError::Intel(m)
                            if m.contains("unreachable")
                                || m.contains("connect")
                                || m.contains("timeout")
                                || m.contains("status 5") =>
                        {
                            format!("⚠️ Intel platform unreachable. {m}")
                        }
                        other => format!("⚠️ Intel turn failed. {other}"),
                    };
                    let _ = post_error_reply(
                        app,
                        parsed.channel_id,
                        parsed.reply_to_event_id.as_deref(),
                        &user_msg,
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
    let (intel_session_id, is_new, already_forwarded) = {
        let mut state = app.state.lock().await;
        if force_new {
            let _ = state.remove_session(mapping_key);
        }
        if let Some(entry) = state.get_session(mapping_key) {
            (
                entry.session_id.clone(),
                false,
                entry.system_prompt_forwarded,
            )
        } else {
            drop(state);
            let created = app.intel.create_session(agent_id, entity_id).await?;
            let entry = SessionEntry {
                session_id: created.session_id.clone(),
                entity_id: entity_id.to_owned(),
                created_at: Utc::now(),
                last_used_at: Utc::now(),
                system_prompt_forwarded: false,
            };
            let mut state = app.state.lock().await;
            state.put_session(mapping_key.to_owned(), entry)?;
            (created.session_id, true, false)
        }
    };

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
                    emit_acp_frame(wire_tx, acp_session_id, frame);
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

    if let Some((code, msg)) = stream.stream_error {
        let brief = match code {
            Some(c) => format!("⚠️ Intel error [{c}]: {msg}"),
            None => format!("⚠️ Intel error: {msg}"),
        };
        if app.cfg.error_replies {
            let _ = post_error_reply(
                app,
                parsed.channel_id,
                parsed.reply_to_event_id.as_deref(),
                &brief,
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

    if !reply_text.is_empty() {
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
    }

    Ok("end_turn".into())
}

fn emit_acp_frame(wire_tx: &WireSender, sid: &str, frame: &SseFrame) {
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

    // Synchronous try_send preserves frame order (spawned tasks can reorder).
    let _ = wire_tx.try_send(wire::WireMsg::Notify(wire::session_update(sid, update)));
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

fn build_entity_id(
    app: &App,
    parsed: &crate::prompt::ParsedPrompt,
) -> Result<String, AdapterError> {
    match app.cfg.entity_mode {
        EntityMode::Channel => {
            if let Some(ch) = parsed.channel_id {
                Ok(format!("buzz:channel:{ch}"))
            } else {
                Ok("buzz:channel:unknown".into())
            }
        }
        EntityMode::Owner => {
            let owner = app
                .relay
                .as_ref()
                .and_then(|r| r.owner_pubkey_hex())
                .unwrap_or_else(|| "unknown".into());
            Ok(format!("buzz:owner:{owner}"))
        }
        EntityMode::Agent => {
            let agent = app
                .relay
                .as_ref()
                .map(|r| r.agent_pubkey_hex())
                .unwrap_or_else(|| "unknown".into());
            Ok(format!("buzz:agent:{agent}"))
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

fn jitter_backoff() -> Duration {
    let base_ms = 500u64;
    let jitter = rand::rng().random_range(0..1000u64);
    Duration::from_millis(base_ms + jitter)
}

async fn reject(wire_tx: &WireSender, id: Value, code: i32, message: &str) {
    wire::send(wire_tx, wire::err(id, code, message)).await;
}

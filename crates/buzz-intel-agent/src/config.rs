//! Configuration loaded from environment variables and CLI flags.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::error::AdapterError;

/// ACP protocol version advertised by this adapter.
pub const PROTOCOL_VERSION: u32 = 2;

/// Default max line size for NDJSON stdin frames (8 MiB).
pub const DEFAULT_MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// How intel sessions are keyed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionMode {
    /// One intel session per Buzz channel (default).
    #[default]
    Channel,
    /// One intel session per ACP session (ephemeral).
    Acp,
}

impl SessionMode {
    fn parse(s: &str) -> Result<Self, AdapterError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "channel" | "" => Ok(Self::Channel),
            "acp" => Ok(Self::Acp),
            other => Err(AdapterError::Config(format!(
                "INTEL_SESSION_MODE must be 'channel' or 'acp', got {other:?}"
            ))),
        }
    }
}

/// How `entity_id` is derived for intel memory scoping.
///
/// Emitted ids are OpenViking-safe: `[a-z0-9]+` only
/// (`buzzchannel<hex>` / `buzzowner<hex>` / `buzzagent<hex>`). Dashes and
/// colons from UUIDs/pubkeys are stripped. Colons in the old
/// `buzz:channel:…` form were rejected by OpenViking's `X-OpenViking-User`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityMode {
    /// `buzzchannel{uuid_hex}` (default).
    #[default]
    Channel,
    /// `buzzowner{owner_pubkey_hex}`.
    Owner,
    /// `buzzagent{agent_pubkey_hex}`.
    Agent,
}

impl EntityMode {
    fn parse(s: &str) -> Result<Self, AdapterError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "channel" | "" => Ok(Self::Channel),
            "owner" => Ok(Self::Owner),
            "agent" => Ok(Self::Agent),
            other => Err(AdapterError::Config(format!(
                "INTEL_ENTITY_MODE must be 'channel', 'owner', or 'agent', got {other:?}"
            ))),
        }
    }
}

/// Whether to forward the harness systemPrompt into a new intel session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForwardSystemPrompt {
    /// Prepend on the first message of each new intel session (default).
    #[default]
    FirstMessage,
    /// Never forward.
    Never,
}

impl ForwardSystemPrompt {
    fn parse(s: &str) -> Result<Self, AdapterError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "first-message" | "first_message" | "" => Ok(Self::FirstMessage),
            "never" => Ok(Self::Never),
            other => Err(AdapterError::Config(format!(
                "INTEL_FORWARD_SYSTEM_PROMPT must be 'first-message' or 'never', got {other:?}"
            ))),
        }
    }
}

/// CLI flags (mirror env vars for manual runs).
#[derive(Debug, Parser)]
#[command(
    name = "buzz-intel-agent",
    about = "ACP adapter for Intelligence Platform agents",
    version
)]
pub struct Cli {
    /// Intelligence Platform gateway base URL.
    #[arg(long, env = "INTEL_GATEWAY_URL")]
    pub gateway_url: Option<String>,

    /// API key (`intel_…`). Prefer file form for desktop-managed secrets.
    #[arg(long, env = "INTEL_API_KEY")]
    pub api_key: Option<String>,

    /// Path to a file containing the API key.
    #[arg(long, env = "INTEL_API_KEY_FILE")]
    pub api_key_file: Option<PathBuf>,

    /// Intel agent name or UUID.
    #[arg(long, env = "INTEL_AGENT")]
    pub agent: Option<String>,

    /// Optional org id sent as `X-Org-Id`.
    #[arg(long, env = "INTEL_ORG_ID")]
    pub org_id: Option<String>,

    /// Session mapping mode: `channel` or `acp`.
    #[arg(long, env = "INTEL_SESSION_MODE", default_value = "channel")]
    pub session_mode: String,

    /// Entity id mode: `channel`, `owner`, or `agent`.
    #[arg(long, env = "INTEL_ENTITY_MODE", default_value = "channel")]
    pub entity_mode: String,

    /// Forward harness systemPrompt: `first-message` or `never`.
    #[arg(
        long,
        env = "INTEL_FORWARD_SYSTEM_PROMPT",
        default_value = "first-message"
    )]
    pub forward_system_prompt: String,

    /// Connect timeout seconds.
    #[arg(long, env = "INTEL_CONNECT_TIMEOUT_SECS", default_value = "30")]
    pub connect_timeout_secs: u64,

    /// Per-frame SSE idle timeout seconds.
    #[arg(long, env = "INTEL_SSE_IDLE_TIMEOUT_SECS", default_value = "570")]
    pub sse_idle_timeout_secs: u64,

    /// Whole-turn timeout seconds.
    #[arg(long, env = "INTEL_TURN_TIMEOUT_SECS", default_value = "3300")]
    pub turn_timeout_secs: u64,

    /// Keepalive cadence while SSE is quiet (seconds).
    #[arg(long, env = "INTEL_KEEPALIVE_SECS", default_value = "60")]
    pub keepalive_secs: u64,

    /// Post owner-visible ⚠️ replies on failures.
    #[arg(long, env = "INTEL_ERROR_REPLIES", default_value = "true")]
    pub error_replies: String,

    /// Directory for the state file (file written as `state.json` inside).
    #[arg(long, env = "INTEL_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Buzz relay base URL (http/https).
    #[arg(long, env = "BUZZ_RELAY_URL")]
    pub relay_url: Option<String>,

    /// Buzz private key (hex or nsec).
    #[arg(long, env = "BUZZ_PRIVATE_KEY")]
    pub private_key: Option<String>,

    /// Optional NIP-OA auth tag JSON.
    #[arg(long, env = "BUZZ_AUTH_TAG")]
    pub auth_tag: Option<String>,

    /// Probe `GET /v1/whoami` and exit 0/1.
    #[arg(long)]
    pub auth_probe: bool,

    /// List agents from `GET /v1/agents` as JSON to stdout and exit.
    #[arg(long)]
    pub list_agents: bool,
}

/// Fully resolved runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Gateway base URL without trailing slash.
    pub gateway_url: String,
    /// Bearer API key.
    pub api_key: String,
    /// Intel agent name or UUID (resolved later to agent_id).
    pub agent: String,
    /// Optional org header.
    pub org_id: Option<String>,
    /// Session mapping mode.
    pub session_mode: SessionMode,
    /// Entity id mode.
    pub entity_mode: EntityMode,
    /// System-prompt forward policy.
    pub forward_system_prompt: ForwardSystemPrompt,
    /// HTTP connect timeout.
    pub connect_timeout: Duration,
    /// SSE idle timeout per frame.
    pub sse_idle_timeout: Duration,
    /// Whole-turn bound.
    pub turn_timeout: Duration,
    /// Keepalive while stream is quiet.
    pub keepalive: Duration,
    /// Whether to post visible error replies.
    pub error_replies: bool,
    /// Path to the state JSON file.
    pub state_path: PathBuf,
    /// Buzz relay base URL (normalized, no trailing slash).
    pub relay_url: Option<String>,
    /// Buzz private key raw string.
    pub private_key: Option<String>,
    /// Optional NIP-OA auth tag JSON.
    pub auth_tag: Option<String>,
    /// Max NDJSON line size.
    pub max_line_bytes: usize,
}

impl Config {
    /// Build config from parsed CLI (which already merges env via clap).
    ///
    /// Missing `INTEL_GATEWAY_URL` / API key / `INTEL_AGENT` are allowed here so
    /// the ACP server can start and return a clear JSON-RPC error from
    /// `initialize`. Call [`Config::require_intel_credentials`] for one-shot
    /// modes (`--auth-probe`, `--list-agents`) and inside `initialize`.
    pub fn from_cli(cli: &Cli) -> Result<Self, AdapterError> {
        let gateway_url = cli
            .gateway_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_owned())
            .unwrap_or_default();

        let api_key = resolve_api_key(cli)?;
        let agent = cli
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_default();

        let org_id = cli
            .org_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let session_mode = SessionMode::parse(&cli.session_mode)?;
        let entity_mode = EntityMode::parse(&cli.entity_mode)?;
        let forward_system_prompt = ForwardSystemPrompt::parse(&cli.forward_system_prompt)?;

        let error_replies = parse_bool(&cli.error_replies, true)?;

        let state_path = resolve_state_path(cli.state_dir.as_ref())?;

        let relay_url = cli
            .relay_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(normalize_relay_url);

        Ok(Self {
            gateway_url,
            api_key,
            agent,
            org_id,
            session_mode,
            entity_mode,
            forward_system_prompt,
            connect_timeout: Duration::from_secs(cli.connect_timeout_secs.max(1)),
            sse_idle_timeout: Duration::from_secs(cli.sse_idle_timeout_secs.max(1)),
            turn_timeout: Duration::from_secs(cli.turn_timeout_secs.max(1)),
            keepalive: Duration::from_secs(cli.keepalive_secs.max(1)),
            error_replies,
            state_path,
            relay_url,
            private_key: cli
                .private_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            auth_tag: cli
                .auth_tag
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
        })
    }

    /// Fail if intel gateway credentials required for operation are missing.
    pub fn require_intel_credentials(&self) -> Result<(), AdapterError> {
        if self.gateway_url.is_empty() {
            return Err(AdapterError::Config(
                "INTEL_GATEWAY_URL is required (or pass --gateway-url)".into(),
            ));
        }
        if self.api_key.is_empty() {
            return Err(AdapterError::Config(
                "INTEL_API_KEY or INTEL_API_KEY_FILE is required (or pass --api-key)".into(),
            ));
        }
        if self.agent.is_empty() {
            return Err(AdapterError::Config(
                "INTEL_AGENT is required (or pass --agent)".into(),
            ));
        }
        Ok(())
    }
}

fn resolve_api_key(cli: &Cli) -> Result<String, AdapterError> {
    if let Some(ref path) = cli.api_key_file {
        warn_if_key_file_world_readable(path);
        let raw = std::fs::read_to_string(path).map_err(|e| {
            AdapterError::Config(format!(
                "failed to read INTEL_API_KEY_FILE {}: {e}",
                path.display()
            ))
        })?;
        let key = raw.trim().to_owned();
        if key.is_empty() {
            return Err(AdapterError::Config("INTEL_API_KEY_FILE is empty".into()));
        }
        return Ok(key);
    }
    Ok(cli
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_default())
}

/// Warn (do not fail) when the API key file is group/world-readable.
fn warn_if_key_file_world_readable(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) => {
                let mode = meta.permissions().mode();
                if mode & 0o077 != 0 {
                    let msg = format!(
                        "INTEL_API_KEY_FILE {} is group/world-accessible (mode {:o}); \
                         prefer chmod 600",
                        path.display(),
                        mode & 0o777
                    );
                    // tracing may not be initialised yet during CLI parse.
                    eprintln!("warning: {msg}");
                    tracing::warn!("{msg}");
                }
            }
            Err(e) => {
                tracing::debug!("could not stat INTEL_API_KEY_FILE {}: {e}", path.display());
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn resolve_state_path(override_dir: Option<&PathBuf>) -> Result<PathBuf, AdapterError> {
    let dir = if let Some(d) = override_dir {
        d.clone()
    } else if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("buzz-intel-agent")
    } else {
        dirs::home_dir()
            .ok_or_else(|| AdapterError::Config("cannot resolve home directory for state".into()))?
            .join(".local/state/buzz-intel-agent")
    };
    Ok(dir.join("state.json"))
}

fn parse_bool(s: &str, default: bool) -> Result<bool, AdapterError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(AdapterError::Config(format!(
            "expected boolean, got {other:?}"
        ))),
    }
}

/// Normalize a relay URL to http(s) base without trailing slash.
pub fn normalize_relay_url(url: &str) -> String {
    let mut u = url.trim().trim_end_matches('/').to_owned();
    if let Some(rest) = u.strip_prefix("ws://") {
        u = format!("http://{rest}");
    } else if let Some(rest) = u.strip_prefix("wss://") {
        u = format!("https://{rest}");
    }
    u.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ws_to_http() {
        assert_eq!(
            normalize_relay_url("wss://relay.example/"),
            "https://relay.example"
        );
        assert_eq!(
            normalize_relay_url("ws://localhost:3000"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn session_mode_parse() {
        assert_eq!(SessionMode::parse("channel").unwrap(), SessionMode::Channel);
        assert_eq!(SessionMode::parse("acp").unwrap(), SessionMode::Acp);
        assert!(SessionMode::parse("nope").is_err());
    }
}

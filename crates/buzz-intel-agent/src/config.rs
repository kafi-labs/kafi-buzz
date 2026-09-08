//! Configuration loaded from environment variables and CLI flags.

use std::fmt;
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
#[derive(Parser)]
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

    /// Response headers timeout seconds — bounds only the time to receive
    /// response **headers** on `.send().await` (connection already
    /// established, request written). Distinct from `connect_timeout`
    /// (TCP/TLS handshake) and from the whole-response timeout the client
    /// deliberately does *not* set (see [`crate::intel::IntelClient::new`]).
    ///
    /// Default 570s (matches `sse_idle_timeout`). A 90s bound (the
    /// previous default, sized off a healthy-turn sample of only *short*
    /// prompts) caused a real, user-visible failure: a heavy prompt hit
    /// two consecutive headers timeouts and the turn failed, returning an
    /// error instead of an answer. Re-sending the identical prompt after
    /// raising the bound completed in 10s. That does not confirm the
    /// working theory that headers are withheld until generation is
    /// underway (so time-to-first-byte scales with output length) — two
    /// candidate causes for the original slowness remain, and the data on
    /// hand does not distinguish them: transient gateway degradation (a
    /// 503 was also observed that day), or headers genuinely gated on
    /// generation for that one call. 570s is sized to ride out
    /// slow-to-first-byte periods under either explanation. It reuses the
    /// per-frame SSE idle bound: both represent the same judgment call
    /// ("how long is silence from the gateway tolerable before treating
    /// it as hung"), so headers-wait and stream-idle-wait share one
    /// threshold instead of two independently-drifting numbers. The
    /// client retries once on a headers timeout, so worst case this phase
    /// costs ~2x this value (~1140s) before failing the turn — still well
    /// under the 3300s whole-turn bound, leaving room for the actual
    /// streamed response. Do not lower this back toward "typical turn
    /// latency" (4-8s healthy) — under either explanation, a short bound
    /// here reproduces the same failure mode.
    #[arg(
        long,
        env = "INTEL_RESPONSE_HEADERS_TIMEOUT_SECS",
        default_value = "570"
    )]
    pub response_headers_timeout_secs: u64,

    /// Per-frame SSE idle timeout seconds.
    #[arg(long, env = "INTEL_SSE_IDLE_TIMEOUT_SECS", default_value = "570")]
    pub sse_idle_timeout_secs: u64,

    /// Whole-turn timeout seconds.
    #[arg(long, env = "INTEL_TURN_TIMEOUT_SECS", default_value = "3300")]
    pub turn_timeout_secs: u64,

    /// Keepalive cadence while SSE is quiet (seconds).
    #[arg(long, env = "INTEL_KEEPALIVE_SECS", default_value = "60")]
    pub keepalive_secs: u64,

    /// Max paid gateway turns per quota window, per channel+agent scope.
    ///
    /// `0` disables the quota. This is an LLM **cost** bound and is unrelated to
    /// the relay's protocol admission rate limiting.
    #[arg(long, env = "INTEL_MAX_TURNS_PER_WINDOW", default_value = "30")]
    pub max_turns_per_window: u32,

    /// Quota window length in seconds.
    #[arg(long, env = "INTEL_QUOTA_WINDOW_SECS", default_value = "3600")]
    pub quota_window_secs: u64,

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

impl fmt::Debug for Cli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cli")
            .field("gateway_url", &self.gateway_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            // This is a path only; Debug never reads or inlines file contents.
            .field("api_key_file", &self.api_key_file)
            .field("agent", &self.agent)
            .field("org_id", &self.org_id)
            .field("session_mode", &self.session_mode)
            .field("entity_mode", &self.entity_mode)
            .field("forward_system_prompt", &self.forward_system_prompt)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field(
                "response_headers_timeout_secs",
                &self.response_headers_timeout_secs,
            )
            .field("sse_idle_timeout_secs", &self.sse_idle_timeout_secs)
            .field("turn_timeout_secs", &self.turn_timeout_secs)
            .field("keepalive_secs", &self.keepalive_secs)
            .field("max_turns_per_window", &self.max_turns_per_window)
            .field("quota_window_secs", &self.quota_window_secs)
            .field("error_replies", &self.error_replies)
            .field("state_dir", &self.state_dir)
            .field("relay_url", &self.relay_url)
            .field(
                "private_key",
                &self.private_key.as_ref().map(|_| "<redacted>"),
            )
            .field("auth_tag", &self.auth_tag.as_ref().map(|_| "<redacted>"))
            .field("auth_probe", &self.auth_probe)
            .field("list_agents", &self.list_agents)
            .finish()
    }
}

/// Fully resolved runtime configuration.
#[derive(Clone)]
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
    /// Timeout bounding only the response-headers phase of a request
    /// (post-connect, pre-body). See [`Cli::response_headers_timeout_secs`].
    pub response_headers_timeout: Duration,
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
    /// Per-scope LLM turn quota (cost bound, not protocol admission).
    pub quota: crate::quota::QuotaConfig,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("gateway_url", &self.gateway_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<unset>"
                } else {
                    "<redacted>"
                },
            )
            .field("agent", &self.agent)
            .field("org_id", &self.org_id)
            .field("session_mode", &self.session_mode)
            .field("entity_mode", &self.entity_mode)
            .field("forward_system_prompt", &self.forward_system_prompt)
            .field("connect_timeout", &self.connect_timeout)
            .field("response_headers_timeout", &self.response_headers_timeout)
            .field("sse_idle_timeout", &self.sse_idle_timeout)
            .field("turn_timeout", &self.turn_timeout)
            .field("keepalive", &self.keepalive)
            .field("error_replies", &self.error_replies)
            .field("state_path", &self.state_path)
            .field("relay_url", &self.relay_url)
            .field(
                "private_key",
                &self.private_key.as_ref().map(|_| "<redacted>"),
            )
            .field("auth_tag", &self.auth_tag.as_ref().map(|_| "<redacted>"))
            .field("max_line_bytes", &self.max_line_bytes)
            .field("quota", &self.quota)
            .finish()
    }
}

impl Config {
    /// Build config from parsed CLI (which already merges env via clap).
    ///
    /// Missing `INTEL_GATEWAY_URL` / API key / `INTEL_AGENT` are allowed here so
    /// the ACP server can start and return a clear JSON-RPC error from
    /// `initialize`. One-shot modes call [`Config::require_gateway_credentials`];
    /// ACP initialization calls [`Config::require_acp_runtime_config`].
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
            response_headers_timeout: Duration::from_secs(cli.response_headers_timeout_secs.max(1)),
            sse_idle_timeout: Duration::from_secs(cli.sse_idle_timeout_secs.max(1)),
            turn_timeout: Duration::from_secs(cli.turn_timeout_secs.max(1)),
            keepalive: Duration::from_secs(cli.keepalive_secs.max(1)),
            quota: crate::quota::QuotaConfig::new(cli.max_turns_per_window, cli.quota_window_secs),
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

    /// One-line, non-secret summary of the resolved config for startup logs.
    ///
    /// Deliberately names every field instead of deriving/using `{:?}` on
    /// `self` — `Config` holds [`Config::api_key`], and a struct-wide `Debug`
    /// dump would print it via the manual [`fmt::Debug`] impl above only if
    /// that impl is ever bypassed; this method exists so no caller needs to
    /// reach for `{:?}` in the first place. Never add `self.api_key` (or
    /// anything derived from it) to this string; the API key must never
    /// appear in logs. See `specs/intel-agent-integration` for the operator
    /// rationale (spend ceiling must be checkable without opening the
    /// secrets file that holds the live gateway key).
    pub fn summary(&self) -> String {
        let max_turns_per_window = self
            .quota
            .max_turns_per_window
            .map_or_else(|| "disabled".to_owned(), |n| n.to_string());
        format!(
            "gateway_url={} agent={} entity_mode={:?} forward_system_prompt={:?} error_replies={} \
             connect_timeout={}s response_headers_timeout={}s sse_idle_timeout={}s turn_timeout={}s \
             max_turns_per_window={} quota_window_secs={}",
            self.gateway_url,
            self.agent,
            self.entity_mode,
            self.forward_system_prompt,
            self.error_replies,
            self.connect_timeout.as_secs(),
            self.response_headers_timeout.as_secs(),
            self.sse_idle_timeout.as_secs(),
            self.turn_timeout.as_secs(),
            max_turns_per_window,
            self.quota.window.as_secs(),
        )
    }

    /// Fail if the gateway URL or API key required by one-shot gateway calls is missing.
    pub fn require_gateway_credentials(&self) -> Result<(), AdapterError> {
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
        Ok(())
    }

    /// Fail if gateway credentials or the agent required by ACP server mode are missing.
    pub fn require_acp_runtime_config(&self) -> Result<(), AdapterError> {
        self.require_gateway_credentials()?;
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

    fn config_for_mode(mode_flag: &str) -> Config {
        let cli = Cli::try_parse_from([
            "buzz-intel-agent",
            "--gateway-url",
            "https://intel.example.test",
            "--api-key",
            "intel_test_key",
            mode_flag,
        ])
        .expect("mode CLI should parse");
        let mut cfg = Config::from_cli(&cli).expect("mode config should resolve");
        // Keep the tests hermetic even if the test runner exports INTEL_AGENT.
        cfg.agent.clear();
        cfg
    }

    #[test]
    fn debug_output_redacts_cli_and_config_secrets_but_keeps_context() {
        let api_secret = "intel_debug_api_secret";
        let private_secret = "nsec_debug_identity_secret";
        let auth_secret = "debug_auth_tag_secret";
        let cli = Cli::try_parse_from([
            "buzz-intel-agent",
            "--gateway-url",
            "https://debug-gateway.example.test",
            "--api-key",
            api_secret,
            "--agent",
            "debug-agent",
            "--private-key",
            private_secret,
            "--auth-tag",
            auth_secret,
        ])
        .expect("debug CLI should parse");
        let cli_debug = format!("{cli:?}");

        assert!(!cli_debug.contains(api_secret));
        assert!(!cli_debug.contains(private_secret));
        assert!(!cli_debug.contains(auth_secret));
        assert!(cli_debug.contains("api_key: Some(\"<redacted>\")"));
        assert!(cli_debug.contains("private_key: Some(\"<redacted>\")"));
        assert!(cli_debug.contains("https://debug-gateway.example.test"));
        assert!(cli_debug.contains("debug-agent"));

        let mut cfg = Config::from_cli(&cli).expect("debug config should resolve");
        cfg.api_key = api_secret.to_string();
        cfg.private_key = Some(private_secret.to_string());
        cfg.auth_tag = Some(auth_secret.to_string());
        let config_debug = format!("{cfg:?}");

        assert!(!config_debug.contains(api_secret));
        assert!(!config_debug.contains(private_secret));
        assert!(!config_debug.contains(auth_secret));
        assert!(config_debug.contains("api_key: \"<redacted>\""));
        assert!(config_debug.contains("private_key: Some(\"<redacted>\")"));
        assert!(config_debug.contains("https://debug-gateway.example.test"));
        assert!(config_debug.contains("debug-agent"));
    }

    /// Dummy key is recognisable ("SECRETVALUE") so a leak is unmistakable
    /// in a failing assertion, not just a plausible-looking string.
    const SUMMARY_TEST_API_KEY: &str = "intel_SECRETVALUE";

    fn summary_test_config() -> Config {
        let cli = Cli::try_parse_from([
            "buzz-intel-agent",
            "--gateway-url",
            "https://intel.example.test",
            "--api-key",
            SUMMARY_TEST_API_KEY,
            "--agent",
            "brain-prod",
            "--response-headers-timeout-secs",
            "570",
            "--sse-idle-timeout-secs",
            "570",
            "--turn-timeout-secs",
            "3300",
            "--max-turns-per-window",
            "7",
            "--quota-window-secs",
            "120",
        ])
        .expect("summary test CLI should parse");
        Config::from_cli(&cli).expect("summary test config should resolve")
    }

    #[test]
    fn summary_contains_quota_and_timeout_values() {
        let summary = summary_test_config().summary();
        assert!(
            summary.contains("max_turns_per_window=7"),
            "summary missing max_turns_per_window: {summary}"
        );
        assert!(
            summary.contains("quota_window_secs=120"),
            "summary missing quota_window_secs: {summary}"
        );
        assert!(summary.contains("response_headers_timeout=570s"));
        assert!(summary.contains("sse_idle_timeout=570s"));
        assert!(summary.contains("turn_timeout=3300s"));
        assert!(summary.contains("gateway_url=https://intel.example.test"));
        assert!(summary.contains("agent=brain-prod"));
    }

    #[test]
    fn summary_never_contains_the_api_key() {
        let summary = summary_test_config().summary();
        assert!(
            !summary.contains(SUMMARY_TEST_API_KEY),
            "summary leaked the API key: {summary}"
        );
        assert!(
            !summary.contains("SECRETVALUE"),
            "summary leaked API key material: {summary}"
        );
    }

    #[test]
    fn summary_disabled_quota_is_readable() {
        let mut cfg = summary_test_config();
        cfg.quota = crate::quota::QuotaConfig::new(0, 3600);
        let summary = cfg.summary();
        assert!(summary.contains("max_turns_per_window=disabled"));
    }

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

    #[test]
    fn list_agents_mode_accepts_gateway_credentials_without_agent() {
        let cfg = config_for_mode("--list-agents");

        cfg.require_gateway_credentials()
            .expect("listing agents must not require an agent selection");
    }

    #[test]
    fn auth_probe_mode_accepts_gateway_credentials_without_agent() {
        let cfg = config_for_mode("--auth-probe");

        cfg.require_gateway_credentials()
            .expect("auth probe must not require an agent selection");
    }

    #[test]
    fn acp_server_mode_still_requires_agent() {
        let cli = Cli::try_parse_from([
            "buzz-intel-agent",
            "--gateway-url",
            "https://intel.example.test",
            "--api-key",
            "intel_test_key",
        ])
        .expect("ACP server CLI should parse");
        let mut cfg = Config::from_cli(&cli).expect("ACP server config should resolve");
        cfg.agent.clear();

        let error = cfg
            .require_acp_runtime_config()
            .expect_err("ACP server mode must require an agent");
        assert_eq!(
            error.to_string(),
            "config: INTEL_AGENT is required (or pass --agent)"
        );
    }

    #[test]
    fn gateway_credential_errors_remain_specific() {
        let mut cfg = config_for_mode("--list-agents");
        cfg.gateway_url.clear();
        let missing_url = cfg
            .require_gateway_credentials()
            .expect_err("missing gateway URL must fail");
        assert_eq!(
            missing_url.to_string(),
            "config: INTEL_GATEWAY_URL is required (or pass --gateway-url)"
        );

        cfg.gateway_url = "https://intel.example.test".to_owned();
        cfg.api_key.clear();
        let missing_key = cfg
            .require_gateway_credentials()
            .expect_err("missing API key must fail");
        assert_eq!(
            missing_key.to_string(),
            "config: INTEL_API_KEY or INTEL_API_KEY_FILE is required (or pass --api-key)"
        );
    }
}

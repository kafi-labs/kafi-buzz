use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::managed_agents::resolve_command;

const INTEL_ROSTER_AGENT_PLACEHOLDER: &str = "buzz-desktop-roster-probe";

/// Unsaved Intelligence Platform credentials used for a roster lookup.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIntelAgentsInput {
    /// Gateway URL currently entered in the agent form.
    pub gateway_url: String,
    /// API key currently entered in the agent form.
    pub api_key: String,
}

/// One selectable Intelligence Platform agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelAgentRosterEntry {
    /// Stable gateway agent id, when present.
    pub id: Option<String>,
    /// Agent name persisted into the Buzz persona `model` field.
    pub name: String,
    /// Optional human-readable gateway description.
    pub description: Option<String>,
}

/// Parsed Intelligence Platform agent roster.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelAgentRosterResponse {
    /// Selectable agents returned by the gateway.
    pub agents: Vec<IntelAgentRosterEntry>,
}

/// Structured roster lookup error returned across the Tauri boundary.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelAgentRosterError {
    code: &'static str,
    message: &'static str,
}

impl IntelAgentRosterError {
    fn auth() -> Self {
        Self {
            code: "auth",
            message: "The gateway rejected the API key. Check it and try again.",
        }
    }

    fn connection() -> Self {
        Self {
            code: "connection",
            message:
                "Could not reach the Intelligence Platform gateway. Check the URL and connection.",
        }
    }

    fn malformed_output() -> Self {
        Self {
            code: "malformedOutput",
            message: "The gateway returned an agent list Buzz could not read.",
        }
    }

    fn unavailable() -> Self {
        Self {
            code: "unavailable",
            message: "The Intelligence Platform agent helper is not installed.",
        }
    }
}

struct CapturedRosterOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// List agents from the Intelligence Platform using credentials currently in the form.
///
/// The API key is passed only through the child environment. It is never placed
/// in argv or included in a returned error.
#[tauri::command]
pub async fn list_intel_agents(
    input: ListIntelAgentsInput,
) -> Result<IntelAgentRosterResponse, IntelAgentRosterError> {
    let gateway_url = input.gateway_url.trim().to_string();
    let api_key = input.api_key.trim().to_string();
    if gateway_url.is_empty() || api_key.is_empty() {
        return Err(IntelAgentRosterError::connection());
    }

    let binary =
        resolve_command("buzz-intel-agent").ok_or_else(IntelAgentRosterError::unavailable)?;
    let output = tokio::task::spawn_blocking(move || {
        build_roster_command(&binary, &gateway_url, &api_key)
            .output()
            .map(|output| CapturedRosterOutput {
                success: output.status.success(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
    })
    .await
    .map_err(|_| IntelAgentRosterError::unavailable())?
    .map_err(|_| IntelAgentRosterError::unavailable())?;

    parse_roster_output(output)
}

fn build_roster_command(binary: &Path, gateway_url: &str, api_key: &str) -> std::process::Command {
    let mut command = std::process::Command::new(binary);
    command
        .arg("--list-agents")
        .env("INTEL_GATEWAY_URL", gateway_url)
        .env("INTEL_API_KEY", api_key)
        // File credentials take precedence in buzz-intel-agent. Remove an
        // inherited file path so the unsaved form value is authoritative.
        .env_remove("INTEL_API_KEY_FILE")
        // Config currently validates INTEL_AGENT before entering one-shot
        // list mode even though the lookup itself does not use an agent.
        .env("INTEL_AGENT", INTEL_ROSTER_AGENT_PLACEHOLDER)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::util::configure_no_window(&mut command);
    command
}

fn parse_roster_output(
    output: CapturedRosterOutput,
) -> Result<IntelAgentRosterResponse, IntelAgentRosterError> {
    if !output.success {
        return Err(classify_roster_failure(&output.stderr));
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| IntelAgentRosterError::malformed_output())?;
    let values = roster_values(&value).ok_or_else(IntelAgentRosterError::malformed_output)?;
    let mut agents = values
        .iter()
        .map(parse_roster_entry)
        .collect::<Result<Vec<_>, _>>()?;
    agents.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    agents.dedup_by(|left, right| left.name == right.name);

    Ok(IntelAgentRosterResponse { agents })
}

fn roster_values(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    value.as_array().or_else(|| {
        ["agents", "items", "data"]
            .iter()
            .find_map(|key| value.get(key).and_then(serde_json::Value::as_array))
    })
}

fn parse_roster_entry(
    value: &serde_json::Value,
) -> Result<IntelAgentRosterEntry, IntelAgentRosterError> {
    if let Some(name) = value
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return Ok(IntelAgentRosterEntry {
            id: None,
            name: name.to_string(),
            description: None,
        });
    }

    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(IntelAgentRosterError::malformed_output)?;
    let id = value
        .get("agent_id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let description = value
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string);

    Ok(IntelAgentRosterEntry {
        id,
        name: name.to_string(),
        description,
    })
}

fn classify_roster_failure(stderr: &[u8]) -> IntelAgentRosterError {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if stderr.contains("intel auth:")
        || stderr.contains("status 401")
        || stderr.contains("status=401")
        || stderr.contains("status 403")
        || stderr.contains("status=403")
        || stderr.contains("unauthorized")
        || stderr.contains("forbidden")
    {
        return IntelAgentRosterError::auth();
    }
    if stderr.contains("list agents json:") {
        return IntelAgentRosterError::malformed_output();
    }
    IntelAgentRosterError::connection()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::*;

    fn output(success: bool, stdout: &str, stderr: &str) -> CapturedRosterOutput {
        CapturedRosterOutput {
            success,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn roster_success_parses_and_sorts_gateway_agents() {
        let parsed = parse_roster_output(output(
            true,
            r#"{"agents":[
                {"agent_id":"2","name":"Zulu","description":"Last"},
                {"id":"1","name":"alpha","description":"First"}
            ]}"#,
            "",
        ))
        .expect("valid roster");

        assert_eq!(
            parsed.agents,
            vec![
                IntelAgentRosterEntry {
                    id: Some("1".to_string()),
                    name: "alpha".to_string(),
                    description: Some("First".to_string()),
                },
                IntelAgentRosterEntry {
                    id: Some("2".to_string()),
                    name: "Zulu".to_string(),
                    description: Some("Last".to_string()),
                },
            ]
        );
    }

    #[test]
    fn roster_401_and_403_are_auth_errors() {
        for stderr in [
            "intel auth: status 401 unauthorized",
            "intel auth: status=403 forbidden",
        ] {
            let error = parse_roster_output(output(false, "", stderr))
                .expect_err("auth failure must be rejected");
            assert_eq!(error.code, "auth");
        }
    }

    #[test]
    fn roster_connection_failure_is_distinct_from_auth() {
        let error = parse_roster_output(output(
            false,
            "",
            "intel: list agents connect: dns lookup failed",
        ))
        .expect_err("connection failure must be rejected");

        assert_eq!(error.code, "connection");
        assert_ne!(error, IntelAgentRosterError::auth());
    }

    #[test]
    fn roster_malformed_stdout_is_structured_and_safe() {
        let secret = "intel_secret_must_not_escape";
        let error = parse_roster_output(output(true, "<html>not json</html>", secret))
            .expect_err("malformed output must be rejected");
        let serialized = serde_json::to_string(&error).expect("serialize error");

        assert_eq!(error.code, "malformedOutput");
        assert!(!serialized.contains(secret));
    }

    #[test]
    fn roster_failure_never_returns_stderr_or_api_key() {
        let secret = "intel_secret_must_not_escape";
        let error = parse_roster_output(output(
            false,
            "",
            &format!("list agents connect: failed with {secret}"),
        ))
        .expect_err("failure must be rejected");
        let serialized = serde_json::to_string(&error).expect("serialize error");
        let debug = format!("{error:?}");

        assert!(!serialized.contains(secret));
        assert!(!debug.contains(secret));
    }

    #[test]
    fn roster_command_passes_api_key_via_env_never_argv() {
        let secret = "intel_secret_must_not_appear_in_argv";
        let command =
            build_roster_command(Path::new("buzz-intel-agent"), "https://intel.test", secret);
        let args = command
            .get_args()
            .map(OsStr::to_string_lossy)
            .collect::<Vec<_>>();
        let api_key_env = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("INTEL_API_KEY"))
            .and_then(|(_, value)| value);

        assert_eq!(args, vec!["--list-agents"]);
        assert!(args.iter().all(|arg| !arg.contains(secret)));
        assert_eq!(api_key_env, Some(OsStr::new(secret)));
    }
}

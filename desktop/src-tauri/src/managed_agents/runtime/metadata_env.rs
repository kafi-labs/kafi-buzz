use crate::managed_agents::KnownAcpRuntime;

/// Returns the (key, value) env var pairs that should be forwarded to the
/// agent process for model and provider selection.
///
/// Model injection is unconditional — even agents that support ACP model
/// switching need the initial bootstrap value. Provider injection is controlled
/// independently from the UI's provider-catalog lock.
pub(crate) fn runtime_metadata_env_vars<'a>(
    model_env_var: Option<&'a str>,
    provider_env_var: Option<&'a str>,
    inject_provider_env: bool,
    effective_model: Option<&'a str>,
    effective_provider: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut vars = Vec::new();
    if let (Some(env_key), Some(model)) = (model_env_var, effective_model) {
        vars.push((env_key, model));
    }
    if inject_provider_env {
        if let (Some(env_key), Some(provider)) = (provider_env_var, effective_provider) {
            vars.push((env_key, provider));
        }
    }
    vars
}

/// Applies runtime-specific model and provider metadata to the actual child command.
pub(crate) fn apply_runtime_metadata_env(
    command: &mut std::process::Command,
    runtime: &KnownAcpRuntime,
    effective_model: Option<&str>,
    effective_provider: Option<&str>,
) {
    for (key, value) in runtime_metadata_env_vars(
        runtime.model_env_var,
        runtime.provider_env_var,
        runtime.inject_provider_env,
        effective_model,
        effective_provider,
    ) {
        command.env(key, value);
    }
}

#[cfg(test)]
mod tests {
    use crate::managed_agents::known_acp_runtime;

    use super::{apply_runtime_metadata_env, runtime_metadata_env_vars};

    #[test]
    fn runtime_metadata_env_vars_injects_model_and_provider() {
        let vars = runtime_metadata_env_vars(
            Some("GOOSE_MODEL"),
            Some("GOOSE_PROVIDER"),
            true,
            Some("gpt-4o"),
            Some("openai"),
        );
        assert_eq!(
            vars,
            vec![("GOOSE_MODEL", "gpt-4o"), ("GOOSE_PROVIDER", "openai")]
        );
    }

    #[test]
    fn runtime_metadata_env_vars_skips_provider_when_injection_is_disabled() {
        let claude = known_acp_runtime("claude").expect("claude runtime must be registered");
        assert!(claude.provider_locked);
        assert!(!claude.inject_provider_env);

        let vars = runtime_metadata_env_vars(
            claude.model_env_var,
            // Claude currently has no provider env key. Use a sentinel to prove
            // the independent injection flag, rather than passing vacuously on None.
            Some("CLAUDE_PROVIDER"),
            claude.inject_provider_env,
            Some("claude-opus-4-7"),
            Some("anthropic"),
        );
        assert!(vars.is_empty());
    }

    #[test]
    fn intel_runtime_metadata_injects_gateway_url_into_child_command() {
        let intel = known_acp_runtime("intel").expect("intel runtime must be registered");
        let gateway_url = "https://intel.example.test";
        let mut command = std::process::Command::new("buzz-intel-agent");

        apply_runtime_metadata_env(
            &mut command,
            intel,
            Some("buzz-cfo-agent"),
            Some(gateway_url),
        );

        let injected_gateway = command
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new("INTEL_GATEWAY_URL"))
            .and_then(|(_, value)| value);
        assert_eq!(injected_gateway, Some(std::ffi::OsStr::new(gateway_url)));
    }

    #[test]
    fn runtime_metadata_env_vars_injects_model_even_with_acp_model_switching() {
        // buzz-agent has supports_acp_model_switching=true but we still inject
        // the model env var because ACP model switching is post-bootstrap
        let vars = runtime_metadata_env_vars(
            Some("BUZZ_AGENT_MODEL"),
            Some("BUZZ_AGENT_PROVIDER"),
            true,
            Some("goose-claude-4-6-opus"),
            Some("databricks"),
        );
        assert_eq!(
            vars,
            vec![
                ("BUZZ_AGENT_MODEL", "goose-claude-4-6-opus"),
                ("BUZZ_AGENT_PROVIDER", "databricks"),
            ]
        );
    }
}

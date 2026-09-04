/// Intelligence Platform must be a first-class KNOWN_ACP_RUNTIMES entry so
/// `discover_acp_runtimes` → `discover_acp_providers` can surface it in the
/// desktop create flow.
#[test]
fn intel_runtime_is_discoverable_with_expected_metadata() {
    let intel = super::super::known_acp_runtime_exact("intel")
        .expect("intel must be registered in KNOWN_ACP_RUNTIMES");

    assert_eq!(intel.id, "intel");
    assert_eq!(intel.label, "Intelligence Platform");
    assert_eq!(intel.commands, &["buzz-intel-agent"]);
    assert_eq!(intel.mcp_command, None);
    assert_eq!(intel.model_env_var, Some("INTEL_AGENT"));
    assert_eq!(intel.provider_env_var, Some("INTEL_GATEWAY_URL"));
    assert!(
        intel.provider_locked,
        "intel must suppress the generic LLM provider catalog"
    );
    assert!(
        intel.inject_provider_env,
        "intel must inject its configured gateway URL into the child"
    );
    assert_eq!(
        intel.required_normalized_fields,
        &["model", "provider"],
        "create/config bridge should treat agent name + gateway as required"
    );
    assert_eq!(
        intel.api_key_env_var,
        Some("INTEL_API_KEY"),
        "create dialog requires INTEL_API_KEY secret field"
    );
    assert!(
        intel
            .login_hint
            .is_some_and(|h| h.contains("INTEL_API_KEY")),
        "login_hint should mention INTEL_API_KEY; got {:?}",
        intel.login_hint
    );

    let probe = intel
        .auth_probe_args
        .expect("intel must define auth_probe_args");
    assert!(!probe.is_empty(), "auth_probe_args must not be empty");
    // runtime_metadata.rs:60-62 — args[0] is the executable, not a flag.
    assert_eq!(
        probe[0], "buzz-intel-agent",
        "auth_probe_args[0] must be the executable (not --auth-probe)"
    );
    assert!(
        probe.contains(&"--auth-probe"),
        "auth_probe_args must include --auth-probe; got {probe:?}"
    );
}

#[test]
fn intel_runtime_resolves_by_command_identity() {
    let by_cmd = super::super::known_acp_runtime("buzz-intel-agent")
        .expect("known_acp_runtime must resolve command → intel");
    assert_eq!(by_cmd.id, "intel");
    let by_path = super::super::known_acp_runtime("/usr/local/bin/buzz-intel-agent")
        .expect("path form must also resolve to intel");
    assert_eq!(by_path.id, "intel");
}

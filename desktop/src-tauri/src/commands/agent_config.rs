use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        config_bridge::{
            read_goose_file_config,
            reader::read_config_surface,
            types::{
                AcpConfigOptionEntry, AcpConfigOptionValue, AcpModelEntry, ConfigOrigin,
                NormalizedField, RuntimeConfigSurface, SessionConfigCache,
            },
        },
        current_instance_id, known_acp_runtime, load_managed_agents, load_personas,
        resolve_effective_prompt_model_provider, save_managed_agents, sync_managed_agent_processes,
        AgentDefinition, GlobalAgentConfig, KnownAcpRuntime, ManagedAgentRecord,
        ManagedAgentRuntimeKey,
    },
};

/// Subset of the goose file config exposed to the frontend for gate evaluation.
///
/// Only the fields the dialog gate needs — not the full `RuntimeConfigSurface`.
/// The gate uses this to know which requirements are already satisfied in the
/// harness config file, so it can show "Set in goose config" rather than
/// surfacing a false missing-key marker.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFileConfigSubset {
    /// Provider set in the harness config file, if any.
    pub provider: Option<String>,
    /// Model set in the harness config file, if any.
    pub model: Option<String>,
    /// Flat credential env keys found in the harness config file's `extra` map
    /// (e.g. `DATABRICKS_HOST`).  Only non-empty values are included.
    pub satisfied_env_keys: Vec<String>,
}

/// Resolve the config surface with persona and global default values applied.
///
/// The pipeline: resolve the linked persona's prompt/model/provider, inject
/// each into the record only where the record lacks its own value, let
/// `read_config_surface` tag those injected fields `BuzzExplicit`, then re-tag
/// exactly the injected fields to `PersonaDefault`.
///
/// Global defaults fill in when neither the record nor the linked persona
/// provides a value. They are re-tagged to `GlobalDefault` so the UI can
/// display "inherited from global defaults".
///
/// The re-tag is triple-gated — a field is re-tagged only when (a) the record
/// did not already have it (`!had_*`), (b) the surface produced the field, and
/// (c) the reader tagged it `BuzzExplicit`. A value the user set explicitly in
/// Buzz keeps `had_* == true` and is never re-tagged.
fn resolve_config_surface(
    mut record: ManagedAgentRecord,
    personas: &[AgentDefinition],
    runtime_meta: Option<&KnownAcpRuntime>,
    session_cache: Option<&SessionConfigCache>,
    global: &GlobalAgentConfig,
) -> RuntimeConfigSurface {
    let had_prompt =
        record.system_prompt.is_some() || record.env_vars.contains_key("BUZZ_ACP_SYSTEM_PROMPT");
    let had_model = record.model.is_some();

    let provider_env_key = runtime_meta.and_then(|m| m.provider_env_var).unwrap_or("");
    let had_provider = record.env_vars.contains_key(provider_env_key);

    let (persona_prompt, persona_model, persona_provider) = resolve_effective_prompt_model_provider(
        record.persona_id.as_deref(),
        personas,
        record.system_prompt.clone(),
        record.model.clone(),
        record.provider.clone(),
    );

    // Build the baseline the reader overrides a live model against, paired with
    // its true origin so the secondary is tagged correctly. Two sources:
    //   - persona-linked, no explicit record model: the persona model is the
    //     baseline (PersonaDefault).
    //   - genuine-explicit (record had its own model) that live-switched: the
    //     record's own model is the baseline (BuzzExplicit). Gated behind
    //     `model_overridden` so a persona edited mid-life (override flag false)
    //     never synthesizes a baseline and false-positives an override.
    // An explicit pick with no live switch has no baseline to override.
    let model_overridden = session_cache.is_some_and(|c| c.model_overridden);
    let baseline = if had_model {
        if model_overridden {
            record
                .model
                .clone()
                .map(|m| (m, ConfigOrigin::BuzzExplicit))
        } else {
            None
        }
    } else {
        // Prefer persona as baseline, fall back to global when persona has none
        // and the model was overridden mid-session (global-default agent).
        persona_model
            .clone()
            .map(|m| (m, ConfigOrigin::PersonaDefault))
            .or_else(|| {
                if model_overridden {
                    global
                        .model
                        .clone()
                        .map(|m| (m, ConfigOrigin::GlobalDefault))
                } else {
                    None
                }
            })
    };

    // Inject resolved persona values into the record where absent.
    if !had_prompt {
        if let Some(p) = persona_prompt {
            record
                .env_vars
                .insert("BUZZ_ACP_SYSTEM_PROMPT".to_string(), p);
        }
    }
    if !had_model {
        record.model = persona_model.clone();
    }
    if !had_provider && !provider_env_key.is_empty() {
        if let Some(prov) = persona_provider {
            record.env_vars.insert(provider_env_key.to_string(), prov);
        }
    }

    // Inject global defaults where neither the record nor the persona had a value.
    // Track injection so we can re-tag to GlobalDefault after the reader.
    let inject_global_model = !had_model && record.model.is_none();
    let inject_global_provider = !had_provider
        && !provider_env_key.is_empty()
        && !record.env_vars.contains_key(provider_env_key);

    if inject_global_model {
        record.model = global.model.clone();
    }
    if inject_global_provider {
        if let Some(ref gprov) = global.provider {
            record
                .env_vars
                .insert(provider_env_key.to_string(), gprov.clone());
        }
    }

    let mut surface = read_config_surface(
        &record,
        runtime_meta,
        session_cache,
        baseline.as_ref().map(|(m, o)| (m.as_str(), o.clone())),
    );

    // Re-tag persona-sourced fields from BuzzExplicit to PersonaDefault.
    if !had_prompt {
        retag_persona_default(&mut surface.normalized.system_prompt);
    }
    if !had_model && !inject_global_model {
        retag_persona_default(&mut surface.normalized.model);
    }
    if !had_provider && !provider_env_key.is_empty() && !inject_global_provider {
        retag_persona_default(&mut surface.normalized.provider);
    }

    // Re-tag global-sourced fields from BuzzExplicit to GlobalDefault.
    if inject_global_model {
        retag_global_default(&mut surface.normalized.model);
    }
    if inject_global_provider {
        retag_global_default(&mut surface.normalized.provider);
    }

    // Re-tag persona-snapshotted model from BuzzExplicit to PersonaDefault.
    // Persona-created agents have record.model set at create time from the
    // persona snapshot — had_model is true, but the model came from the persona,
    // not an explicit user choice. Re-tag when the record model matches the
    // persona model and no live override is active. Only applies when a persona
    // is actually linked — non-persona agents with an explicit model keep BuzzExplicit.
    if had_model && !model_overridden && record.persona_id.is_some() {
        if let (Some(ref record_model), Some(ref persona_model_val)) =
            (&record.model, &persona_model)
        {
            if record_model == persona_model_val {
                retag_persona_default(&mut surface.normalized.model);
            }
        }
    }

    surface
}

/// Re-tag a field's origin from `BuzzExplicit` to `PersonaDefault`, leaving any
/// other origin untouched. No-op when the field is absent.
fn retag_persona_default(field: &mut Option<NormalizedField>) {
    if let Some(field) = field {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::PersonaDefault;
        }
    }
}

/// Get the file-layer config for a runtime — used by the Create/Edit/Persona
/// dialogs to know which requirements are already satisfied in the harness
/// config file (e.g. `~/.config/goose/config.yaml`), so they can show
/// "Set in goose config" instead of surfacing a false required-field marker.
///
/// Returns `null` when the runtime has no config file or it cannot be parsed.
/// Currently only "goose" is supported; other runtimes return `null`.
#[tauri::command]
pub async fn get_runtime_file_config(
    runtime_id: String,
) -> Result<Option<RuntimeFileConfigSubset>, String> {
    tokio::task::spawn_blocking(move || match runtime_id.as_str() {
        "goose" => {
            let cfg = read_goose_file_config()?;
            let satisfied_env_keys = cfg
                .extra
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, _)| k)
                .collect();
            Some(RuntimeFileConfigSubset {
                provider: cfg.provider,
                model: cfg.model,
                satisfied_env_keys,
            })
        }
        _ => None,
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Return the key names of all non-empty baked build env vars.
///
/// Internal (Block) builds bake provider credentials and other env pairs into
/// the binary at compile time via `BUZZ_BUILD_AGENT_ENV`. The backend readiness
/// gate already treats these keys as satisfying their requirements (Layer 1 of
/// `resolve_effective_agent_env`). This command exposes the *key names only* —
/// never the values — so the frontend dialogs can apply the same logic and avoid
/// surfacing a spurious "Required" badge for keys that are covered by the baked
/// env.
///
/// OSS builds have no baked env, so this returns an empty list — OSS behavior
/// is unchanged.
#[tauri::command]
pub fn get_baked_build_env_keys() -> Vec<String> {
    crate::managed_agents::baked_build_env()
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| k)
        .collect()
}

/// A single baked build env entry returned to the frontend.
///
/// Values are masked in Rust so unmasked secret values never cross the
/// Tauri IPC boundary. The `masked` flag lets the frontend style masked
/// rows distinctly.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BakedEnvEntry {
    pub key: String,
    /// The display value — real value for non-secret keys, `••••••` for
    /// secret keys whose names match the secret heuristic.
    pub value: String,
    /// `true` when the value was replaced by the mask placeholder.
    pub masked: bool,
}

/// Returns `true` when a baked-env key is safe to display unmasked in the UI.
///
/// This uses an explicit allowlist of keys that are known safe (non-secret).
/// Any key NOT in this set is masked — default-deny for a security surface.
///
/// Allowlist (case-insensitive):
/// - `BUZZ_AGENT_PROVIDER`, `BUZZ_AGENT_MODEL` — agent runtime selection
/// - `BUZZ_AGENT_THINKING_EFFORT` — non-secret enum (none/minimal/low/medium/high/xhigh/max)
/// - `DATABRICKS_HOST`, `DATABRICKS_MODEL` — Block non-secret defaults
fn is_safe_to_reveal(key: &str) -> bool {
    const SAFE_KEYS: &[&str] = &[
        "BUZZ_AGENT_PROVIDER",
        "BUZZ_AGENT_MODEL",
        "BUZZ_AGENT_THINKING_EFFORT",
        "DATABRICKS_HOST",
        "DATABRICKS_MODEL",
    ];
    let upper = key.to_ascii_uppercase();
    SAFE_KEYS.iter().any(|safe| upper == *safe)
}

/// Expose the baked build env to the frontend with values shown, but any
/// key not in the safe-to-reveal allowlist has its value replaced by `••••••`.
///
/// Provider and model arrive as `BUZZ_AGENT_PROVIDER` / `BUZZ_AGENT_MODEL`
/// keys in `baked_build_env()` and are included in the returned list like any
/// other key. Empty-value keys are filtered out (same as
/// `get_baked_build_env_keys`).
///
/// OSS builds return an empty list — the baked-env section is hidden entirely
/// in OSS installations.
#[tauri::command]
pub fn get_baked_build_env() -> Vec<BakedEnvEntry> {
    crate::managed_agents::baked_build_env()
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(key, value)| {
            let masked = !is_safe_to_reveal(&key);
            let display_value = if masked {
                "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string()
            } else {
                value
            };
            BakedEnvEntry {
                key,
                value: display_value,
                masked,
            }
        })
        .collect()
}

/// Re-tag a field's origin from `BuzzExplicit` to `GlobalDefault`, leaving any
/// other origin untouched. No-op when the field is absent.
fn retag_global_default(field: &mut Option<NormalizedField>) {
    if let Some(field) = field {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::GlobalDefault;
        }
    }
}

/// Get the full config surface for a managed agent.
///
/// Returns normalized + advanced config from all available tiers.
/// Pre-spawn agents show config file values with ACP tiers marked as pending.
/// Persona-sourced values are resolved by `resolve_config_surface`.
#[tauri::command]
pub async fn get_agent_config_surface(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeConfigSurface, String> {
    let record = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }
        records
            .into_iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };

    let personas = load_personas(&app).unwrap_or_default();
    let effective_cmd = crate::managed_agents::record_agent_command(&record, &personas);
    let runtime_meta = known_acp_runtime(&effective_cmd);
    let runtime_key = ManagedAgentRuntimeKey::new(
        pubkey.clone(),
        &crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &crate::relay::relay_ws_url_with_override(&state),
        ),
    )?;
    let session_cache = state.get_session_cache(&runtime_key);
    let global = crate::managed_agents::load_global_agent_config(&app).unwrap_or_default();

    Ok(resolve_config_surface(
        record,
        &personas,
        runtime_meta,
        session_cache.as_ref(),
        &global,
    ))
}

/// Store a `session_config_captured` observer event payload into the session cache.
///
/// Called by the TypeScript observer relay when it decrypts a `session_config_captured`
/// event from a running agent. The payload contains raw ACP session/new fields.
#[tauri::command]
pub fn put_agent_session_config(
    pubkey: String,
    payload: serde_json::Value,
    app: AppHandle,
    state: State<'_, AppState>,
) {
    let record_relay_url = {
        let _guard = match state.managed_agents_store_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match load_managed_agents(&app) {
            Ok(records) => match records.into_iter().find(|r| r.pubkey == pubkey) {
                Some(record) => record.relay_url,
                None => return,
            },
            _ => return,
        }
    };

    // Pair identity: prefer the relay URL the harness attached to the payload
    // (same pattern as lifecycle frames). Older harnesses don't attach one;
    // fall back to the record's effective relay — with no attached URL the
    // frame can only have arrived over the active workspace relay, which is
    // exactly what effective_agent_relay_url resolves to absent a pin.
    let relay_url = payload
        .get("relayUrl")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            crate::relay::effective_agent_relay_url(
                &record_relay_url,
                &crate::relay::relay_ws_url_with_override(&state),
            )
        });

    let config_options = parse_config_options(payload.get("configOptions"));
    let available_modes = parse_modes(&config_options, payload.get("modes"));
    let (available_models, current_model) = parse_models(payload.get("models"));
    let model_overridden = payload
        .get("modelOverridden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let cache = SessionConfigCache {
        config_options,
        available_modes,
        available_models,
        current_model,
        model_overridden,
        goose_native_config: None,
        captured_at: crate::util::now_iso(),
    };

    let Ok(runtime_key) = ManagedAgentRuntimeKey::new(pubkey, &relay_url) else {
        return;
    };
    state.put_session_cache(runtime_key, cache);
}

fn parse_config_options(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionEntry> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|opt| {
            let config_id = opt
                .get("id")
                .or_else(|| opt.get("configId"))?
                .as_str()?
                .to_string();
            Some(AcpConfigOptionEntry {
                config_id,
                category: opt
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                display_name: opt
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                current_value: opt
                    .get("value")
                    .or_else(|| opt.get("currentValue"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                options: parse_option_values(opt.get("options")),
            })
        })
        .collect()
}

fn parse_option_values(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionValue> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|o| {
            let value = o.get("value").and_then(|v| v.as_str())?.to_string();
            Some(AcpConfigOptionValue {
                value,
                display_name: o
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

fn parse_modes(
    config_options: &[AcpConfigOptionEntry],
    raw: Option<&serde_json::Value>,
) -> Vec<String> {
    if let Some(arr) = raw.and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|m| m.as_str().map(str::to_string))
            .collect();
    }
    // Fall back: extract mode options from configOptions with category "mode".
    config_options
        .iter()
        .filter(|o| o.category.as_deref() == Some("mode"))
        .flat_map(|o| o.options.iter().map(|v| v.value.clone()))
        .collect()
}

fn parse_models(raw: Option<&serde_json::Value>) -> (Vec<AcpModelEntry>, Option<String>) {
    let raw = match raw {
        Some(v) => v,
        None => return (Vec::new(), None),
    };

    // Object shape: { currentModelId, availableModels: [...] }
    if let Some(obj) = raw.as_object() {
        let current_model = obj
            .get("currentModelId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let models = obj
            .get("availableModels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let model_id = m
                            .get("modelId")
                            .or_else(|| m.get("id"))
                            .and_then(|v| v.as_str())?
                            .to_string();
                        Some(AcpModelEntry {
                            model_id,
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return (models, current_model);
    }

    // Array shape: [{ modelId, isCurrent, ... }]
    let arr = match raw.as_array() {
        Some(a) => a,
        None => return (Vec::new(), None),
    };
    let mut current_model = None;
    let models = arr
        .iter()
        .filter_map(|m| {
            let model_id = m
                .get("modelId")
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())?
                .to_string();
            if m.get("isCurrent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                current_model = Some(model_id.clone());
            }
            Some(AcpModelEntry {
                model_id,
                name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                description: m
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect();
    (models, current_model)
}

#[cfg(test)]
mod tests;

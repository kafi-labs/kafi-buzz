//! `buzz-intel-agent` — ACP adapter for Intelligence Platform agents.
//!
//! Speaks ACP (JSON-RPC 2.0 NDJSON) on stdio and REST/SSE toward the intel
//! gateway. Posts replies to the Buzz relay with `buzz-sdk`.
//!
//! See `specs/intel-agent-integration/04-adapter-spec.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod acp;
pub mod chunk;
pub mod config;
pub mod error;
pub mod intel;
pub mod prompt;
pub mod quota;
pub mod reply;
pub mod session_ensure;
pub mod state;
pub mod wire;

use clap::Parser;

use crate::config::{Cli, Config};
use crate::error::AdapterError;
use crate::intel::IntelClient;

/// Entry point used by the binary: parse CLI, handle one-shot modes, or run ACP.
pub async fn run() -> Result<(), AdapterError> {
    let cli = Cli::parse();

    // Missing intel config is allowed for the ACP server path so `initialize`
    // can return a JSON-RPC error; one-shot modes require only gateway credentials.
    let cfg = Config::from_cli(&cli)?;

    if cli.auth_probe {
        cfg.require_gateway_credentials()?;
        return auth_probe(&cfg).await;
    }
    if cli.list_agents {
        cfg.require_gateway_credentials()?;
        return list_agents(&cfg).await;
    }

    // ACP server: tracing to stderr only.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    acp::run_server(cfg).await
}

async fn auth_probe(cfg: &Config) -> Result<(), AdapterError> {
    // Minimal logging on stderr for probe failures.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .init();

    let client = IntelClient::new(cfg)?;
    match client.whoami().await {
        Ok(v) => {
            eprintln!("auth-probe: ok");
            // Optional detail to stderr only.
            if let Ok(s) = serde_json::to_string(&v) {
                eprintln!("{s}");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("auth-probe: failed: {e}");
            Err(e)
        }
    }
}

async fn list_agents(cfg: &Config) -> Result<(), AdapterError> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .init();

    let client = IntelClient::new(cfg)?;
    let agents = client.list_agents().await?;
    // Spec: JSON to stdout (this is a one-shot mode, not ACP).
    println!("{}", serde_json::to_string_pretty(&agents)?);
    Ok(())
}

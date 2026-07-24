//! Binary entry point for `buzz-intel-agent`.

fn main() {
    // Install rustls ring provider before any TLS client is built (same
    // rationale as buzz-cli / buzz-acp multi-package builds).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Error: failed to build tokio runtime: {e}");
            std::process::exit(2);
        }
    };

    if let Err(e) = rt.block_on(buzz_intel_agent::run()) {
        eprintln!("Error: {e}");
        // auth-probe / list-agents failures exit 1
        std::process::exit(1);
    }
}

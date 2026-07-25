# 11 — Live E2E Results (buzz-intel-agent ↔ intel-platform)

(Prior iteration sections may live in git history / orchestrator copies; this file was recreated for Iteration-8 append.)

## Iteration-8 — persistent service liveness (remote)

**When:** 2026-07-25T00:18:11Z  
**Target:** `wss://vm-buzz-relay-dev-wren.exe.xyz` · channel `b6b6fab0-1e90-46e0-9540-7a445124dd59` · always-on `buzz-intel-agent.service` (no local harness)  
**Verdict: PASS** — prompt `316c0b33ad4a57654621ad109edebaad1b17fcb32d489e5e06ec5dff73d7e62c` → reply `997b232e44f2edf7a38d994dc32e8708088e8491a5bfc8da51676634c3d265a7`: Masih hidup! Ada yang bisa saya bantu?

## Iteration-9 CI gate

**When:** 2026-07-25T01:15Z · branch `feat/intel-acp-adapter` @ `62cb06cd` (7 commits) · `just ci`  
**Verdict: RED** — `fmt-check` PASS; `clippy` FAIL (2× `buzz-intel-agent`); remaining stages not run. Failures: (1) `clippy::items_after_test_module` in `crates/buzz-intel-agent/src/acp.rs:917` — production helpers after `mod tests`; (2) `clippy::sliced_string_as_bytes` in `crates/buzz-intel-agent/src/intel.rs:948` — use `&full.as_bytes()[w[0]..w[1]]`. Not trivial fmt; left unfixed per gate policy. **Not PR-ready until clippy is green.**

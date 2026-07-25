# 11 — Live E2E Results (buzz-intel-agent ↔ intel-platform)

(Prior iteration sections may live in git history / orchestrator copies; this file was recreated for Iteration-8 append.)

## Iteration-8 — persistent service liveness (remote)

**When:** 2026-07-25T00:18:11Z  
**Target:** `wss://vm-buzz-relay-dev-wren.exe.xyz` · channel `b6b6fab0-1e90-46e0-9540-7a445124dd59` · always-on `buzz-intel-agent.service` (no local harness)  
**Verdict: PASS** — prompt `316c0b33ad4a57654621ad109edebaad1b17fcb32d489e5e06ec5dff73d7e62c` → reply `997b232e44f2edf7a38d994dc32e8708088e8491a5bfc8da51676634c3d265a7`: Masih hidup! Ada yang bisa saya bantu?

## Iteration-9 CI gate

**When:** 2026-07-25T01:48Z · branch `feat/intel-acp-adapter` · `just ci`  
**Verdict: GREEN** — full gate pass after clippy fix commit `7f940957` (`7f9409577bf292f65e2d3a19d227357edc52bf42`).

| Stage | Result |
|---|---|
| fmt-check | PASS |
| clippy (workspace) | PASS |
| desktop-check (biome + file-sizes + px-text + pubkey) | PASS |
| desktop-tauri-fmt-check | PASS |
| desktop-tauri-clippy | PASS |
| web-check | PASS |
| mobile-check (dart format + flutter analyze + file-sizes) | PASS |
| test-unit (buzz-core/auth/db/conformance/push-gateway) | PASS |
| desktop-test | PASS |
| desktop-build | PASS |
| desktop-tauri-check | PASS |
| desktop-tauri-test (1616 passed) | PASS |
| web-build | PASS |
| mobile-test (541 passed) | PASS |

Fixes applied: (1) move production helpers above `#[cfg(test)] mod tests` in `acp.rs`; (2) `&full.as_bytes()[w[0]..w[1]]` in `intel.rs`; (3) ratchet file-size overrides for intel create-flow growth; (4) biome `noUselessTernary` on e2eBridge `provider_locked`. **PR-ready (local gate).** Not pushed.

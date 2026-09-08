# Adversarial verification: a440c5e6a (intel-agent re-land)

Independent verifier, worktree `ops-verify-reland-gates`, based on a440c5e6a.
Author's own tests are not re-run as evidence of anything beyond "author's
tests still pass" — findings below are things those tests do not construct.

## Checks run (want=/got=)

| Check | want | got | exit |
|---|---|---|---|
| `cargo test -p buzz-intel-agent --lib -j1 --test-threads=1` | 88 passed | 88 passed, 0 failed | 0 |
| `cargo test -p buzz-intel-agent --test mock_gateway_e2e -j1 --test-threads=1` | 14 passed | 14 passed, 0 failed | 0 |
| `cargo fmt -p buzz-intel-agent --check` | clean | clean (no diff) | 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | claimed exit 0 | **not run** — host load/swap risk (see task brief); scoped fmt+test above substituted | n/a |
| `git grep` 3 target markers on parent commit 37070335d | 0 hits | 0 hits | confirms markers attributable to this diff |
| `git grep` 3 target markers + positive control on HEAD | hits present | 40 hits total across the 4 terms | confirms present post-change |

Release-build+strings marker verification (author's own step) was not
independently repeated here — the debug-profile test binary already proves
the code paths execute (e.g. `whoami_headers_timeout_is_retryable` measures
real elapsed time against a hung mock server), which is stronger evidence
than a string being linked into a binary.

## TASK 1 findings

### FINDING 1 (HIGH): shutdown-interrupted-turn notice loses a race against a
freshly-dispatched `session/prompt`, silently reverting to no-notice for
exactly the turn most likely to be in flight at shutdown

**File/lines:** `crates/buzz-intel-agent/src/acp.rs`
- `handle_request`, `"session/prompt"` arm, L260-309: registers the task in
  `app.prompt_tasks` (a `JoinSet`) via `tasks.spawn(...)` *before* the task
  body has run.
- `session_prompt`, L505-508: inside that spawned task body,
  unconditionally does `let _ = s.cancel_tx.send(CancelCause::None);` then
  `s.cancel_tx.subscribe()` — this is the *first* code in the task to touch
  the cancel channel.
- `graceful_shutdown`, L171-177: iterates `app.sessions` and does
  `s.cancel_tx.send(CancelCause::Shutdown)` for every session, with no
  interaction with `app.prompt_tasks` until after this loop completes.
- `run_server`, L143-161: races `read_fut` (the stdin read loop) against
  `sigterm.recv()` in a `tokio::select!`; `read_fut` *also* completes
  naturally on stdin EOF, which the doc comments treat as an equally valid
  shutdown trigger ("SIGTERM/stdin EOF" throughout the commit message and
  `CancelCause::Shutdown`'s own doc comment).

**Concrete trigger:** a client sends a `session/prompt` line and then closes
stdin (or the process receives SIGTERM) before that spawned task's body gets
its first poll. `tokio::spawn`/`JoinSet::spawn` never runs synchronously —
there is always at least one scheduling gap between "task registered" and
"task body starts executing" — so this is not a contrived timing edge case.
It is also the *natural* shape of "one last prompt, then shut down", which
is presumably the primary real-world scenario (a supervising harness sends a
final message then tears the subprocess down for a redeploy).

**Sequence:**
1. `dispatch` → `handle_request` registers the new turn's task in
   `prompt_tasks` and returns. The task has not run yet.
2. Shutdown triggers (EOF or SIGTERM). `graceful_shutdown` locks `sessions`
   and sends `CancelCause::Shutdown` on this session's `cancel_tx`.
3. The task now gets its first poll, runs `session_prompt`, and executes
   `s.cancel_tx.send(CancelCause::None)` — this **unconditionally overwrites**
   the `Shutdown` value just sent in step 2 (last-write-wins, `watch`
   channel has no compare-and-set). It then `subscribe()`s, capturing `None`.
4. The turn proceeds as an ordinary, uncancelled turn. Nothing in
   `run_turn`/`ensure_and_run` ever observes `CancelCause::Shutdown` for
   this receiver, because it was clobbered before this receiver existed.
   `notify_if_shutdown_cancelled` (L390-420) checks
   `*cancel_rx.borrow() != CancelCause::Shutdown` and will never see it.
5. `graceful_shutdown`'s bounded wait on `prompt_tasks`
   (`SHUTDOWN_GRACE_TIMEOUT` = 2500ms) times out (a live turn takes far
   longer), logs `"in-flight session/prompt task(s) did not finish...;
   proceeding"`, and returns anyway.
6. `run_server` returns; `main.rs` (L19) drops the `tokio::runtime::Runtime`
   at the end of `block_on`, which aborts the still-running task outright.

**Net effect:** the turn is dropped by shutdown exactly as this feature
describes, but the "⚠️ This turn was interrupted by a service restart"
notice — the entire point of this sub-feature — never posts, and the asker
gets nothing. This is silent, not merely delayed.

**Why the author's tests miss it:** zero tests reference `SIGTERM`,
`graceful_shutdown`, or `CancelCause::Shutdown` in the unit or e2e suites
(`grep` returns no hits in `mock_gateway_e2e.rs`). The feature's happy path
(cancel already visible before/registered task starts touching the channel)
is plausible to hand-test manually and looks correct; the loss only shows up
when the *arrival* of the new turn and the *shutdown* are concurrent, which
requires either real process-level signal timing or a scheduler-level
stall — neither is exercised.

**Not empirically reproduced:** deterministically forcing a two-task async
scheduling race without either instrumenting production code (out of scope
for a verifier) or a flaky sleep-based test is not something I attempted.
This is reported as CONFIRMED by static trace + `tokio::sync::watch`
semantics (send is last-write-wins with no CAS, spawn always requires a
scheduling gap before first poll), not by empirical timing capture. I did
verify `CancelCause` derives `PartialEq, Eq` and the comparisons are exact
equality, so there's no rounding/tolerance that could mask this.

**Suggested fix shape (not applied — verifier does not fix):**
`session_prompt`'s reset should not unconditionally stomp the channel; it
should either check for a pre-existing `Shutdown` value before resetting (and
skip starting the turn / immediately notify instead), or `graceful_shutdown`
should re-send `Shutdown` to sessions *after* joining/waiting once
(insufficient alone — the task could still not have run at all before the
process tears down), or simplest: `prompt_tasks` should be locked/consulted
by `graceful_shutdown` and the *task registration* itself, atomically with
respect to the per-session send, e.g. check the session's *current* status
under the same lock used for `app.sessions`, not two independent locks.

### Confirmed NOT regressed (re: "what might the re-land have lost")
- Debug secret-redaction (`config.rs` `fmt::Debug` impls): present, tests
  pass (`debug_output_redacts_cli_and_config_secrets_but_keeps_context`,
  `summary_never_contains_the_api_key`,
  `intel_session_created_summary_never_contains_the_api_key`).
- Incomplete-response detection (`notify_incomplete_answer`, both call
  sites in `ensure_and_run`): present, untouched by the diff, e2e-covered
  (`e2e_channel_less_incomplete_response_notifies_acp_client`,
  `e2e_clean_eof_without_terminal_does_not_publish_partial_response`,
  `e2e_done_with_empty_or_whitespace_response_posts_safe_error`).
- Quota eviction (`quota.rs` `evict_expired_at`): file untouched by this
  diff at all (diff only touches acp/chunk/config/error/intel/lib.rs), tests
  pass (`expired_windows_are_evicted`,
  `enforcement_evicts_expired_scopes_via_check_and_record`).

### response_headers_timeout / 429 handling: no defect found
All 4 outbound call sites (`whoami`, `list_agents`, `create_session`,
`send_message_stream`) route through the new `send_bounded` wrapper — grep
confirms no direct `self.http....send().await` remains outside it. The
timeout is exercised against a real hung `axum` listener in
`whoami_headers_timeout_is_retryable` /
`send_message_stream_headers_timeout_is_retryable`, measuring wall-clock
elapsed — this is a behavioral test, not a string-presence check.
`IntelRateLimited` is matched ahead of the generic transient-retry arm in
both `run_turn`'s outer match (covers session-creation `?`-propagated 429)
and `ensure_and_run`'s SSE loop (covers mid-stream 429) — traced by hand,
no gap between the two call paths.

## TASK 2: quota bypass on the wren-deployed binary — RESOLVED, not
unmeasurable

Prior state (per orchestrator brief): inference only — `integration/intel-gates`
has session-keyed channel-less quota traffic (breaks the shared-bucket
anti-bypass test), and the wren binary is *believed* built from that branch.
Goal: an independent discriminator against the actual binary, or an honest
"unmeasurable."

### Markers rejected as false-positive traps (checked, not used)
- `"acp:"` — present in **both** the fixed and buggy source trees, because
  `session_mapping_key` (`acp.rs`) legitimately builds `format!("acp:{acp_session_id}")`
  for ACP session-mode mapping keys in both versions. A blind `strings | grep
  "acp:"` on the binary would read PRESENT regardless of which branch built
  it — exactly the http-header-table trap the brief warned about.
- `"nochannel"` — present in **both**, because `quota::scope_key`'s
  `channel.unwrap_or("nochannel")` fallback compiles into the binary
  whether or not any caller ever passes `None` for `channel` (dead branch in
  the buggy build, live in the fixed build, textually identical either way).

### Working discriminator: per-function binary disassembly, not blind `strings`
The two source trees differ only in `quota_scope_key`'s own body
(`crates/buzz-intel-agent/src/acp.rs`):
- Fixed (this branch, `feat/reland-intel-gates`): 2-arg
  `fn quota_scope_key(cfg: &Config, parsed: &ParsedPrompt) -> String`, channel
  falls back to `None` — no `format!("acp:...")` call anywhere in this
  function.
- Buggy (`integration/intel-gates`, `git show integration/intel-gates:crates/buzz-intel-agent/src/acp.rs`):
  3-arg `fn quota_scope_key(cfg: &Config, acp_session_id: &str, parsed: &ParsedPrompt) -> String`,
  channel falls back to `.unwrap_or_else(|| format!("acp:{acp_session_id}"))`
  — this function itself references the `"acp:"` literal.

So the question becomes function-scoped: does *this specific function's*
compiled instruction range reference the `"acp:"` string constant, not
whether the string exists anywhere in the binary.

**Method (all read-only, via `ssh vm-buzz-relay-dev-wren.exe.xyz`):**
1. `/opt/buzz-intel/bin/buzz-intel-agent`: ELF, **not stripped**
   (`BuildID=da573a1ee18c867a4c41bd6cec228321187c2590`), sha256
   `0785fb17560f8caf288dce5af476fea55329885aa76b051a10558610688344cd`
   (matches the sha recorded in prior session memory for the binary running
   on wren).
2. `nm -S` located `buzz_intel_agent::acp::quota_scope_key` at
   `0x436cb0`, size `0x584` bytes, and located the `"acp:"` string constant's
   address (`0x5e8fb`, byte-dumped via `objdump -s`: `04 61 63 70 3a` =
   length-prefixed `"acp:"`).
3. **Positive control:** disassembled `buzz_intel_agent::acp::session_mapping_key`
   (`0x437630`, size `0x116`) — known from source to reference `"acp:"` in
   *both* branches. Found `lea ...,%rsi # 5e8fb` at offset `0x4376d8`,
   inside the function's own range. Confirms the method correctly detects a
   known-present reference.
4. **Negative controls:** disassembled `jitter_backoff` (`0x436910`,
   `0x8f` bytes) and `intel::intel_rate_limited_message` (`0x441f60`,
   `0xb9` bytes) — neither has any source-level reason to touch `"acp:"`.
   `grep 5e8fb` on both disassembly ranges: no match, in both cases.
   Confirms the method does not spuriously match everywhere.
5. **Target probe:** disassembled `quota_scope_key`'s own range
   (`0x436cb0`–`0x437234`). Found `lea -0x3d87b2(%rip),%rsi # 5e8fb` at
   `0x4370a6`, inside the function's own range, immediately preceded by a
   `lea` loading the `Display::fmt` vtable pointer for the formatted
   argument (`0x43709a`) — the standard codegen shape for
   `format!("acp:{}", x)`'s literal-piece array.

**want:** `quota_scope_key` (fixed/shared-bucket source) has no reason to
reference `"acp:"` at all.
**got:** the deployed binary's `quota_scope_key` references `"acp:"`
internally, matching only the session-keyed/buggy source.

**Verdict: CONFIRMED, not inferred.** The wren-deployed binary
(`sha256:0785fb17...`) contains the session-keyed `quota_scope_key`, i.e.
channel-less ACP traffic is scoped by ACP session id rather than the shared
`nochannel` bucket. `e2e_missing_channel_cannot_bypass_quota_with_new_acp_session`
would fail against this binary's actual logic (a new ACP session gets a
fresh quota key, defeating the anti-bypass design) — this is a live,
currently-deployed quota bypass, not a hypothetical from source review.

Not attempted: sending real ACP traffic at the live wren process to observe
the bypass behaviorally. That would consume real quota/gateway calls against
a semi-live service and was out of scope for a read-only verification pass.

## VERDICT: SOUND-WITH-FINDINGS

1. **HIGH** — shutdown-interrupted-turn notice race (Task 1, Finding 1),
   `acp.rs` `session_prompt`/`graceful_shutdown`. Silent, not cosmetic: the
   turn is still dropped, only the notice is lost, in the highest-probability
   trigger case (last-prompt-then-EOF).
2. **HIGH (pre-existing, now confirmed not inferred)** — quota bypass live
   on the wren-deployed binary (Task 2). Not introduced by a440c5e6a — this
   commit explicitly and correctly declined to port the change that causes
   it — but the currently-running production-adjacent binary has it, and
   this is now evidenced at the binary level, not just by branch provenance.

The re-land itself (a440c5e6a) is internally sound: all four claimed
markers verified via source, tests, or binary trace as attributable to this
commit; nothing in scope (secret redaction, incomplete-response detection,
quota eviction) regressed; 429/response-headers-timeout wiring traced
end-to-end with no gap. Finding 1 is a defect in the newly-added code that
the author's own tests do not construct. Finding 2 is a live production
exposure, external to this commit's own diff, that the orchestrator's open
question asked to be closed or marked unmeasurable — it is closed.

# Adversarial re-verification: f13dacc4c (fixes round-1 finding on 56e09c7e4)

Round 2. Round 1 (bae594993) found a 4th unconditional writer to
`AcpSession.cancel_tx` at `acp.rs:295` that 56e09c7e4's "all three writers"
claim missed. f13dacc4c claims to guard it and correct the count to four.
This round re-verifies the fix, not the finding — per the brief, checked
whether the refactor regressed the original site, whether four is actually
complete, and reproduced the author's new RED→GREEN and mutation-pairing
claims myself.

## Verdict: SOUND

All claims in f13dacc4c hold up under independent reproduction. No new
finding this round.

## Method note

Tested by overlaying f13dacc4c's exact `acp.rs` onto my worktree (`git show
f13dacc4c:crates/buzz-intel-agent/src/acp.rs > crates/buzz-intel-agent/src/acp.rs`),
running/mutating there, then restoring via `git checkout HEAD --
crates/buzz-intel-agent/src/acp.rs` (verified byte-identical to HEAD after
restore). Never committed to a branch, never pushed.

## 1. Does the refactor regress the original (56e09c7e4) site? — NO, confirmed

`reset_cancel_for_new_turn` is now a one-line wrapper:
```rust
fn reset_cancel_for_new_turn(cancel_tx: &watch::Sender<CancelCause>) {
    set_cancel_unless_shutdown(cancel_tx, CancelCause::None);
}
```
`set_cancel_unless_shutdown`'s body is character-for-character the same
`send_if_modified` closure `reset_cancel_for_new_turn` had before, with
`CancelCause::None` replaced by the `cause` parameter. Since this call site
always passes `CancelCause::None`, the substitution is behaviorally a
no-op. Call site unchanged: still inside the same `app.sessions.lock()`
scope in `session_prompt` (line 542–567, guard call at line 563).

Confirmed empirically, not just by inspection: the three original
regression tests (`reset_cancel_for_new_turn_does_not_clobber_shutdown`,
`_intentionally_still_clobbers_user`, `_is_a_no_op_from_none`) are
untouched, still call `reset_cancel_for_new_turn` directly (not the new
function), and all three still pass against f13dacc4c's code: `cargo test
-p buzz-intel-agent reset_cancel_for_new_turn --lib` → 3 passed, 0 failed.
OBSERVED EXECUTING.

## 2. Is four actually the count? — YES, confirmed by grepping wider, not narrower

Grepped `CancelCause`, `cancel_tx`, and `.cancel_tx\b` across the whole
repo (not just this file), plus a scan for any `.clone()` of the sender
that could write from elsewhere:

- `crates/buzz-intel-agent/src/acp.rs:175` — `graceful_shutdown` →
  `Shutdown`. Unguarded by design (the terminal value; guarding it would
  be a no-op benefit at best, since it always "wins" whether guarded or
  not — writing `Shutdown` over an existing `Shutdown` is idempotent
  either way).
- `crates/buzz-intel-agent/src/acp.rs:305` — panic-recovery → `User`. Now
  routed through `set_cancel_unless_shutdown`. **Guarded** (this round's
  fix).
- `crates/buzz-intel-agent/src/acp.rs:471` — `cancel_session` → `User`.
  Unguarded by design (the pinned, documented User-wedge gap from round 1 —
  re-confirmed still correctly reasoned; nothing in this round changes
  that consumer set).
- `crates/buzz-intel-agent/src/acp.rs:523` (via `reset_cancel_for_new_turn`
  at line 563) — reset → `None`. **Guarded** (56e09c7e4's original fix).
- `crates/buzz-intel-agent/src/acp.rs:445` — `session_new`'s
  `watch::channel(CancelCause::None)` — this is channel *construction*,
  before the session is inserted into `app.sessions` (insert happens
  after, line 452). No other writer can reach this session's `cancel_tx`
  yet, so it isn't part of the racing-writer class. Correctly not counted.

No `.clone()` of any `cancel_tx`/`AcpSession.cancel_tx` exists anywhere in
the repo — every write goes through the session's own field via
`sessions.get_mut(...)`, all under `app.sessions.lock()`. Confirmed no
writer is reachable via a smuggled-out clone.

`crates/buzz-intel-agent/src/intel.rs:1253` sends `CancelCause::User` on a
`cancel_tx`, but that's a freshly-constructed, test-local channel inside
`#[cfg(test)] mod tests` (constructed at `intel.rs:1225` inside a single
test function, never touching `AcpSession`) — not a production writer,
correctly out of scope.

**Checked wider still, per the brief's "other channels obtained from a
session" instruction:** `crates/buzz-agent/src/lib.rs` has its own,
unrelated `cancel_tx: watch::Sender<bool>` on a different `Session`
struct, in a different crate (`buzz-agent`, the separate minimal
ACP-compliant agent per `AGENTS.md`'s repo-structure table — not
`buzz-intel-agent`). It's a plain boolean flag with no third "terminal
value that must survive" state — structurally not an instance of this bug
class (there's nothing analogous to `Shutdown` to lose). Out of scope for
this fix's writer-count claim, correctly not one of "the four."

**Conclusion: four is complete** for the class the fix claims to close
(writers of `AcpSession.cancel_tx: watch::Sender<CancelCause>` in
`buzz-intel-agent/src/acp.rs`), and the corrected commit body's
enumeration (:175 unguarded-by-design, :305 now-guarded, :471
unguarded-by-design, :523 guarded) matches what's actually in the file.

## 3. Mutation-pairing claim — verified myself, holds exactly as described

Ran three variants of `set_cancel_unless_shutdown` against both new tests
(`set_cancel_unless_shutdown_protects_shutdown_regardless_of_requested_cause`,
`_applies_requested_cause_when_not_shutdown`):

| variant | `protects_shutdown` | `applies_requested_cause` |
|---|---|---|
| real guard (`send_if_modified`) | ok | ok |
| unconditional `send(cause)` (the original-bug shape) | **FAILED** (`left: None, right: Shutdown`) | ok |
| true no-op (writes nothing) | ok (vacuously) | **FAILED** (`left: None, right: User`) |

This is the exact pairing claimed in the commit: the unconditional-send
mutant is caught only by `protects_shutdown`; the no-op mutant is caught
only by `applies_requested_cause`. I additionally checked the
non-obvious direction the brief asked about — whether either test is
*redundant* given the other, not just whether the pair together is
sufficient:
- Removing `applies_requested_cause` would let the no-op mutant ship
  undetected (`protects_shutdown` passes vacuously against it).
- Removing `protects_shutdown` would let the *original bug itself* ship
  undetected: `applies_requested_cause_when_not_shutdown`'s table only
  ever starts from `prior ∈ {None, User}` (never `Shutdown`), so an
  unconditional-send mutant satisfies it trivially — this test alone
  would never have caught the regression this whole fix exists for.

Both tests are load-bearing; neither is redundant. Claim confirmed, not
just accepted. OBSERVED EXECUTING for all three variants above (exit
codes: real guard → 0; unconditional-send → 101 confirmed; no-op → 101
confirmed). File restored byte-identical to `f13dacc4c` after each
mutation (`diff <(git show f13dacc4c:...) crates/buzz-intel-agent/src/acp.rs`
→ empty).

## 4. RED→GREEN for the new guard — reproduced myself

- Baseline (real guard): `cargo test -p buzz-intel-agent
  set_cancel_unless_shutdown --lib` → exit 0, 2/2 pass.
- Reverted to `let _ = cancel_tx.send(cause);` (the exact pre-fix shape
  `acp.rs:295` used to have): exit 101, `protects_shutdown_...` FAILED
  with `left: None, right: Shutdown` — matches the commit transcript
  verbatim (down to which of the two tests fails and the exact
  left/right values).
- Restored the real guard: exit 0, 2/2 pass; full crate suite `cargo test
  -p buzz-intel-agent --lib` → **93 passed, 0 failed** (matches the
  claimed 88 original + 3 (56e09c7e4) + 2 (f13dacc4c) = 93). `git diff
  --stat` against `f13dacc4c`'s file: empty.

## 5. Housekeeping claims — reran myself

- `cargo fmt -p buzz-intel-agent -- --check` → exit 0.
- `cargo clippy -p buzz-intel-agent --all-targets -- -D warnings` → exit
  0 (host load ~4-5 on 12 cores at the time, no need to defer).
- Did not run the 14 e2e tests this round either (same rationale as round
  1 — unrelated to this specific race, no code path here touches e2e
  surface).

## Non-blocking observation (not a finding)

`set_cancel_unless_shutdown_protects_shutdown_regardless_of_requested_cause`
loops over `[CancelCause::None, CancelCause::User]` with a plain
`assert_eq!` inside the loop, which panics (and thus stops the test) on
the *first* failing iteration. Under the unconditional-send mutant I
confirmed only the `None` iteration is what shows up in the failure
output — the `User` iteration would fail too under that same mutant (the
mutant applies uniformly to any `cause`), but the test never gets to
execute it to say so once `None` has already panicked. This doesn't
affect correctness of the shipped guard (confirmed independently — the
GREEN run exercises both iterations, since `assert_eq!` only stops
execution on a failing case) and isn't worth blocking on; noting it only
because table-style loop tests with an early-panicking assertion are a
recurring shape worth a second look elsewhere in this codebase.

Signed-off-by: independent verifier pass, orchestrator buzz-architect.

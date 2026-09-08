# Adversarial verification: 56e09c7e4 (shutdown-notice silent-loss fix)

Independent verifier pass on `crates/buzz-intel-agent/src/acp.rs` commit
56e09c7e4 ("fix(intel-agent): stop shutdown-interrupted notice from being
silently lost"). Method: read the actual lock scopes at every write site,
then reproduce the author's RED→GREEN transcript myself rather than trust it.

## Verdict: SOUND-WITH-FINDINGS

The fix itself is correct and the regression test is real, falsifiable, and
bound to production code. One finding: the commit's own "generalized the
analysis" claim is incomplete — it enumerates three writers to `cancel_tx`
and there are actually four.

## What was verified directly

1. **Mutex-serialization claim (three enumerated writers) — CONFIRMED.**
   `app.sessions.lock().await` is held across both the read and the write
   at all three call sites the commit names:
   - `graceful_shutdown` — lock held lines 172–177, `Shutdown` sent inside.
   - `cancel_session` — lock held lines 458–468, `User` sent inside.
   - `session_prompt`'s reset — lock held lines 522–548, `reset_cancel_for_new_turn`
     (and therefore its `send_if_modified`) called inside, at line 544.

   Given this, these three writers can never execute concurrently on the
   same session's `cancel_tx`; they only ever race in *temporal order*
   (whichever acquires the lock later wins under last-write-wins). The
   `send_if_modified` CAS is what makes the *later* writer's decision
   conditional instead of an unconditional stomp — that part of the fix is
   correct regardless of whether `send_if_modified`'s own internal locking
   is load-bearing (it isn't, here, since `app.sessions` already prevents
   true concurrency; the fix works because it's now a *conditional* write,
   not because of extra atomicity `send_if_modified` adds on top).
   Evidence: SHIPPED + REFERENCED + code-read REACHABLE (lock scopes read
   directly, not inferred from the commit message).

2. **RED→GREEN reproduced myself — OBSERVED EXECUTING.**
   Reverted `reset_cancel_for_new_turn` to the pre-fix
   `let _ = cancel_tx.send(CancelCause::None);`, ran
   `cargo test -p buzz-intel-agent reset_cancel_for_new_turn --lib`:
   exit 101, `reset_cancel_for_new_turn_does_not_clobber_shutdown` FAILED
   with `left: None, right: Shutdown` — matches the commit transcript
   verbatim. Restored the guard, same command: exit 0, 3/3 passed. Full
   crate suite after restore: `cargo test -p buzz-intel-agent --lib` — 91
   passed, 0 failed, working tree diff empty (clean revert confirmed via
   `git diff --stat`).

3. **Non-vacuousness of the other two new tests — OBSERVED EXECUTING.**
   Mutated `reset_cancel_for_new_turn` to a true no-op (`{}`, sends
   nothing). Result: `reset_cancel_for_new_turn_intentionally_still_clobbers_user`
   FAILED (`left: User, right: None`) — this test alone catches a
   do-nothing implementation that the other two would pass vacuously
   (Shutdown-stays-Shutdown and None-stays-None are both trivially true of
   a no-op). Confirms the three tests are jointly discriminating, not
   individually vacuous. Reverted cleanly (diff empty after restore).

4. **User-wedge justification (why `User` is deliberately left racy) —
   CONFIRMED true by tracing consumers, not just accepted.**
   - `cancel_session` never checks `busy`; it sends `User` and bumps
     `epoch` unconditionally whenever the session exists (line 458–467) —
     so a duplicate/late/no-turn-in-flight cancel really can leave a
     "stray" `User` on the channel with nothing about to consume it.
   - The only code that ever writes `CancelCause::None` back is
     `reset_cancel_for_new_turn`, called once at the very start of every
     `session/prompt` (line 544) — there is no other clearing path.
   - `run_turn`'s own top-of-function check (line 634:
     `if *cancel_rx.borrow() != CancelCause::None { ...; return
     Err(Cancelled) }`) fires for *any* non-`None` value, not just
     `Shutdown` — so if a generalized guard left a stray `User` in place,
     the *next*, entirely unrelated turn on that session would
     immediately self-cancel at line 634, forever (nothing else would ever
     clear it). This is exactly the "permanently wedge the session"
     failure the commit describes, and it is real, not hypothetical: I
     traced it to a concrete sequence (redundant `session/cancel` firing
     after its target turn already finished naturally → stray `User` →
     hypothetically-guarded reset preserves it → next turn dead on
     arrival). The asymmetry is justified.
   - `reset_cancel_for_new_turn_intentionally_still_clobbers_user` pins
     exactly this behavior (`User` → reset → `None`). Confirmed accurate.

5. **Reachability-defect framing (level check) — endorsed.** All three new
   tests bind to `reset_cancel_for_new_turn`, the actual function
   `session_prompt` calls (not a reimplementation), and assert on the
   value *left in the channel* after the defect's exact ordering, not on
   the shutdown-notice string or on `CancelCause::Shutdown` merely being
   referenced. Marker/string checks are structurally incapable of
   distinguishing broken from fixed here, as the commit says — the
   broken version already ships that text. Agreed, and independently
   re-derived, not just repeated from the commit message.

## Finding: the "three writers" enumeration is incomplete — a fourth exists

**FINDING (medium; currently inert, but the commit's completeness claim is
false).** The commit states: *"app.sessions is a single mutex guarding all
three writers (this reset, cancel_session's User send, graceful_shutdown's
Shutdown send), so the identical race shape exists for User"* and *"only
the one write site that was wrong changes."*

There is a **fourth** writer to the same `cancel_tx`, holding the same
`app.sessions` lock, that the commit does not mention and did not touch:
`crates/buzz-intel-agent/src/acp.rs:295`, inside `handle_request`'s
`session/prompt` panic-recovery branch:

```rust
if let Some(ref sid) = session_id_hint {
    let mut sessions = app.sessions.lock().await;
    if let Some(s) = sessions.get_mut(sid) {
        s.busy = false;
        let _ = s.cancel_tx.send(CancelCause::User);   // <-- unconditional, unguarded
    }
}
```

This is a plain, unconditional `send`, structurally identical to the
pre-fix `reset_cancel_for_new_turn` — the exact primitive flaw the commit
calls out ("an unconditional 'reset to None' cannot coexist with 'a
terminal value must survive' under last-write-wins"). It applies equally
to an unconditional `send(User)`.

**Reachability.** This branch runs when the `session/prompt` task
registered in `app.prompt_tasks` (`tasks.spawn(...)`, line 272) panics —
the very `JoinSet` `graceful_shutdown` explicitly, bounded-ly waits on via
`tasks.join_next()` (lines 184–202) *after* it has already sent `Shutdown`
to every session (lines 172–177). A task that is mid-flight when
`graceful_shutdown` sends `Shutdown`, and that panics anywhere afterward
inside `session_prompt`/`run_turn` (the very reason this `catch_unwind`
harness exists — panics here are an anticipated, not hypothetical,
condition), will overwrite that session's `Shutdown` with `User` at line
295. Evidence level: REACHABLE by code-path tracing (confirmed lock scope,
confirmed spawn/JoinSet identity, confirmed this is the shutdown-grace
window), not OBSERVED EXECUTING — I did not construct a live end-to-end
repro (would need a forced panic mid-turn racing a real
`graceful_shutdown`).

**Mechanism, OBSERVED EXECUTING.** I added a temporary unit test
reproducing the exact write shape (`tx.send(Shutdown)` then
`tx.send(User)`, mirroring line 295 verbatim) and ran it — confirmed the
channel reads back `User`, i.e. `Shutdown` is silently gone. Reverted the
test immediately after (`git diff --stat` on `acp.rs` empty — nothing left
in the tree from this check).

**Current externally-observable impact: none that I could find.** I
traced every consumer of `cancel_tx`/`cancel_rx` in this crate
(`notify_if_shutdown_cancelled` at line 1425, the mid-retry check at line
894, `run_turn`'s top-of-function check at line 634) and could not find
one that re-reads this session's `cancel_tx` *after* a panic in a way that
would behave differently for `User` vs `Shutdown` — the panicking turn's
own checks already ran (or didn't) before the panic, and no new turn can
be dispatched for that session afterward because `read_loop` is no longer
reading stdin once `graceful_shutdown` has started. So today this is a
landmine, not a live symptom: the invariant the commit says it established
("a terminal value must survive") is not actually universal on this
channel, and the next person who adds a consumer that inspects
post-panic `cancel_tx` state will silently reintroduce this exact bug
class, believing (per this commit's commit message) that it was already
closed everywhere but `User`-after-`reset`.

**Recommendation (not applied — verification only):** route line 295's
send through the same `send_if_modified`-style guard, or fold it into
`reset_cancel_for_new_turn`'s CAS so there is exactly one write primitive
for this channel. Out of scope for me to fix; flagging for the author.

## What did not need re-litigating

- `fmt`/`clippy` claims: reran both scoped to the crate myself —
  `cargo fmt -p buzz-intel-agent -- --check` exit 0; `cargo clippy -p
  buzz-intel-agent --all-targets -- -D warnings` exit 0. OBSERVED
  EXECUTING.
- Full unit suite: `cargo test -p buzz-intel-agent --lib` — 91 passed, 0
  failed, OBSERVED EXECUTING (did not run the 14 e2e tests — out of scope
  for this specific race, requires more infra than warranted here).
- No other production use of `CancelCause` exists outside
  `crates/buzz-intel-agent/{acp,error,intel}.rs`; `intel.rs`'s writer
  (`cancel_tx.send(CancelCause::User)` at line 1253) is inside
  `#[cfg(test)]` — not a production site, correctly out of scope.

Signed-off-by: independent verifier pass, orchestrator buzz-architect.

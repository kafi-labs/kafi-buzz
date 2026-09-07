# 15 — Split-Tier Topology: Harness Off Wren, Dialing the Public Relay URL

**Status:** Proven 2026-09-07 in `ops/split-tier-topology` (branch based on `37070335d`).
**Applies to:** the decision, made by the owner, to resolve [14-colocation-hairpin-constraint.md](14-colocation-hairpin-constraint.md)
by choosing Option A from that doc ("run the harness off-box, against the public URL") rather than
Option B (boot-persistent local TLS terminator) or Option C (general no-co-location policy).

This doc proves the chosen topology actually works, with a new dedicated VM, real commands, and a
real reboot — not by reasoning about it.

---

## 0. Scope of what this proves, and what it does not

**In scope, proven below:**
- A harness process running on a *different* exe.dev VM than the relay can resolve, dial, and
  complete NIP-42 protocol handshakes against the relay's public `wss://` hostname.
- This survives a reboot of the new VM — the exe.dev `/etc/hosts` rewrite (documented in doc 14) is
  confirmed to still occur at every boot, and confirmed to not reintroduce the hairpin, because it
  only ever pins the *rebooting VM's own* hostname, never any other host's.

**Out of scope, NOT attempted, per the owner's explicit conditions:**
- No fix to Breakage A (`INTEL_AGENT` pointing at a deleted agent). The full `buzz-intel-agent`
  harness cannot start end-to-end because of this, unrelated to topology.
- No intel gateway calls, no intel agent creation/publication/invocation.
- No changes to `vm-buzz-relay-dev-wren.exe.xyz` beyond reading from it and dialing its public URL.
  Nothing was stopped, deleted, reconfigured, or deployed onto it.
- No merge, no deploy to production, no push. This doc is committed locally in the worktree only.

---

## 1. What was created

One new VM, via `exe.dev`:

```
$ printf 'new --name=vm-buzz-harness-topo --cpu=2 --memory=4GB --disk=20GB \
    --tag=buzz,dev,split-tier-topology \
    --comment="split-tier topology proof VM, dials wren public wss, created by split-tier lane 2026-09-07" \
    --no-email\n' | ssh exe.dev

Creating vm-buzz-harness-topo using image boldsoftware/exeuntu...
Coding agent   https://vm-buzz-harness-topo.shelley.exe.xyz
App            https://vm-buzz-harness-topo.exe.xyz
SSH            ssh vm-buzz-harness-topo.exe.xyz
```

- Image: `boldsoftware/exeuntu` (same family as every other VM in the account; passing `--image`
  explicitly failed with "not found or not accessible" — the default resolves it correctly, an
  explicit pin does not).
- 2 vCPU / 4GB RAM / 20GB disk, x86_64 (confirmed `uname -m`), Docker 29.1.3 preinstalled.
- Tagged `buzz,dev,split-tier-topology` and commented, so it is identifiable and not confused with
  `wren` (`#do-not-delete`) or any other VM in the 27-VM account inventory enumerated with `ls` before
  creating anything.
- `buzz-cli` (the `buzz` binary) was cross-... actually **natively** built on the VM itself (it is
  x86_64, same arch as the source repo's CI target, so no cross-compilation was needed): the exact
  commit (`37070335d`, this branch's base) was transferred via `git bundle` (create → scp → fetch →
  delete, per [13-deploy-runbook.md](13-deploy-runbook.md)'s "never `git push`" pattern) and built
  inside `rust:1.95.0-slim-bookworm` (matching `rust-toolchain.toml`) with the crate's cargo/git
  caches persisted under `~/cargo-cache-*` on the VM. Build: `Finished release profile [optimized]
  target(s) in 5m 29s`, clean, no errors.

---

## 2. Proof 1 — resolution agreement from inside the new VM (owner condition 1)

The premise this proves: the `/etc/hosts` rewrite (doc 14) is a **platform** behavior — the new VM
gets its own fresh rewrite — but it only ever pins the *rebooting VM's own* public hostname. Since
this VM's own name is `vm-buzz-harness-topo`, not `vm-buzz-relay-dev-wren`, wren's name is absent
from this VM's `/etc/hosts` and falls through `files dns` to real DNS, which is authoritative and
correct for a name this VM does not own.

This needed checking, not assuming: on `wren` itself, `getent` and `dig` **disagree** (that
disagreement is exactly the doc-14 bug). A single `dig` from a laptop, or from the wrong host, would
have hidden it — this is the same trap that hid the original bug for weeks. Run **from inside**
`vm-buzz-harness-topo`, immediately after creation:

```
$ getent hosts vm-buzz-relay-dev-wren.exe.xyz
161.210.92.49   vm-buzz-relay-dev-wren.exe.xyz

$ dig +short vm-buzz-relay-dev-wren.exe.xyz
161.210.92.49

$ grep -n "exe.xyz" /etc/hosts
3:10.42.0.42 vm-buzz-harness-topo.exe.xyz vm-buzz-harness-topo

$ grep -n "^hosts:" /etc/nsswitch.conf
12:hosts:          files dns
```

`getent` (which respects `nsswitch.conf`'s `files dns` order, i.e. what any application on this VM
actually gets when it resolves the name) and `dig` (which bypasses `/etc/hosts` and asks DNS
directly) **agree** — both return `161.210.92.49`, wren's real, externally-reachable IP. `/etc/hosts`
has exactly one `exe.xyz` line, and it is this VM's own name pointing at this VM's own overlay
address (`10.42.0.42`) — not wren's name, not wren's hairpin address. `files` has nothing to win with
for wren's name, so `dns` resolves it, correctly.

Then a real connection, not just a resolution check — the doc-14 finding was that even a "healthy"
DNS answer proves nothing if nothing living answers the connection:

```
$ curl -sS -o /tmp/health_body.txt -w "http_code=%{http_code} exit=%{exitcode} time=%{time_total}\n" \
    --max-time 10 https://vm-buzz-relay-dev-wren.exe.xyz/health
http_code=200 exit=0 time=0.405341
$ cat /tmp/health_body.txt
ok

$ curl -sS -i --max-time 10 -H "Connection: Upgrade" -H "Upgrade: websocket" \
    -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
    https://vm-buzz-relay-dev-wren.exe.xyz/
HTTP/2 200
content-type: application/json
...
{"name":"Buzz Relay", ... "self":"d7a0b76ffb7041e220429ea4ce69752b94c0a609794ce53666ee3af76eeb3ec0"}
```

(`curl wss://...` was also run, per the measurement-discipline warning: it returned `http_code=000
exit=1` — "Protocol wss not supported" — which is **not** evidence of anything about the relay; it is
libcurl not speaking that scheme. Recorded here only to show it was checked and correctly not
reported as a finding.)

**Condition 1: met.** DNS and the resolver applications actually use agree, from inside the new VM,
for wren's hostname specifically, and a live HTTPS connection with a real TLS handshake succeeds.

### 2a. Correction made during review

The first pass of this proof reported only the `dig` result and summarized it as "resolves via DNS
to its real public IP," without printing the `getent` output alongside it. The orchestrator (buzz-
architect) caught this: `dig` is not the resolver a harness process actually uses, and on wren itself
`getent` and `dig` disagree — checking only `dig` would have repeated exactly the blind spot that hid
the original bug. Both were already captured in the session's raw command output; the gap was in the
report, not the check. Re-run and reported verbatim above.

---

## 3. Proof 2 — the original symptom path, reproduced and now succeeding (owner condition 2)

**Why not the full intel harness:** `buzz-intel-agent`'s init sequence resolves `INTEL_AGENT` (agent
lookup) before it ever reaches the relay-connect step. `INTEL_AGENT` points at a deleted throwaway
agent (Breakage A, unfixed, not this lane's job) — every start dies at agent lookup and never
exercises the relay dial at all. Working around this (e.g. by pointing at a different intel agent)
was explicitly out of scope. So the relay-connect step is exercised directly instead, with
`buzz-cli`, which needs no intel gateway.

**The original symptom, exactly:** *"initial relay connect attempt N failed: Connection refused"* —
a harness process, co-located with the relay, dialing the relay's own public `wss://` hostname,
getting an immediate TCP-level refusal because (per doc 14) the hostname hairpins to the VM's own
overlay IP where nothing listens on 443.

Two `buzz-cli` commands exercise two different real network paths from `vm-buzz-harness-topo` to
`wss://vm-buzz-relay-dev-wren.exe.xyz`, both authenticated with a freshly generated, never-registered
test keypair (`openssl rand -hex 32`, passed as `BUZZ_PRIVATE_KEY`; the community-scoped
`~/.config/buzz/wren-owner/owner.sk` used for the owner smoke test in doc 13 was intentionally not
used or touched — this test needs no elevated identity to prove the topology question):

### 3a. Raw WebSocket connect + NIP-42 challenge/response AUTH

`buzz users set-presence` is the one CLI path that publishes over a raw WebSocket (`buzz_ws_client::
publish_event`) rather than the HTTP bridge — kind 20001 is ephemeral and the relay only accepts
ephemeral kinds over WS. This is the literal `wss://` dial the original symptom describes:

```
$ export BUZZ_RELAY_URL="https://vm-buzz-relay-dev-wren.exe.xyz"
$ export BUZZ_PRIVATE_KEY="<freshly generated hex, never registered>"
$ time ./buzz users set-presence --status online
{"error":"error","message":"Authentication failed: restricted: not a relay member","retryable":false}

real  0m0.254s
```

This is a **rejection**, but a completely different kind of rejection than the original symptom. It
was verified against the relay's own source (`buzz-relay/src/handlers/auth.rs:216-238`,
`enforce_relay_membership`) that this exact message fires *after* the NIP-42 signature has already
been cryptographically verified — the connection succeeded, the TLS handshake succeeded, the
WebSocket upgrade succeeded, the relay sent its AUTH challenge, the client signed and sent a
kind:22242 event, and the relay validated that signature before ever reaching the membership check
that produced this message. A "Connection refused" never gets this far — it never opens a TCP
connection at all, let alone completes a signature-verified challenge/response. 0.25 seconds of real
round-trip protocol exchange versus an instant OS-level refusal are not the same failure mode.

The rejection itself (`not a relay member`) is expected and correct: the test key was never added to
any community's membership, deliberately — this test is scoped to the network/protocol question, not
to producing an authorized session, and doing the latter would have needed either DB access to
wren (out of scope — no write access, nothing but read/dial permitted) or borrowing a real identity
(also out of scope).

### 3b. HTTP REQ bridge, kinds specified (avoiding the p-gate)

`buzz channels list` uses the HTTP `/query` bridge (`POST /query`, NIP-98-signed, not NIP-42) with a
Nostr filter that **is source-verified to include `kinds: [39000]`**
(`crates/buzz-cli/src/client.rs:2411` builds this filter) — the AGENTS.md gotcha ("relay queries must
specify kinds — omitting kinds triggers the p-gate / 403") is satisfied by construction here, not by
luck:

```
$ time ./buzz channels list
{"error":"auth_error","message":"relay error 403: relay_membership_required","retryable":false}

real  0m0.223s
```

Same shape of result: a fast (0.22s), well-formed, protocol-level 403 — the connection, TLS, and HTTP
request/response cycle all completed; the relay evaluated the (unauthenticated-for-this-community)
identity and correctly declined. Not a network failure.

**Condition 2: met.** Both the raw-WS/NIP-42 path and the HTTP-REQ-bridge path reach the relay's
public hostname from a *different* exe.dev VM, complete their respective protocol handshakes, and
receive real application-level responses in a few hundred milliseconds — categorically different from
the original "Connection refused." Full authorized access (an actual owned/member session) remains
blocked, but only by Breakage A's absence of a working intel identity for this topology to hand to —
never by the network path itself.

---

## 4. Proof 3 — survives an actual reboot (owner condition 3)

exe.dev rewrites `/etc/hosts` at *every* boot (doc 14, §1) — this is a platform property, so the new
VM will do it too, every time. The only way to know the split-tier topology survives that is to
actually reboot the new VM and re-run both proofs, not to reason about a unit file.

```
$ ssh vm-buzz-harness-topo.exe.xyz 'uptime -s'
2026-09-07 05:04:42                          # baseline, before reboot

$ printf 'restart vm-buzz-harness-topo\n' | ssh exe.dev
Restarting vm-buzz-harness-topo...
VM "vm-buzz-harness-topo" restarted successfully

$ ssh vm-buzz-harness-topo.exe.xyz 'uptime -s'
2026-09-07 05:19:15                          # different from baseline — a real reboot happened
```

Post-reboot, `/etc/hosts` re-rewrites (as expected — this is not skipped), and still only for this
VM's own name:

```
$ grep -n "exe.xyz" /etc/hosts
3:10.42.0.42 vm-buzz-harness-topo.exe.xyz vm-buzz-harness-topo
$ getent hosts vm-buzz-relay-dev-wren.exe.xyz
161.210.92.49   vm-buzz-relay-dev-wren.exe.xyz
$ dig +short vm-buzz-relay-dev-wren.exe.xyz
161.210.92.49
```

`getent`/`dig` still agree on wren after the fresh rewrite. Re-running both protocol proofs from
§3 with a newly generated test key, post-reboot:

```
$ ./buzz users set-presence --status online
{"error":"error","message":"Authentication failed: restricted: not a relay member","retryable":false}

$ ./buzz channels list
{"error":"auth_error","message":"relay error 403: relay_membership_required","retryable":false}
```

Identical results, pre- and post-reboot. The `buzz` binary itself also survived on disk across the
reboot (VM disk is persistent, not ephemeral) — no rebuild was needed to re-prove this.

**Condition 3: met.**

---

## 5. What remains blocked, honestly

- **Breakage A (dead `INTEL_AGENT`) is untouched and unfixed.** No full `buzz-intel-agent` run was
  attempted on the new VM, and none would get past agent lookup even if attempted — that failure is
  upstream of anything this lane's proofs exercise. This is not this lane's scope, and nothing here
  should be read as evidence that the intel harness works end-to-end.
- **No authorized/member session was established** against wren from the new VM. Both proofs in §3
  deliberately stop at a correct, fast, protocol-level authorization rejection rather than attempting
  to acquire or borrow a real community membership, per the "no live gateway calls," "don't borrow
  someone else's intel agent," and "read-only on wren" constraints. A real deploy would need a
  registered agent/owner identity provisioned the normal way (see doc 13 §6) — that provisioning step
  is unaffected by anything proven or changed here.
- **No intel agent, gateway call, or Intelligence Platform interaction of any kind was made.** Nothing
  on that platform was created, published, modified, or invoked.

---

## 6. What an operator needs to do to rebuild this topology for real

1. **Keep the relay on `wren`.** Nothing about the relay's deployment changes. It keeps listening on
   `:3000` behind the existing exe.dev tunnel exactly as before.
2. **Fix Breakage A first** (repoint `INTEL_AGENT` at a live, non-deleted agent) — otherwise the
   harness dies at agent lookup on *any* VM, split-tier or not, and this topology work has nothing to
   attach to. That is a separate, already-identified, not-yet-fixed problem.
3. **Provision a dedicated harness VM** (this proof's `vm-buzz-harness-topo` is disposable/proof-only;
   a real deployment should get a purpose-named VM, e.g. `vm-buzz-harness-dev` or similar, sized for
   the actual harness workload rather than this proof's minimal 2 vCPU/4GB).
4. **Set `BUZZ_RELAY_URL` (or the harness's equivalent config) to wren's public `wss://` hostname**,
   never `ws://localhost:3000` and never wren's own hostname resolved on wren itself — the new VM
   should dial `wss://vm-buzz-relay-dev-wren.exe.xyz` exactly as an external client would, because
   from any *other* VM that hostname resolves correctly (§2), and no config to route around the
   hairpin is needed because the hairpin does not apply to a name that is not the resolving host's
   own.
5. **Deploy the harness binaries to the new VM following [13-deploy-runbook.md](13-deploy-runbook.md)
   verbatim** — git bundle transfer (never `git push`), build on the VM (this proof confirms the VM's
   Docker + a pinned Rust image is a working native build path — no cross-compilation needed since
   exe.dev VMs are x86_64), sha verification, marker-string proof that the right code shipped, and the
   documented smoke-test cautions (one prompt at a time, `nostr:npub1…` addressing, full JSON not
   `--format compact`).
6. **Re-verify all three conditions in this doc against the real target VM before trusting it**,
   especially condition 1 — every exe.dev VM gets its own fresh `/etc/hosts` rewrite at every boot,
   and this doc's proof that it's harmless rests on the rewrite only ever containing the *rebooting
   VM's own* hostname. That should hold structurally (it is the same platform mechanism observed
   here and in doc 14), but "should hold" is exactly the kind of claim this whole restore effort
   exists to stop making without checking.
7. **Do not assume a green health check means the harness works.** Per doc 13 §6a, a fast smoke
   response proves the service is reachable, not correct — and per this doc, a fast *rejection* is
   also meaningful evidence (it proves the network/protocol path, not authorization). Distinguish
   the two kinds of "fast response" when reading harness logs on the new deployment.

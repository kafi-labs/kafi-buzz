# 14 — Co-location Hairpin Constraint (harness + relay on the same exe.dev VM)

**Status:** Established 2026-09-04 during the wren restore (`ops/wren-restore`). Not a wren-specific
defect — a property of any exe.dev VM that runs both the relay and an always-on harness that dials
the relay by its own public hostname.
**Applies to:** any deployment that co-locates `buzz-intel-agent`/`buzz-acp` with the relay it talks
to, on an exe.dev VM.

---

## 1. The mechanism

exe.dev regenerates `/etc/hosts` on every VM at boot, adding a static entry that maps the VM's own
public hostname to its **own private overlay IP** (not the public tunnel IP that external clients
resolve to):

```
# managed by exe.dev
127.0.0.1 localhost
10.42.0.42 vm-buzz-relay-dev-wren.exe.xyz vm-buzz-relay-dev-wren
```

`/etc/nsswitch.conf` resolves `hosts: files dns` — `/etc/hosts` wins over public DNS. So any process
running **on** the VM that resolves the VM's own public hostname always gets the private overlay IP,
never the public tunnel IP. External clients query DNS directly (no local `/etc/hosts` override) and
correctly land on the public tunnel.

The relay listens on `:3000` only. **Nothing terminates TLS on `:443` on the VM itself** — TLS for
the public `wss://` endpoint is terminated externally, at the exe.dev tunnel gateway, off-box. The
only thing this VM has ever had on a low port is `buzz-port80-proxy`, an `alpine/socat` container
(`network=host`) running `TCP-LISTEN:80,fork,reuseaddr TCP:127.0.0.1:3000` — port 80 to 3000, not 443,
and not the path the harness uses.

So a harness on the VM configured with the VM's own public `wss://` hostname resolves to the VM's
own overlay IP, dials that IP on `:443`, and finds no listener there at all. The connection is
refused immediately, on every attempt, regardless of any other configuration (agent name, quota,
etc.) — this happens before the harness ever gets far enough to subscribe to anything.

## 2. Proof commands and their real outputs

All run from a shell on `vm-buzz-relay-dev-wren.exe.xyz` (`exedev` user), during the restore:

```bash
$ curl -sS -o /dev/null -w "http_code=%{http_code} exit=%{exitcode}\n" --max-time 8 \
    https://vm-buzz-relay-dev-wren.exe.xyz/health
curl: (7) Failed to connect to vm-buzz-relay-dev-wren.exe.xyz port 443 after 0 ms: Couldn't connect to server
http_code=000 exit=7

$ curl -sS -o /dev/null -w "http_code=%{http_code} exit=%{exitcode}\n" --max-time 8 \
    http://localhost:3000/health
http_code=200 exit=0

$ docker inspect buzz-prod-relay-1 --format '{{range $k,$v := .NetworkSettings.Networks}}{{$v.IPAddress}}{{end}}'
172.18.0.2
$ curl -sS -o /dev/null -w "http_code=%{http_code}\n" --max-time 8 http://172.18.0.2:3000/health
http_code=200

$ getent hosts vm-buzz-relay-dev-wren.exe.xyz
10.42.0.42      vm-buzz-relay-dev-wren.exe.xyz vm-buzz-relay-dev-wren
$ dig +short vm-buzz-relay-dev-wren.exe.xyz        # bypasses /etc/hosts
161.210.92.49                                       # the real, externally-reachable IP

$ ip -4 addr show | grep 'inet '
    inet 127.0.0.1/8 scope host lo
    inet 10.42.0.42/16 brd 10.42.255.255 scope global eth0   # matches getent's answer — this IS the VM's own address
    ...

$ sudo ss -tlnp | grep ':443'
                                                      # (nothing — no listener on :443 anywhere on the VM)
```

Externally (from a laptop, not the VM), the same hostname works correctly end to end: `/health` →
200, `/` → 200, and a real WebSocket-upgrade probe (`Connection: Upgrade`, `Upgrade: websocket`,
`Sec-WebSocket-Key`) → 200. **The tunnel is not down.** It is only unreachable from inside the VM,
because of the `/etc/hosts` hairpin above.

### Timing evidence tying this to a boot event

```bash
$ stat /etc/hosts
Modify: 2026-08-08 21:02:59.100554591 +0000
Change: 2026-08-08 21:02:59.100554591 +0000
 Birth: 2026-08-08 21:02:59.100554591 +0000

$ uptime -s
2026-08-08 21:02:58
$ who -b
         system boot  2026-08-08 21:02
```

`/etc/hosts`'s birth/modify time is one second after the recorded boot time, and there has been no
reboot since (uptime confirms this is still the current, only boot). Read together with the
mission's own incident timeline — the crash-loop this restore fixed started at Aug 08 23:58, about
three hours **after** this boot — the hairpin (this doc's finding) was already live for ~3h before
the unrelated deleted-agent failure started firing. The deleted-agent failure (`INTEL_AGENT` pointing
at a since-deleted throwaway) failed earlier in the harness's own init sequence (agent lookup, before
relay connect), so it masked the hairpin completely: every crash-loop cycle in the historical journal
died at the agent-lookup step and never got far enough to hit the relay-connect step where the
hairpin would have shown up on its own.

**Honest limit:** there is no artifact of what `/etc/hosts` contained *before* this boot. It's
established that the *current* entry was (re)written at the last boot and hasn't changed since; it is
**not** established what the prior boot's `/etc/hosts` looked like, or whether this exact failure mode
existed on this VM before Aug 08. That would need exe.dev's own provisioning logs, which are off-VM
and out of this restore's reach.

## 3. Why a loopback fix doesn't work (architectural, not policy)

The obvious-looking fix — point the harness at `ws://localhost:3000` instead of the public hostname —
was tested directly and fails closed:

```bash
$ BUZZ_RELAY_URL=ws://localhost:3000 buzz --format compact channels list
{"error":"relay_error","message":"relay error 404: relay: no community is configured for this host","retryable":false}
```

This isn't a missing config knob that could be added. The relay resolves its deployment community
from the `Host` header against the `communities.host` **database column** (a lookup, not a config
file), and `buzz-core/src/tenant.rs` only allows a `TenantContext` to be constructed off that
resolution path — a `TenantContext` cannot exist unless it was produced by resolving a real host row
or already scoped from one. There's no alias mechanism. Adding `localhost` (or any second name) as a
row in `communities.host` doesn't add a second route to the *same* community — it creates a **second,
independent community**, with its own scope: different quota bucket, different data, not a shortcut
to the one the public hostname resolves to.

So `ws://localhost:3000` fails at the community-resolution gate itself (HTTP 404, before NIP-42 AUTH
is ever attempted) — the system is working as designed, not misconfigured. Loopback is closed
architecturally for this deployment shape, not just closed by an unset flag.

## 4. Options for the next deployment, priced (not ranked)

**A — Run the harness off-box, against the public URL.**
Verified working externally: the public `wss://` endpoint answers correctly from outside the VM
(health, root, and a real WS-upgrade probe all returned 200). Cost: the harness needs to live
somewhere other than the relay's own VM (another host, or a laptop-class always-on runner), which is
an operational/ownership question (who runs it, what keeps it alive), not a technical blocker. No
architectural changes needed.

**B — A boot-persistent local TLS terminator, with the public hostname kept resolvable to it.**
Put something on the VM that terminates TLS on `:443` (or whatever port the harness is told to use)
and proxies to `127.0.0.1:3000`, so the harness's own `wss://<public-hostname>` connection succeeds
locally instead of hairpinning to nothing. Cost: `/etc/hosts` is rewritten by exe.dev at every boot
(proven above — same VM, same mechanism, would recur on any exe.dev VM), so anything relying on the
current entry pointing where you want must be installed as a **boot-time unit** (systemd unit +
ordering, or a boot hook) that re-applies after exe.dev's own hosts rewrite, not a one-time edit — a
one-time `/etc/hosts` patch or ad hoc listener would silently revert on the next reboot with no
warning. Also needs a cert for the public hostname reachable from a process running on the VM itself.

**C — Don't co-locate.** Put the relay and the harness on different hosts (or different exe.dev VMs)
such that the harness's target is never its own machine's hostname. Cost: infrastructure/topology
change — more moving parts to provision and keep in sync (relay URL, network policy between the two),
but it removes the whole class of hairpin bugs by construction, and it's the only option of the three
that also generalizes past this one relay: **any** exe.dev VM that tries to dial its own public
hostname from on-box will reproduce this, since the `/etc/hosts` rewrite is a platform behavior, not
a wren-specific misconfiguration.

No option above is recommended here — each has a real, different cost, and the choice affects
ownership and topology beyond this restore's scope.

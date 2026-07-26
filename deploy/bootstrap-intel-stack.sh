#!/usr/bin/env bash
# bootstrap-intel-stack.sh — from-scratch Buzz relay + always-on intel agent on exe.dev
#
# Usage:
#   ./deploy/bootstrap-intel-stack.sh vm-buzz-repro-<short>
#
# Prerequisites (on the laptop that drives the deploy):
#   - ssh exe.dev works; docker available on the target VM (exeuntu)
#   - Prebuilt linux/amd64 binaries at:
#       artifacts/linux-amd64-57141538/{buzz,buzz-acp,buzz-intel-agent}
#       artifacts/linux-amd64-57141538/compute-auth-tag (optional)
#     (buzz = agent-first CLI for channel create / mention — runs ON the VM)
#   - INTEL_API_KEY_FILE (default: ~/.config/buzz/intel-e2e.key)
#   - Local tools: openssl, python3+coincurve; cargo only when the prebuilt
#     compute-auth-tag cannot run on the driver host
#
# This script does NOT commit, push, or switch git branches. It copies compose.yml
# to a scratch dir and hand-patches buzz-git-init (no cherry-pick).
#
# Channel create and owner @mention run ON THE VM via the bundled linux/amd64
# `buzz` CLI against http://127.0.0.1:3000. No laptop SSH tunnel is required.
# With RELAY_URL=ws://127.0.0.1:3000, Host matching is automatic for on-VM clients.
#
# ---------------------------------------------------------------------------
# Improvisation ledger (2026-07-26 repro deploy + follow-ups)
# ---------------------------------------------------------------------------
# CLOSED by this script / artifact bundle:
#   I6  linux/amd64 `buzz` CLI in artifacts — channel/mention no longer need a
#       host-arch binary or scp of a macOS binary (was Exec format error).
#   I7  laptop ssh -L 3000 tunnel for Host fidelity — obsolete once I6 lands;
#       on-VM CLI + RELAY_URL=ws://127.0.0.1:3000 makes Host match automatic.
#   I3  optional 8000:3000 publish — not required for loopback path; omitted.
#   I9  macOS tar xattrs — use COPYFILE_DISABLE=1 when packaging from macOS.
#   I5  prebuilt linux/amd64 `compute-auth-tag` removes the cargo dependency on
#       matching driver hosts; macOS/non-linux drivers still fall back to cargo.
#
# STILL OPEN (must stay hand-worked or wait for upstream):
#   I1  compose.yml lacks buzz-git-init until a branch lands — still hand-patched
#       into a COPY here (no cherry-pick / no git ops).
#   I2  public wss://$VM ideal vs loopback — default remains loopback because
#       fresh exe.dev 443 may auth-wall; set USE_PUBLIC_ORIGIN=1 to try public.
#   I4  sudo still required for /opt/buzz-intel + systemd (exeuntu reality).
#   I8  agent may start with 0 channels if started before channel+member; this
#       script creates the channel then restarts the agent (addresses I8).
#
# OBSOLETE comments (do not revive):
#   - "scp host buzz CLI and run via ssh -L 3000:…" — I6/I7, deleted below.
# ---------------------------------------------------------------------------
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VM_BASENAME="${1:?Usage: $0 <vm-name-without-.exe.xyz>}"
VM="${VM_BASENAME}.exe.xyz"
ARTIFACT_DIR="${ARTIFACT_DIR:-$ROOT/artifacts/linux-amd64-57141538}"
INTEL_KEY_FILE="${INTEL_API_KEY_FILE:-$HOME/.config/buzz/intel-e2e.key}"
INTEL_AGENT_NAME="${INTEL_AGENT:-buzz-e2e-assistant}"
INTEL_GATEWAY="${INTEL_GATEWAY_URL:-https://intel-platform.exe.xyz}"
MAX_TURNS="${INTEL_MAX_TURNS_PER_WINDOW:-120}"
QUOTA_SECS="${INTEL_QUOTA_WINDOW_SECS:-3600}"
CHANNEL_NAME="${CHANNEL_NAME:-repro-e2e}"

die() { echo "ERROR: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null || die "missing $1"; }

need ssh; need scp; need openssl; need python3; need tar
[[ -x "$ARTIFACT_DIR/buzz-acp" ]] || die "missing $ARTIFACT_DIR/buzz-acp"
[[ -x "$ARTIFACT_DIR/buzz-intel-agent" ]] || die "missing $ARTIFACT_DIR/buzz-intel-agent"
[[ -x "$ARTIFACT_DIR/buzz" ]] || die "missing $ARTIFACT_DIR/buzz (linux/amd64 CLI — required for on-VM channel create / mention)"
AUTH_TAG_HELPER="$ARTIFACT_DIR/compute-auth-tag"
if [[ -e "$AUTH_TAG_HELPER" && ! -x "$AUTH_TAG_HELPER" ]]; then
  echo "WARN: optional $AUTH_TAG_HELPER is not executable; cargo fallback will be used" >&2
fi
[[ -f "$INTEL_KEY_FILE" ]] || die "missing INTEL_API_KEY_FILE=$INTEL_KEY_FILE"
[[ -f "$ROOT/deploy/compose/compose.yml" ]] || die "missing deploy/compose/compose.yml"

# Optional sanity: refuse if someone dropped a non-ELF host binary into the bundle
if command -v file >/dev/null; then
  file "$ARTIFACT_DIR/buzz" | grep -qi 'ELF.*x86-64\|ELF.*x86_64' \
    || die "$ARTIFACT_DIR/buzz is not an ELF x86-64 binary (I6 regression)"
fi

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/buzz-bootstrap.XXXXXX")
cleanup_scratch() { rm -rf "$SCRATCH"; }
trap cleanup_scratch EXIT

mkdir -p "$SCRATCH/compose" "$SCRATCH/agent" "$SCRATCH/keys"
cp -a "$ROOT/deploy/compose/." "$SCRATCH/compose/"

# --- hand-patch buzz-git-init into the compose COPY (I1 — still open upstream) ---
python3 - "$SCRATCH/compose/compose.yml" <<'PY'
import sys
from pathlib import Path
p = Path(sys.argv[1])
text = p.read_text()
if "buzz-git-init" in text:
    print("compose already has buzz-git-init")
    raise SystemExit(0)
old_dep = """      minio-init:
        condition: service_completed_successfully
    # Probe /_readiness"""
new_dep = """      minio-init:
        condition: service_completed_successfully
      buzz-git-init:
        condition: service_completed_successfully
    # Probe /_readiness"""
if old_dep not in text:
    raise SystemExit("compose.yml: cannot find minio-init depends_on anchor to patch")
text = text.replace(old_dep, new_dep, 1)
if "\nvolumes:\n" not in text:
    raise SystemExit("compose.yml: no volumes section")
insert = """
  # One-shot: chown buzz-git-data to uid 1000 (relay USER buzz). Mirrors minio-init.
  buzz-git-init:
    image: alpine:3.20
    volumes:
      - buzz-git-data:/data
    entrypoint: >
      /bin/sh -euc '
        chown -R 1000:1000 /data
      '
    restart: "no"
    networks:
      - buzz-net

"""
text = text.replace("\nvolumes:\n", insert + "volumes:\n", 1)
p.write_text(text)
print("patched buzz-git-init into", p)
PY

cp -a "$ARTIFACT_DIR/buzz-acp" "$ARTIFACT_DIR/buzz-intel-agent" "$ARTIFACT_DIR/buzz" "$SCRATCH/agent/"
cp -a "$INTEL_KEY_FILE" "$SCRATCH/agent/intel-e2e.key"
chmod 600 "$SCRATCH/agent/intel-e2e.key"
chmod 755 "$SCRATCH/agent/buzz-acp" "$SCRATCH/agent/buzz-intel-agent" "$SCRATCH/agent/buzz"

# --- generate owner + agent keys (hex) ---
python3 - <<PY
from pathlib import Path
from coincurve import PrivateKey
keys = Path("$SCRATCH/keys")
keys.mkdir(exist_ok=True)

def gen(name):
    sk = PrivateKey()
    secret = sk.secret.hex()
    pub = sk.public_key.format(compressed=False)[1:33].hex()
    (keys / f"{name}.sk").write_text(secret + "\n")
    (keys / f"{name}.sk").chmod(0o600)
    (keys / f"{name}.pub").write_text(pub + "\n")
    print(f"{name}_pub={pub}")
    return secret, pub

owner_sk, owner_pub = gen("owner")
agent_sk, agent_pub = gen("agent")
Path("$SCRATCH/pubkeys.env").write_text(f"OWNER_PUB={owner_pub}\nAGENT_PUB={agent_pub}\n")
PY
# shellcheck disable=SC1090
source "$SCRATCH/pubkeys.env"

# --- NIP-OA auth tag (I5 — prebuilt helper on linux/amd64, cargo fallback elsewhere) ---
OWNER_SK=$(tr -d '\n' < "$SCRATCH/keys/owner.sk")
AGENT_SK=$(tr -d '\n' < "$SCRATCH/keys/agent.sk")
AUTH_TAG=""
if [[ -x "$AUTH_TAG_HELPER" && "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
  AUTH_TAG=$("$AUTH_TAG_HELPER" "$OWNER_SK" "$AGENT_PUB" "" 2>/dev/null) || AUTH_TAG=""
fi
if [[ -z "$AUTH_TAG" ]] && command -v cargo >/dev/null; then
  AUTH_TAG=$(cargo run --quiet --release -p buzz-sdk --example compute_auth_tag -- "$OWNER_SK" "$AGENT_PUB" "" 2>/dev/null) || AUTH_TAG=""
fi
[[ -n "$AUTH_TAG" ]] \
  || die "neither runnable prebuilt $AUTH_TAG_HELPER nor cargo fallback for buzz-sdk compute_auth_tag produced a non-empty auth tag"
printf '%s\n' "$AUTH_TAG" > "$SCRATCH/keys/auth_tag.json"
chmod 600 "$SCRATCH/keys/auth_tag.json"

# --- compose .env ---
# Default: loopback (I2). Host for on-VM clients is automatically 127.0.0.1:3000,
# matching RELAY_URL — this is why the old laptop tunnel on local :3000 (I7) is gone.
# Set USE_PUBLIC_ORIGIN=1 to try wss://$VM (requires working public WS, no auth wall).
if [[ "${USE_PUBLIC_ORIGIN:-0}" == "1" ]]; then
  RELAY_URL="wss://${VM}"
  AGENT_RELAY_URL="wss://${VM}"
  CLI_RELAY_URL="https://${VM}"
else
  RELAY_URL="ws://127.0.0.1:3000"
  AGENT_RELAY_URL="ws://127.0.0.1:3000"
  CLI_RELAY_URL="http://127.0.0.1:3000"
fi

cat > "$SCRATCH/compose/.env" <<EOF
BUZZ_IMAGE=ghcr.io/block/buzz:main
BUZZ_DOMAIN=${VM}
RELAY_URL=${RELAY_URL}
BUZZ_MEDIA_BASE_URL=https://${VM}/media
BUZZ_MEDIA_SERVER_DOMAIN=${VM}
BUZZ_CORS_ORIGINS=https://${VM}
BUZZ_REQUIRE_AUTH_TOKEN=true
BUZZ_REQUIRE_RELAY_MEMBERSHIP=true
BUZZ_ALLOW_NIP_OA_AUTH=true
BUZZ_AUTO_MIGRATE=true
BUZZ_GIT_CONFORMANCE_PROBE=true
RUST_LOG=buzz_relay=info,buzz_db=info,buzz_auth=info,buzz_pubsub=info,tower_http=info
RELAY_OWNER_PUBKEY=${OWNER_PUB}
BUZZ_RELAY_PRIVATE_KEY=${OWNER_SK}
BUZZ_GIT_HOOK_HMAC_SECRET=$(openssl rand -hex 32)
POSTGRES_DB=buzz
POSTGRES_USER=buzz
POSTGRES_PASSWORD=$(openssl rand -hex 16)
REDIS_PASSWORD=$(openssl rand -hex 16)
TYPESENSE_API_KEY=$(openssl rand -hex 16)
BUZZ_S3_ACCESS_KEY=$(openssl rand -hex 12)
BUZZ_S3_SECRET_KEY=$(openssl rand -hex 24)
BUZZ_S3_BUCKET=buzz-media
BUZZ_HTTP_PORT=3000
EOF
chmod 600 "$SCRATCH/compose/.env"

AUTH_TAG_JSON=$(python3 -c 'import json,pathlib; print(json.dumps(pathlib.Path("'"$SCRATCH"'/keys/auth_tag.json").read_text().strip()))')
cat > "$SCRATCH/agent/run-harness.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
export BUZZ_RELAY_URL="${AGENT_RELAY_URL}"
export BUZZ_PRIVATE_KEY="${AGENT_SK}"
export BUZZ_AUTH_TAG=${AUTH_TAG_JSON}
export BUZZ_ACP_AGENT_COMMAND="/opt/buzz-intel/bin/buzz-intel-agent"
export BUZZ_ACP_AGENT_ARGS=""
export INTEL_GATEWAY_URL="${INTEL_GATEWAY}"
export INTEL_AGENT="${INTEL_AGENT_NAME}"
export INTEL_API_KEY_FILE="/opt/buzz-intel/secrets/intel-e2e.key"
export INTEL_ENTITY_MODE="channel"
export INTEL_ERROR_REPLIES="true"
export INTEL_STATE_DIR="/var/lib/buzz-intel-agent"
export INTEL_MAX_TURNS_PER_WINDOW="${MAX_TURNS}"
export INTEL_QUOTA_WINDOW_SECS="${QUOTA_SECS}"
export BUZZ_ACP_IDLE_TIMEOUT="3600"
export BUZZ_ACP_MAX_TURN_DURATION="7200"
export RUST_LOG="\${RUST_LOG:-info}"
exec /opt/buzz-intel/bin/buzz-acp
EOF
chmod 700 "$SCRATCH/agent/run-harness.sh"

cat > "$SCRATCH/agent/buzz-intel-agent.service" <<'EOF'
[Unit]
Description=Buzz Intel ACP agent harness (always-on first-class agent)
After=network-online.target docker.service
Wants=network-online.target
[Service]
Type=simple
User=exedev
Group=exedev
WorkingDirectory=/opt/buzz-intel
ExecStart=/opt/buzz-intel/run-harness.sh
Restart=always
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ReadWritePaths=/var/lib/buzz-intel-agent /opt/buzz-intel
LimitNOFILE=65536
[Install]
WantedBy=multi-user.target
EOF

# Worker command auditing: ATTEMPTED AND REMOVED (2026-07-26).
# A ~/.ssh/rc hook was shipped here and proven non-functional on a throwaway VM:
# the audit log stayed empty because ~/.ssh/rc does NOT receive
# $SSH_ORIGINAL_COMMAND for an ordinary 'ssh host cmd'. sshd only sets that for
# an sshd_config ForceCommand or an authorized_keys command= wrapper.
#
# The correct mechanism, if this is wanted later, is a ForceCommand wrapper that
# logs and then execs the original command. It MUST be validated on a throwaway
# VM before shipping: a broken wrapper sits in the ssh path and can lock you out
# of the host.
#
# Until then there is NO host-side audit trail on VMs built by this script.
# Worker actions are known only from the workers' own reports.

# On-VM e2e: create channel, add agent, restart agent, owner @mention with marker
# (runs after compose + systemd install via a second ssh; values expanded by driver)
MARKER="BOOTSTRAP-$(openssl rand -hex 4)"
cat > "$SCRATCH/agent/onvm-e2e.sh" <<EOF
#!/usr/bin/env bash
# Runs entirely on the VM with the bundled linux/amd64 buzz CLI.
set -euo pipefail
export BUZZ_RELAY_URL="${CLI_RELAY_URL}"
export BUZZ_PRIVATE_KEY="\$(tr -d '\\n' < /home/exedev/repro-keys/owner.sk)"
BUZZ=/opt/buzz-intel/bin/buzz
AGENT_PUB=\$(tr -d '\\n' < /home/exedev/repro-keys/agent.pub)
MARKER="${MARKER}"
CHANNEL_NAME="${CHANNEL_NAME}"

# bech32 npub for mention body
AGENT_NPUB=\$(python3 - <<'PY'
CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def bech32_polymod(values):
    GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
    chk = 1
    for v in values:
        b = (chk >> 25) & 0xff
        chk = ((chk & 0x1ffffff) << 5) ^ v
        for i in range(5):
            chk ^= GEN[i] if ((b >> i) & 1) else 0
    return chk
def bech32_hrp_expand(hrp):
    return [ord(x) >> 5 for x in hrp] + [0] + [ord(x) & 31 for x in hrp]
def bech32_create_checksum(hrp, data):
    values = bech32_hrp_expand(hrp) + data
    polymod = bech32_polymod(values + [0, 0, 0, 0, 0, 0]) ^ 1
    return [(polymod >> 5 * (5 - i)) & 31 for i in range(6)]
def convertbits(data, frombits, tobits, pad=True):
    acc = 0
    bits = 0
    ret = []
    maxv = (1 << tobits) - 1
    for value in data:
        acc = (acc << frombits) | value
        bits += frombits
        while bits >= tobits:
            bits -= tobits
            ret.append((acc >> bits) & maxv)
    if pad and bits:
        ret.append((acc << (tobits - bits)) & maxv)
    return ret
def encode(hrp, witprog):
    data = convertbits(witprog, 8, 5)
    combined = data + bech32_create_checksum(hrp, data)
    return hrp + "1" + "".join(CHARSET[d] for d in combined)
pub = open("/home/exedev/repro-keys/agent.pub").read().strip()
print(encode("npub", bytes.fromhex(pub)))
PY
)

echo "Creating channel \${CHANNEL_NAME}..."
CREATE_OUT=\$(\$BUZZ channels create --name "\$CHANNEL_NAME" --type stream --visibility open --description "bootstrap e2e")
echo "\$CREATE_OUT"
CHANNEL_ID=\$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("channel_id",""))' "\$CREATE_OUT")
[[ -n "\$CHANNEL_ID" ]] || { echo "failed to parse channel_id from: \$CREATE_OUT" >&2; exit 1; }
echo "CHANNEL_ID=\$CHANNEL_ID"

echo "Adding agent as channel member..."
\$BUZZ channels add-member --channel "\$CHANNEL_ID" --pubkey "\$AGENT_PUB"
sleep 1

# I8: agent may have started with 0 channels — restart so it rediscovers membership
echo "Restarting agent to pick up channel subscription..."
sudo systemctl restart buzz-intel-agent
sleep 3
systemctl is-active buzz-intel-agent
journalctl -u buzz-intel-agent -n 15 --no-pager | sed -E 's/(nsec1|INTEL_API_KEY|BUZZ_PRIVATE|api[_-]?key[=: ]+)[A-Za-z0-9._+/=-]{6,}/<redacted>/gi' || true

CONTENT="nostr:\${AGENT_NPUB} bootstrap e2e \${MARKER} — reply with the marker and the word pong"
echo "Sending owner @mention with marker \${MARKER}..."
SEND_OUT=\$(\$BUZZ messages send --channel "\$CHANNEL_ID" --content "\$CONTENT")
echo "\$SEND_OUT"
PROMPT_ID=\$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("event_id",""))' "\$SEND_OUT")

echo "Waiting up to 180s for agent reply containing \${MARKER}..."
HIT=NO
for i in \$(seq 1 36); do
  sleep 5
  \$BUZZ messages get --channel "\$CHANNEL_ID" --limit 20 > /tmp/bootstrap-msgs.json 2>/dev/null || true
  STATUS=\$(MARKER="\$MARKER" AGENT_PUB="\$AGENT_PUB" PROMPT_ID="\$PROMPT_ID" python3 - <<'PY'
import json, os
from pathlib import Path
marker = os.environ["MARKER"]
agent = os.environ["AGENT_PUB"]
prompt = os.environ["PROMPT_ID"]
raw = Path("/tmp/bootstrap-msgs.json").read_text() if Path("/tmp/bootstrap-msgs.json").exists() else ""
try:
    data = json.loads(raw) if raw.strip() else []
except Exception:
    print("NO")
    raise SystemExit
if isinstance(data, dict):
    data = data.get("events") or data.get("messages") or data.get("data") or []
for ev in data if isinstance(data, list) else []:
    if not isinstance(ev, dict):
        continue
    if ev.get("id") == prompt:
        continue
    if ev.get("pubkey") == agent and marker in (ev.get("content") or ""):
        print("YES")
        print("event_id=" + (ev.get("id") or ""))
        print("content=" + (ev.get("content") or "").replace("\\n", " "))
        raise SystemExit
print("NO")
PY
)
  echo "poll_\$i: \$(echo "\$STATUS" | head -1)"
  if echo "\$STATUS" | head -1 | grep -q YES; then
    HIT=YES
    echo "\$STATUS"
    break
  fi
done

echo "MARKER=\$MARKER"
echo "CHANNEL_ID=\$CHANNEL_ID"
echo "PROMPT_ID=\$PROMPT_ID"
echo "HIT=\$HIT"
if [[ "\$HIT" != "YES" ]]; then
  echo "WARN: no agent reply with marker within timeout — check journalctl -u buzz-intel-agent" >&2
  journalctl -u buzz-intel-agent -n 40 --no-pager | sed -E 's/(nsec1|INTEL_API_KEY|BUZZ_PRIVATE|api[_-]?key[=: ]+)[A-Za-z0-9._+/=-]{6,}/<redacted>/gi' || true
  exit 2
fi
EOF
chmod 700 "$SCRATCH/agent/onvm-e2e.sh"

# I9: avoid macOS extended-attribute noise on Linux extract
export COPYFILE_DISABLE=1
tar -C "$SCRATCH" -czf "$SCRATCH/payload.tgz" compose agent keys
scp -o BatchMode=yes "$SCRATCH/payload.tgz" "$VM:/tmp/payload.tgz"

ssh -o BatchMode=yes "$VM" 'set -euo pipefail
cd /tmp && rm -rf compose agent keys && tar -xzf payload.tgz
mkdir -p /home/exedev/buzz/deploy
rm -rf /home/exedev/buzz/deploy/compose
mv compose /home/exedev/buzz/deploy/compose
chmod 600 /home/exedev/buzz/deploy/compose/.env
sudo mkdir -p /opt/buzz-intel/bin /opt/buzz-intel/secrets /var/lib/buzz-intel-agent
sudo install -m 755 agent/buzz-acp /opt/buzz-intel/bin/buzz-acp
sudo install -m 755 agent/buzz-intel-agent /opt/buzz-intel/bin/buzz-intel-agent
sudo install -m 755 agent/buzz /opt/buzz-intel/bin/buzz
sudo install -m 600 agent/intel-e2e.key /opt/buzz-intel/secrets/intel-e2e.key
sudo install -m 700 agent/run-harness.sh /opt/buzz-intel/run-harness.sh
sudo install -m 700 agent/onvm-e2e.sh /opt/buzz-intel/onvm-e2e.sh
sudo cp agent/buzz-intel-agent.service /etc/systemd/system/buzz-intel-agent.service
sudo chown -R exedev:exedev /opt/buzz-intel /var/lib/buzz-intel-agent
mkdir -p /home/exedev/repro-keys && cp -a keys/* /home/exedev/repro-keys/ && chmod 700 /home/exedev/repro-keys && chmod 600 /home/exedev/repro-keys/*
# sanity: CLI is ELF (I6)
file /opt/buzz-intel/bin/buzz | grep -qi ELF || { echo "buzz CLI not ELF on VM" >&2; exit 1; }
cd /home/exedev/buzz/deploy/compose
docker compose --env-file .env -f compose.yml pull
docker compose --env-file .env -f compose.yml up -d --wait
curl -sS -m 10 -o /dev/null -w "health=%{http_code}\n" http://127.0.0.1:3000/health
# confirm git-init ran (I1 patch)
docker compose --env-file .env -f compose.yml ps -a buzz-git-init || true
AGENT_PUB=$(tr -d "\n" < /home/exedev/repro-keys/agent.pub)
docker compose --env-file .env -f compose.yml exec -T relay /usr/local/bin/buzz-admin add-member --pubkey "$AGENT_PUB" --role member
sudo systemctl daemon-reload
sudo systemctl enable --now buzz-intel-agent
sleep 2
systemctl is-active buzz-intel-agent
# On-VM channel create + mention (I6/I7 closed — no laptop tunnel)
/opt/buzz-intel/onvm-e2e.sh
'

echo "Bootstrap complete for $VM"
echo "Owner/agent pubs:"; cat "$SCRATCH/pubkeys.env"
echo "Marker used: $MARKER"
echo "Keys snapshot (optional): /tmp/${VM_BASENAME}-keys — delete after use"
cp -a "$SCRATCH/keys" "/tmp/${VM_BASENAME}-keys" 2>/dev/null || true
cp -a "$SCRATCH/pubkeys.env" "/tmp/${VM_BASENAME}-pubkeys.env" 2>/dev/null || true
echo "DONE — no SSH tunnel; channel/mention ran on-VM via artifacts/.../buzz"

#!/usr/bin/env bash
#
# Detect when the read-only state deployed on the Intel/Buzz VM has drifted
# from this checkout. This exists because a healthy service can silently keep
# running binaries and images built before behavior that is already committed.

set -uo pipefail

REMOTE_HOST="${BUZZ_DRIFT_HOST:-vm-buzz-relay-dev-wren.exe.xyz}"
SSH_CONNECT_TIMEOUT="${BUZZ_DRIFT_SSH_TIMEOUT:-10}"
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
CONFIG_FILE="$REPO_ROOT/crates/buzz-intel-agent/src/config.rs"

drift_count=0
error_count=0

drift() {
  drift_count=$((drift_count + 1))
  printf '  DRIFT: %s\n' "$*"
}

error() {
  error_count=$((error_count + 1))
  printf '  ERROR: %s\n' "$*"
}

contains_line() {
  needle=$1
  haystack=$2
  printf '%s\n' "$haystack" | grep -F -x -q -- "$needle"
}

printf 'Buzz deployed drift check\n'
printf '  repo:   %s\n' "$REPO_ROOT"
printf '  remote: %s\n' "$REMOTE_HOST"

if ! command -v git >/dev/null 2>&1; then
  error "git is not available locally"
  printf '\nSUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
    "$drift_count" "$error_count"
  exit 1
fi

if ! local_head=$(git -C "$REPO_ROOT" rev-parse --verify HEAD 2>/dev/null); then
  error "$REPO_ROOT is not a readable git checkout"
  printf '\nSUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
    "$drift_count" "$error_count"
  exit 1
fi

local_branch=$(git -C "$REPO_ROOT" symbolic-ref --quiet --short HEAD 2>/dev/null || printf '%s' '(detached)')
local_head_iso=$(git -C "$REPO_ROOT" show -s --format='%cI' "$local_head" 2>/dev/null || printf '%s' 'unknown')

printf '  branch: %s\n' "$local_branch"
printf '  HEAD:   %s (%s)\n' "$local_head" "$local_head_iso"

if [ ! -r "$CONFIG_FILE" ]; then
  error "cannot read authoritative Intel config at $CONFIG_FILE"
  printf '\nSUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
    "$drift_count" "$error_count"
  exit 1
fi

# Parse only clap's env declarations. This deliberately derives the inventory
# and defaults from config.rs instead of maintaining a second hard-coded list.
documented_vars=()
documented_defaults=()
documented_optional=()
documented_lines=()
in_arg=0
attribute=''
attribute_env_line=0
pending_field_index=-1
line_number=0
env_pattern='env[[:space:]]*=[[:space:]]*"(INTEL_[A-Z0-9_]+)"'
default_pattern='default_value[[:space:]]*=[[:space:]]*"([^"]*)"'
field_pattern='pub[[:space:]]+[a-zA-Z0-9_]+:[[:space:]]*(.*),'

while IFS= read -r line || [ -n "$line" ]; do
  line_number=$((line_number + 1))

  if [ "$pending_field_index" -ge 0 ] && [[ "$line" =~ $field_pattern ]]; then
    if [[ "${BASH_REMATCH[1]}" == Option\<* ]]; then
      documented_optional[$pending_field_index]=1
    else
      documented_optional[$pending_field_index]=0
    fi
    pending_field_index=-1
  fi

  if [[ "$line" == *'#[arg('* ]]; then
    in_arg=1
    attribute=$line
    attribute_env_line=0
  elif [ "$in_arg" -eq 1 ]; then
    attribute="$attribute $line"
  fi

  if [ "$in_arg" -eq 1 ] && [[ "$line" =~ $env_pattern ]]; then
    attribute_env_line=$line_number
  fi

  if [ "$in_arg" -eq 1 ] && [[ "$line" == *')]'* ]]; then
    if [[ "$attribute" =~ $env_pattern ]]; then
      index=${#documented_vars[@]}
      documented_vars[$index]=${BASH_REMATCH[1]}
      if [[ "$attribute" =~ $default_pattern ]]; then
        documented_defaults[$index]=${BASH_REMATCH[1]}
      else
        documented_defaults[$index]='<none declared>'
      fi
      documented_optional[$index]=0
      documented_lines[$index]=$attribute_env_line
      pending_field_index=$index
    fi
    in_arg=0
    attribute=''
    attribute_env_line=0
  fi
done < "$CONFIG_FILE"

if [ "${#documented_vars[@]}" -eq 0 ]; then
  error "parsed zero INTEL_* variables from $CONFIG_FILE"
  printf '\nSUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
    "$drift_count" "$error_count"
  exit 1
fi

# Config defers some operational requirements until ACP initialization. Parse
# those required singletons/alternative groups from the authoritative error
# clauses rather than assuming every Rust Option is operationally optional.
required_groups=()
required_group_count=0
while IFS= read -r requirement || [ -n "$requirement" ]; do
  [ -n "$requirement" ] || continue
  group=${requirement% is required}
  required_groups[$required_group_count]=$group
  required_group_count=$((required_group_count + 1))
done < <(
  grep -oE 'INTEL_[A-Z0-9_]+( or INTEL_[A-Z0-9_]+)* is required' \
    "$CONFIG_FILE" |
    sort -u
)

remote_file=$(mktemp "${TMPDIR:-/tmp}/buzz-deployed-drift.XXXXXX") || {
  error "could not create a local temporary file"
  printf '\nSUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
    "$drift_count" "$error_count"
  exit 1
}
remote_stderr="${remote_file}.stderr"
cleanup() {
  status=$?
  trap - EXIT
  rm -f "$remote_file" "$remote_stderr"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# The remote probe only reads metadata, process environment names, image
# labels, and --version output. It never writes files or changes services.
if ! ssh \
  -o BatchMode=yes \
  -o "ConnectTimeout=$SSH_CONNECT_TIMEOUT" \
  "$REMOTE_HOST" 'bash -s' >"$remote_file" 2>"$remote_stderr" <<'REMOTE_PROBE'
set -uo pipefail

clean_field() {
  printf '%s' "$1" | tr '\t\r\n' '   '
}

printf 'HOST\t%s\n' "$(hostname 2>/dev/null || printf '%s' unknown)"

unit='buzz-intel-agent.service'
if ! systemctl show "$unit" --no-pager >/dev/null 2>&1; then
  printf 'REMOTE_ERROR\tsystemd unit %s is unavailable or unreadable\n' "$unit"
else
  fragment=$(systemctl show "$unit" -p FragmentPath --value --no-pager 2>/dev/null || true)
  active=$(systemctl show "$unit" -p ActiveState --value --no-pager 2>/dev/null || true)
  main_pid=$(systemctl show "$unit" -p MainPID --value --no-pager 2>/dev/null || true)
  env_files=$(systemctl show "$unit" -p EnvironmentFiles --value --no-pager 2>/dev/null || true)
  printf 'UNIT\t%s\t%s\t%s\t%s\n' \
    "$(clean_field "${fragment:--}")" \
    "$(clean_field "${active:--}")" \
    "$(clean_field "${main_pid:--}")" \
    "$(clean_field "${env_files:--}")"

  unit_environment=$(systemctl show "$unit" -p Environment --value --no-pager 2>/dev/null || true)
  printf '%s\n' "$unit_environment" |
    grep -o 'INTEL_[A-Z0-9_]*=' 2>/dev/null |
    sed 's/=$//' |
    sort -u |
    while IFS= read -r var; do
      [ -n "$var" ] && printf 'UNIT_ENV\t%s\n' "$var"
    done

  if [ -z "$main_pid" ] || [ "$main_pid" = 0 ] || [ ! -r "/proc/$main_pid/environ" ]; then
    printf 'REMOTE_ERROR\tcannot read effective environment for %s MainPID %s\n' \
      "$unit" "${main_pid:-unknown}"
  else
    tr '\0' '\n' < "/proc/$main_pid/environ" |
      sed -n 's/^\(INTEL_[A-Z0-9_]*\)=.*/\1/p' |
      sort -u |
      while IFS= read -r var; do
        [ -n "$var" ] && printf 'PROCESS_ENV\t%s\n' "$var"
      done
  fi
fi

for binary_name in buzz-intel-agent buzz-acp; do
  binary_path="/opt/buzz-intel/bin/$binary_name"
  if [ ! -f "$binary_path" ]; then
    binary_path="/opt/buzz-intel/$binary_name"
  fi

  if [ ! -f "$binary_path" ]; then
    printf 'BINARY_MISSING\t%s\t/opt/buzz-intel/bin/%s\n' \
      "$binary_name" "$binary_name"
    continue
  fi

  if ! sha=$(sha256sum "$binary_path" 2>/dev/null | awk '{print $1}'); then
    printf 'REMOTE_ERROR\tcannot sha256 %s\n' "$binary_path"
    continue
  fi
  if ! stat_data=$(stat -c '%s|%Y|%y' "$binary_path" 2>/dev/null); then
    printf 'REMOTE_ERROR\tcannot stat %s\n' "$binary_path"
    continue
  fi
  size=${stat_data%%|*}
  stat_rest=${stat_data#*|}
  mtime_epoch=${stat_rest%%|*}
  mtime_iso=${stat_rest#*|}

  version_output=$(timeout 5 "$binary_path" --version </dev/null 2>&1)
  version_rc=$?
  version_output=$(clean_field "$version_output")
  [ -n "$version_output" ] || version_output='<no output>'

  printf 'BINARY\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$binary_name" "$binary_path" "$sha" "$size" "$mtime_epoch" \
    "$(clean_field "$mtime_iso")" "$version_rc" "$version_output"
done

if ! command -v docker >/dev/null 2>&1; then
  printf 'DOCKER_ERROR\tdocker is not installed on the remote host\n'
elif ! container_ids=$(docker ps -q 2>/dev/null); then
  printf 'DOCKER_ERROR\tdocker exists but running containers cannot be listed\n'
elif [ -z "$container_ids" ]; then
  printf 'DOCKER_ERROR\tdocker reports no running containers\n'
else
  relay_count=0
  for container_id in $container_ids; do
    image_id=$(docker inspect -f '{{.Image}}' "$container_id" 2>/dev/null || true)
    image_ref=$(docker inspect -f '{{.Config.Image}}' "$container_id" 2>/dev/null || true)
    container_name=$(docker inspect -f '{{.Name}}' "$container_id" 2>/dev/null || true)
    compose_service=$(docker inspect \
      -f '{{index .Config.Labels "com.docker.compose.service"}}' \
      "$container_id" 2>/dev/null || true)
    image_title=$(docker image inspect \
      -f '{{index .Config.Labels "org.opencontainers.image.title"}}' \
      "$image_id" 2>/dev/null || true)

    if [ "$compose_service" != relay ] && [ "$image_title" != Buzz ]; then
      continue
    fi

    relay_count=$((relay_count + 1))
    revision=$(docker image inspect \
      -f '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
      "$image_id" 2>/dev/null || true)
    [ -n "$revision" ] && [ "$revision" != '<no value>' ] || revision='<missing>'

    printf 'RELAY\t%s\t%s\t%s\t%s\t%s\n' \
      "$(clean_field "${container_name#/}")" \
      "$(clean_field "${container_id}")" \
      "$(clean_field "${image_ref:--}")" \
      "$(clean_field "${image_id:--}")" \
      "$(clean_field "$revision")"
  done

  if [ "$relay_count" -eq 0 ]; then
    printf 'DOCKER_ERROR\tno running Buzz relay container was found\n'
  fi
fi
REMOTE_PROBE
then
  error "SSH probe failed for $REMOTE_HOST"
  if [ -s "$remote_stderr" ]; then
    while IFS= read -r line; do
      printf '    ssh: %s\n' "$line"
    done < "$remote_stderr"
  fi
  printf '\nSUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
    "$drift_count" "$error_count"
  exit 1
fi

remote_host_name='unknown'
unit_fragment='unknown'
unit_active='unknown'
unit_pid='unknown'
unit_env_files='-'
unit_env_names=''
process_env_names=''
binary_names=()
binary_paths=()
binary_shas=()
binary_sizes=()
binary_epochs=()
binary_mtimes=()
binary_version_rcs=()
binary_versions=()
binary_count=0
relay_names=()
relay_ids=()
relay_refs=()
relay_image_ids=()
relay_revisions=()
relay_count=0
remote_errors=()
remote_error_count=0

while IFS=$'\t' read -r record field1 field2 field3 field4 field5 field6 field7 field8; do
  case "$record" in
    HOST)
      remote_host_name=$field1
      ;;
    UNIT)
      unit_fragment=$field1
      unit_active=$field2
      unit_pid=$field3
      unit_env_files=$field4
      ;;
    UNIT_ENV)
      unit_env_names="${unit_env_names}${field1}"$'\n'
      ;;
    PROCESS_ENV)
      process_env_names="${process_env_names}${field1}"$'\n'
      ;;
    BINARY)
      index=$binary_count
      binary_names[$index]=$field1
      binary_paths[$index]=$field2
      binary_shas[$index]=$field3
      binary_sizes[$index]=$field4
      binary_epochs[$index]=$field5
      binary_mtimes[$index]=$field6
      binary_version_rcs[$index]=$field7
      binary_versions[$index]=$field8
      binary_count=$((binary_count + 1))
      ;;
    BINARY_MISSING)
      remote_errors[$remote_error_count]="missing remote binary $field1 (looked under /opt/buzz-intel)"
      remote_error_count=$((remote_error_count + 1))
      ;;
    RELAY)
      index=$relay_count
      relay_names[$index]=$field1
      relay_ids[$index]=$field2
      relay_refs[$index]=$field3
      relay_image_ids[$index]=$field4
      relay_revisions[$index]=$field5
      relay_count=$((relay_count + 1))
      ;;
    REMOTE_ERROR|DOCKER_ERROR)
      remote_errors[$remote_error_count]=$field1
      remote_error_count=$((remote_error_count + 1))
      ;;
    '')
      ;;
    *)
      remote_errors[$remote_error_count]="unrecognized remote probe record: $record"
      remote_error_count=$((remote_error_count + 1))
      ;;
  esac
done < "$remote_file"

printf '\nRemote host\n'
printf '  hostname: %s\n' "$remote_host_name"
for ((i = 0; i < remote_error_count; i++)); do
  error "${remote_errors[$i]}"
done

printf '\nDeployed binaries\n'
agent_artifact_epoch=''
if [ "$binary_count" -eq 0 ]; then
  error "no deployed Buzz binaries could be inspected"
else
  for ((i = 0; i < binary_count; i++)); do
    printf '  %s\n' "${binary_names[$i]}"
    printf '    path:    %s\n' "${binary_paths[$i]}"
    printf '    sha256:  %s\n' "${binary_shas[$i]}"
    printf '    size:    %s bytes\n' "${binary_sizes[$i]}"
    printf '    mtime:   %s (epoch %s)\n' \
      "${binary_mtimes[$i]}" "${binary_epochs[$i]}"
    if [ "${binary_version_rcs[$i]}" -eq 0 ]; then
      printf '    version: %s\n' "${binary_versions[$i]}"
    else
      printf '    version: unsupported (exit %s; %s)\n' \
        "${binary_version_rcs[$i]}" "${binary_versions[$i]}"
    fi

    if [ "${binary_names[$i]}" = buzz-intel-agent ]; then
      agent_artifact_epoch=${binary_epochs[$i]}
    fi

    source_path="crates/${binary_names[$i]}"
    newer_count=$(git -C "$REPO_ROOT" rev-list \
      --count --since="@${binary_epochs[$i]}" HEAD -- "$source_path" \
      2>/dev/null || printf '%s' error)
    if [ "$newer_count" = error ]; then
      error "could not compare local commits with ${binary_names[$i]} mtime"
    elif [ "$newer_count" -gt 0 ]; then
      printf '    chronology: predates %s local commit(s) touching %s\n' \
        "$newer_count" "$source_path"
    else
      printf '    chronology: no newer local commit touches %s\n' "$source_path"
    fi
  done
fi

printf '\nSystemd Intel environment\n'
printf '  unit:        buzz-intel-agent.service\n'
printf '  fragment:    %s\n' "$unit_fragment"
printf '  active:      %s\n' "$unit_active"
printf '  MainPID:     %s\n' "$unit_pid"
printf '  env files:   %s\n' "$unit_env_files"
printf '  unit INTEL_* vars: '
if [ -n "$unit_env_names" ]; then
  printf '\n'
  printf '%s' "$unit_env_names" | while IFS= read -r var; do
    [ -n "$var" ] && printf '    %s\n' "$var"
  done
else
  printf '(none; the service may populate them through its launcher)\n'
fi
printf '  effective MainPID INTEL_* vars:\n'
if [ -n "$process_env_names" ]; then
  printf '%s' "$process_env_names" | while IFS= read -r var; do
    [ -n "$var" ] && printf '    %s\n' "$var"
  done
else
  printf '    (none)\n'
fi

ok_vars=()
ok_count=0
optional_vars=()
optional_count=0
alternative_vars=()
alternative_count=0
default_vars=()
default_count=0

for ((group_index = 0; group_index < required_group_count; group_index++)); do
  group=${required_groups[$group_index]}
  group_satisfied=0
  while IFS= read -r alternative; do
    if contains_line "$alternative" "$process_env_names"; then
      group_satisfied=1
      break
    fi
  done < <(printf '%s\n' "$group" | sed 's/ or /\
/g')

  if [ "$group_satisfied" -eq 0 ]; then
    drift "required config is absent; set one of: $group"
  fi
done

for ((i = 0; i < ${#documented_vars[@]}; i++)); do
  var=${documented_vars[$i]}
  default=${documented_defaults[$i]}
  if contains_line "$var" "$process_env_names"; then
    ok_vars[$ok_count]=$var
    ok_count=$((ok_count + 1))
  elif [ "$default" != '<none declared>' ]; then
    config_line=${documented_lines[$i]}
    blame_data=$(git -C "$REPO_ROOT" blame \
      --porcelain -L "$config_line,$config_line" \
      HEAD -- crates/buzz-intel-agent/src/config.rs 2>/dev/null || true)
    introduced_commit=${blame_data%% *}
    introduced_epoch=$(printf '%s\n' "$blame_data" |
      sed -n 's/^committer-time //p' |
      head -n 1)

    if [ -n "$agent_artifact_epoch" ] &&
      [ -n "$introduced_epoch" ] &&
      [ "$introduced_epoch" -gt "$agent_artifact_epoch" ]; then
      short_commit=${introduced_commit:0:12}
      drift "$var is absent and was introduced in $short_commit after the deployed buzz-intel-agent was built"
    else
      default_vars[$default_count]="$var=$default"
      default_count=$((default_count + 1))
    fi
  else
    belongs_to_required_group=0
    required_group_is_satisfied=0
    for ((group_index = 0; group_index < required_group_count; group_index++)); do
      group=${required_groups[$group_index]}
      group_contains_var=0
      group_satisfied=0
      while IFS= read -r alternative; do
        [ "$alternative" = "$var" ] && group_contains_var=1
        contains_line "$alternative" "$process_env_names" &&
          group_satisfied=1
      done < <(printf '%s\n' "$group" | sed 's/ or /\
/g')

      if [ "$group_contains_var" -eq 1 ]; then
        belongs_to_required_group=1
        [ "$group_satisfied" -eq 1 ] && required_group_is_satisfied=1
      fi
    done

    if [ "$belongs_to_required_group" -eq 1 ]; then
      if [ "$required_group_is_satisfied" -eq 1 ]; then
        alternative_vars[$alternative_count]=$var
        alternative_count=$((alternative_count + 1))
      fi
      # An unsatisfied group was already emitted once above.
    elif [ "${documented_optional[$i]}" -eq 1 ]; then
      optional_vars[$optional_count]=$var
      optional_count=$((optional_count + 1))
    else
      drift "$var is required by config.rs but absent from the service's effective environment"
    fi
  fi
done

printf '  classified config.rs inventory:\n'
printf '    OK: '
if [ "$ok_count" -eq 0 ]; then
  printf '(none)\n'
else
  for ((i = 0; i < ok_count; i++)); do
    [ "$i" -eq 0 ] || printf ', '
    printf '%s' "${ok_vars[$i]}"
  done
  printf '\n'
fi

if [ "$alternative_count" -gt 0 ]; then
  printf '    OK (required alternative set): '
  for ((i = 0; i < alternative_count; i++)); do
    [ "$i" -eq 0 ] || printf ', '
    printf '%s' "${alternative_vars[$i]}"
  done
  printf '\n'
fi

if [ "$optional_count" -gt 0 ]; then
  printf '    OK (optional, unset): '
  for ((i = 0; i < optional_count; i++)); do
    [ "$i" -eq 0 ] || printf ', '
    printf '%s' "${optional_vars[$i]}"
  done
  printf '\n'
fi

printf '    DEFAULT: '
if [ "$default_count" -eq 0 ]; then
  printf '(none)\n'
else
  for ((i = 0; i < default_count; i++)); do
    [ "$i" -eq 0 ] || printf ', '
    printf '%s' "${default_vars[$i]}"
  done
  printf '\n'
fi

if [ "$unit_active" != active ]; then
  error "buzz-intel-agent.service is not active (reported: $unit_active)"
fi

printf '\nRunning relay container images\n'
if [ "$relay_count" -eq 0 ]; then
  error "no running relay image revision was available for comparison"
else
  for ((i = 0; i < relay_count; i++)); do
    revision=${relay_revisions[$i]}
    printf '  %s\n' "${relay_names[$i]}"
    printf '    container: %s\n' "${relay_ids[$i]}"
    printf '    image ref: %s\n' "${relay_refs[$i]}"
    printf '    image id:  %s\n' "${relay_image_ids[$i]}"
    printf '    revision:  %s\n' "$revision"

    if [ "$revision" = '<missing>' ]; then
      drift "${relay_names[$i]} image has no org.opencontainers.image.revision label"
    elif [ "$revision" = "$local_head" ]; then
      printf '    compare:   exact match with local HEAD\n'
    else
      drift "${relay_names[$i]} revision $revision does not equal local HEAD $local_head"
      if git -C "$REPO_ROOT" cat-file -e "$revision^{commit}" 2>/dev/null; then
        if git -C "$REPO_ROOT" merge-base --is-ancestor "$revision" "$local_head" 2>/dev/null; then
          ahead=$(git -C "$REPO_ROOT" rev-list --count "$revision..$local_head")
          printf '    relation:  local HEAD is %s commit(s) ahead\n' "$ahead"
        elif git -C "$REPO_ROOT" merge-base --is-ancestor "$local_head" "$revision" 2>/dev/null; then
          behind=$(git -C "$REPO_ROOT" rev-list --count "$local_head..$revision")
          printf '    relation:  local HEAD is %s commit(s) behind\n' "$behind"
        else
          divergence=$(git -C "$REPO_ROOT" rev-list \
            --left-right --count "$revision...$local_head" 2>/dev/null || printf '%s' 'unknown')
          printf '    relation:  histories diverged (deployed-only/local-only: %s)\n' \
            "$divergence"
        fi
      else
        printf '    relation:  deployed revision is not present in this checkout\n'
      fi
    fi
  done
fi

printf '\n'
if [ "$drift_count" -eq 0 ] && [ "$error_count" -eq 0 ]; then
  printf 'SUMMARY: PASS (deployed artifacts and relay revision match this checkout)\n'
  exit 0
fi

printf 'SUMMARY: DRIFT (%s drift finding(s), %s check error(s))\n' \
  "$drift_count" "$error_count"
exit 1

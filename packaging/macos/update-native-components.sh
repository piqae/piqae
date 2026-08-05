#!/usr/bin/env bash
set -euo pipefail

source_root=${1:-}
expected_version=${2:-}
channel=${3:-}
support_root="$HOME/Library/Application Support/Spool"
install_root="$support_root/bin"
launch_agents="$HOME/Library/LaunchAgents"
agent_plist="$launch_agents/com.piqae.node.agent.plist"
agent_label="com.piqae.node.agent"
domain="gui/$UID"
local_port=39100

fail() { echo "$1" >&2; exit 1; }
[[ "$(uname -s)" == Darwin ]] || fail "Native component updates require macOS."
[[ "$EUID" -ne 0 ]] || fail "Native component updates must run as the desktop user."
[[ "$source_root" == /* && -d "$source_root" ]] || fail "The embedded component directory is invalid."
[[ "$expected_version" =~ ^[0-9A-Za-z][0-9A-Za-z.-]*$ ]] || fail "The update version is invalid."
[[ "$channel" == signed-release || "$channel" == unsigned-preview ]] || fail "The update channel is invalid."

agent_source="$source_root/piqae-agent"
executor_source="$source_root/piqae-executor-cups"
for component in "$agent_source" "$executor_source"; do
  [[ -x "$component" ]] || fail "The update is missing a native component."
done

component_version() {
  "$1" --version 2>/dev/null | awk 'NF >= 2 { print $2; exit }'
}

if [[ "$channel" == signed-release ]]; then
  app_root=$(cd "$source_root/../../.." && pwd)
  app_team=$(/usr/bin/codesign -dv --verbose=4 "$app_root" 2>&1 | awk -F= '$1 == "TeamIdentifier" { print $2; exit }')
  [[ -n "$app_team" ]] || fail "The signed app team could not be verified."
  for component in "$agent_source" "$executor_source"; do
    /usr/bin/codesign --verify --strict "$component" >/dev/null 2>&1 || fail "An embedded component signature is invalid."
    /usr/bin/codesign -dv --verbose=4 "$component" 2>&1 | grep -F "Authority=Developer ID Application:" >/dev/null ||
      fail "An embedded component is not Developer ID signed."
    component_team=$(/usr/bin/codesign -dv --verbose=4 "$component" 2>&1 | awk -F= '$1 == "TeamIdentifier" { print $2; exit }')
    [[ "$component_team" == "$app_team" ]] || fail "An embedded component is signed by a different team."
  done
fi
[[ "$(component_version "$agent_source")" == "$expected_version" ]] || fail "The embedded agent version does not match the app."
[[ "$(component_version "$executor_source")" == "$expected_version" ]] || fail "The embedded executor version does not match the app."

agent_destination="$install_root/piqae-agent"
executor_destination="$install_root/piqae-executor-cups"
if [[ -x "$agent_destination" && -x "$executor_destination" ]] &&
  [[ "$(component_version "$agent_destination")" == "$expected_version" ]] &&
  [[ "$(component_version "$executor_destination")" == "$expected_version" ]]
then
  exit 0
fi

[[ -r "$agent_plist" ]] || fail "The installed node service definition is unavailable."
token_file="$support_root/local.token"
[[ -r "$token_file" ]] || fail "The installed node token is unavailable."
token=$(tr -d '\r\n' < "$token_file")
[[ -n "$token" && ${#token} -le 1024 && "$token" =~ ^[A-Za-z0-9_-]+$ ]] || fail "The installed node token is invalid."
status=$(printf 'header = "Authorization: Bearer %s"\n' "$token" | /usr/bin/curl --config - --fail --silent --show-error --max-time 5 "http://127.0.0.1:$local_port/v1/local/status") ||
  fail "The installed node did not provide authenticated idle status."
queued=$(printf '%s' "$status" | /usr/bin/plutil -extract queued_jobs raw -o - - 2>/dev/null || true)
active=$(printf '%s' "$status" | /usr/bin/plutil -extract active_jobs raw -o - - 2>/dev/null || true)
[[ "$queued" =~ ^[0-9]+$ && "$active" =~ ^[0-9]+$ ]] || fail "The installed node returned invalid queue status."
[[ "$queued" -eq 0 && "$active" -eq 0 ]] || fail "The native update is waiting for the print queue to become idle."

mkdir -p "$install_root"
stage=$(mktemp -d "$install_root/.piqae-native-update.XXXXXX")
backup=$(mktemp -d "$install_root/.piqae-native-backup.XXXXXX")
cleanup() { rm -rf -- "$stage" "$backup"; }
trap cleanup EXIT
install -m 0755 "$agent_source" "$stage/piqae-agent"
install -m 0755 "$executor_source" "$stage/piqae-executor-cups"

rollback() {
  /bin/launchctl bootout "$domain/$agent_label" >/dev/null 2>&1 || true
  rm -f -- "$agent_destination" "$executor_destination"
  [[ ! -e "$backup/piqae-agent" ]] || mv -f "$backup/piqae-agent" "$agent_destination"
  [[ ! -e "$backup/piqae-executor-cups" ]] || mv -f "$backup/piqae-executor-cups" "$executor_destination"
  /bin/launchctl bootstrap "$domain" "$agent_plist" >/dev/null 2>&1 || true
  /bin/launchctl enable "$domain/$agent_label" >/dev/null 2>&1 || true
  /bin/launchctl kickstart "$domain/$agent_label" >/dev/null 2>&1 || true
}

/bin/launchctl bootout "$domain/$agent_label" || fail "The installed node could not be stopped safely."
[[ ! -e "$agent_destination" ]] || mv "$agent_destination" "$backup/piqae-agent"
[[ ! -e "$executor_destination" ]] || mv "$executor_destination" "$backup/piqae-executor-cups"
if ! mv "$stage/piqae-agent" "$agent_destination" ||
  ! mv "$stage/piqae-executor-cups" "$executor_destination"
then
  rollback
  fail "The native component switch failed; the previous components were restored."
fi

if ! /bin/launchctl bootstrap "$domain" "$agent_plist" ||
  ! /bin/launchctl enable "$domain/$agent_label" ||
  ! /bin/launchctl kickstart "$domain/$agent_label"
then
  rollback
  fail "The updated node could not be started; the previous components were restored."
fi

healthy=false
for _ in {1..40}; do
  if printf 'header = "Authorization: Bearer %s"\n' "$token" |
    /usr/bin/curl --config - --fail --silent --max-time 2 "http://127.0.0.1:$local_port/v1/local/status" >/dev/null 2>&1
  then
    healthy=true
    break
  fi
  sleep 0.25
done
if [[ "$healthy" != true ]]; then
  rollback
  fail "The updated node failed its health check; the previous components were restored."
fi

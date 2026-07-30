#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Piqae's per-user package can only be installed on macOS." >&2
  exit 2
fi
if [[ "$EUID" -eq 0 ]]; then
  echo "Run this installer as the desktop user, without sudo." >&2
  exit 2
fi

package_root=$(cd "$(dirname "$0")" && pwd)
payload="$package_root/payload"
app_source="$payload/Piqae.app"
agent_source="$payload/piqae-agent"
executor_source="$payload/piqae-executor-cups"
agent_template="$payload/com.piqae.agent.plist.in"
menu_template="$payload/com.piqae.menu.plist.in"

for required in "$app_source" "$agent_source" "$executor_source" "$agent_template" "$menu_template"; do
  if [[ ! -e "$required" ]]; then
    echo "Package is incomplete: missing $required" >&2
    exit 1
  fi
done

channel=$(/usr/libexec/PlistBuddy -c "Print :PiqaeBuildChannel" "$app_source/Contents/Info.plist")
if [[ "$channel" == "signed-release" ]]; then
  for signed_component in "$app_source" "$agent_source" "$executor_source"; do
    if ! codesign --verify --deep --strict "$signed_component" >/dev/null 2>&1 ||
      ! codesign -dv --verbose=4 "$signed_component" 2>&1 |
        grep -F "Authority=Developer ID Application:" >/dev/null
    then
      echo "Signed package verification failed for $signed_component." >&2
      exit 1
    fi
  done
elif [[ "$channel" != "unsigned-preview" ]]; then
  echo "Package has an unknown release channel; refusing installation." >&2
  exit 1
fi

# These shipped paths and service identifiers are intentionally stable through
# the V1 rebrand so an upgrade retains node identity, profiles, and queue state.
support_root="$HOME/Library/Application Support/Spool"
install_root="$support_root/bin"
app_root="$HOME/Applications/Piqae.app"
legacy_app_root="$HOME/Applications/Spool.app"
launch_agents="$HOME/Library/LaunchAgents"
log_root="$HOME/Library/Logs/Spool"
agent_plist="$launch_agents/com.c4coffee.spool.agent.plist"
menu_plist="$launch_agents/com.c4coffee.spool.menu.plist"
agent_label="com.c4coffee.spool.agent"
menu_label="com.c4coffee.spool.menu"
domain="gui/$UID"
local_port=39100

launch_agent_pid() {
  launchctl print "$domain/$agent_label" 2>/dev/null |
    awk '$1 == "pid" && $2 == "=" && $3 ~ /^[0-9]+$/ { print $3; exit }'
}

listener_pid() {
  { /usr/sbin/lsof -nP -t -iTCP:"$local_port" -sTCP:LISTEN 2>/dev/null || true; } |
    sort -n |
    head -n 1
}

is_loaded=false
if launchctl print "$domain/$agent_label" >/dev/null 2>&1; then
  is_loaded=true
fi

if [[ "$is_loaded" == true ]]; then
  managed_pid=$(launch_agent_pid)
  bound_pid=$(listener_pid)
  if [[ -z "$managed_pid" || -z "$bound_pid" || "$managed_pid" != "$bound_pid" ]]; then
    echo "The Piqae local port is not owned by the installed LaunchAgent. Stop any development node before installing." >&2
    exit 1
  fi
  token_file="$support_root/local.token"
  if [[ ! -r "$token_file" ]]; then
    echo "The running agent's local token is unavailable; refusing an unverified handoff." >&2
    exit 1
  fi
  token=$(tr -d '\r\n' < "$token_file")
  if [[ -z "$token" || "${#token}" -gt 1024 || ! "$token" =~ ^[A-Za-z0-9_-]+$ ]]; then
    echo "The running agent's local token is invalid; refusing the handoff." >&2
    exit 1
  fi
  status=$(
    printf 'header = "Authorization: Bearer %s"\n' "$token" |
      curl \
        --config - \
        --fail \
        --silent \
        --show-error \
        --max-time 5 \
        http://127.0.0.1:39100/v1/local/status
  ) || {
    echo "The running agent did not provide idle status; refusing an unverified handoff." >&2
    exit 1
  }
  queued=$(printf '%s' "$status" | plutil -extract queued_jobs raw -o - - 2>/dev/null || true)
  active=$(printf '%s' "$status" | plutil -extract active_jobs raw -o - - 2>/dev/null || true)
  if [[ ! "$queued" =~ ^[0-9]+$ || ! "$active" =~ ^[0-9]+$ ]]; then
    echo "The running agent returned invalid queue status; refusing the handoff." >&2
    exit 1
  fi
  if [[ "$queued" -ne 0 || "$active" -ne 0 ]]; then
    echo "Piqae has $queued queued and $active active jobs. Drain them before updating." >&2
    exit 1
  fi
elif [[ -n "$(listener_pid)" ]]; then
  echo "The Piqae local port is already in use. Stop the other local node before installing." >&2
  exit 1
fi

if [[ "$is_loaded" == true ]]; then
  launchctl bootout "$domain/$agent_label"
fi
launchctl bootout "$domain/$menu_label" >/dev/null 2>&1 || true
/usr/bin/osascript \
  -e 'tell application id "com.c4coffee.spool.menu" to quit' \
  >/dev/null 2>&1 || true
for _ in {1..20}; do
  if ! pgrep -x PiqaeMenu >/dev/null 2>&1 &&
    ! pgrep -x SpoolMenu >/dev/null 2>&1
  then
    break
  fi
  sleep 0.25
done
if pgrep -x PiqaeMenu >/dev/null 2>&1 || pgrep -x SpoolMenu >/dev/null 2>&1; then
  echo "Piqae Menu did not quit; no files were replaced." >&2
  exit 1
fi

mkdir -p "$install_root" "$HOME/Applications" "$launch_agents" "$log_root"
install -m 0755 "$agent_source" "$install_root/piqae-agent"
install -m 0755 "$executor_source" "$install_root/piqae-executor-cups"

app_stage=$(mktemp -d "$HOME/Applications/.piqae-app.XXXXXX")
trap 'rm -rf -- "$app_stage"' EXIT
ditto "$app_source" "$app_stage/Piqae.app"
installed_app=""
if [[ -e "$app_root" ]]; then
  installed_app="$app_root"
elif [[ -e "$legacy_app_root" ]]; then
  installed_app="$legacy_app_root"
fi
if [[ -n "$installed_app" ]]; then
  previous_app="$HOME/Applications/Piqae.previous.$(date -u +%Y%m%dT%H%M%SZ).app"
  mv "$installed_app" "$previous_app"
  echo "Previous app retained at $previous_app"
fi
mv "$app_stage/Piqae.app" "$app_root"

render_plist() {
  local source=$1
  local destination=$2
  sed \
    -e "s|@INSTALL_ROOT@|${install_root//&/\\&}|g" \
    -e "s|@DATA_DIR@|${support_root//&/\\&}|g" \
    -e "s|@LOG_FILE@|${log_root//&/\\&}/agent.log|g" \
    -e "s|@APP_ROOT@|${app_root//&/\\&}|g" \
    "$source" > "$destination"
  plutil -lint "$destination"
  chmod 0644 "$destination"
}

agent_plist_stage=$(mktemp "$launch_agents/.piqae-agent.XXXXXX")
menu_plist_stage=$(mktemp "$launch_agents/.piqae-menu.XXXXXX")
render_plist "$agent_template" "$agent_plist_stage"
render_plist "$menu_template" "$menu_plist_stage"
mv "$agent_plist_stage" "$agent_plist"
mv "$menu_plist_stage" "$menu_plist"

launchctl bootstrap "$domain" "$agent_plist"
launchctl enable "$domain/$agent_label"
launchctl kickstart "$domain/$agent_label"
launchctl bootstrap "$domain" "$menu_plist"
launchctl enable "$domain/$menu_label"

healthy=false
for _ in {1..40}; do
  installed_pid=$(listener_pid)
  if [[ -n "$installed_pid" ]]; then
    installed_command=$(ps -p "$installed_pid" -o command= 2>/dev/null || true)
    if [[ "$installed_command" == "$install_root/piqae-agent"* ]] &&
      curl \
        --fail \
        --silent \
        --show-error \
        --max-time 2 \
        --config <(printf 'header = "Authorization: Bearer %s"\n' "$(tr -d '\r\n' < "$support_root/local.token")") \
        "http://127.0.0.1:$local_port/v1/local/status" \
        >/dev/null
    then
      healthy=true
      break
    fi
  fi
  sleep 0.25
done
if [[ "$healthy" != true ]]; then
  echo "The installed Piqae node did not become healthy within 10 seconds." >&2
  exit 1
fi

if codesign --verify --deep --strict "$app_root" >/dev/null 2>&1 &&
  codesign -dv --verbose=4 "$app_root" 2>&1 |
    grep -F "Authority=Developer ID Application:" >/dev/null
then
  echo "Installed a Developer ID-signed Piqae app for the current user."
else
  echo "Installed an unsigned Preview build. macOS may block it; no Gatekeeper bypass was applied."
fi
echo "Agent data remains in $support_root."

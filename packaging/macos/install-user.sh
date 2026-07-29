#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Spool's per-user package can only be installed on macOS." >&2
  exit 2
fi
if [[ "$EUID" -eq 0 ]]; then
  echo "Run this installer as the desktop user, without sudo." >&2
  exit 2
fi

package_root=$(cd "$(dirname "$0")" && pwd)
payload="$package_root/payload"
app_source="$payload/Spool.app"
agent_source="$payload/spool-agent"
executor_source="$payload/spool-executor-cups"
agent_template="$payload/com.c4coffee.spool.agent.plist.in"
menu_template="$payload/com.c4coffee.spool.menu.plist.in"

for required in "$app_source" "$agent_source" "$executor_source" "$agent_template" "$menu_template"; do
  if [[ ! -e "$required" ]]; then
    echo "Package is incomplete: missing $required" >&2
    exit 1
  fi
done

support_root="$HOME/Library/Application Support/Spool"
install_root="$support_root/bin"
app_root="$HOME/Applications/Spool.app"
launch_agents="$HOME/Library/LaunchAgents"
log_root="$HOME/Library/Logs/Spool"
agent_plist="$launch_agents/com.c4coffee.spool.agent.plist"
menu_plist="$launch_agents/com.c4coffee.spool.menu.plist"
agent_label="com.c4coffee.spool.agent"
menu_label="com.c4coffee.spool.menu"
domain="gui/$UID"

is_loaded=false
if launchctl print "$domain/$agent_label" >/dev/null 2>&1; then
  is_loaded=true
fi

if [[ "$is_loaded" == true ]]; then
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
    echo "Spool has $queued queued and $active active jobs. Drain them before updating." >&2
    exit 1
  fi
fi

if [[ "$is_loaded" == true ]]; then
  launchctl bootout "$domain/$agent_label"
fi
launchctl bootout "$domain/$menu_label" >/dev/null 2>&1 || true

mkdir -p "$install_root" "$HOME/Applications" "$launch_agents" "$log_root"
install -m 0755 "$agent_source" "$install_root/spool-agent"
install -m 0755 "$executor_source" "$install_root/spool-executor-cups"

app_stage=$(mktemp -d "$HOME/Applications/.spool-app.XXXXXX")
trap 'rm -rf -- "$app_stage"' EXIT
ditto "$app_source" "$app_stage/Spool.app"
if [[ -e "$app_root" ]]; then
  previous_app="$HOME/Applications/Spool.previous.$(date -u +%Y%m%dT%H%M%SZ).app"
  mv "$app_root" "$previous_app"
  echo "Previous app retained at $previous_app"
fi
mv "$app_stage/Spool.app" "$app_root"

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

agent_plist_stage=$(mktemp "$launch_agents/.spool-agent.XXXXXX")
menu_plist_stage=$(mktemp "$launch_agents/.spool-menu.XXXXXX")
render_plist "$agent_template" "$agent_plist_stage"
render_plist "$menu_template" "$menu_plist_stage"
mv "$agent_plist_stage" "$agent_plist"
mv "$menu_plist_stage" "$menu_plist"

launchctl bootstrap "$domain" "$agent_plist"
launchctl enable "$domain/$agent_label"
launchctl kickstart "$domain/$agent_label"
launchctl bootstrap "$domain" "$menu_plist"
launchctl enable "$domain/$menu_label"

if codesign --verify --deep --strict "$app_root" >/dev/null 2>&1 &&
  codesign -dv --verbose=4 "$app_root" 2>&1 |
    grep -F "Authority=Developer ID Application:" >/dev/null
then
  echo "Installed a Developer ID-signed Spool app for the current user."
else
  echo "Installed an unsigned Preview build. macOS may block it; no Gatekeeper bypass was applied."
fi
echo "Agent data remains in $support_root."

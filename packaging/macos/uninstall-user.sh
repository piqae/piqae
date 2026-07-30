#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" || "$EUID" -eq 0 ]]; then
  echo "Run this uninstaller as the macOS desktop user, without sudo." >&2
  exit 2
fi

domain="gui/$UID"
launch_agents="$HOME/Library/LaunchAgents"
agent_plist="$launch_agents/com.c4coffee.spool.agent.plist"
menu_plist="$launch_agents/com.c4coffee.spool.menu.plist"
app_root="$HOME/Applications/Piqae.app"
legacy_app_root="$HOME/Applications/Spool.app"
install_root="$HOME/Library/Application Support/Spool/bin"

launchctl bootout "$domain/com.c4coffee.spool.agent" >/dev/null 2>&1 || true
launchctl bootout "$domain/com.c4coffee.spool.menu" >/dev/null 2>&1 || true

rm -f -- "$agent_plist" "$menu_plist"
rm -rf -- "$app_root" "$legacy_app_root" "$install_root"

echo "Removed the per-user app, binaries, and LaunchAgents."
echo "Queue, identity, configuration, and logs were preserved under your Library."

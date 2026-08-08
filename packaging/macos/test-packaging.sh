#!/usr/bin/env bash
set -euo pipefail

script_root=$(cd "$(dirname "$0")" && pwd)
agent_plist="$script_root/com.piqae.agent.plist.in"
install_script="$script_root/install-user.sh"

grep -F '<key>PIQAE_LOG_FILE</key>' "$agent_plist" >/dev/null
[[ $(grep -F '<string>/dev/null</string>' "$agent_plist" | wc -l | tr -d ' ') == 2 ]]
if grep -A1 -E 'Standard(Out|Error)Path' "$agent_plist" | grep -F '@LOG_FILE@' >/dev/null; then
  echo "launchd still owns an unbounded agent log" >&2
  exit 1
fi
grep -F '@LOG_FILE@' "$agent_plist" >/dev/null
grep -F 'Library/Logs/Spool' "$install_script" >/dev/null

echo "macOS logging packaging tests passed."

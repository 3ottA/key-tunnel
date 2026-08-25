#!/usr/bin/env bash
set -euo pipefail

socket="/run/remote-input-bridge/ydotool.sock"
for _ in {1..50}; do
  if [[ -S "$socket" ]]; then
    chown root:remote-input "$socket"
    chmod 0660 "$socket"
    exit 0
  fi
  sleep 0.1
done

echo "ydotool socket was not created: $socket" >&2
exit 1

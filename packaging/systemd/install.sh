#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: sudo ./install.sh /path/to/remote-input-receiver" >&2
  exit 2
fi

receiver="$1"
test -f "$receiver"
supported_ydotool="1.0.4"
actual_ydotool="$(ydotoold --version 2>&1 | head -n 1)"
if [[ "$actual_ydotool" != *"$supported_ydotool"* ]]; then
  echo "unsupported ydotoold: '$actual_ydotool' (expected $supported_ydotool)" >&2
  exit 1
fi
getent group remote-input >/dev/null || groupadd --system remote-input
install -d -m 0755 /usr/local/libexec/remote-input-bridge /etc/remote-input-bridge
install -m 0755 "$receiver" /usr/local/libexec/remote-input-bridge/remote-input-receiver
install -m 0644 receiver.toml /etc/remote-input-bridge/receiver.toml
install -m 0644 remote-input-ydotoold.service /etc/systemd/system/remote-input-ydotoold.service
systemctl daemon-reload
systemctl enable --now remote-input-ydotoold.service
echo "Installed receiver and ydotoold unit. Add the SSH user to group 'remote-input',"
echo "install the restricted authorized_keys entry, then verify after reboot."

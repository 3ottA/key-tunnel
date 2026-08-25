#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: sudo ./install.sh /path/to/remote-input-receiver" >&2
  exit 2
fi

receiver="$1"
if [[ "$receiver" != /* ]]; then
  receiver="$(pwd)/$receiver"
fi
test -f "$receiver"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"
supported_ydotool="1.0.4"
if ! command -v ydotoold >/dev/null 2>&1; then
  echo "ydotoold was not found in PATH" >&2
  exit 1
fi

# Arch's ydotoold does not implement --version; it prints "unknown".
# Validate the package version instead, ignoring the Arch package revision
# (for example, 1.0.4-2 has upstream version 1.0.4).
if command -v pacman >/dev/null 2>&1; then
  installed_ydotool="$(pacman -Q ydotool 2>/dev/null | awk 'NR == 1 { print $2 }')"
  actual_ydotool="${installed_ydotool%%-*}"
else
  actual_ydotool="$(ydotoold --version 2>&1 | head -n 1)"
fi
if [[ "$actual_ydotool" != "$supported_ydotool" ]]; then
  echo "unsupported ydotoold: '$actual_ydotool' (expected package version $supported_ydotool)" >&2
  echo "check with: pacman -Qi ydotool" >&2
  exit 1
fi
getent group remote-input >/dev/null || groupadd --system remote-input
remote_input_gid="$(getent group remote-input | cut -d: -f3)"
test -n "$remote_input_gid"
install -d -m 0755 /usr/local/libexec/remote-input-bridge /etc/remote-input-bridge
install -m 0755 "$receiver" /usr/local/libexec/remote-input-bridge/remote-input-receiver
install -m 0644 receiver.toml /etc/remote-input-bridge/receiver.toml
sed "s/__REMOTE_INPUT_GID__/$remote_input_gid/g" \
  remote-input-ydotoold.service > /etc/systemd/system/remote-input-ydotoold.service
chmod 0644 /etc/systemd/system/remote-input-ydotoold.service
systemctl daemon-reload
systemctl enable remote-input-ydotoold.service
systemctl restart remote-input-ydotoold.service
echo "Installed receiver and ydotoold unit. Add the SSH user to group 'remote-input',"
echo "install the restricted authorized_keys entry, then verify after reboot."

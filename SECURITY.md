# Security policy

Key Tunnel transports raw keyboard events and can control an unlocked desktop.
Treat its SSH key and the receiver's `ydotoold` socket as high-impact credentials.

## Supported version

Only the latest version on the `main` branch is supported while the project is
in its MVP stage.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Use a private
[GitHub security advisory](https://github.com/3ottA/key-tunnel/security/advisories/new)
and include reproduction steps, affected versions, and the expected impact.

## Deployment requirements

- Use a dedicated SSH key restricted with `restrict,command="..."`.
- Pin the server host key with `StrictHostKeyChecking=yes`.
- Keep the `ydotoold` socket at `0600`, or `0660` with a dedicated group.
- Run only on a trusted LAN or through a trusted VPN.
- Never expose the receiver as an unrestricted shell command.
- Never commit private keys, real hostnames, LAN addresses, usernames, or logs.

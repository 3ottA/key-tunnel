# Key Tunnel

Lightweight remote keyboard bridge from Windows 11 to Arch/Omarchy over one persistent SSH session. The Windows client captures physical key events, suppresses them locally while remote mode is active, and the Linux receiver injects validated events through a private `ydotoold` socket.

Key Tunnel is designed for a setup where the keyboard is connected to Windows while the display is connected to an Omarchy machine. A configurable global hotkey switches safely between local and remote input.

> [!WARNING]
> This software can inject keyboard input into an unlocked desktop. Read [SECURITY.md](SECURITY.md), use a dedicated restricted SSH key, and deploy it only on a trusted LAN or VPN.

## Estado del MVP

- Protocolo binario fijo, versionado, big-endian y con checksum; maneja lecturas parciales.
- `keydown`, `keyup`, repetición, E0/E1 y modificadores izquierdo/derecho.
- Toggle global y hotkey de emergencia registrados con `MOD_NOREPEAT`.
- Cola acotada de 4096 eventos; overflow o desconexión desactivan la supresión inmediatamente.
- SSH sin PTY, forwarding, compresión ni aceptación automática de host keys.
- Receptor con secuencias estrictas, límite de 2000 eventos/s y liberación ante EOF/error/señal.
- Estado persistido en `%LOCALAPPDATA%\RemoteInputBridge\status.json` sin contenido escrito.

## Compilar

Requiere Rust estable.

```powershell
cargo build --release -p remote-input-client
```

En Arch/Omarchy:

```bash
cargo build --release -p remote-input-receiver
```

El adaptador de `ydotoold` está fijado a **ydotool 1.0.4** y al ABI Linux x86_64 little-endian de 24 bytes. En Arch, el instalador valida la versión del paquete con `pacman` porque el binario no implementa `ydotoold --version`. El instalador rechaza otra versión y el receptor falla de forma segura en otra arquitectura. Hay que repetir la prueba de compatibilidad antes de cambiar ese pin.

## Configurar Windows

1. Copiar [`packaging/windows/config.example.toml`](packaging/windows/config.example.toml), ajustar host/usuario y usar una clave SSH exclusiva.
2. Aprovisionar previamente el host en `known_hosts`; no se usa `accept-new`.
3. Compilar y ejecutar como el usuario actual:

```powershell
.\packaging\windows\install.ps1 `
  -ClientBinary .\target\release\remote-input-client.exe `
  -StatusBinary .\target\release\remote-input-status.exe `
  -Config C:\ruta\config.toml
```

La tarea se registra `At log on` con token interactivo y reinicio ante fallo. No es un Windows Service. Para consultar diagnóstico:

```powershell
& "$env:LOCALAPPDATA\RemoteInputBridge\remote-input-status.exe"
```

## Configurar Omarchy

Instalar `ydotool`, compilar el receptor y ejecutar desde `packaging/systemd`:

```bash
sudo ./install.sh ../../target/release/remote-input-receiver
sudo usermod -aG remote-input USUARIO_SSH
```

Copiar la línea de [`authorized_keys.example`](packaging/systemd/authorized_keys.example) en `~/.ssh/authorized_keys`, reemplazar la clave y conservar `restrict,command=...`. La clave no debe compartir uso con una sesión shell.

Verificación mínima:

```bash
systemctl status remote-input-ydotoold
stat -c '%a %U %G' /run/remote-input-bridge/ydotool.sock
```

El socket esperado es `0660 root:remote-input`; el instalador convierte el GID del grupo a formato numérico porque el `ydotoold` de Arch no aplica correctamente el nombre del grupo. El directorio padre debe permitir travesía al usuario del puente aunque permanezca como `root:root`. Reiniciar ambos equipos antes de considerar terminado el despliegue.

### Instalación sin root en una sesión Omarchy existente

Si `ydotool.service` ya corre como unidad de usuario, el receptor puede instalarse en `~/.local/libexec/remote-input-bridge/` y usar [`receiver-user.toml`](packaging/systemd/receiver-user.toml). La entrada restringida debe apuntar a esas rutas absolutas. Para iniciarlo desde boot, incluso antes del primer login gráfico, habilitar y verificar lingering:

```bash
loginctl enable-linger "$USER"
loginctl show-user "$USER" -p Linger
systemctl --user enable --now ydotool.service
```

### Teclado Windows: Alt izquierdo como Super

Para mantener intacto el teclado conectado directamente a Omarchy y remapear sólo el teclado remoto, copiar el contenido de [`omarchy-remote-keyboard.lua`](packaging/systemd/omarchy-remote-keyboard.lua) al final de `~/.config/hypr/input.lua` y recargar Hyprland. La opción XKB `altwin:swap_lalt_lwin` convierte Alt izquierdo en Super y Windows izquierdo en Alt exclusivamente para `ydotoold-virtual-device`.

## Señales de estado

- `CONNECTING`: conexión en curso.
- `LOCAL`: SSH listo, teclado en Windows.
- `REMOTE`: forwarding y supresión activos.
- `ERROR`: fallo de SSH, protocolo o configuración; teclado local restaurado.

Los cambios emiten sonidos si `notify_on_toggle = true`. El hotkey de emergencia nunca se reenvía, desactiva el hook lógico, intenta `RELEASE_ALL` y cierra esa sesión SSH.

## Limitaciones conocidas

- Windows no permite capturar `Ctrl+Alt+Del`.
- El hook no cruza hacia aplicaciones con un nivel de integridad superior; ejecutar elevado queda fuera del MVP.
- El layout final lo decide Hyprland para `ydotoold virtual device`; el puente conserva posiciones físicas.
- El MVP no incluye mouse, portapapeles ni selección de múltiples servidores.

#[cfg(not(target_os = "linux"))]
compile_error!("remote-input-receiver only supports Linux");

use remote_input_protocol::{Frame, KeyState, MessageType, MAX_LINUX_KEY_CODE, VERSION};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, BufReader, BufWriter};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
const SUPPORTED_YDOTOOL_VERSION: &str = "1.0.4";

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    ydotool_socket: PathBuf,
    max_events_per_second: u32,
}

impl Default for Config {
    fn default() -> Self {
        let socket = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".ydotool_socket");
        Self {
            ydotool_socket: socket,
            max_events_per_second: 2_000,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("remote-input-receiver: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config()?;
    validate_platform()?;
    let socket = Arc::new(YdotoolSocket::connect(&config.ydotool_socket)?);
    let pressed = Arc::new(Mutex::new(BTreeSet::new()));
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = running.clone();
    let signal_socket = socket.clone();
    let signal_pressed = pressed.clone();
    ctrlc::set_handler(move || {
        signal_flag.store(false, Ordering::Release);
        if let Err(error) = release_all(&signal_socket, &signal_pressed) {
            eprintln!("release-all failed in signal handler: {error}");
        }
    })?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    let hello = Frame::read_from(&mut input).map_err(|e| format!("expected HELLO: {e}"))?;
    if hello.message_type != MessageType::Hello {
        return Err("first frame must be HELLO".into());
    }
    let mut ready = Frame::control(MessageType::Ready, hello.sequence, monotonic_micros());
    ready.data = u32::from(VERSION);
    ready.write_to(&mut output)?;

    let mut sequence = hello.sequence;
    let mut rate = RateLimit::new(config.max_events_per_second);
    let result = loop {
        if !running.load(Ordering::Acquire) {
            break Ok(());
        }
        let frame = match Frame::read_from(&mut input) {
            Ok(frame) => frame,
            Err(remote_input_protocol::ReadError::Io(e))
                if e.kind() == io::ErrorKind::UnexpectedEof =>
            {
                break Ok(())
            }
            Err(error) => break Err(format!("invalid frame: {error}").into()),
        };
        if !running.load(Ordering::Acquire) {
            break Ok(());
        }
        if frame.sequence <= sequence {
            break Err("non-increasing sequence".into());
        }
        sequence = frame.sequence;
        match frame.message_type {
            MessageType::KeyEvent => {
                if !rate.allow() {
                    break Err("event rate limit exceeded".into());
                }
                match frame.key_state {
                    KeyState::Down => apply_key(&socket, &pressed, frame.key_code, 1)?,
                    KeyState::Repeat => apply_key(&socket, &pressed, frame.key_code, 2)?,
                    KeyState::Up => apply_key(&socket, &pressed, frame.key_code, 0)?,
                    KeyState::None => unreachable!(),
                }
            }
            MessageType::ReleaseAll => release_all(&socket, &pressed)?,
            MessageType::Ping => {
                let mut pong =
                    Frame::control(MessageType::Pong, frame.sequence, monotonic_micros());
                pong.data = frame.data;
                pong.write_to(&mut output)?;
            }
            MessageType::Goodbye => break Ok(()),
            _ => break Err(format!("unexpected {:?} frame", frame.message_type).into()),
        }
    };
    if result.is_err() {
        let mut error = Frame::control(
            MessageType::Error,
            sequence.wrapping_add(1),
            monotonic_micros(),
        );
        error.data = 1;
        let _ = error.write_to(&mut output);
    }
    if let Err(error) = release_all(&socket, &pressed) {
        eprintln!("release-all failed during shutdown: {error}");
    }
    result
}

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let mut path = PathBuf::from("/etc/remote-input-bridge/receiver.toml");
    while let Some(arg) = args.next() {
        if arg == "--config" {
            path = PathBuf::from(args.next().ok_or("--config requires a path")?);
        } else if arg == "--version" {
            println!(
                "remote-input-receiver {} protocol {}",
                env!("CARGO_PKG_VERSION"),
                VERSION,
            );
            println!("ydotool socket ABI: {SUPPORTED_YDOTOOL_VERSION} (x86_64)");
            std::process::exit(0);
        } else {
            return Err(format!("unknown argument: {}", arg.to_string_lossy()).into());
        }
    }
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = fs::read_to_string(path)?;
    let cfg: Config = toml::from_str(&text)?;
    if cfg.max_events_per_second < 10 || cfg.max_events_per_second > 100_000 {
        return Err("max_events_per_second must be 10..100000".into());
    }
    Ok(cfg)
}

/// ydotool master currently accepts native Linux `struct input_event` datagrams.
/// Fail closed on an ABI that is not the pinned x86_64 layout.
fn validate_platform() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(not(all(target_arch = "x86_64", target_endian = "little"))) {
        return Err("ydotool adapter requires little-endian x86_64 Linux".into());
    }
    if std::mem::size_of::<libc::input_event>() != 24 {
        return Err("unexpected struct input_event ABI; expected 24 bytes".into());
    }
    Ok(())
}

struct YdotoolSocket(UnixDatagram);
impl YdotoolSocket {
    fn connect(path: &Path) -> io::Result<Self> {
        let socket = UnixDatagram::unbound()?;
        socket.connect(path)?;
        Ok(Self(socket))
    }
    fn key(&self, code: u16, value: i32) -> io::Result<()> {
        if code > MAX_LINUX_KEY_CODE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "key code out of range",
            ));
        }
        self.send_event(EV_KEY, code, value)?;
        self.send_event(EV_SYN, SYN_REPORT, 0)
    }
    fn send_event(&self, kind: u16, code: u16, value: i32) -> io::Result<()> {
        let event = libc::input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: kind,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const libc::input_event).cast::<u8>(),
                std::mem::size_of_val(&event),
            )
        };
        let sent = self.0.send(bytes)?;
        if sent != bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short ydotool datagram",
            ));
        }
        Ok(())
    }
}

fn apply_key(
    socket: &YdotoolSocket,
    pressed: &Mutex<BTreeSet<u16>>,
    key: u16,
    value: i32,
) -> io::Result<()> {
    let mut state = pressed
        .lock()
        .map_err(|_| io::Error::other("pressed-key state poisoned"))?;
    if value == 0 {
        state.remove(&key);
    } else if value == 1 {
        state.insert(key);
    }
    socket.key(key, value)
}

fn release_all(socket: &YdotoolSocket, pressed: &Mutex<BTreeSet<u16>>) -> io::Result<()> {
    let mut state = pressed
        .lock()
        .map_err(|_| io::Error::other("pressed-key state poisoned"))?;
    let keys: Vec<_> = state.iter().copied().collect();
    state.clear();
    for key in keys {
        socket.key(key, 0)?;
    }
    Ok(())
}

struct RateLimit {
    limit: u32,
    count: u32,
    started: Instant,
}
impl RateLimit {
    fn new(limit: u32) -> Self {
        Self {
            limit,
            count: 0,
            started: Instant::now(),
        }
    }
    fn allow(&mut self) -> bool {
        if self.started.elapsed() >= Duration::from_secs(1) {
            self.started = Instant::now();
            self.count = 0;
        }
        self.count += 1;
        self.count <= self.limit
    }
}

fn monotonic_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

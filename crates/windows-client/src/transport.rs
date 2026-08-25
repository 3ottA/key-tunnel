use crate::config::Config;
use crate::status::Status;
use remote_input_protocol::{Frame, KeyState, MessageType};
use std::io::{BufReader, BufWriter};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Copy, Debug)]
pub struct RawKey {
    pub code: u16,
    pub state: KeyState,
    pub flags: u16,
    pub timestamp_micros: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Outbound {
    Key(RawKey),
    ReleaseAll,
    Emergency,
    Shutdown,
}

pub fn run(
    config: Config,
    rx: Receiver<Outbound>,
    connected: &'static AtomicBool,
    active: &'static AtomicBool,
    status: std::sync::Arc<Status>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        status.update(|s| {
            s.state = "CONNECTING".into();
            s.connected = false;
            s.remote = false;
        });
        match connect(&config) {
            Ok(mut session) => {
                connected.store(true, Ordering::Release);
                status.update(|s| {
                    s.state = "LOCAL".into();
                    s.connected = true;
                    s.last_error = None;
                });
                backoff = Duration::from_secs(1);
                match drive(&rx, &mut session, active, &status) {
                    DriveResult::Shutdown => {
                        let _ = session.child.kill();
                        return;
                    }
                    DriveResult::Reconnect(error) => {
                        fail_open(connected, active, &status, Some(error));
                        let _ = session.child.kill();
                    }
                }
            }
            Err(error) => fail_open(connected, active, &status, Some(error)),
        }
        match rx.recv_timeout(backoff) {
            Ok(Outbound::Shutdown) => return,
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

struct Session {
    child: Child,
    writer: BufWriter<std::process::ChildStdin>,
    responses: Receiver<Result<Frame, String>>,
    next_sequence: u64,
}

fn connect(config: &Config) -> Result<Session, String> {
    let mut command = Command::new("ssh.exe");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "Compression=no",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ForwardAgent=no",
        "-o",
        "ForwardX11=no",
        "-o",
        &format!(
            "ServerAliveInterval={}",
            config.server_alive_interval_seconds
        ),
        "-o",
        &format!("ServerAliveCountMax={}", config.server_alive_count_max),
        "-o",
        &format!("UserKnownHostsFile={}", config.known_hosts.display()),
        "-i",
        &config.ssh_key.to_string_lossy(),
        &format!("{}@{}", config.user, config.server),
        &config.remote_command,
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start ssh.exe: {e}"))?;
    let stdin = child.stdin.take().ok_or("ssh stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("ssh stdout unavailable")?;
    let mut writer = BufWriter::new(stdin);
    let mut reader = BufReader::new(stdout);
    let hello = Frame::control(MessageType::Hello, 1, now_micros());
    hello
        .write_to(&mut writer)
        .map_err(|e| format!("HELLO write failed: {e}"))?;
    let ready = Frame::read_from(&mut reader).map_err(|e| format!("READY read failed: {e}"))?;
    if ready.message_type != MessageType::Ready
        || ready.sequence != 1
        || ready.data != u32::from(remote_input_protocol::VERSION)
    {
        return Err("invalid READY response".into());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || loop {
        match Frame::read_from(&mut reader) {
            Ok(frame) => {
                if tx.send(Ok(frame)).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = tx.send(Err(error.to_string()));
                break;
            }
        }
    });
    Ok(Session {
        child,
        writer,
        responses: rx,
        next_sequence: 2,
    })
}

enum DriveResult {
    Shutdown,
    Reconnect(String),
}

fn drive(
    rx: &Receiver<Outbound>,
    session: &mut Session,
    active: &AtomicBool,
    status: &Status,
) -> DriveResult {
    let mut last_ping = Instant::now();
    let mut pending_ping: Option<(u64, Instant)> = None;
    let mut was_active = false;
    loop {
        let is_active = active.load(Ordering::Acquire);
        if was_active && !is_active {
            let frame = next_control(session, MessageType::ReleaseAll);
            if let Err(e) = frame.write_to(&mut session.writer) {
                return DriveResult::Reconnect(format!("automatic release-all failed: {e}"));
            }
        }
        was_active = is_active;
        if let Ok(Some(exit)) = session.child.try_wait() {
            return DriveResult::Reconnect(format!("ssh exited with {exit}"));
        }
        while let Ok(response) = session.responses.try_recv() {
            match response {
                Ok(frame) if frame.message_type == MessageType::Pong => {
                    if let Some((sequence, sent)) = pending_ping.take() {
                        if sequence == frame.sequence {
                            status
                                .update(|s| s.latency_ms = Some(sent.elapsed().as_millis() as u64));
                        }
                    }
                }
                Ok(frame) if frame.message_type == MessageType::Error => {
                    return DriveResult::Reconnect(format!("receiver error {}", frame.data))
                }
                Ok(frame) => {
                    return DriveResult::Reconnect(format!(
                        "unexpected receiver frame: {:?}",
                        frame.message_type
                    ))
                }
                Err(error) => {
                    return DriveResult::Reconnect(format!("receiver stream failed: {error}"))
                }
            }
        }
        if last_ping.elapsed() >= Duration::from_secs(5) {
            let frame = next_control(session, MessageType::Ping);
            pending_ping = Some((frame.sequence, Instant::now()));
            if let Err(e) = frame.write_to(&mut session.writer) {
                return DriveResult::Reconnect(format!("PING failed: {e}"));
            }
            last_ping = Instant::now();
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Outbound::Key(key)) => {
                if !active.load(Ordering::Acquire) {
                    continue;
                }
                let sequence = take_sequence(session);
                let frame = Frame::key(
                    sequence,
                    key.code,
                    key.state,
                    key.flags,
                    key.timestamp_micros,
                );
                if let Err(e) = frame.write_to(&mut session.writer) {
                    return DriveResult::Reconnect(format!("key write failed: {e}"));
                }
            }
            Ok(Outbound::ReleaseAll) => {
                let frame = next_control(session, MessageType::ReleaseAll);
                if let Err(e) = frame.write_to(&mut session.writer) {
                    return DriveResult::Reconnect(format!("release-all failed: {e}"));
                }
            }
            Ok(Outbound::Emergency) => {
                let frame = next_control(session, MessageType::ReleaseAll);
                let _ = frame.write_to(&mut session.writer);
                let frame = next_control(session, MessageType::Goodbye);
                let _ = frame.write_to(&mut session.writer);
                return DriveResult::Reconnect("emergency disconnect requested".into());
            }
            Ok(Outbound::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                active.store(false, Ordering::Release);
                let frame = next_control(session, MessageType::ReleaseAll);
                let _ = frame.write_to(&mut session.writer);
                let frame = next_control(session, MessageType::Goodbye);
                let _ = frame.write_to(&mut session.writer);
                return DriveResult::Shutdown;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn fail_open(connected: &AtomicBool, active: &AtomicBool, status: &Status, error: Option<String>) {
    active.store(false, Ordering::Release);
    connected.store(false, Ordering::Release);
    status.update(|s| {
        s.state = "ERROR".into();
        s.connected = false;
        s.remote = false;
        s.last_error = error;
    });
}

fn take_sequence(session: &mut Session) -> u64 {
    let value = session.next_sequence;
    session.next_sequence = session.next_sequence.wrapping_add(1);
    value
}
fn next_control(session: &mut Session, kind: MessageType) -> Frame {
    Frame::control(kind, take_sequence(session), now_micros())
}
fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

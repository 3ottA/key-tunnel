use remote_input_protocol::{Frame, KeyState, MessageType, VERSION};
use std::env;
use std::io::{BufReader, BufWriter};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    if let Err(error) = run() {
        eprintln!("remote-input-probe: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut ssh_config = None;
    let mut identity = None;
    let mut host = None;
    let mut inject_test_key = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--ssh-config" => ssh_config = Some(args.next().ok_or("missing --ssh-config value")?),
            "--identity" => identity = Some(args.next().ok_or("missing --identity value")?),
            "--host" => host = Some(args.next().ok_or("missing --host value")?),
            "--inject-test-key" => inject_test_key = true,
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let host = host.ok_or("--host is required")?;
    let mut command = Command::new("ssh");
    command.args(["-T", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes"]);
    if let Some(config) = ssh_config {
        command.args(["-F", &config]);
    }
    if let Some(key) = identity {
        command.args(["-i", &key]);
    }
    command.arg(host).arg("remote-input-receiver");
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = BufWriter::new(child.stdin.take().ok_or("ssh stdin unavailable")?);
    let mut output = BufReader::new(child.stdout.take().ok_or("ssh stdout unavailable")?);
    Frame::control(MessageType::Hello, 1, now_micros()).write_to(&mut input)?;
    let ready = Frame::read_from(&mut output)?;
    if ready.message_type != MessageType::Ready
        || ready.sequence != 1
        || ready.data != u32::from(VERSION)
    {
        return Err(format!("invalid READY: {ready:?}").into());
    }
    let mut sequence = 2;
    if inject_test_key {
        // Left Shift down/up is observable by ydotoold while avoiding text or lock-state changes.
        Frame::key(sequence, 42, KeyState::Down, 0, now_micros()).write_to(&mut input)?;
        sequence += 1;
        Frame::key(sequence, 42, KeyState::Up, 0, now_micros()).write_to(&mut input)?;
        sequence += 1;
    }
    Frame::control(MessageType::ReleaseAll, sequence, now_micros()).write_to(&mut input)?;
    sequence += 1;
    Frame::control(MessageType::Goodbye, sequence, now_micros()).write_to(&mut input)?;
    drop(input);
    let exit = child.wait()?;
    if !exit.success() {
        return Err(format!("ssh/receiver exited with {exit}").into());
    }
    println!("READY protocol={VERSION} injection={inject_test_key} exit={exit}");
    Ok(())
}

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

use crate::config;
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub version: String,
    pub protocol_version: u8,
    pub state: String,
    pub connected: bool,
    pub remote: bool,
    pub latency_ms: Option<u64>,
    pub dropped_events: u64,
    pub last_error: Option<String>,
    pub updated_unix_ms: u128,
}

pub struct Status(Mutex<Snapshot>);
impl Status {
    pub fn new() -> Self {
        Self(Mutex::new(Snapshot {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: remote_input_protocol::VERSION,
            state: "CONNECTING".into(),
            connected: false,
            remote: false,
            latency_ms: None,
            dropped_events: 0,
            last_error: None,
            updated_unix_ms: 0,
        }))
    }
    pub fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut value) = self.0.lock() {
            f(&mut value);
            value.updated_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let path = config::status_path();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_vec_pretty(&*value) {
                let _ = fs::write(path, json);
            }
        }
    }
}

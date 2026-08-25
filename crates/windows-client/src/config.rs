use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: String,
    pub user: String,
    pub ssh_key: PathBuf,
    pub known_hosts: PathBuf,
    pub toggle_hotkey: String,
    pub emergency_hotkey: String,
    #[serde(default = "default_alive_interval")]
    pub server_alive_interval_seconds: u32,
    #[serde(default = "default_alive_count")]
    pub server_alive_count_max: u32,
    #[serde(default = "default_true")]
    pub notify_on_toggle: bool,
    #[serde(default = "default_remote_command")]
    pub remote_command: String,
}

fn default_alive_interval() -> u32 {
    15
}
fn default_alive_count() -> u32 {
    3
}
fn default_true() -> bool {
    true
}
fn default_remote_command() -> String {
    "remote-input-receiver".into()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let config: Self = toml::from_str(&fs::read_to_string(path)?)?;
        if config.server.trim().is_empty() || config.user.trim().is_empty() {
            return Err("server and user are required".into());
        }
        if !config.ssh_key.is_file() {
            return Err(format!("SSH key does not exist: {}", config.ssh_key.display()).into());
        }
        if !config.known_hosts.is_file() {
            return Err(format!(
                "known_hosts does not exist: {}",
                config.known_hosts.display()
            )
            .into());
        }
        if !(1..=300).contains(&config.server_alive_interval_seconds) {
            return Err("server_alive_interval_seconds must be 1..300".into());
        }
        if !(1..=20).contains(&config.server_alive_count_max) {
            return Err("server_alive_count_max must be 1..20".into());
        }
        Ok(config)
    }
}

pub fn data_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("RemoteInputBridge")
}

pub fn default_config_path() -> PathBuf {
    data_dir().join("config.toml")
}
pub fn status_path() -> PathBuf {
    data_dir().join("status.json")
}

use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let path = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("RemoteInputBridge/status.json");
    match fs::read_to_string(&path)
        .and_then(|s| serde_json::from_str::<Value>(&s).map_err(std::io::Error::other))
    {
        Ok(status) => println!("{}", serde_json::to_string_pretty(&status).unwrap()),
        Err(error) => {
            eprintln!("No status available at {}: {error}", path.display());
            std::process::exit(1);
        }
    }
}

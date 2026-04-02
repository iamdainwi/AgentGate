use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGateConfig {
    pub log_level: String,
    pub log_format: LogFormat,
    /// Path to the SQLite database. Defaults to `~/.agentgate/logs.db`.
    pub db_path: PathBuf,
    /// Name used to identify the wrapped server in invocation records.
    pub server_name: String,
    /// Optional path to a TOML policy file. No policy enforcement when absent.
    pub policy_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Default for AgentGateConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            log_format: LogFormat::Pretty,
            db_path: default_db_path(),
            server_name: "unknown".to_string(),
            policy_path: None,
        }
    }
}

fn default_db_path() -> PathBuf {
    dirs_path().join("logs.db")
}

fn dirs_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".agentgate")
}

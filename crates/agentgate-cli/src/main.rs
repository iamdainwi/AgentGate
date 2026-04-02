use agentgate_core::config::{agentgate_dir, AgentGateConfig};
use agentgate_core::proxy::stdio::StdioProxy;
use agentgate_core::storage::{InvocationFilter, StorageReader};
use anyhow::Result;
use clap::{Parser, Subcommand};
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(name = "agentgate", about = "AI Agent Security & Observability Gateway")]
struct Cli {
    /// Path to a config TOML file [default: ~/.agentgate/config.toml]
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Override the database path
    #[arg(long, global = true)]
    db: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Wrap an MCP server process, proxying and logging all tool calls
    Wrap {
        /// Path to a TOML policy file to enforce
        #[arg(long)]
        policy: Option<std::path::PathBuf>,

        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Query and display logged tool invocations
    Logs {
        /// Filter by tool name
        #[arg(long)]
        tool: Option<String>,
        /// Filter by status (allowed, denied, error, rate_limited)
        #[arg(long)]
        status: Option<String>,
        /// Number of records to display
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Output as newline-delimited JSON
        #[arg(long)]
        jsonl: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let mut config = load_config(cli.config.as_deref());
    if let Some(db) = cli.db {
        config.db_path = db;
    }

    match cli.command {
        Commands::Wrap { policy, command } => {
            if command.is_empty() {
                eprintln!("error: no command specified. Usage: agentgate wrap -- <cmd> [args...]");
                std::process::exit(1);
            }

            let (cmd, args) = command.split_first().expect("non-empty checked above");
            config.server_name = std::path::Path::new(cmd)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(cmd)
                .to_string();

            if policy.is_some() {
                config.policy_path = policy;
            }

            StdioProxy::new(config).run(cmd, args).await?;
        }

        Commands::Logs { tool, status, limit, jsonl } => {
            let reader = StorageReader::open(&config.db_path)?;
            let filter = InvocationFilter { tool, status, limit };

            if jsonl {
                reader.export_jsonl(&filter, &mut std::io::stdout())?;
            } else {
                let records = reader.query(&filter)?;
                if records.is_empty() {
                    println!("No invocations found.");
                    return Ok(());
                }
                print_table(&records);
            }
        }
    }

    Ok(())
}

fn load_config(explicit: Option<&std::path::Path>) -> AgentGateConfig {
    let path = explicit
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| agentgate_dir().join("config.toml"));

    if path.exists() {
        match AgentGateConfig::load_toml(&path) {
            Ok(c) => {
                tracing::debug!("Loaded config from {}", path.display());
                return c;
            }
            Err(e) => {
                tracing::warn!("Failed to load config from {}: {e}", path.display());
            }
        }
    }

    AgentGateConfig::default()
}

#[derive(Tabled)]
struct InvocationRow {
    #[tabled(rename = "Timestamp")]
    timestamp: String,
    #[tabled(rename = "Server")]
    server_name: String,
    #[tabled(rename = "Tool")]
    tool_name: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Latency (ms)")]
    latency_ms: String,
    #[tabled(rename = "Policy Hit")]
    policy_hit: String,
}

fn print_table(records: &[agentgate_core::storage::InvocationRecord]) {
    let rows: Vec<InvocationRow> = records
        .iter()
        .map(|r| InvocationRow {
            timestamp: r.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            server_name: r.server_name.clone(),
            tool_name: r.tool_name.clone(),
            status: r.status.as_str().to_string(),
            latency_ms: r
                .latency_ms
                .map(|l| l.to_string())
                .unwrap_or_else(|| "-".to_string()),
            policy_hit: r.policy_hit.clone().unwrap_or_else(|| "-".to_string()),
        })
        .collect();

    println!("{}", Table::new(rows));
}

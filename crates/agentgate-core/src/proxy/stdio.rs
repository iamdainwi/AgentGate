use crate::config::AgentGateConfig;
use crate::logging::structured::{log_event, Direction, LogEvent};
use crate::protocol::jsonrpc::{JsonRpcMessage, JsonRpcRequest};
use crate::protocol::mcp;
use crate::storage::{InvocationRecord, InvocationStatus, StorageWriter};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

/// Metadata captured at the moment a `tools/call` request is seen.
struct PendingCall {
    tool_name: String,
    arguments: Option<Value>,
    started_at: Instant,
}

type PendingMap = Arc<Mutex<HashMap<String, PendingCall>>>;

pub struct StdioProxy {
    config: AgentGateConfig,
}

impl StdioProxy {
    pub fn new(config: AgentGateConfig) -> Self {
        Self { config }
    }

    pub async fn run(&self, command: &str, args: &[String]) -> Result<()> {
        tracing::info!("Starting stdio proxy for: {} {:?}", command, args);

        let storage = StorageWriter::spawn(self.config.db_path.clone())?;
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("Failed to spawn: {command}"))?;

        let child_stdin = child.stdin.take().expect("stdin piped");
        let child_stdout = child.stdout.take().expect("stdout piped");
        let child_stderr = child.stderr.take().expect("stderr piped");

        let task_a = tokio::spawn(proxy_inbound(child_stdin, Arc::clone(&pending)));
        let task_b = tokio::spawn(proxy_response(
            child_stdout,
            Arc::clone(&pending),
            storage,
            self.config.server_name.clone(),
        ));
        let task_c = tokio::spawn(pipe_stderr(child_stderr));

        let status = child.wait().await.context("Failed to wait for child")?;

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task_a).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task_b).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task_c).await;

        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }

        Ok(())
    }
}

async fn proxy_inbound(
    mut child_stdin: tokio::process::ChildStdin,
    pending: PendingMap,
) -> Result<()> {
    let mut reader = BufReader::new(tokio::io::stdin()).lines();

    while let Some(line) = reader.next_line().await? {
        if line.is_empty() {
            continue;
        }

        match JsonRpcMessage::parse(&line) {
            Ok(msg) => {
                log_event(&LogEvent {
                    timestamp: Utc::now(),
                    direction: Direction::Inbound,
                    message: msg.clone(),
                    raw: line.clone(),
                });
                track_outgoing_call(&msg, &pending);
            }
            Err(e) => {
                tracing::warn!("Inbound parse error: {e}");
            }
        }

        child_stdin.write_all(line.as_bytes()).await?;
        child_stdin.write_all(b"\n").await?;
        child_stdin.flush().await?;
    }

    Ok(())
}

async fn proxy_response(
    child_stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    storage: StorageWriter,
    server_name: String,
) -> Result<()> {
    let mut reader = BufReader::new(child_stdout).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        if line.is_empty() {
            continue;
        }

        match JsonRpcMessage::parse(&line) {
            Ok(msg) => {
                log_event(&LogEvent {
                    timestamp: Utc::now(),
                    direction: Direction::Response,
                    message: msg.clone(),
                    raw: line.clone(),
                });
                flush_pending_call(&msg, &pending, &storage, &server_name);
            }
            Err(e) => {
                tracing::warn!("Response parse error: {e}");
            }
        }

        stdout.write_all(line.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn pipe_stderr(child_stderr: tokio::process::ChildStderr) -> Result<()> {
    let mut reader = BufReader::new(child_stderr).lines();
    let mut stderr = tokio::io::stderr();
    while let Some(line) = reader.next_line().await? {
        stderr.write_all(line.as_bytes()).await?;
        stderr.write_all(b"\n").await?;
        stderr.flush().await?;
    }
    Ok(())
}

/// Record a `tools/call` request in the pending map so its response can be correlated.
fn track_outgoing_call(msg: &JsonRpcMessage, pending: &PendingMap) {
    let JsonRpcMessage::Request(req) = msg else {
        return;
    };
    if req.method != mcp::TOOLS_CALL {
        return;
    }
    let id = id_key(req);
    let (tool_name, arguments) = extract_tool_call_params(req);

    pending.lock().unwrap().insert(
        id,
        PendingCall {
            tool_name,
            arguments,
            started_at: Instant::now(),
        },
    );
}

/// Match a response to its pending call, build an InvocationRecord, and enqueue it.
fn flush_pending_call(
    msg: &JsonRpcMessage,
    pending: &PendingMap,
    storage: &StorageWriter,
    server_name: &str,
) {
    let JsonRpcMessage::Response(resp) = msg else {
        return;
    };

    let key = resp.id.to_string();
    let Some(call) = pending.lock().unwrap().remove(&key) else {
        return;
    };

    let latency_ms = call.started_at.elapsed().as_millis() as i64;
    let status = if resp.error.is_some() {
        InvocationStatus::Error
    } else {
        InvocationStatus::Allowed
    };

    let record = InvocationRecord {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        agent_id: None,
        session_id: None,
        server_name: server_name.to_string(),
        tool_name: call.tool_name,
        arguments: call.arguments,
        result: resp.result.clone(),
        latency_ms: Some(latency_ms),
        status,
        policy_hit: None,
    };

    storage.record(record);
}

fn id_key(req: &JsonRpcRequest) -> String {
    req.id
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn extract_tool_call_params(req: &JsonRpcRequest) -> (String, Option<Value>) {
    let Some(params) = &req.params else {
        return ("unknown".to_string(), None);
    };

    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let arguments = params.get("arguments").cloned();

    (tool_name, arguments)
}

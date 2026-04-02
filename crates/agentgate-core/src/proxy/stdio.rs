use crate::config::AgentGateConfig;
use crate::logging::structured::{log_event, Direction, LogEvent};
use crate::policy::{PolicyDecision, PolicyEngine};
use crate::protocol::jsonrpc::{JsonRpcError, JsonRpcMessage, JsonRpcRequest, JsonRpcResponse};
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
use tokio::sync::mpsc::{self, UnboundedSender};
use uuid::Uuid;

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

        let engine = self
            .config
            .policy_path
            .as_deref()
            .map(|p| {
                let e = PolicyEngine::load(p)?;
                PolicyEngine::spawn_watcher(Arc::clone(&e), p.to_path_buf());
                Ok::<_, anyhow::Error>(e)
            })
            .transpose()?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let (stdout_tx, mut stdout_rx) = mpsc::unbounded_channel::<String>();

        let stdout_writer = tokio::spawn(async move {
            let mut out = tokio::io::stdout();
            while let Some(line) = stdout_rx.recv().await {
                out.write_all(line.as_bytes()).await?;
                out.write_all(b"\n").await?;
                out.flush().await?;
            }
            Ok::<_, anyhow::Error>(())
        });

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

        let task_a = tokio::spawn(proxy_inbound(
            child_stdin,
            Arc::clone(&pending),
            engine,
            storage.clone(),
            self.config.server_name.clone(),
            stdout_tx.clone(),
        ));

        let task_b = tokio::spawn(proxy_response(
            child_stdout,
            Arc::clone(&pending),
            storage,
            self.config.server_name.clone(),
            stdout_tx,
        ));

        let task_c = tokio::spawn(pipe_stderr(child_stderr));

        let status = child.wait().await.context("Failed to wait for child")?;

        let timeout = std::time::Duration::from_secs(2);
        let _ = tokio::time::timeout(timeout, task_a).await;
        let _ = tokio::time::timeout(timeout, task_b).await;
        let _ = tokio::time::timeout(timeout, task_c).await;
        let _ = tokio::time::timeout(timeout, stdout_writer).await;

        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }

        Ok(())
    }
}

async fn proxy_inbound(
    mut child_stdin: tokio::process::ChildStdin,
    pending: PendingMap,
    engine: Option<Arc<PolicyEngine>>,
    storage: StorageWriter,
    server_name: String,
    stdout_tx: UnboundedSender<String>,
) -> Result<()> {
    let mut reader = BufReader::new(tokio::io::stdin()).lines();

    while let Some(line) = reader.next_line().await? {
        if line.is_empty() {
            continue;
        }

        let msg = match JsonRpcMessage::parse(&line) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Inbound parse error: {e}");
                child_stdin.write_all(line.as_bytes()).await?;
                child_stdin.write_all(b"\n").await?;
                child_stdin.flush().await?;
                continue;
            }
        };

        log_event(&LogEvent {
            timestamp: Utc::now(),
            direction: Direction::Inbound,
            message: msg.clone(),
            raw: line.clone(),
        });

        // Only evaluate tools/call requests through the policy engine.
        if let JsonRpcMessage::Request(ref req) = msg {
            if req.method == mcp::TOOLS_CALL {
                let (tool_name, arguments) = extract_tool_call_params(req);

                if let Some(ref engine) = engine {
                    let decision = engine.evaluate(&tool_name, arguments.as_ref());
                    match decision {
                        PolicyDecision::Deny { rule_id, message } => {
                            let err_resp =
                                build_error_response(&req.id, -32603, &message);
                            let serialized = serde_json::to_string(&err_resp)?;
                            stdout_tx.send(serialized)?;
                            storage.record(denied_record(
                                &tool_name,
                                arguments,
                                &server_name,
                                &rule_id,
                            ));
                            continue;
                        }
                        PolicyDecision::RateLimited { rule_id } => {
                            let msg_str = format!(
                                "Rate limit exceeded for rule '{rule_id}'"
                            );
                            let err_resp =
                                build_error_response(&req.id, -32029, &msg_str);
                            let serialized = serde_json::to_string(&err_resp)?;
                            stdout_tx.send(serialized)?;
                            storage.record(rate_limited_record(
                                &tool_name,
                                arguments,
                                &server_name,
                                &rule_id,
                            ));
                            continue;
                        }
                        PolicyDecision::Redact { rule_id, arguments: redacted } => {
                            let forwarded = rebuild_tools_call(req, redacted.clone());
                            let serialized = serde_json::to_string(&forwarded)?;
                            pending.lock().unwrap().insert(
                                id_key(req),
                                PendingCall {
                                    tool_name: tool_name.clone(),
                                    arguments: Some(redacted),
                                    started_at: Instant::now(),
                                },
                            );
                            tracing::info!(
                                rule_id = %rule_id,
                                tool = %tool_name,
                                "Arguments redacted"
                            );
                            child_stdin.write_all(serialized.as_bytes()).await?;
                            child_stdin.write_all(b"\n").await?;
                            child_stdin.flush().await?;
                            continue;
                        }
                        PolicyDecision::Allow => {}
                    }
                }

                pending.lock().unwrap().insert(
                    id_key(req),
                    PendingCall {
                        tool_name,
                        arguments,
                        started_at: Instant::now(),
                    },
                );
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
    stdout_tx: UnboundedSender<String>,
) -> Result<()> {
    let mut reader = BufReader::new(child_stdout).lines();

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
                stdout_tx.send(line)?;
            }
            Err(e) => {
                tracing::warn!("Response parse error: {e}");
                stdout_tx.send(line)?;
            }
        }
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

    storage.record(InvocationRecord {
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
    });
}

fn build_error_response(id: &Option<Value>, code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: id.clone().unwrap_or(Value::Null),
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        }),
    }
}

fn rebuild_tools_call(original: &JsonRpcRequest, new_arguments: Value) -> JsonRpcRequest {
    let mut params = original.params.clone().unwrap_or(Value::Object(Default::default()));
    if let Value::Object(ref mut map) = params {
        map.insert("arguments".to_string(), new_arguments);
    }
    JsonRpcRequest {
        jsonrpc: original.jsonrpc.clone(),
        id: original.id.clone(),
        method: original.method.clone(),
        params: Some(params),
    }
}

fn denied_record(
    tool_name: &str,
    arguments: Option<Value>,
    server_name: &str,
    rule_id: &str,
) -> InvocationRecord {
    InvocationRecord {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        agent_id: None,
        session_id: None,
        server_name: server_name.to_string(),
        tool_name: tool_name.to_string(),
        arguments,
        result: None,
        latency_ms: None,
        status: InvocationStatus::Denied,
        policy_hit: Some(rule_id.to_string()),
    }
}

fn rate_limited_record(
    tool_name: &str,
    arguments: Option<Value>,
    server_name: &str,
    rule_id: &str,
) -> InvocationRecord {
    InvocationRecord {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        agent_id: None,
        session_id: None,
        server_name: server_name.to_string(),
        tool_name: tool_name.to_string(),
        arguments,
        result: None,
        latency_ms: None,
        status: InvocationStatus::RateLimited,
        policy_hit: Some(rule_id.to_string()),
    }
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

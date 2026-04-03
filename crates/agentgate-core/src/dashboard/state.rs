use crate::policy::PolicyEngine;
use crate::storage::InvocationRecord;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Shared state injected into every dashboard API handler via axum's `State` extractor.
#[derive(Clone)]
pub struct DashboardState {
    pub db_path: PathBuf,
    /// Path of the active policy TOML file, if one was loaded.
    pub policy_path: Option<PathBuf>,
    /// Live policy engine — used to trigger hot-reload after a PUT /api/policies.
    pub policy_engine: Option<Arc<PolicyEngine>>,
    /// Sender half of the live invocation broadcast — handlers subscribe per WebSocket connection.
    pub live_tx: broadcast::Sender<InvocationRecord>,
    /// Bearer token required for all authenticated API routes and WebSocket connections.
    pub auth_token: String,
}

/// Resolve the dashboard API key:
/// - If `configured` is `Some(s)` and `s` is non-empty, use it as-is.
/// - Otherwise auto-generate a 32-character hex token (128-bit, CSPRNG via UUID v4),
///   print it to stderr, and return it.
///
/// Call this once at startup and store the result in `DashboardState`.
pub fn resolve_auth_token(configured: Option<&str>) -> String {
    if let Some(key) = configured.filter(|s| !s.trim().is_empty()) {
        eprintln!("[agentgate] Dashboard auth: using key from config.");
        return key.to_string();
    }

    // 16 random bytes from UUID v4 → 32 lowercase hex characters.
    // Formatting the raw bytes avoids the version/variant field overwrite that
    // UUID's hyphenated string form imposes, giving the full 128 bits of randomness.
    let raw = uuid::Uuid::new_v4();
    let token: String = raw
        .as_bytes()
        .iter()
        .fold(String::with_capacity(32), |mut s, b| {
            use std::fmt::Write;
            write!(s, "{b:02x}").ok();
            s
        });

    eprintln!(
        "[agentgate] Dashboard token: {token}\n\
         [agentgate] Open http://127.0.0.1:7070 and enter this token, or pass it as:\n\
         [agentgate]   curl -H 'Authorization: Bearer {token}' http://127.0.0.1:7070/api/invocations"
    );
    token
}

/// Compatibility shim used where the token is always auto-generated (no config key available).
#[inline]
pub fn generate_and_print_token() -> String {
    resolve_auth_token(None)
}

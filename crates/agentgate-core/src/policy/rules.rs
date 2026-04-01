use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFile {
    pub metadata: PolicyMetadata,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMetadata {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    /// Tool name to match, or `"*"` for all tools.
    pub tool: String,
    /// Condition expression string. Absent means unconditional match.
    #[serde(default)]
    pub condition: Option<String>,
    pub action: RuleAction,
    #[serde(default)]
    pub message: Option<String>,
    // redact-specific
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub replacement: Option<String>,
    // rate_limit-specific
    #[serde(default)]
    pub max_calls: Option<u64>,
    #[serde(default)]
    pub window_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Deny,
    Redact,
    RateLimit,
}

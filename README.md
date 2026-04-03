# AgentGate

**Nginx/Kong for AI agents — a transparent security & observability gateway for MCP tool calls.**

AgentGate sits between AI coding agents (Claude Code, Cursor, Codex, Devin, Ollama, llama.cpp, or any agentic framework) and the MCP servers they call. It intercepts every `tools/call` invocation, enforces declarative TOML policies, rate-limits, redacts secrets, and logs everything to a local SQLite database — with zero behavior change to the agent or server, and sub-millisecond overhead on the proxy hot path.

```
┌──────────┐      ┌─────────────────────────────────────┐      ┌────────────┐
│ AI Agent │ ───▶ │             AgentGate                │ ───▶ │ MCP Server │
│          │ ◀─── │  policy · rate-limit · redact · audit│ ◀─── │            │
└──────────┘      └─────────────────────────────────────┘      └────────────┘
                                    │
                   ┌────────────────┼────────────────┐
                   ▼                ▼                ▼
              ┌────────┐      ┌─────────┐      ┌──────────┐
              │SQLite  │      │Metrics  │      │Dashboard │
              │logs.db │      │:9090    │      │:7070     │
              └────────┘      └─────────┘      └──────────┘
```

## Why

AI agents run tool calls autonomously — reading files, executing shell commands, making API requests. Without a gateway layer there is no way to answer:

- **"What did the agent actually do?"** — no audit trail across tool calls
- **"Can I restrict what it's allowed to do?"** — no policy layer between agents and tools
- **"Why did my API bill spike?"** — no rate limiting for agent-initiated calls
- **"Is a secret leaking through tool output?"** — no redaction before the agent sees results

AgentGate solves all four with a single `agentgate wrap --` prefix.

## Features

- **Transparent proxying** — stdio, SSE, and HTTP transports; agents and servers require no changes
- **Declarative TOML policies** — allow, deny, redact, and rate-limit rules with hot-reload
- **Condition expressions** — match on `arguments.field matches 'regex'`, `time.hour < 9`, boolean logic (`and`, `or`, `not`)
- **Secret redaction** — strip API keys, tokens, and PII from tool results at the gateway boundary
- **Rate limiting** — global, per-tool, and per-rule token bucket rate limiting
- **Circuit breaker** — automatic trip on repeated errors (Closed → Open → HalfOpen → Closed)
- **SQLite audit log** — every invocation persisted with latency, status, and policy hit
- **Log retention** — configurable max age and max rows with automatic pruning
- **Prometheus metrics** — 6 metrics covering calls, latency, denials, rate limits, circuit state, and sessions
- **Real-time dashboard** — Next.js 15 UI with live WebSocket feed, analytics, and in-browser policy editor
- **Single binary** — dashboard static assets embedded via `rust-embed`; no external dependencies

## Quick Start

### Install

**Cargo** (from source):

```bash
cargo install agentgate
```

**Shell installer** (Linux / macOS):

```bash
curl -fsSL https://raw.githubusercontent.com/iamdainwi/AgentGate/main/install.sh | sh
```

**Homebrew**:

```bash
brew install agentgate
```

**Docker**:

```bash
docker run --rm -v ~/.agentgate:/data ghcr.io/iamdainwi/agentgate wrap -- <mcp-server>
```

### Initialize config

```bash
agentgate init
```

This scaffolds `~/.agentgate/config.toml` and `~/.agentgate/policies/default.toml`.

### Wrap any MCP server

```bash
agentgate wrap -- npx @modelcontextprotocol/server-filesystem /tmp
```

Every `tools/call` is now logged to `~/.agentgate/logs.db`. The agent and server see no change.

### Add a policy

```bash
agentgate wrap --policy policies/default.toml -- npx @modelcontextprotocol/server-filesystem /tmp
```

### Dashboard API Key

The built-in dashboard (port **7070** by default) is protected by an API key. On startup, AgentGate prints the key to your terminal:

```
[agentgate] Dashboard token: a1b2c3d4e5f6...
[agentgate] Open http://127.0.0.1:7070 and enter this token, or pass it as:
[agentgate]   curl -H 'Authorization: Bearer a1b2c3d4...' http://127.0.0.1:7070/api/invocations
```

**Look for the `[agentgate] Dashboard token:` line in your terminal output** — copy that value and paste it into the dashboard login prompt, or pass it as a `Bearer` token in API requests.

To use a fixed key instead of a random one, set it in `~/.agentgate/config.toml`:

```toml
dashboard_api_key = "your-secret-key-here"
```

### Query logs

```bash
agentgate logs                        # last 50 invocations
agentgate logs --tool read_file       # filter by tool
agentgate logs --status denied        # filter by outcome
agentgate logs --limit 200 --jsonl    # JSONL export
agentgate logs --db /path/to/logs.db  # custom database
```

### Example output

```
+---------------------+--------+-----------+-------------+-------------+------------+
| Timestamp           | Server | Tool      | Status      | Latency (ms)| Policy Hit |
+---------------------+--------+-----------+-------------+-------------+------------+
| 2026-04-03 09:01:12 | fs     | read_file | allowed     | 9           | -          |
| 2026-04-03 09:01:14 | fs     | bash      | denied      | 1           | no-shell   |
| 2026-04-03 09:01:17 | fs     | read_file | rate_limited| 0           | -          |
+---------------------+--------+-----------+-------------+-------------+------------+
```

### Check installation

```bash
agentgate doctor
```

Validates config directory, config.toml parsing, database writability, dashboard port availability, and policy file validity.

## CLI Commands

| Command   | Description                                              |
| --------- | -------------------------------------------------------- |
| `wrap`    | Wrap a stdio MCP server, proxying and logging all calls  |
| `serve`   | Start an SSE or HTTP transport proxy                     |
| `logs`    | Query and display logged tool invocations                |
| `init`    | Scaffold default config and policy in `~/.agentgate/`    |
| `doctor`  | Check installation for common problems                   |

## Policy Engine

Create a TOML policy file and pass it with `--policy`. Rules are evaluated top-to-bottom; first match wins. The policy file is **hot-reloaded on change** — no restart needed.

### Actions

| Action       | Behavior                                                            |
| ------------ | ------------------------------------------------------------------- |
| `allow`      | Forward the call to the MCP server                                  |
| `deny`       | Block with a JSON-RPC error (code `-32603`) and a custom message    |
| `redact`     | Apply regex substitution to arguments before forwarding             |
| `rate_limit` | Token bucket rate limiting per rule (`max_calls` / `window_seconds`)|

### Conditions

The policy engine supports an expression language for fine-grained matching:

```
arguments.command matches 'rm.*-rf'        # field-level regex
arguments contains_pattern 'sk-[a-zA-Z0-9]+'  # search entire arguments JSON
time.hour < 9                              # time-based (UTC)
time.hour > 18 or time.hour < 9           # boolean OR
!arguments.command matches 'safe-pattern'  # negation
```

Operators: `matches`, `contains_pattern`, `<`, `>`, `<=`, `>=`, `==`, `and`, `or`, `not` / `!`

### Example policy

```toml
# policies/default.toml
[metadata]
name = "default"
version = "1.0"

[[rules]]
id        = "no-rm-rf"
tool      = "bash"
condition = "arguments.command matches '(rm\\s+-rf|rm\\s+-fr)'"
action    = "deny"
message   = "Recursive force-delete is not permitted"

[[rules]]
id        = "no-destructive-sql"
tool      = "bash"
condition = "arguments.command matches '(DROP\\s+TABLE|DROP\\s+DATABASE|TRUNCATE\\s+TABLE)'"
action    = "deny"
message   = "Destructive SQL statements are not permitted"

[[rules]]
id        = "readonly-after-hours"
tool      = "write_file"
condition = "time.hour < 9 or time.hour > 18"
action    = "deny"
message   = "File writes are disabled outside business hours (09:00-18:00 UTC)"

[[rules]]
id          = "redact-secrets"
tool        = "*"
condition   = "arguments contains_pattern '(sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]+|xoxb-[a-zA-Z0-9-]+)'"
action      = "redact"
pattern     = "(sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]+|xoxb-[a-zA-Z0-9-]+)"
replacement = "[REDACTED]"

[[rules]]
id          = "api-rate-limit"
tool        = "http_request"
action      = "rate_limit"
max_calls   = 100
window_seconds = 60
```

### Redaction

`redact` rules apply regex substitution to tool **results before they reach the agent**. Secrets are scrubbed at the gateway boundary, not just in stored logs.

### Example policies

10 ready-made policies are included in `policies/examples/`:

| File                           | Description                                 |
| ------------------------------ | ------------------------------------------- |
| `01-no-shell-escape.toml`      | Block shell execution entirely              |
| `02-no-network-exfil.toml`     | Prevent network-based data exfiltration     |
| `03-production-readonly.toml`  | Read-only mode for production environments  |
| `04-redact-pii.toml`           | Strip personally identifiable information   |
| `05-api-throttle.toml`         | Throttle API calls to prevent bill spikes   |
| `06-no-sensitive-paths.toml`   | Block access to sensitive file paths        |
| `07-business-hours-only.toml`  | Restrict operations to business hours       |
| `08-redact-api-keys.toml`      | Strip API keys from tool results            |
| `09-allow-list.toml`           | Whitelist-only mode                         |
| `10-defence-in-depth.toml`     | Layered security policy                     |

## Transport Support

| Mode      | Command                                             | Use case                       |
| --------- | --------------------------------------------------- | ------------------------------ |
| **stdio** | `agentgate wrap -- <cmd>`                           | Any stdio MCP server           |
| **SSE**   | `agentgate serve --transport sse --upstream <url>`  | Server-Sent Events MCP servers |
| **HTTP**  | `agentgate serve --transport http --upstream <url>` | HTTP MCP servers               |

For SSE/HTTP transports, the proxy binds on port 7072 by default:

```bash
agentgate serve \
  --transport sse \
  --upstream http://localhost:3001 \
  --port 7072 \
  --policy policies/default.toml \
  --dashboard-port 7070
```

Custom headers can be passed to the upstream with `--header`:

```bash
agentgate serve --transport http --upstream https://api.example.com \
  --header "Authorization: Bearer ${API_TOKEN}" \
  --header "X-Custom: value"
```

## Metrics

Expose a Prometheus `/metrics` endpoint alongside the proxy:

```bash
agentgate wrap --metrics-port 9090 -- <mcp-server>
```

Six metrics are exported:

| Metric                                 | Type      | Description                                   |
| -------------------------------------- | --------- | --------------------------------------------- |
| `agentgate_tool_calls_total`           | Counter   | Tool calls by tool name and status            |
| `agentgate_tool_call_duration_seconds` | Histogram | Latency distribution per tool                 |
| `agentgate_policy_denials_total`       | Counter   | Policy denials by rule ID                     |
| `agentgate_rate_limit_hits_total`      | Counter   | Rate limit hits by scope                      |
| `agentgate_circuit_breaker_state`      | Gauge     | Circuit state (0=closed, 1=open, 2=half-open) |
| `agentgate_active_sessions`            | Gauge     | In-flight tool calls                          |

A ready-made Grafana dashboard is at `dashboards/grafana.json`.

## Dashboard

A Next.js 15 real-time dashboard is served on port 7070 by default:

```bash
agentgate wrap --dashboard-port 7070 -- <mcp-server>
# or
agentgate serve --transport sse --upstream http://localhost:3001 --dashboard-port 7070
```

Build the static UI first (only needed when building from source):

```bash
cd dashboard && npm install && npm run build
```

Pages:

| Page       | Path          | Description                                         |
| ---------- | ------------- | --------------------------------------------------- |
| Overview   | `/`           | KPI cards, call rate sparkline, live WebSocket feed  |
| Activity   | `/activity`   | Filterable, paginated invocations table              |
| Violations | `/violations` | Denied/rate-limited calls grouped by policy rule     |
| Analytics  | `/analytics`  | Per-tool call volume, error rate, latency chart      |
| Settings   | `/settings`   | In-browser TOML policy editor with live reload       |

The WebSocket endpoint (`/api/ws/live?token=...`) streams every persisted invocation in real time.

### REST API

| Endpoint              | Description                                        |
| --------------------- | -------------------------------------------------- |
| `GET /api/invocations`| Paginated invocations (query: `limit`, `offset`, `tool`, `status`) |
| `GET /api/overview`   | Stats: total calls, denials, avg latency, sparkline|
| `GET /api/tools`      | Per-tool call volume, error rate, latency          |
| `GET /api/violations` | Policy violations grouped by rule                  |
| `GET /api/ws/live`    | WebSocket live invocation stream                   |
| `GET /metrics`        | Prometheus text format                             |

All REST endpoints require `Authorization: Bearer <token>` header.

## Configuration

All options can be set via `~/.agentgate/config.toml` (or `--config <path>`):

```toml
log_level  = "info"                    # debug, info, warn, error
log_format = "pretty"                  # pretty or json
db_path    = "~/.agentgate/logs.db"    # SQLite database path
# policy_path = "~/.agentgate/policies/default.toml"

# Dashboard port (default: 7070)
# dashboard_port = 7070

# Fixed API key for the dashboard (random 32-char key generated if omitted)
# dashboard_api_key = "your-secret-key-here"

[rate_limits]
global_max_calls_per_minute   = 500    # Across all tools
per_tool_max_calls_per_minute = 100    # Per individual tool
per_agent_max_calls_per_minute = 200   # Per agent session

[circuit_breaker]
error_threshold  = 5                   # Errors before tripping open
window_seconds   = 30                  # Error counting window
cooldown_seconds = 60                  # Recovery delay before half-open probe

[log_retention]
retention_days = 30                    # Delete records older than N days (0 = disabled)
max_rows       = 500000                # Cap total rows (0 = disabled)
```

CLI flags always override the config file.

## How It Works

AgentGate intercepts the JSON-RPC 2.0 stream between agent and MCP server:

1. **Inbound** — Each message from the agent is parsed. `tools/call` requests are evaluated against the policy engine and rate limiter. Blocked calls get an immediate JSON-RPC error response; allowed calls are forwarded to the MCP server and tracked in a pending-call map (DashMap for fine-grained concurrent access).
2. **Response** — Responses from the MCP server are correlated with their pending call (for latency tracking), circuit-breaker state is updated, redaction is applied, and the (possibly scrubbed) response is forwarded to the agent.
3. **Persistence** — Records are enqueued on a bounded channel and written to SQLite by a background thread. The proxy hot path is never blocked by I/O.
4. **Live stream** — Every persisted record is broadcast to WebSocket subscribers via a `tokio::broadcast` channel, powering the dashboard's live feed.

JSON-RPC notifications (id-less messages) are forwarded immediately without tracking.

### Performance

- **Fast-path JSON-RPC parsing** — byte-level heuristic to locate `"method"` field, avoiding double deserialization
- **O(k) policy evaluation** — rules indexed by tool name; only matching rules are checked
- **Non-blocking storage** — bounded `sync_channel` (10k capacity) with background OS thread for SQLite writes
- **Backpressure** — stdout write channel capped at 256 messages to prevent slow agents from overwhelming servers

## Project Structure

```
agentgate/
├── crates/
│   ├── agentgate-core/                # Core library
│   │   └── src/
│   │       ├── config.rs              # Configuration structs & TOML loading
│   │       ├── metrics.rs             # Prometheus metrics exporter
│   │       ├── dashboard/             # REST + WebSocket API server (axum)
│   │       │   ├── api.rs             # REST endpoints
│   │       │   ├── server.rs          # HTTP server setup
│   │       │   ├── state.rs           # Shared dashboard state
│   │       │   └── ws.rs              # WebSocket live feed
│   │       ├── logging/
│   │       │   └── structured.rs      # Structured event logging
│   │       ├── policy/                # TOML policy engine with hot-reload
│   │       │   ├── engine.rs          # Rule indexing & evaluation
│   │       │   ├── rules.rs           # Rule & action definitions
│   │       │   └── condition.rs       # Expression parser & evaluator
│   │       ├── protocol/
│   │       │   ├── jsonrpc.rs         # JSON-RPC 2.0 parsing
│   │       │   └── mcp.rs            # MCP method constants
│   │       ├── proxy/
│   │       │   ├── evaluation.rs      # Policy + rate-limit + circuit-breaker logic
│   │       │   ├── stdio.rs           # Child process stdio proxy
│   │       │   ├── sse.rs             # SSE transport proxy
│   │       │   └── http.rs            # HTTP transport proxy
│   │       ├── ratelimit/             # Token bucket + circuit breaker
│   │       │   ├── limiter.rs         # Global & per-tool rate limiter
│   │       │   ├── token_bucket.rs    # Token bucket implementation
│   │       │   └── circuit_breaker.rs # Closed/Open/HalfOpen state machine
│   │       └── storage/               # SQLite persistence
│   │           └── sqlite.rs          # Schema, reader, writer, retention
│   └── agentgate-cli/                 # CLI binary
│       └── src/main.rs                # Commands: wrap, serve, logs, init, doctor
├── dashboard/                         # Next.js 15 frontend (static export)
│   └── src/
│       ├── app/                       # App Router pages
│       ├── components/                # React components
│       └── lib/                       # API client & types
├── dashboards/
│   └── grafana.json                   # Grafana dashboard
├── policies/
│   ├── default.toml                   # Default policy (5 rules)
│   └── examples/                      # 10 ready-made example policies
├── docs/                              # Man page & documentation
├── tests/integration/                 # Integration tests
├── fuzz/                              # Fuzz testing
├── Formula/                           # Homebrew formula
├── Dockerfile                         # Multi-stage build
├── install.sh                         # Shell installer
└── config.example.toml                # Example configuration
```

## Tech Stack

| Component  | Technology                           |
| ---------- | ------------------------------------ |
| Core       | Rust, Tokio, Serde                   |
| Protocol   | JSON-RPC 2.0, MCP                    |
| Storage    | SQLite (rusqlite, WAL mode)          |
| API server | Axum 0.7, Tower-HTTP                 |
| Metrics    | Prometheus 0.13                      |
| Dashboard  | Next.js 15, React, Tailwind CSS, Recharts |
| CLI        | Clap 4, Tabled                       |
| Concurrency| DashMap, tokio::broadcast            |
| File watch | Notify 6 (FSEvents on macOS)         |
| Embedding  | Rust-embed 8                         |

## Roadmap

- [x] **Phase 0** — MCP stdio proxy with structured logging
- [x] **Phase 1** — SQLite persistence, CLI log queries, JSONL export
- [x] **Phase 2** — Declarative TOML policy engine (deny/allow/redact rules)
- [x] **Phase 3** — Rate limiting (token bucket) & circuit breaker
- [x] **Phase 4** — SSE & HTTP transport support
- [x] **Phase 5** — Prometheus metrics & Grafana dashboard
- [x] **Phase 6** — Real-time dashboard (Next.js 15)
- [x] **Phase 7** — Distribution (Docker, Homebrew, installer)

## Building from Source

```bash
git clone https://github.com/iamdainwi/AgentGate.git
cd AgentGate
cargo build --release

# Optional: build the dashboard UI
cd dashboard && npm install && npm run build
```

The binary is at `target/release/agentgate`. The dashboard static files are embedded from `dashboard/out/`.

### Running tests

```bash
cargo test                              # unit + integration tests
cargo bench                             # criterion benchmarks (proxy evaluation hot path)
```

## License

MIT

## Author

**Dainwi Choudhary** — [@iamdainwi](https://github.com/iamdainwi)

- [LinkedIn](https://www.linkedin.com/in/dainwi-choudhary/)
- [Portfolio](https://dainwi.vercel.app)

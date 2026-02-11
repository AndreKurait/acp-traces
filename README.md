# acp-traces

Add OpenTelemetry tracing to any [Agent Client Protocol](https://agentclientprotocol.com) agent with zero code changes. One binary, one line in your editor config.

`acp-traces` sits between your editor (Zed, JetBrains) and your AI agent (Kiro, Claude Code, Gemini CLI), intercepting the ACP JSON-RPC messages and emitting [OTel GenAI semantic convention](https://opentelemetry.io/docs/specs/semconv/gen-ai/) traces to any OTLP-compatible backend.

```
Editor ──stdin──▶ [acp-traces] ──stdin──▶ Agent
Editor ◀──stdout── [acp-traces] ◀──stdout── Agent
                       │
                       └──▶ OTLP ──▶ Jaeger / Phoenix / Opik / Grafana Tempo
```

## Quick Start

### Install

```bash
# macOS / Linux
brew tap AndreKurait/tap
brew install acp-traces

# Or download from GitHub Releases
# https://github.com/AndreKurait/acp-traces/releases
```

### Configure Zed

Add to your Zed `settings.json`:

```json
{
  "agent_servers": {
    "kiro-traced": {
      "type": "custom",
      "command": "acp-traces",
      "args": ["--", "kiro-cli", "acp"],
      "env": {}
    }
  }
}
```

That's it. Start a Jaeger instance (`docker run -d -p 4317:4317 -p 16686:16686 jaegertracing/all-in-one`) and open http://localhost:16686 to see your traces.

### Use with Opik (HTTP)

```json
"args": ["--otlp-endpoint", "http://localhost:4318", "--otlp-protocol", "http", "--", "kiro-cli", "acp"]
```

## What Gets Traced

Every ACP interaction becomes an OTel span:

| ACP Event | Span | What you see |
|---|---|---|
| `session/prompt` → response | `invoke_agent` | Full prompt turn: duration, TTFT, stop reason |
| Tool calls (read, edit, search, execute…) | `execute_tool` | Each tool: name, kind, duration, arguments, result |
| `fs/read_text_file`, `fs/write_text_file` | `execute_tool` | File I/O the agent requests from the editor |
| `terminal/create`, `terminal/write` | `execute_tool` | Terminal commands the agent runs |
| `initialize`, `session/new` | Protocol spans | Connection setup, capability negotiation |

### Timing

All timing is captured implicitly from the message flow — no agent cooperation needed:

- **Total turn time** — how long the agent takes to respond
- **Time to first token** — when the first streaming chunk arrives
- **Tool execution time** — per-tool duration
- **Human approval latency** — how long permission requests take

### Metrics

| Metric | Description |
|---|---|
| `gen_ai.client.operation.duration` | Histogram of agent turn durations |
| `gen_ai.server.time_to_first_token` | Histogram of time to first response chunk |

## CLI Reference

```
acp-traces [OPTIONS] -- <command> [args...]
```

| Option | Default | Description |
|---|---|---|
| `--otlp-endpoint <URL>` | `http://localhost:4317` | OTLP endpoint |
| `--otlp-protocol <PROTO>` | `grpc` | `grpc` or `http` |
| `--service-name <NAME>` | `acp-agent` | OTel `service.name` resource attribute |
| `--record-content` | off | Record `gen_ai.input/output.messages` (contains sensitive data) |
| `-v, --verbose` | warn | Log verbosity to stderr (repeat: `-vv`, `-vvv`) |

## Semantic Conventions

Traces follow [OTel GenAI Semantic Conventions v1.39](https://opentelemetry.io/docs/specs/semconv/gen-ai/), the same standard used by OpenAI, Bedrock, and MCP instrumentations.

Key attributes on every span:

| Attribute | Example |
|---|---|
| `gen_ai.operation.name` | `invoke_agent`, `execute_tool` |
| `gen_ai.agent.name` | `kiro` |
| `gen_ai.provider.name` | `acp.kiro` |
| `gen_ai.conversation.id` | ACP session ID |
| `gen_ai.tool.name` | `Reading configuration file` |
| `gen_ai.tool.type` | `extension`, `datastore`, `function` |
| `gen_ai.response.finish_reasons` | `["end_turn"]` |
| `network.transport` | `pipe` |

ACP-specific extensions use the `acp.*` namespace:

| Attribute | Description |
|---|---|
| `acp.tool.kind` | Original ACP tool kind (`read`, `edit`, `think`, `search`…) |
| `acp.client.name` | IDE identity (e.g. `zed`) |
| `acp.agent.version` | Agent version |
| `acp.time_to_first_token_ms` | TTFT per invocation |

---

## Contributing

### Build from source

```bash
git clone https://github.com/AndreKurait/acp-traces.git
cd acp-traces
cargo build --release
# Binary at target/release/acp-traces
```

### Run checks

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

### Release process

Push a semver tag to trigger the release workflow:

```bash
git tag -a v0.2.0 -m "Description"
git push origin v0.2.0
```

This builds binaries for all platforms, creates a GitHub Release, and auto-updates the Homebrew formula.

### Architecture

- `src/main.rs` — CLI, child process spawning, bidirectional stdio pipe
- `src/acp.rs` — JSON-RPC message parser and ACP data extractor
- `src/spans.rs` — OTel span manager, session state machine, GenAI semconv mapping
- `src/telemetry.rs` — OTLP exporter setup (gRPC + HTTP)
- `DESIGN.md` — Full design document with spec mappings and open questions

## License

Apache-2.0

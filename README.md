# acp-traces

OTel tracing proxy for the [Agent Client Protocol](https://agentclientprotocol.com). Wraps any ACP agent with OpenTelemetry instrumentation following the [GenAI Semantic Conventions v1.39](https://opentelemetry.io/docs/specs/semconv/gen-ai/).

```
Editor (stdin) ──→ [acp-traces] ──→ Agent (child stdin)
                       │
Editor (stdout) ←── [acp-traces] ←── Agent (child stdout)
                       │
                       └──→ OTLP ──→ Jaeger / Phoenix / Opik
```

## Install

```bash
cargo install --path .
```

Or download a release binary from GitHub Actions artifacts.

## Usage

```bash
acp-traces [OPTIONS] -- <command> [args...]
```

### Options

| Flag | Default | Description |
|---|---|---|
| `--otlp-endpoint <URL>` | `http://localhost:4317` | OTLP endpoint |
| `--otlp-protocol <PROTO>` | `grpc` | `grpc` or `http` |
| `--service-name <NAME>` | `acp-agent` | OTel service.name |
| `--record-content` | off | Record `gen_ai.input/output.messages` (sensitive) |
| `-v, --verbose` | warn | Increase log verbosity (repeatable: `-v`, `-vv`, `-vvv`) |

### Examples

```bash
# Trace kiro-cli with Jaeger (gRPC on 4317)
acp-traces -- kiro-cli acp

# Trace with Opik (HTTP on 4318)
acp-traces --otlp-endpoint http://localhost:4318 --otlp-protocol http -- kiro-cli acp

# Debug mode with content recording
acp-traces -vv --record-content -- claude-code acp
```

## Zed Configuration

Add to your Zed `settings.json`:

```json
{
  "agent_servers": {
    "kiro-traced": {
      "type": "custom",
      "command": "acp-traces",
      "args": ["--otlp-endpoint", "http://localhost:4317", "--", "kiro-cli", "acp"],
      "env": {}
    }
  }
}
```

## What Gets Traced

| ACP Event | OTel Span | `gen_ai.operation.name` |
|---|---|---|
| `session/prompt` → response | `invoke_agent {agent}` | `invoke_agent` |
| `session/update` tool_call → completed | `execute_tool {title}` | `execute_tool` |
| `fs/read_text_file`, `fs/write_text_file` | `execute_tool fs/...` | `execute_tool` |
| `terminal/create`, `terminal/write` | `execute_tool terminal/...` | `execute_tool` |
| `initialize`, `session/new`, etc. | `{method}` | — (RPC spans) |

### Metrics

| Metric | Description |
|---|---|
| `gen_ai.client.operation.duration` | Histogram of `invoke_agent` durations |
| `gen_ai.server.time_to_first_token` | Histogram of time to first `agent_message_chunk` |

### Key Attributes

- `gen_ai.agent.name` — agent identity from `initialize`
- `gen_ai.conversation.id` — ACP session ID
- `gen_ai.provider.name` — `acp.{agent_name}`
- `gen_ai.tool.name`, `gen_ai.tool.type`, `gen_ai.tool.call.id`
- `acp.tool.kind` — original ACP tool kind (`read`, `edit`, `think`, etc.)
- `acp.client.name` / `acp.client.version` — IDE identity
- `network.transport` — `pipe` (stdio)

## License

Apache-2.0

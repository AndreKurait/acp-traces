# acp-traces

[![CI](https://github.com/AndreKurait/acp-traces/actions/workflows/ci.yml/badge.svg)](https://github.com/AndreKurait/acp-traces/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**Proof of concept: OpenTelemetry tracing for Agent Client Protocol (ACP) over stdio.**

`acp-traces` launches an agent, forwards newline-delimited JSON-RPC between the agent and its client, and observes supported messages to produce traces and metrics. It does not modify the agent or inject fields into the protocol.

This is an experimental project, not a production-ready monitoring agent. Client compatibility and semantic-convention coverage are incomplete. This README describes `main`; installed releases may not include the [unreleased fixes](CHANGELOG.md).

```text
Client ──stdin──▶ acp-traces ──stdin──▶ Agent
Client ◀──stdout── acp-traces ◀──stdout── Agent
                       │
                       └── OTLP traces + metrics ──▶ Collector/backend
```

## Getting started

You need an ACP agent that communicates over stdio and an OTLP receiver. The proxy does not provide either one.

### Install

Published binaries are available through Homebrew or [GitHub Releases](https://github.com/AndreKurait/acp-traces/releases):

```bash
brew install AndreKurait/tap/acp-traces
acp-traces --version
```

To try the current development code with a stable Rust toolchain:

```bash
git clone https://github.com/AndreKurait/acp-traces.git
cd acp-traces
cargo install --path . --locked
```

### Run an agent

Pass the agent command after `--`. For example, if `kiro-cli acp` is available locally:

```bash
acp-traces --otlp-endpoint http://localhost:4317 -- kiro-cli acp
```

Configure your ACP client to launch the proxy in place of the agent. A Zed custom-agent configuration can look like this; adapt it to your client's supported configuration and your agent's arguments:

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

For an OTLP/HTTP receiver:

```bash
acp-traces --otlp-protocol http --otlp-endpoint http://localhost:4318 -- kiro-cli acp
```

HTTP exports use `/v1/traces` and `/v1/metrics` beneath the supplied base URL, preserving any path prefix and query parameters. `http-json` selects JSON encoding; `http` selects protobuf. An explicit `/v1/traces` endpoint is also accepted for compatibility with earlier configurations. Standard OTLP environment variables supported by the Rust exporter can override endpoint/header settings.

TLS exporter features are not enabled in this build. Use a local collector to forward telemetry to a remote TLS/authenticated backend.

## What it records

| Observed traffic | Telemetry |
|---|---|
| `initialize` | Protocol span, agent/client metadata, root connection span |
| `session/prompt` and its response | `invoke_agent` span, duration, stop reason |
| `agent_message_chunk` | Time to first observed message chunk |
| Tool-call notifications | `execute_tool` spans with tool title/kind and completion status |
| Supported filesystem/terminal requests | Tool spans correlated with their responses |
| Other JSON-RPC requests | Method spans with response/error status |

Two histograms are exported: `gen_ai.client.operation.duration` and `gen_ai.server.time_to_first_token`, both in seconds. The latter measures the first observed ACP message chunk, not model inference time. Metrics export periodically and at shutdown; `OTEL_METRIC_EXPORT_INTERVAL` controls the periodic interval in milliseconds.

Attributes use OpenTelemetry's evolving GenAI conventions plus `acp.*` metadata. This is a practical mapping, not a claim of full semantic-convention conformance. See [DESIGN.md](DESIGN.md) for the implemented behavior and limitations.

## Privacy

Content recording is **off by default**. Prompts, response text, tool arguments/results, and JSON-RPC error payloads are omitted from exported spans unless `--record-content` is enabled. Default error statuses retain the numeric error code.

Metadata is still exported: session IDs, agent/client names and versions, method names, tool titles/kinds, timestamps, and error codes. Tool titles may contain paths or other sensitive information. Default mode is not anonymization. Child stderr passes through unchanged, and the proxy's logs go to stderr.

`--record-content` can export source code, credentials, file contents, and other private data to your configured receiver. Use a receiver you trust and inspect its retention/access settings. The proxy does not redact secrets from opted-in content. See [SECURITY.md](SECURITY.md).

## CLI

```text
acp-traces [OPTIONS] -- <command> [args...]
```

| Option | Default | Purpose |
|---|---|---|
| `--otlp-endpoint <URL>` | `http://localhost:4317` | OTLP receiver; use port 4318 for a typical HTTP receiver |
| `--otlp-protocol <PROTO>` | `grpc` | `grpc`, `http`, or `http-json` |
| `--service-name <NAME>` | `acp-agent` | `service.name` resource attribute |
| `--record-content` | off | Include content and detailed error payloads |
| `-v`, `-vv`, `-vvv` | warnings | Increase proxy logging to stderr |

## Behavior and limitations

- Bytes and line endings are forwarded unchanged, including messages the observer cannot parse. Stdout is reserved for agent traffic.
- Closing client stdin closes agent stdin and gives the agent five seconds to exit before terminating it. Final stdout is drained before telemetry shutdown. The agent's exit code is preserved; on Unix, signal exits use `128 + signal`.
- Telemetry observation uses a bounded message queue. Under load it can apply backpressure. Individual message sizes, opted-in output accumulation, and retained session state are not capped; avoid untrusted or unbounded streams.
- The observer expects newline-delimited JSON-RPC. It does not support Content-Length framing, inject distributed trace context, or forward process-group signals.
- Tool notifications are associated with the active prompt. The implementation assumes at most one active prompt per session and does not reconstruct arbitrary out-of-order histories.
- Content capture covers text prompt blocks and observed tool payloads; it does not reconstruct every resource/image/audio block.
- Unit/integration tests exercise protocol correlation, privacy defaults, process shutdown, and OTLP/HTTP export. They are not an exhaustive editor/agent/backend compatibility matrix.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md). Useful contributions include minimized ACP fixtures, compatibility reports, resource bounds, and tests for cancellation and abnormal process exits. Read the [Code of Conduct](CODE_OF_CONDUCT.md) before participating.

**Code map:** `src/main.rs` forwards stdio and manages the child; `src/acp.rs` parses messages; `src/spans.rs` correlates requests and constructs spans; `src/telemetry.rs` configures exporters.

## License

Apache-2.0. See [LICENSE](LICENSE).

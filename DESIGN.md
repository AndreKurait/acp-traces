# acp-traces implementation notes

`acp-traces` is a proof of concept for observing ACP over newline-delimited JSON-RPC. This document describes the current implementation, rather than a complete protocol or semantic-convention specification.

## Data flow

Two Tokio tasks forward raw lines between the client and a child process. Valid UTF-8 lines are also sent to a bounded observation queue before forwarding, so a response cannot overtake its request in the observer. Parse failures do not change forwarded bytes. One task owns the span manager; it does not share mutable tracing state with the forwarding tasks.

On client EOF, the child gets EOF on stdin and up to five seconds to finish. On child exit, stdout is drained before the observation channel closes and pending spans are ended. Telemetry is then flushed. The program exits explicitly because Tokio's blocking stdin read cannot be cancelled merely by aborting its task.

## Request correlation

Pending requests are keyed by **request direction and JSON-RPC ID**. Each peer has its own ID namespace: the client and agent can both have request `1` outstanding. Responses resolve the opposite direction's request key. JSON serialization of the ID preserves the distinction between numeric and string IDs.

A session keeps its active prompt, first-chunk time, optional accumulated output, and tool-call spans. A completed prompt clears its parent context so later requests cannot inherit a completed invocation. Concurrent prompts for the same session are not supported.

## Span mapping

| ACP traffic | Span / attributes |
|---|---|
| First `initialize` request | `acp_session` root connection span |
| `initialize` response | Agent name/version and protocol version are learned |
| `session/prompt` | Client span named `invoke_agent` or `invoke_agent <agent>` |
| Tool-call notifications during a prompt | Internal `execute_tool <title>` spans parented to the prompt |
| Recognized `fs/*` and `terminal/*` requests | Internal tool spans, using the method name |
| Other requests | Internal method spans, including permission requests |

The connection root ends on shutdown. Child spans can be exported before the root. Session IDs use `gen_ai.conversation.id`; local protocol details use `acp.*`. No `traceparent` is injected into ACP traffic.

Raw ACP stop reasons are recorded as a string array in `gen_ai.response.finish_reasons`. When content recording is enabled, output-message `finish_reason` maps `end_turn` to `stop`, token/request limits to `length`, and `refusal` to `content_filter`; other values pass through.

A JSON-RPC error marks the associated span as failed. By default, its status includes only the numeric error code and `error.type` is the code (or `_OTHER`). Detailed error JSON is recorded only with content capture enabled.

## Content and memory

Without `--record-content`, prompt/response/tool payload attributes are not emitted and response chunks are not accumulated. Tool titles and protocol metadata remain observable and may themselves be sensitive.

With content capture, text prompt blocks and response chunks become `gen_ai.input.messages` and `gen_ai.output.messages`. Tool `rawInput`/`rawOutput` and recognized filesystem/terminal arguments/results can become tool attributes. Error statuses may contain full error JSON. Completed prompts release their accumulated output.

The queue bounds the number of pending observed messages, not the size of an individual message. Message-size limits, content truncation, session eviction, and arbitrary concurrent prompt handling remain future work.

## Export

Trace and metric exporters share a resource containing `service.name`. Supported transports are OTLP/gRPC, OTLP/HTTP protobuf, and OTLP/HTTP JSON. HTTP base URLs are expanded to the separate `/v1/traces` and `/v1/metrics` paths. Existing explicit signal paths are normalized first.

The dependency configuration does not enable exporter TLS features. A local collector is the current path to a remote TLS/authenticated backend.

A periodic metric reader exports invocation-duration and first-observed-chunk histograms. Both providers are flushed/shut down before normal process exit. Export failures are logged to stderr; there is no persistent telemetry buffer or delivery guarantee.

## Validation

- Parser unit tests cover requests, notifications, and extraction helpers.
- Span tests replay messages through the real `SpanManager`, including overlapping bidirectional IDs and content/error privacy defaults.
- Unix subprocess tests cover stdin EOF, child exit while client stdin remains open, exit codes, final stdout, and unparseable bytes.
- A local OTLP/HTTP receiver test verifies that both signals reach their separate endpoints.
- These checks do not establish compatibility with every ACP client or backend. Windows/macOS release targets are built independently; cross-compiled artifacts are not all executed in CI.

## References

- [ACP protocol](https://agentclientprotocol.com/protocol/overview)
- [OpenTelemetry GenAI conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/)
- [OTLP exporter configuration](https://opentelemetry.io/docs/specs/otel/protocol/exporter/)

These standards evolve. Changes to mappings should include captured, sanitized fixtures and a reference to the relevant convention or protocol behavior.

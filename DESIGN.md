# acp-traces: Design Document

## Overview

A Rust binary that sits between an ACP client (Zed, JetBrains) and an ACP agent
(kiro-cli, claude-code, etc.) as a stdio pipe, intercepting JSON-RPC messages
and emitting OpenTelemetry traces conforming to the **OTel GenAI Semantic
Conventions v1.39**.

**Convention choice: OTel GenAI semconv only.** No OpenInference attributes.

## Spec Sources

- OTel GenAI Agent Spans: https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-agent-spans/
- OTel GenAI Spans (Inference/Tool): https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-spans/
- OTel GenAI Events: https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-events/
- OTel GenAI Metrics: https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-metrics/
- OTel GenAI MCP: https://opentelemetry.io/docs/specs/semconv/gen-ai/mcp/
- OTel RPC JSON-RPC: https://opentelemetry.io/docs/specs/semconv/rpc/json-rpc/
- ACP Protocol: https://agentclientprotocol.com/protocol/overview

## Message Flow

```
Editor (stdin) ──→ [acp-traces] ──→ Agent (child stdin)
                       │
Editor (stdout) ←── [acp-traces] ←── Agent (child stdout)
                       │
                       ├──→ OTLP gRPC ──→ Jaeger/Phoenix/Opik
                       │
                   stderr passed through unchanged
```

---

# Part 1: Finalized Mappings (MUST / clear spec guidance)

These are locked. The spec uses MUST/REQUIRED or the mapping is unambiguous.

## 1.1 `invoke_agent` span — ACP `session/prompt`

Created when editor sends `session/prompt`. Ended when agent responds with `stopReason`.

**Span name:** `invoke_agent {gen_ai.agent.name}` (spec: "SHOULD be `invoke_agent {gen_ai.agent.name}`")
**Span kind:** `CLIENT` (spec: "SHOULD be CLIENT")
**Span status:** Error if JSON-RPC error response (spec: "SHOULD follow Recording Errors")

### Required attributes (MUST set)

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.operation.name` | `"invoke_agent"` | Required. Well-known value for agent invocation. |
| `gen_ai.provider.name` | `"acp.{agentInfo.name}"` (e.g. `"acp.kiro"`) | Required. Follows dotted pattern (`aws.bedrock`, `gcp.vertex_ai`). Custom value per spec. |

### Conditionally Required attributes

| Attribute | Value | Condition | Spec basis |
|---|---|---|---|
| `error.type` | JSON-RPC error code or message | "if the operation ended in an error" | Stable attribute. MUST use well-known value `_OTHER` if no specific error type. |
| `gen_ai.agent.name` | `agentInfo.name` or `agentInfo.title` | "when available" | From `initialize` response. |
| `gen_ai.agent.id` | `agentInfo.name` | "if applicable" | Programmatic identifier. |
| `gen_ai.conversation.id` | `params.sessionId` | "when available" | Direct mapping — ACP sessionId IS the conversation. |

### Recommended attributes

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.response.finish_reasons` | `["end_turn"]`, `["cancelled"]`, `["max_tokens"]`, etc. | Direct mapping from ACP `stopReason`. ACP values: `end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`. |
| `gen_ai.usage.input_tokens` | Not available | We don't have token counts at ACP layer. Omit. |
| `gen_ai.usage.output_tokens` | Not available | Same. Omit. |
| `gen_ai.response.model` | Not available | Agent doesn't expose which LLM it uses. Omit. |

### Opt-In attributes (only with `--record-content`)

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.input.messages` | Constructed from `params.prompt[]` ContentBlocks. Format: `[{"role":"user","parts":[{"type":"text","content":"..."}]}]` | "MUST follow Input messages JSON schema." Content is sensitive — spec says "SHOULD NOT capture by default." |
| `gen_ai.output.messages` | Accumulated from `session/update` `agent_message_chunk` notifications. Format: `[{"role":"assistant","parts":[{"type":"text","content":"..."}],"finish_reason":"end_turn"}]` | "MUST follow Output messages JSON schema." Same sensitivity. |

### ACP → `gen_ai.input.messages` content block mapping

ACP ContentBlocks map to the OTel GenAI input messages JSON schema parts:

| ACP ContentBlock type | OTel part type | Mapping |
|---|---|---|
| `text` | `{"type":"text","content":"..."}` | Direct. |
| `image` | `{"type":"image","data":"...","media_type":"image/png"}` | Direct — ACP `data` (base64) + `mimeType`. |
| `audio` | `{"type":"audio","data":"...","media_type":"audio/wav"}` | Direct — ACP `data` (base64) + `mimeType`. |
| `resource` | See [Open Question 1](#oq1-embedded-resources) | No standard OTel part type for file resources. |
| `resource_link` | See [Open Question 1](#oq1-embedded-resources) | No standard OTel part type for resource links. |

### ACP `stopReason` → `gen_ai.response.finish_reasons` mapping

| ACP stopReason | OTel finish_reason | Notes |
|---|---|---|
| `end_turn` | `"stop"` | Spec well-known value for normal completion. |
| `max_tokens` | `"max_tokens"` | Direct match — spec doesn't define this but it's conventional. |
| `max_turn_requests` | `"max_tokens"` | Closest match (resource limit). |
| `refusal` | `"content_filter"` | Closest match (model refused). |
| `cancelled` | `"cancelled"` | No standard value — see [Open Question 2](#oq2-finish-reasons). |

## 1.2 `execute_tool` span — ACP tool_call notifications

Created on `session/update` with `sessionUpdate: "tool_call"`.
Ended on `tool_call_update` with `status: "completed"` or `"failed"`.

**Span name:** `execute_tool {gen_ai.tool.name}` (spec: "SHOULD be `execute_tool {gen_ai.tool.name}`")
**Span kind:** `INTERNAL` (spec: "SHOULD be INTERNAL")
**Parent:** the active `invoke_agent` span for this session

### Required attributes

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.operation.name` | `"execute_tool"` | Required. Well-known value. |

### Conditionally Required attributes

| Attribute | Value | Condition |
|---|---|---|
| `error.type` | Error description | If `status: "failed"` |

### Recommended attributes

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.tool.name` | `update.title` (e.g. "Reading configuration file") | "Recommended" |
| `gen_ai.tool.call.id` | `update.toolCallId` | "Recommended if available" |
| `gen_ai.tool.type` | Mapped from ACP `kind` — see mapping table below | "Recommended if available" |
| `gen_ai.tool.description` | Not separately available from ACP (title serves this role) | Omit to avoid duplication with tool.name. |

### Opt-In attributes

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.tool.call.arguments` | `update.rawInput` | "Opt-In. May contain sensitive information." |
| `gen_ai.tool.call.result` | `update.rawOutput` or `update.content[].content.text` | "Opt-In. May contain sensitive information." |

### ACP `kind` → `gen_ai.tool.type` mapping

The spec defines `gen_ai.tool.type` semantics by **execution context**:
- `function` = client-side execution (agent generates params, client runs it)
- `extension` = agent-side execution (agent calls external APIs/systems)
- `datastore` = data retrieval for RAG/knowledge

| ACP `kind` | `gen_ai.tool.type` | Reasoning |
|---|---|---|
| `read` | `"datastore"` | Reading/retrieving data |
| `search` | `"datastore"` | Searching/querying data |
| `fetch` | `"datastore"` | Fetching external data |
| `edit` | `"extension"` | Agent-side file modification |
| `delete` | `"extension"` | Agent-side file removal |
| `move` | `"extension"` | Agent-side file move |
| `execute` | `"extension"` | Agent-side command execution |
| `think` | `"extension"` | Agent-side reasoning (closest fit) |
| `other` | `"extension"` | Default |

The original ACP `kind` is preserved in `acp.tool.kind` (custom attribute).

## 1.3 `execute_tool` span — ACP `fs/*` and `terminal/*` requests

These are the agent asking the **client** (editor) to perform an action.

**Span name:** `execute_tool {method_name}`
**Span kind:** `INTERNAL`
**Parent:** the active `invoke_agent` span for this session

### Attributes

| Attribute | Value | Spec basis |
|---|---|---|
| `gen_ai.operation.name` | `"execute_tool"` | Required |
| `gen_ai.tool.name` | Method name (`"fs/read_text_file"`, `"fs/write_text_file"`, `"terminal/create"`) | Recommended |
| `gen_ai.tool.call.id` | JSON-RPC `id` (stringified) | Recommended |
| `gen_ai.tool.type` | `"function"` | These are client-side execution — the editor runs them. Matches spec definition exactly. |
| `gen_ai.tool.call.arguments` | `params` JSON (opt-in) | e.g. `{"path":"/src/main.rs","line":10}` |
| `gen_ai.tool.call.result` | `result` JSON (opt-in) | e.g. `{"content":"def hello():..."}` |
| `error.type` | JSON-RPC error code | If error response |

## 1.4 Protocol lifecycle spans — `initialize`, `authenticate`, `session/new`, `session/load`

Not GenAI operations. Use OTel RPC/JSON-RPC semantic conventions.

**Span name:** ACP method name
**Span kind:** `INTERNAL`

| Attribute | Value | Spec basis |
|---|---|---|
| `rpc.system` | `"jsonrpc"` | OTel RPC semconv. Required. |
| `rpc.method` | Method name | OTel RPC semconv. Required. |
| `rpc.jsonrpc.request_id` | JSON-RPC `id` | OTel JSON-RPC semconv. Recommended. |
| `rpc.jsonrpc.error_code` | Error code (if error response) | OTel JSON-RPC semconv. Cond. Required. |
| `rpc.jsonrpc.error_message` | Error message (if error response) | OTel JSON-RPC semconv. Cond. Required. |

On the `initialize` response, we extract `agentInfo` and `clientInfo` and store
them in proxy state for use on subsequent spans.

## 1.5 Metrics

| Metric | Type | Unit | Buckets | Status |
|---|---|---|---|---|
| `gen_ai.client.operation.duration` | Histogram | `s` | `[0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64, 1.28, 2.56, 5.12, 10.24, 20.48, 40.96, 81.92]` | **Required** |
| `gen_ai.server.time_to_first_token` | Histogram | `s` | `[0.001, 0.005, 0.01, 0.02, 0.04, 0.06, 0.08, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0]` | Recommended |

Omitted (spec: "MUST NOT report" without token counts):
- `gen_ai.client.token.usage`
- `gen_ai.server.time_per_output_token`
- `gen_ai.server.request.duration` (we're not the server)

### ACP-specific attributes (mirroring MCP semconv pattern)

Following the MCP semconv pattern where `mcp.*` attributes supplement `gen_ai.*`,
we define `acp.*` for ACP-specific details. The `gen_ai.*` attributes are the
common layer backends render; `acp.*` is supplementary protocol metadata.

| Attribute | Type | On which spans | Source |
|---|---|---|---|
| `acp.method.name` | string | All ACP spans | ACP JSON-RPC method (e.g. `"session/prompt"`, `"fs/read_text_file"`) |
| `acp.protocol.version` | int | All ACP spans | From `initialize` protocolVersion |
| `acp.tool.kind` | string | execute_tool (from tool_call) | Original ACP kind: `read`, `edit`, `delete`, `move`, `search`, `execute`, `think`, `fetch`, `other` |
| `acp.tool.locations` | string (JSON) | execute_tool | File paths/lines: `[{"path":"/src/main.py","line":42}]` |
| `acp.agent.version` | string | invoke_agent | From `agentInfo.version` |
| `acp.client.name` | string | invoke_agent | IDE name from `clientInfo.name` |
| `acp.client.version` | string | invoke_agent | IDE version from `clientInfo.version` |
| `acp.permission.outcome` | string | request_permission span | `"allow_once"`, `"allow_always"`, `"reject_once"`, `"reject_always"`, `"cancelled"` |
| `acp.time_to_first_token_ms` | int | invoke_agent | Computed TTFT |

### Standard non-GenAI attributes on all spans

| Attribute | Value | Spec basis |
|---|---|---|
| `jsonrpc.request.id` | JSON-RPC `id` (stringified) | OTel JSONRPC registry (same as MCP semconv uses) |
| `network.transport` | `"pipe"` | OTel network registry. Spec: "SHOULD be `pipe` if the transport is stdio." |

### Context propagation

Following the MCP semconv pattern, we SHOULD inject `traceparent`/`tracestate`
into ACP `params._meta` on outgoing requests and extract on incoming. This
enables trace continuity if the agent has its own OTel instrumentation.

Deferred to v2 — requires modifying messages in flight, which adds complexity.

---

# Part 2: Open Questions (one remaining)

## OQ1: Embedded resources in `gen_ai.input.messages`

**Problem:** ACP prompts can include `resource` and `resource_link` ContentBlocks
(file contents with URI, mimeType, text). The OTel GenAI input messages JSON
schema defines part types: `text`, `image`, `audio`, `tool_call`,
`tool_call_response`, `refusal`. There is no `resource` or `file` part type.
The MCP semconv doesn't address this either.

**Decision for v1:** Encode resources as `text` parts with the file content.
This is schema-compliant but loses the URI/mimeType metadata. We accept this
limitation for now.

## Resolved questions

**OQ2 (finish_reasons):** Use ACP values verbatim — `"end_turn"`, `"cancelled"`,
`"max_tokens"`, `"refusal"`, `"max_turn_requests"`. The spec defines no
well-known values for `gen_ai.response.finish_reasons`; they're just examples.

**OQ3 (provider.name):** Use `agentInfo.name` (e.g. `"kiro"`). Spec says
"a custom value MAY be used" and "SHOULD be set based on the instrumentation's
best knowledge."

**OQ4 (permission requests):** Model as a regular method span with
`acp.method.name` = `"session/request_permission"`, following MCP's pattern for
`elicitation/create`. Record outcome in `acp.permission.outcome`.

**OQ5 (streaming):** Accumulate `agent_message_chunk` text, emit as
`gen_ai.output.messages` at span end. Tool calls as child spans (they have
duration). Streaming chunks are explicitly TODO in the OTel spec.

**OQ6 (think tool type):** Map to `gen_ai.tool.type` = `"extension"` (agent-side
operation). Preserve original kind in `acp.tool.kind` = `"think"`.

**OQ7 (multi-turn sessions):** No parent span. Each `invoke_agent` is a root
span linked by `gen_ai.conversation.id`. Long-lived spans are an anti-pattern.
MCP semconv uses the same approach (`mcp.session.id` + session duration metrics,
not long-lived spans).

---

# Part 3: State Machine

The proxy maintains per-session state:

```
GlobalState {
    agent_name: Option<String>,          // from initialize response
    agent_version: Option<String>,       // from initialize response
    client_name: Option<String>,         // from initialize request
    client_version: Option<String>,      // from initialize request
    sessions: HashMap<String, SessionState>,
}

SessionState {
    session_id: String,
    active_prompt_span: Option<SpanContext>,  // invoke_agent span
    active_tool_spans: HashMap<String, Span>, // toolCallId → execute_tool span
    pending_requests: HashMap<String, PendingRequest>, // JSON-RPC id → span + metadata
    accumulated_output: Vec<String>,         // agent_message_chunk text accumulator
    prompt_start_time: Option<Instant>,      // for duration metric
    first_chunk_time: Option<Instant>,       // for TTFT
}

PendingRequest {
    span: Span,
    method: String,
    session_id: Option<String>,
    start_time: Instant,
}
```

---

# Part 4: CLI

```
acp-traces [OPTIONS] -- <command> [args...]

Options:
  --otlp-endpoint <URL>    OTLP gRPC endpoint [default: http://localhost:4317]
  --service-name <NAME>    OTel service name [default: acp-agent]
  --record-content         Enable recording gen_ai.input/output.messages (opt-in per spec)
```

## Zed Config

```json
{
  "agent_servers": {
    "kiro": {
      "type": "custom",
      "command": "acp-traces",
      "args": ["--otlp-endpoint", "http://localhost:4317", "--", "kiro-cli", "acp"],
      "env": {}
    }
  }
}
```

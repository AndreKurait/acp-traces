use crate::acp::{self, Direction, MessageType};
use opentelemetry::{
    metrics::{Histogram, Meter},
    trace::{Span, SpanContext, SpanKind, Status, TraceContextExt, Tracer},
    Context, KeyValue,
};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

struct SessionState {
    prompt_span: Option<opentelemetry::global::BoxedSpan>,
    prompt_span_context: Option<SpanContext>,
    prompt_start: Option<Instant>,
    first_chunk_time: Option<Instant>,
    accumulated_output: String,
    tool_spans: HashMap<String, opentelemetry::global::BoxedSpan>,
}

struct PendingRequest {
    span: Option<opentelemetry::global::BoxedSpan>,
    method: String,
    session_id: Option<String>,
    start: Instant,
}

pub struct SpanManager {
    tracer: opentelemetry::global::BoxedTracer,
    duration_histogram: Histogram<f64>,
    ttft_histogram: Histogram<f64>,
    record_content: bool,
    agent_name: Option<String>,
    agent_version: Option<String>,
    client_name: Option<String>,
    client_version: Option<String>,
    protocol_version: Option<i64>,
    sessions: HashMap<String, SessionState>,
    pending: HashMap<String, PendingRequest>,
    /// Root span for the entire ACP session — parents all other spans.
    session_span: Option<opentelemetry::global::BoxedSpan>,
    session_span_context: Option<SpanContext>,
}

impl SpanManager {
    pub fn new(
        tracer: opentelemetry::global::BoxedTracer,
        meter: Meter,
        record_content: bool,
    ) -> Self {
        let duration_histogram = meter
            .f64_histogram("gen_ai.client.operation.duration")
            .with_unit("s")
            .with_description("GenAI operation duration")
            .build();
        let ttft_histogram = meter
            .f64_histogram("gen_ai.server.time_to_first_token")
            .with_unit("s")
            .with_description("Time to generate first token")
            .build();

        Self {
            tracer,
            duration_histogram,
            ttft_histogram,
            record_content,
            agent_name: None,
            agent_version: None,
            client_name: None,
            client_version: None,
            protocol_version: None,
            sessions: HashMap::new(),
            pending: HashMap::new(),
            session_span: None,
            session_span_context: None,
        }
    }

    pub fn process_message(&mut self, direction: Direction, line: &str) {
        let msg = match acp::parse(line) {
            Some(m) => m,
            None => return,
        };

        match msg {
            MessageType::Request { id, method, params } => {
                self.handle_request(direction, id, &method, &params);
            }
            MessageType::Response { id, result, error } => {
                self.handle_response(id, result.as_ref(), error.as_ref());
            }
            MessageType::Notification { method, params } => {
                self.handle_notification(direction, &method, &params);
            }
        }
    }

    fn handle_request(&mut self, direction: Direction, id: Value, method: &str, params: &Value) {
        tracing::debug!(direction = ?direction, method = %method, "request");

        match method {
            "initialize" => {
                if let Some((name, version)) = acp::extract_client_info(params) {
                    self.client_name = Some(name.to_string());
                    self.client_version = version.map(|v| v.to_string());
                }
                // Create the root session span that parents everything.
                if self.session_span.is_none() {
                    let root = self
                        .tracer
                        .span_builder("acp_session")
                        .with_kind(SpanKind::Internal)
                        .with_attributes(vec![
                            KeyValue::new("acp.method.name", "session"),
                            KeyValue::new("network.transport", "pipe"),
                        ])
                        .start(&self.tracer);
                    self.session_span_context = Some(root.span_context().clone());
                    self.session_span = Some(root);
                }
                let span = self.start_under_root(
                    self.tracer
                        .span_builder("initialize")
                        .with_kind(SpanKind::Internal)
                        .with_attributes(vec![
                            KeyValue::new("rpc.system", "jsonrpc"),
                            KeyValue::new("rpc.method", "initialize"),
                            KeyValue::new("acp.method.name", "initialize"),
                            KeyValue::new("network.transport", "pipe"),
                        ]),
                );
                self.pending.insert(
                    id.to_string(),
                    PendingRequest {
                        span: Some(span),
                        method: method.to_string(),
                        session_id: None,
                        start: Instant::now(),
                    },
                );
            }
            "session/prompt" => {
                let session_id = acp::extract_session_id(params)
                    .unwrap_or("unknown")
                    .to_string();
                let span_name = match &self.agent_name {
                    Some(name) => format!("invoke_agent {name}"),
                    None => "invoke_agent".to_string(),
                };
                let mut attrs = vec![
                    KeyValue::new("gen_ai.operation.name", "invoke_agent"),
                    KeyValue::new("gen_ai.conversation.id", session_id.clone()),
                    KeyValue::new("acp.method.name", "session/prompt"),
                    KeyValue::new("network.transport", "pipe"),
                ];
                if let Some(ref name) = self.agent_name {
                    attrs.push(KeyValue::new("gen_ai.provider.name", format!("acp.{name}")));
                    attrs.push(KeyValue::new("gen_ai.agent.name", name.clone()));
                    attrs.push(KeyValue::new("gen_ai.agent.id", name.clone()));
                }
                if let Some(ref v) = self.agent_version {
                    attrs.push(KeyValue::new("acp.agent.version", v.clone()));
                }
                if let Some(ref n) = self.client_name {
                    attrs.push(KeyValue::new("acp.client.name", n.clone()));
                }
                if let Some(ref v) = self.client_version {
                    attrs.push(KeyValue::new("acp.client.version", v.clone()));
                }
                if self.record_content {
                    if let Some(text) = acp::extract_prompt_text(params) {
                        let input_msg = serde_json::json!([{
                            "role": "user",
                            "parts": [{"type": "text", "content": text}]
                        }]);
                        attrs.push(KeyValue::new(
                            "gen_ai.input.messages",
                            input_msg.to_string(),
                        ));
                    }
                }
                let span = self.start_under_root(
                    self.tracer
                        .span_builder(span_name)
                        .with_kind(SpanKind::Client)
                        .with_attributes(attrs),
                );
                let span_context = span.span_context().clone();
                let now = Instant::now();
                self.sessions
                    .entry(session_id.clone())
                    .or_insert_with(|| SessionState {
                        prompt_span: None,
                        prompt_span_context: None,
                        prompt_start: None,
                        first_chunk_time: None,
                        accumulated_output: String::new(),
                        tool_spans: HashMap::new(),
                    });
                let session = self.sessions.get_mut(&session_id).unwrap();
                session.prompt_span = Some(span);
                session.prompt_span_context = Some(span_context);
                session.prompt_start = Some(now);
                session.first_chunk_time = None;
                session.accumulated_output.clear();
                self.pending.insert(
                    id.to_string(),
                    PendingRequest {
                        span: None,
                        method: method.to_string(),
                        session_id: Some(session_id),
                        start: now,
                    },
                );
            }
            m if acp::is_fs_or_terminal_method(m) => {
                let session_id = acp::extract_session_id(params).map(|s| s.to_string());
                let span_name = format!("execute_tool {m}");
                let mut attrs = vec![
                    KeyValue::new("gen_ai.operation.name", "execute_tool"),
                    KeyValue::new("gen_ai.tool.name", m.to_string()),
                    KeyValue::new("gen_ai.tool.call.id", id.to_string()),
                    KeyValue::new("gen_ai.tool.type", "function"),
                    KeyValue::new("acp.method.name", m.to_string()),
                    KeyValue::new("network.transport", "pipe"),
                ];
                if let Some(ref sid) = session_id {
                    attrs.push(KeyValue::new("gen_ai.conversation.id", sid.clone()));
                }
                if self.record_content {
                    attrs.push(KeyValue::new(
                        "gen_ai.tool.call.arguments",
                        params.to_string(),
                    ));
                }
                let builder = self
                    .tracer
                    .span_builder(span_name)
                    .with_kind(SpanKind::Internal)
                    .with_attributes(attrs);
                let span = match session_id
                    .as_deref()
                    .and_then(|sid| self.parent_context_for_session(sid))
                {
                    Some(cx) => builder.start_with_context(&self.tracer, &cx),
                    None => builder.start(&self.tracer),
                };
                self.pending.insert(
                    id.to_string(),
                    PendingRequest {
                        span: Some(span),
                        method: m.to_string(),
                        session_id,
                        start: Instant::now(),
                    },
                );
            }
            _ => {
                // Other requests: session/new, session/load, authenticate, etc.
                let span = self.start_under_root(
                    self.tracer
                        .span_builder(method.to_string())
                        .with_kind(SpanKind::Internal)
                        .with_attributes(vec![
                            KeyValue::new("rpc.system", "jsonrpc"),
                            KeyValue::new("rpc.method", method.to_string()),
                            KeyValue::new("acp.method.name", method.to_string()),
                            KeyValue::new("network.transport", "pipe"),
                            KeyValue::new("jsonrpc.request.id", id.to_string()),
                        ]),
                );
                self.pending.insert(
                    id.to_string(),
                    PendingRequest {
                        span: Some(span),
                        method: method.to_string(),
                        session_id: acp::extract_session_id(params).map(|s| s.to_string()),
                        start: Instant::now(),
                    },
                );
            }
        }
    }

    fn handle_response(&mut self, id: Value, result: Option<&Value>, error: Option<&Value>) {
        let key = id.to_string();
        let pending = match self.pending.remove(&key) {
            Some(p) => p,
            None => return,
        };

        tracing::debug!(method = %pending.method, "response");

        match pending.method.as_str() {
            "initialize" => {
                if let Some(mut span) = pending.span {
                    if let Some(res) = result {
                        if let Some((name, version)) = acp::extract_agent_info(res) {
                            self.agent_name = Some(name.to_string());
                            self.agent_version = version.map(|v| v.to_string());
                            span.set_attribute(KeyValue::new(
                                "gen_ai.agent.name",
                                name.to_string(),
                            ));
                            span.set_attribute(KeyValue::new("gen_ai.agent.id", name.to_string()));
                        }
                        self.protocol_version = res.get("protocolVersion").and_then(|v| v.as_i64());
                        if let Some(pv) = self.protocol_version {
                            span.set_attribute(KeyValue::new("acp.protocol.version", pv));
                        }
                    }
                    if let Some(err) = error {
                        span.set_status(Status::error(err.to_string()));
                        span.set_attribute(KeyValue::new(
                            "error.type",
                            err.get("code")
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "_OTHER".to_string()),
                        ));
                    }
                    // Update root session span with agent info
                    if let Some(ref name) = self.agent_name {
                        if let Some(ref mut root) = self.session_span {
                            root.set_attribute(KeyValue::new("gen_ai.agent.name", name.clone()));
                        }
                    }
                    span.end();
                }
            }
            "session/prompt" => {
                if let Some(ref session_id) = pending.session_id {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        if let Some(mut span) = session.prompt_span.take() {
                            let duration = pending.start.elapsed().as_secs_f64();
                            if let Some(res) = result {
                                if let Some(reason) = acp::extract_stop_reason(res) {
                                    span.set_attribute(KeyValue::new(
                                        "gen_ai.response.finish_reasons",
                                        format!("[\"{reason}\"]"),
                                    ));
                                    if self.record_content && !session.accumulated_output.is_empty()
                                    {
                                        let finish = acp::map_stop_reason_to_finish_reason(reason);
                                        let output_msg = serde_json::json!([{
                                            "role": "assistant",
                                            "parts": [{"type": "text", "content": &session.accumulated_output}],
                                            "finish_reason": finish
                                        }]);
                                        span.set_attribute(KeyValue::new(
                                            "gen_ai.output.messages",
                                            output_msg.to_string(),
                                        ));
                                    }
                                }
                            }
                            if self.record_content
                                && !session.accumulated_output.is_empty()
                                && result.and_then(|r| acp::extract_stop_reason(r)).is_none()
                            {
                                // No stop reason available — emit without finish_reason
                                let output_msg = serde_json::json!([{
                                    "role": "assistant",
                                    "parts": [{"type": "text", "content": &session.accumulated_output}]
                                }]);
                                span.set_attribute(KeyValue::new(
                                    "gen_ai.output.messages",
                                    output_msg.to_string(),
                                ));
                            }
                            if let Some(first) = session.first_chunk_time {
                                if let Some(start) = session.prompt_start {
                                    let ttft = first.duration_since(start).as_secs_f64();
                                    span.set_attribute(KeyValue::new(
                                        "acp.time_to_first_token_ms",
                                        (ttft * 1000.0) as i64,
                                    ));
                                    self.ttft_histogram.record(
                                        ttft,
                                        &[KeyValue::new("gen_ai.operation.name", "invoke_agent")],
                                    );
                                }
                            }
                            if let Some(err) = error {
                                span.set_status(Status::error(err.to_string()));
                                span.set_attribute(KeyValue::new(
                                    "error.type",
                                    err.get("code")
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "_OTHER".to_string()),
                                ));
                            }
                            span.end();
                            self.duration_histogram.record(
                                duration,
                                &[KeyValue::new("gen_ai.operation.name", "invoke_agent")],
                            );
                        }
                    }
                }
            }
            m if acp::is_fs_or_terminal_method(m) => {
                if let Some(mut span) = pending.span {
                    if self.record_content {
                        if let Some(res) = result {
                            span.set_attribute(KeyValue::new(
                                "gen_ai.tool.call.result",
                                res.to_string(),
                            ));
                        }
                    }
                    if let Some(err) = error {
                        span.set_status(Status::error(err.to_string()));
                        span.set_attribute(KeyValue::new(
                            "error.type",
                            err.get("code")
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "_OTHER".to_string()),
                        ));
                    }
                    span.end();
                }
            }
            _ => {
                if let Some(mut span) = pending.span {
                    if let Some(err) = error {
                        span.set_status(Status::error(err.to_string()));
                    }
                    span.end();
                }
            }
        }
    }

    /// Get a parent Context for creating child spans under the active prompt span.
    fn parent_context_for_session(&self, session_id: &str) -> Option<Context> {
        self.sessions
            .get(session_id)
            .and_then(|s| s.prompt_span_context.as_ref())
            .map(|sc| Context::new().with_remote_span_context(sc.clone()))
    }

    /// Get the root session context for parenting top-level spans.
    fn root_context(&self) -> Option<Context> {
        self.session_span_context
            .as_ref()
            .map(|sc| Context::new().with_remote_span_context(sc.clone()))
    }

    /// Start a span as a child of the root session span (or as root if none exists).
    fn start_under_root(
        &self,
        builder: opentelemetry::trace::SpanBuilder,
    ) -> opentelemetry::global::BoxedSpan {
        match self.root_context() {
            Some(cx) => builder.start_with_context(&self.tracer, &cx),
            None => builder.start(&self.tracer),
        }
    }

    fn handle_notification(&mut self, _direction: Direction, method: &str, params: &Value) {
        if method != "session/update" {
            return;
        }

        let session_id = match acp::extract_session_id(params) {
            Some(s) => s.to_string(),
            None => return,
        };
        let update_type = match acp::extract_update_type(params) {
            Some(t) => t.to_string(),
            None => return,
        };

        tracing::debug!(session = %session_id, update = %update_type, "notification");

        match update_type.as_str() {
            "agent_message_chunk" => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    if session.first_chunk_time.is_none() {
                        session.first_chunk_time = Some(Instant::now());
                    }
                    if let Some(text) = acp::extract_chunk_text(params) {
                        session.accumulated_output.push_str(text);
                    }
                }
            }
            "tool_call" => {
                let tool_call_id = match acp::extract_tool_call_id(params) {
                    Some(id) => id.to_string(),
                    None => return,
                };
                let title = acp::extract_tool_call_title(params).unwrap_or("unknown tool");
                let kind = acp::extract_tool_call_kind(params).unwrap_or("other");
                let span_name = format!("execute_tool {title}");
                let mut attrs = vec![
                    KeyValue::new("gen_ai.operation.name", "execute_tool"),
                    KeyValue::new("gen_ai.tool.name", title.to_string()),
                    KeyValue::new("gen_ai.tool.call.id", tool_call_id.clone()),
                    KeyValue::new("gen_ai.tool.type", acp::map_tool_kind_to_type(kind)),
                    KeyValue::new("gen_ai.conversation.id", session_id.clone()),
                    KeyValue::new("acp.method.name", "session/update"),
                    KeyValue::new("acp.tool.kind", kind.to_string()),
                    KeyValue::new("network.transport", "pipe"),
                ];
                if self.record_content {
                    if let Some(raw) = params.get("update").and_then(|u| u.get("rawInput")) {
                        attrs.push(KeyValue::new("gen_ai.tool.call.arguments", raw.to_string()));
                    }
                }
                let builder = self
                    .tracer
                    .span_builder(span_name)
                    .with_kind(SpanKind::Internal)
                    .with_attributes(attrs);
                let span = match self.parent_context_for_session(&session_id) {
                    Some(cx) => builder.start_with_context(&self.tracer, &cx),
                    None => builder.start(&self.tracer),
                };
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.tool_spans.insert(tool_call_id, span);
                }
            }
            "tool_call_update" => {
                let tool_call_id = match acp::extract_tool_call_id(params) {
                    Some(id) => id.to_string(),
                    None => return,
                };
                let status = acp::extract_tool_call_status(params).unwrap_or("");
                if status == "completed" || status == "failed" {
                    if let Some(session) = self.sessions.get_mut(&session_id) {
                        if let Some(mut span) = session.tool_spans.remove(&tool_call_id) {
                            if status == "failed" {
                                span.set_status(Status::error("tool call failed"));
                                span.set_attribute(KeyValue::new("error.type", "tool_error"));
                            }
                            if self.record_content {
                                if let Some(raw) =
                                    params.get("update").and_then(|u| u.get("rawOutput"))
                                {
                                    span.set_attribute(KeyValue::new(
                                        "gen_ai.tool.call.result",
                                        raw.to_string(),
                                    ));
                                }
                            }
                            span.end();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn shutdown(&mut self) {
        // End any lingering spans
        for (_, mut session) in self.sessions.drain() {
            if let Some(mut span) = session.prompt_span.take() {
                span.set_status(Status::error("session ended unexpectedly"));
                span.end();
            }
            for (_, mut span) in session.tool_spans.drain() {
                span.set_status(Status::error("session ended unexpectedly"));
                span.end();
            }
        }
        for (_, pending) in self.pending.drain() {
            if let Some(mut span) = pending.span {
                span.set_status(Status::error("process exited before response"));
                span.end();
            }
        }
        // End the root session span last
        if let Some(mut root) = self.session_span.take() {
            root.end();
        }
    }
}

use opentelemetry::trace::{Span, SpanKind, TraceContextExt, Tracer, TracerProvider};
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct TestExporter {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl TestExporter {
    fn new() -> Self {
        Self {
            spans: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn spans(&self) -> Vec<SpanData> {
        self.spans.lock().unwrap().clone()
    }
}

impl opentelemetry_sdk::trace::SpanExporter for TestExporter {
    fn export(
        &mut self,
        batch: Vec<SpanData>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send>,
    > {
        self.spans.lock().unwrap().extend(batch);
        Box::pin(std::future::ready(Ok(())))
    }
}

fn setup() -> (SdkTracerProvider, TestExporter) {
    let exporter = TestExporter::new();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    (provider, exporter)
}

#[test]
fn child_span_shares_trace_id_with_parent_via_remote_context() {
    let (provider, exporter) = setup();
    let tracer = provider.tracer("test");

    let parent = tracer
        .span_builder("invoke_agent")
        .with_kind(SpanKind::Client)
        .start(&tracer);
    let parent_sc = parent.span_context().clone();
    let parent_trace_id = parent_sc.trace_id();
    let parent_span_id = parent_sc.span_id();

    // Same technique used in our span hierarchy fix
    let parent_cx = opentelemetry::Context::new().with_remote_span_context(parent_sc);
    let child = tracer
        .span_builder("execute_tool")
        .with_kind(SpanKind::Internal)
        .start_with_context(&tracer, &parent_cx);
    let child_trace_id = child.span_context().trace_id();

    assert_eq!(
        parent_trace_id, child_trace_id,
        "child must share parent trace ID"
    );

    drop(child);
    drop(parent);
    let _ = provider.force_flush();

    let spans = exporter.spans();
    assert_eq!(spans.len(), 2);
    let child_data = spans.iter().find(|s| s.name == "execute_tool").unwrap();
    assert_eq!(
        child_data.parent_span_id, parent_span_id,
        "child parent_span_id must match"
    );
}

#[test]
fn output_message_includes_finish_reason() {
    use acp_traces::acp::map_stop_reason_to_finish_reason;

    let finish = map_stop_reason_to_finish_reason("end_turn");
    let output_msg = serde_json::json!([{
        "role": "assistant",
        "parts": [{"type": "text", "content": "Hello"}],
        "finish_reason": finish
    }]);

    let msg = &output_msg[0];
    assert_eq!(msg["role"], "assistant");
    assert_eq!(msg["finish_reason"], "stop");
    assert_eq!(msg["parts"][0]["type"], "text");
}

#[test]
fn finish_reason_mappings() {
    use acp_traces::acp::map_stop_reason_to_finish_reason;
    assert_eq!(map_stop_reason_to_finish_reason("end_turn"), "stop");
    assert_eq!(map_stop_reason_to_finish_reason("max_tokens"), "length");
    assert_eq!(
        map_stop_reason_to_finish_reason("max_turn_requests"),
        "length"
    );
    assert_eq!(
        map_stop_reason_to_finish_reason("refusal"),
        "content_filter"
    );
    assert_eq!(map_stop_reason_to_finish_reason("cancelled"), "cancelled");
}

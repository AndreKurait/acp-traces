use acp_traces::{acp::Direction, spans::SpanManager};
use opentelemetry::{global::BoxedTracer, metrics::MeterProvider, trace::TracerProvider};
use opentelemetry_sdk::{
    metrics::SdkMeterProvider,
    trace::{SdkTracerProvider, SpanData},
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
struct TestExporter(Arc<Mutex<Vec<SpanData>>>);

impl opentelemetry_sdk::trace::SpanExporter for TestExporter {
    async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.lock().unwrap().extend(batch);
        Ok(())
    }
}

fn setup(record_content: bool) -> (SpanManager, TestExporter, SdkTracerProvider) {
    let exporter = TestExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = BoxedTracer::new(Box::new(provider.tracer("test")));
    let meter = SdkMeterProvider::default().meter("test");
    (
        SpanManager::new(tracer, meter, record_content),
        exporter,
        provider,
    )
}

#[test]
fn bidirectional_requests_with_the_same_id_complete_independently() {
    let (mut mgr, exporter, _provider) = setup(false);
    mgr.process_message(
        Direction::EditorToAgent,
        r#"{"id":1,"method":"session/prompt","params":{"sessionId":"s1","prompt":[]}}"#,
    );
    mgr.process_message(
        Direction::AgentToEditor,
        r#"{"id":1,"method":"fs/read_text_file","params":{"sessionId":"s1","path":"/test"}}"#,
    );
    mgr.process_message(
        Direction::EditorToAgent,
        r#"{"id":1,"result":{"content":"data"}}"#,
    );
    mgr.process_message(
        Direction::AgentToEditor,
        r#"{"id":1,"result":{"stopReason":"end_turn"}}"#,
    );
    mgr.shutdown();

    let spans = exporter.0.lock().unwrap();
    assert_eq!(spans.len(), 2);
    assert!(spans
        .iter()
        .all(|s| s.status == opentelemetry::trace::Status::Unset));
    let prompt = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
    let tool = spans
        .iter()
        .find(|s| s.name == "execute_tool fs/read_text_file")
        .unwrap();
    assert_eq!(tool.parent_span_id, prompt.span_context.span_id());
    assert_eq!(tool.span_context.trace_id(), prompt.span_context.trace_id());
}

#[test]
fn default_mode_omits_content_and_error_payloads() {
    let (mut mgr, exporter, _provider) = setup(false);
    mgr.process_message(Direction::EditorToAgent,
        r#"{"id":1,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"secret prompt"}]}}"#);
    mgr.process_message(Direction::AgentToEditor,
        r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"secret output"}}}}"#);
    mgr.process_message(Direction::AgentToEditor,
        r#"{"id":2,"method":"fs/write_text_file","params":{"sessionId":"s1","content":"secret file"}}"#);
    mgr.process_message(Direction::EditorToAgent,
        r#"{"id":2,"error":{"code":-32602,"message":"secret file error","data":{"token":"secret token"}}}"#);
    mgr.process_message(
        Direction::AgentToEditor,
        r#"{"id":1,"error":{"code":-32603,"message":"secret prompt error"}}"#,
    );
    mgr.shutdown();

    let spans = exporter.0.lock().unwrap();
    assert_eq!(spans.len(), 2);
    let exported = format!("{spans:?}");
    assert!(
        !exported.contains("secret"),
        "content leaked into exported spans: {exported}"
    );
    assert!(exported.contains("JSON-RPC error -32602"));
    assert!(exported.contains("JSON-RPC error -32603"));
}

#[test]
fn record_content_is_an_explicit_opt_in() {
    let (mut mgr, exporter, _provider) = setup(true);
    mgr.process_message(Direction::EditorToAgent,
        r#"{"id":1,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"test prompt"}]}}"#);
    mgr.process_message(Direction::AgentToEditor,
        r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"test output"}}}}"#);
    mgr.process_message(
        Direction::AgentToEditor,
        r#"{"id":1,"result":{"stopReason":"end_turn"}}"#,
    );
    mgr.shutdown();

    let spans = exporter.0.lock().unwrap();
    let prompt = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
    let attribute = |name: &str| {
        prompt
            .attributes
            .iter()
            .find(|a| a.key.as_str() == name)
            .unwrap()
            .value
            .to_string()
    };
    let input: serde_json::Value =
        serde_json::from_str(&attribute("gen_ai.input.messages")).unwrap();
    let output: serde_json::Value =
        serde_json::from_str(&attribute("gen_ai.output.messages")).unwrap();
    assert_eq!(input[0]["parts"][0]["content"], "test prompt");
    assert_eq!(output[0]["parts"][0]["content"], "test output");
    assert_eq!(output[0]["finish_reason"], "stop");
    let reasons = prompt
        .attributes
        .iter()
        .find(|a| a.key.as_str() == "gen_ai.response.finish_reasons")
        .unwrap();
    assert_eq!(
        reasons.value,
        opentelemetry::Value::Array(opentelemetry::Array::String(vec!["end_turn".into()]))
    );
}

#[test]
fn tools_after_a_completed_prompt_do_not_inherit_its_span() {
    let (mut mgr, exporter, _provider) = setup(false);
    mgr.process_message(
        Direction::EditorToAgent,
        r#"{"id":1,"method":"session/prompt","params":{"sessionId":"s1","prompt":[]}}"#,
    );
    mgr.process_message(
        Direction::AgentToEditor,
        r#"{"id":1,"result":{"stopReason":"end_turn"}}"#,
    );
    mgr.process_message(
        Direction::AgentToEditor,
        r#"{"id":2,"method":"fs/read_text_file","params":{"sessionId":"s1"}}"#,
    );
    mgr.process_message(Direction::EditorToAgent, r#"{"id":2,"result":{}}"#);
    mgr.shutdown();
    let spans = exporter.0.lock().unwrap();
    let tool = spans
        .iter()
        .find(|s| s.name == "execute_tool fs/read_text_file")
        .unwrap();
    assert_eq!(tool.parent_span_id, opentelemetry::trace::SpanId::INVALID);
}

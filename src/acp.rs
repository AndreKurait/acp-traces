use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    EditorToAgent,
    AgentToEditor,
}

#[derive(Debug)]
pub enum MessageType {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<Value>,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub fn parse(line: &str) -> Option<MessageType> {
    let v: Value = serde_json::from_str(line).ok()?;
    let obj = v.as_object()?;

    if let Some(method) = obj.get("method").and_then(|m| m.as_str()) {
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = obj.get("id") {
            Some(MessageType::Request {
                id: id.clone(),
                method: method.to_string(),
                params,
            })
        } else {
            Some(MessageType::Notification {
                method: method.to_string(),
                params,
            })
        }
    } else {
        obj.get("id").map(|id| MessageType::Response {
            id: id.clone(),
            result: obj.get("result").cloned(),
            error: obj.get("error").cloned(),
        })
    }
}

pub fn extract_session_id(params: &Value) -> Option<&str> {
    params.get("sessionId").and_then(|v| v.as_str())
}

pub fn extract_prompt_text(params: &Value) -> Option<String> {
    let prompt = params.get("prompt")?.as_array()?;
    let texts: Vec<&str> = prompt
        .iter()
        .filter_map(|block| {
            if block.get("type")?.as_str()? == "text" {
                block.get("text")?.as_str()
            } else {
                None
            }
        })
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

pub fn extract_update_type(params: &Value) -> Option<&str> {
    params.get("update")?.get("sessionUpdate")?.as_str()
}

pub fn extract_chunk_text(params: &Value) -> Option<&str> {
    params.get("update")?.get("content")?.get("text")?.as_str()
}

pub fn extract_tool_call_id(params: &Value) -> Option<&str> {
    params.get("update")?.get("toolCallId")?.as_str()
}

pub fn extract_tool_call_title(params: &Value) -> Option<&str> {
    params.get("update")?.get("title")?.as_str()
}

pub fn extract_tool_call_kind(params: &Value) -> Option<&str> {
    params.get("update")?.get("kind")?.as_str()
}

pub fn extract_tool_call_status(params: &Value) -> Option<&str> {
    params.get("update")?.get("status")?.as_str()
}

pub fn extract_agent_info(result: &Value) -> Option<(&str, Option<&str>)> {
    let info = result.get("agentInfo")?;
    let name = info.get("name")?.as_str()?;
    let version = info.get("version").and_then(|v| v.as_str());
    Some((name, version))
}

pub fn extract_client_info(params: &Value) -> Option<(&str, Option<&str>)> {
    let info = params.get("clientInfo")?;
    let name = info.get("name")?.as_str()?;
    let version = info.get("version").and_then(|v| v.as_str());
    Some((name, version))
}

pub fn extract_stop_reason(result: &Value) -> Option<&str> {
    result.get("stopReason")?.as_str()
}

pub fn map_tool_kind_to_type(kind: &str) -> &'static str {
    match kind {
        "read" | "search" | "fetch" => "datastore",
        "edit" | "delete" | "move" | "execute" | "think" | "other" => "extension",
        _ => "extension",
    }
}

pub fn is_fs_or_terminal_method(method: &str) -> bool {
    matches!(
        method,
        "fs/read_text_file"
            | "fs/write_text_file"
            | "terminal/create"
            | "terminal/write"
            | "terminal/resize"
            | "terminal/release"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request() {
        let line =
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#;
        match parse(line).unwrap() {
            MessageType::Request { id, method, params } => {
                assert_eq!(id, 1);
                assert_eq!(method, "initialize");
                assert_eq!(params["protocolVersion"], 1);
            }
            _ => panic!("expected request"),
        }
    }

    #[test]
    fn parse_response() {
        let line = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#;
        match parse(line).unwrap() {
            MessageType::Response { id, result, error } => {
                assert_eq!(id, 1);
                assert!(result.is_some());
                assert!(error.is_none());
            }
            _ => panic!("expected response"),
        }
    }

    #[test]
    fn parse_notification() {
        let line = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}"#;
        match parse(line).unwrap() {
            MessageType::Notification { method, params } => {
                assert_eq!(method, "session/update");
                assert_eq!(extract_update_type(&params), Some("agent_message_chunk"));
                assert_eq!(extract_chunk_text(&params), Some("hello"));
            }
            _ => panic!("expected notification"),
        }
    }

    #[test]
    fn parse_error_response() {
        let line =
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"Invalid Request"}}"#;
        match parse(line).unwrap() {
            MessageType::Response { error, .. } => {
                let err = error.unwrap();
                assert_eq!(err["code"], -32600);
            }
            _ => panic!("expected response"),
        }
    }

    #[test]
    fn extract_prompt() {
        let params: Value = serde_json::from_str(r#"{"sessionId":"s1","prompt":[{"type":"text","text":"fix the bug"},{"type":"resource","resource":{"uri":"file:///main.rs","text":"fn main() {}"}}]}"#).unwrap();
        assert_eq!(
            extract_prompt_text(&params),
            Some("fix the bug".to_string())
        );
        assert_eq!(extract_session_id(&params), Some("s1"));
    }

    #[test]
    fn tool_kind_mapping() {
        assert_eq!(map_tool_kind_to_type("read"), "datastore");
        assert_eq!(map_tool_kind_to_type("search"), "datastore");
        assert_eq!(map_tool_kind_to_type("fetch"), "datastore");
        assert_eq!(map_tool_kind_to_type("edit"), "extension");
        assert_eq!(map_tool_kind_to_type("think"), "extension");
        assert_eq!(map_tool_kind_to_type("execute"), "extension");
        assert_eq!(map_tool_kind_to_type("unknown"), "extension");
    }

    #[test]
    fn agent_info_extraction() {
        let result: Value = serde_json::from_str(r#"{"protocolVersion":1,"agentInfo":{"name":"kiro","title":"Kiro","version":"1.25.0"}}"#).unwrap();
        let (name, version) = extract_agent_info(&result).unwrap();
        assert_eq!(name, "kiro");
        assert_eq!(version, Some("1.25.0"));
    }

    #[test]
    fn fs_method_detection() {
        assert!(is_fs_or_terminal_method("fs/read_text_file"));
        assert!(is_fs_or_terminal_method("terminal/create"));
        assert!(!is_fs_or_terminal_method("session/prompt"));
    }
}

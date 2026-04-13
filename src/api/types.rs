use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    /// Ollama-specific: enable thinking/reasoning mode for supported models
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub think: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// Response and delta types are populated by serde deserialization;
// not all fields are read by application code, but they must exist
// for correct JSON mapping.
#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct ChatResponse {
    pub id: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct Choice {
    pub index: u32,
    pub message: Option<Message>,
    pub delta: Option<DeltaMessage>,
    pub finish_reason: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct DeltaMessage {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct DeltaToolCall {
    pub index: u32,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<DeltaFunctionCall>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DeltaFunctionCall {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ModelInfo {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_chat_request_without_tools() {
        let req = ChatRequest {
            model: "llama3:8b".into(),
            messages: vec![Message {
                role: "user".into(),
                content: Some("hello".into()),
                tool_calls: None,
                tool_call_id: None,
            }],
            stream: true,
            tools: None,
            think: false,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("tools").is_none());
        assert_eq!(json["stream"], true);
    }

    #[test]
    fn deserialize_streaming_delta() {
        let json = r#"{"id":"chatcmpl-123","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        let delta = resp.choices[0].delta.as_ref().unwrap();
        assert_eq!(delta.content.as_deref(), Some("Hello"));
    }

    #[test]
    fn deserialize_tool_call_delta() {
        let json = r#"{"id":"chatcmpl-456","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"shell","arguments":""}}]},"finish_reason":null}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        let tc = &resp.choices[0]
            .delta
            .as_ref()
            .unwrap()
            .tool_calls
            .as_ref()
            .unwrap()[0];
        assert_eq!(tc.id.as_deref(), Some("call_abc"));
        assert_eq!(tc.function.as_ref().unwrap().name.as_deref(), Some("shell"));
    }

    #[test]
    fn deserialize_models_response() {
        let json = r#"{"data":[{"id":"llama3:8b"},{"id":"codellama:13b"}]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].id, "llama3:8b");
    }

    #[test]
    fn think_true_serializes() {
        let req = ChatRequest {
            model: "llama3:8b".into(),
            messages: vec![],
            stream: true,
            tools: None,
            think: true,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["think"], true);
    }

    #[test]
    fn think_false_is_omitted() {
        let req = ChatRequest {
            model: "llama3:8b".into(),
            messages: vec![],
            stream: true,
            tools: None,
            think: false,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("think").is_none());
    }

    #[test]
    fn message_optional_fields_omitted() {
        let msg = Message {
            role: "user".into(),
            content: Some("hello".into()),
            tool_calls: None,
            tool_call_id: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json.get("tool_calls").is_none());
        assert!(json.get("tool_call_id").is_none());
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "hello");
    }

    #[test]
    fn message_with_tool_call_id() {
        let msg = Message {
            role: "tool".into(),
            content: Some("result".into()),
            tool_calls: None,
            tool_call_id: Some("call_123".into()),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["tool_call_id"], "call_123");
    }

    #[test]
    fn tool_definition_roundtrip() {
        let def = ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "test".into(),
                description: "A test tool".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
        };
        let json = serde_json::to_string(&def).unwrap();
        let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.function.name, "test");
        assert_eq!(parsed.tool_type, "function");
    }

    #[test]
    fn tool_call_serialization() {
        let tc = ToolCall {
            id: "call_abc".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command":"ls"}"#.into(),
            },
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["id"], "call_abc");
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "shell");
    }

    #[test]
    fn deserialize_usage() {
        let json = r#"{"prompt_tokens":142,"completion_tokens":87,"total_tokens":229}"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.prompt_tokens, 142);
        assert_eq!(usage.completion_tokens, 87);
        assert_eq!(usage.total_tokens, 229);
    }

    #[test]
    fn chat_response_with_usage() {
        let json = r#"{"id":"1","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn chat_response_without_usage() {
        let json = r#"{"id":"1","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert!(resp.usage.is_none());
    }
}

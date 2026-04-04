use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
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

#[derive(Debug, Deserialize, Clone)]
pub struct ChatResponse {
    pub id: String,
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Choice {
    pub index: u32,
    pub message: Option<Message>,
    pub delta: Option<DeltaMessage>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DeltaMessage {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

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
        let tc = &resp.choices[0].delta.as_ref().unwrap().tool_calls.as_ref().unwrap()[0];
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
}

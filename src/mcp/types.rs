use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct McpToolsResult {
    pub tools: Vec<McpToolInfo>,
}

// McpCallResult and McpContent are populated by serde deserialization
// in the transport impls (serde_json::from_value); not all fields are
// read by application code.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct McpCallResult {
    pub content: Vec<McpContent>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_jsonrpc_request() {
        let req = JsonRpcRequest::new(1, "tools/list", None);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "tools/list");
        assert!(json.get("params").is_none());
    }

    #[test]
    fn deserialize_tools_result() {
        let json = r#"{"tools":[{"name":"read","description":"Read a file","inputSchema":{"type":"object","properties":{"path":{"type":"string"}}}}]}"#;
        let result: McpToolsResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.tools[0].name, "read");
    }

    #[test]
    fn deserialize_call_result() {
        let json = r#"{"content":[{"type":"text","text":"file contents here"}]}"#;
        let result: McpCallResult = serde_json::from_str(json).unwrap();
        assert_eq!(
            result.content[0].text.as_deref(),
            Some("file contents here")
        );
    }
}

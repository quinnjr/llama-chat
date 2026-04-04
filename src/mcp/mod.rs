pub mod types;
pub mod stdio;
pub mod sse;
pub mod http;

use anyhow::Result;
use async_trait::async_trait;

use crate::api::types::{ToolDefinition, FunctionDefinition};
use crate::config::mcp_config::McpServerEntry;
use types::McpToolInfo;

#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn initialize(&mut self) -> Result<()>;
    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>>;
    async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String>;
}

pub struct McpServer {
    pub name: String,
    pub transport: Box<dyn McpTransport>,
    pub tools: Vec<McpToolInfo>,
}

impl McpServer {
    pub async fn connect(name: String, entry: &McpServerEntry) -> Result<Self> {
        let mut transport: Box<dyn McpTransport> = match entry.detected_transport() {
            "stdio" => {
                let cmd = entry.command.as_deref().unwrap_or("echo");
                let args = entry.args.clone().unwrap_or_default();
                Box::new(stdio::StdioTransport::new(cmd, &args))
            }
            "sse" => {
                let url = entry.url.as_deref().unwrap_or("");
                Box::new(sse::SseTransport::new(url))
            }
            _ => {
                let url = entry.url.as_deref().unwrap_or("");
                Box::new(http::StreamableHttpTransport::new(url))
            }
        };

        transport.initialize().await?;
        let tools = transport.list_tools().await?;

        Ok(Self { name, transport, tools })
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: format!("mcp_{}_{}", self.name, t.name),
                description: t.description.clone().unwrap_or_default(),
                parameters: t.input_schema.clone(),
            },
        }).collect()
    }

    pub async fn call_tool(&mut self, tool_name: &str, arguments: serde_json::Value) -> Result<String> {
        self.transport.call_tool(tool_name, arguments).await
    }
}

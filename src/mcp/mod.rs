pub mod http;
pub mod sse;
pub mod stdio;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::mcp_config::McpServerEntry;
use types::McpToolInfo;

#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn initialize(&mut self) -> Result<()>;
    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>>;
    async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String>;
}

pub struct McpServer {
    #[allow(dead_code)]
    pub name: String,
    pub transport: Box<dyn McpTransport>,
    pub tools: Vec<McpToolInfo>,
}

impl McpServer {
    #[cfg(not(tarpaulin_include))]
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

        Ok(Self {
            name,
            transport,
            tools,
        })
    }

    #[cfg(not(tarpaulin_include))]
    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<String> {
        self.transport.call_tool(tool_name, arguments).await
    }
}

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mcp::McpTransport;
use crate::mcp::types::*;

#[cfg(not(tarpaulin_include))]
pub struct StreamableHttpTransport {
    url: String,
    http: Client,
    session_id: Option<String>,
    next_id: AtomicU64,
}

#[cfg(not(tarpaulin_include))]
impl StreamableHttpTransport {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.into(),
            http: Client::new(),
            session_id: None,
            next_id: AtomicU64::new(1),
        }
    }

    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest::new(id, method, params);

        let mut http_req = self.http.post(&self.url).json(&req);
        if let Some(ref sid) = self.session_id {
            http_req = http_req.header("Mcp-Session-Id", sid);
        }

        let resp = http_req
            .send()
            .await
            .context("failed to send MCP request")?;

        if let Some(sid) = resp.headers().get("Mcp-Session-Id") {
            self.session_id = sid.to_str().ok().map(String::from);
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await?;

        if content_type.contains("text/event-stream") {
            for line in body.lines() {
                if let Some(data) = line.strip_prefix("data: ")
                    && let Ok(rpc_resp) = serde_json::from_str::<JsonRpcResponse>(data)
                {
                    if let Some(ref err) = rpc_resp.error {
                        anyhow::bail!("MCP error {}: {}", err.code, err.message);
                    }
                    return Ok(rpc_resp);
                }
            }
            anyhow::bail!("no valid JSON-RPC response in SSE stream");
        } else {
            let rpc_resp: JsonRpcResponse =
                serde_json::from_str(&body).context("failed to parse MCP response")?;
            if let Some(ref err) = rpc_resp.error {
                anyhow::bail!("MCP error {}: {}", err.code, err.message);
            }
            Ok(rpc_resp)
        }
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl McpTransport for StreamableHttpTransport {
    async fn initialize(&mut self) -> Result<()> {
        self.send_request(
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "llama-chat", "version": "0.1.0" }
            })),
        )
        .await?;
        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        let resp = self.send_request("tools/list", None).await?;
        let result: McpToolsResult =
            serde_json::from_value(resp.result.context("no result in tools/list response")?)?;
        Ok(result.tools)
    }

    async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String> {
        let resp = self
            .send_request(
                "tools/call",
                Some(serde_json::json!({"name": name, "arguments": arguments})),
            )
            .await?;
        let result: McpCallResult =
            serde_json::from_value(resp.result.context("no result in tools/call response")?)?;
        Ok(result
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

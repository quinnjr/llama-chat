use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mcp::McpTransport;
use crate::mcp::types::*;

#[cfg(not(tarpaulin_include))]
pub struct SseTransport {
    url: String,
    messages_url: Option<String>,
    http: Client,
    next_id: AtomicU64,
}

#[cfg(not(tarpaulin_include))]
impl SseTransport {
    pub fn new(url: &str) -> Self {
        Self { url: url.into(), messages_url: None, http: Client::new(), next_id: AtomicU64::new(1) }
    }

    async fn send_request(&self, method: &str, params: Option<serde_json::Value>) -> Result<JsonRpcResponse> {
        let url = self.messages_url.as_deref().context("SSE transport not initialized")?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest::new(id, method, params);

        let resp = self.http.post(url).json(&req).send().await.context("failed to send MCP request")?;
        let body = resp.text().await?;

        for line in body.lines() {
            if let Some(data) = line.strip_prefix("data: ")
                && let Ok(rpc_resp) = serde_json::from_str::<JsonRpcResponse>(data) {
                    if let Some(ref err) = rpc_resp.error {
                        anyhow::bail!("MCP error {}: {}", err.code, err.message);
                    }
                    return Ok(rpc_resp);
                }
        }
        let rpc_resp: JsonRpcResponse = serde_json::from_str(&body).context("failed to parse MCP SSE response")?;
        Ok(rpc_resp)
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl McpTransport for SseTransport {
    async fn initialize(&mut self) -> Result<()> {
        let resp = self.http.get(&self.url).send().await.context("failed to connect to SSE endpoint")?;

        // Real SSE servers never close the connection, so we must not call
        // resp.text() — it would block forever. Instead, stream the bytes and
        // read line-by-line, breaking as soon as we find the endpoint URL.
        let mut byte_stream = resp.bytes_stream();
        let mut line_buf = String::new();

        'outer: while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk.context("SSE stream read error")?;
            line_buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(nl) = line_buf.find('\n') {
                let line = line_buf[..nl].trim_end_matches('\r').to_string();
                line_buf = line_buf[nl + 1..].to_string();

                if let Some(data) = line.strip_prefix("data: ")
                    && (data.starts_with("http") || data.starts_with('/')) {
                        self.messages_url = Some(if data.starts_with('/') {
                            let base = &self.url[..self.url.rfind('/').unwrap_or(self.url.len())];
                            format!("{}{}", base, data)
                        } else {
                            data.to_string()
                        });
                        break 'outer;
                    }
            }
        }

        if self.messages_url.is_none() {
            self.messages_url = Some(self.url.clone());
        }

        self.send_request("initialize", Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "llama-chat", "version": "0.1.0" }
        }))).await?;

        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        let resp = self.send_request("tools/list", None).await?;
        let result: McpToolsResult = serde_json::from_value(resp.result.context("no result in tools/list response")?)?;
        Ok(result.tools)
    }

    async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String> {
        let resp = self.send_request("tools/call", Some(serde_json::json!({"name": name, "arguments": arguments}))).await?;
        let result: McpCallResult = serde_json::from_value(resp.result.context("no result in tools/call response")?)?;
        Ok(result.content.iter().filter_map(|c| c.text.as_deref()).collect::<Vec<_>>().join("\n"))
    }
}

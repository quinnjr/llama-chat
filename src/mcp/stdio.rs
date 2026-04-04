use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mcp::McpTransport;
use crate::mcp::types::*;

#[cfg(not(tarpaulin_include))]
pub struct StdioTransport {
    command: String,
    args: Vec<String>,
    child: Option<Child>,
    stdin: Option<tokio::process::ChildStdin>,
    reader: Option<BufReader<tokio::process::ChildStdout>>,
    next_id: AtomicU64,
}

#[cfg(not(tarpaulin_include))]
impl StdioTransport {
    pub fn new(command: &str, args: &[String]) -> Self {
        Self {
            command: command.into(),
            args: args.to_vec(),
            child: None,
            stdin: None,
            reader: None,
            next_id: AtomicU64::new(1),
        }
    }

    async fn send_request(&mut self, method: &str, params: Option<serde_json::Value>) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest::new(id, method, params);
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        let stdin = self.stdin.as_mut().context("transport not initialized")?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;

        let reader = self.reader.as_mut().context("transport not initialized")?;
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;

        let resp: JsonRpcResponse = serde_json::from_str(&response_line)
            .context("failed to parse MCP response")?;

        if let Some(ref err) = resp.error {
            anyhow::bail!("MCP error {}: {}", err.code, err.message);
        }

        Ok(resp)
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl McpTransport for StdioTransport {
    async fn initialize(&mut self) -> Result<()> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context(format!("failed to spawn MCP server: {}", self.command))?;

        self.stdin = child.stdin.take();
        let stdout = child.stdout.take().context("no stdout from MCP server")?;
        self.reader = Some(BufReader::new(stdout));
        self.child = Some(child);

        self.send_request("initialize", Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "llama-chat", "version": "0.1.0" }
        }))).await?;

        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        let resp = self.send_request("tools/list", None).await?;
        let result: McpToolsResult = serde_json::from_value(
            resp.result.context("no result in tools/list response")?
        )?;
        Ok(result.tools)
    }

    async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String> {
        let resp = self.send_request("tools/call", Some(serde_json::json!({
            "name": name, "arguments": arguments
        }))).await?;

        let result: McpCallResult = serde_json::from_value(
            resp.result.context("no result in tools/call response")?
        )?;

        Ok(result.content.iter().filter_map(|c| c.text.as_deref()).collect::<Vec<_>>().join("\n"))
    }
}

#[cfg(not(tarpaulin_include))]
impl Drop for StdioTransport {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
        }
    }
}

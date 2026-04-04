use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use tokio::sync::mpsc;

use crate::api::stream::parse_sse_line;
use crate::api::types::*;
use crate::config::settings::ServerConfig;

pub struct ApiClient {
    http: Client,
    server: ServerConfig,
}

impl ApiClient {
    pub fn new(server: ServerConfig) -> Self {
        Self {
            http: Client::new(),
            server,
        }
    }

    pub fn server(&self) -> &ServerConfig {
        &self.server
    }

    pub fn set_server(&mut self, server: ServerConfig) {
        self.server = server;
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/models", self.server.url);
        let mut req = self.http.get(&url);
        if let Some(ref key) = self.server.api_key {
            req = req.bearer_auth(key);
        }
        let resp: ModelsResponse = req
            .send()
            .await
            .context("failed to connect to server")?
            .json()
            .await
            .context("failed to parse models response")?;
        Ok(resp.data.into_iter().map(|m| m.id).collect())
    }

    pub async fn chat_stream(
        &self,
        request: ChatRequest,
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<()> {
        let url = format!("{}/chat/completions", self.server.url);
        let mut req = self.http.post(&url).json(&request);
        if let Some(ref key) = self.server.api_key {
            req = req.bearer_auth(key);
        }
        let response = req
            .send()
            .await
            .context("failed to connect to server")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut done_sent = false;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if let Some(resp) = parse_sse_line(&line) {
                    for choice in &resp.choices {
                        if let Some(ref delta) = choice.delta {
                            if let Some(ref content) = delta.content {
                                let _ = tx.send(StreamEvent::Token(content.clone()));
                            }
                            if let Some(ref tool_calls) = delta.tool_calls {
                                for tc in tool_calls {
                                    let _ = tx.send(StreamEvent::ToolCallDelta(tc.clone()));
                                }
                            }
                        }
                        if choice.finish_reason.is_some() {
                            let _ = tx.send(StreamEvent::Done);
                            done_sent = true;
                        }
                    }
                }
            }
        }

        // Fallback: stream ended without a finish_reason (e.g. connection drop)
        if !done_sent {
            let _ = tx.send(StreamEvent::Done);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Token(String),
    ToolCallDelta(DeltaToolCall),
    Done,
}

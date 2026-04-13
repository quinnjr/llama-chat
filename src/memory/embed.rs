//! OpenAI-compatible embeddings client.
//!
//! POSTs to `{server_url}/embeddings` with `{"model":"…","input":["…", …]}`
//! and returns `Vec<Vec<f32>>` in the same order.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::settings::ServerConfig;
use crate::memory::types::MemoryError;

#[derive(Clone)]
pub struct EmbeddingClient {
    http: Client,
    server: ServerConfig,
    model: String,
}

impl EmbeddingClient {
    pub fn new(server: ServerConfig, model: String) -> Self {
        Self { http: Client::new(), server, model }
    }

    pub fn model(&self) -> &str { &self.model }

    /// Embed one or more inputs. Returns `None` on any non-fatal failure
    /// (network, HTTP error, malformed JSON) so callers can gracefully
    /// fall back to FTS-only retrieval.
    pub async fn embed(&self, inputs: Vec<String>) -> Result<Option<Vec<Vec<f32>>>, MemoryError> {
        if inputs.is_empty() {
            return Ok(Some(vec![]));
        }
        let url = format!("{}/embeddings", self.server.url);
        let body = EmbedRequest { model: &self.model, input: &inputs };
        let mut req = self.http.post(&url).json(&body);
        if let Some(ref key) = self.server.api_key {
            req = req.bearer_auth(key);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[memory] embed transport error: {e}");
                return Ok(None);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            eprintln!("[memory] embed http {status}: {body}");
            return Ok(None);
        }

        let parsed: EmbedResponse = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[memory] embed parse: {e}");
                return Ok(None);
            }
        };

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(parsed.data.len());
        out.resize(parsed.data.len(), Vec::new());
        for item in parsed.data {
            if item.index < out.len() {
                out[item.index] = item.embedding;
            }
        }
        Ok(Some(out))
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

#[derive(Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
    index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty_ok() {
        // Smoke test of the short-circuit branch; no network.
        let server = ServerConfig {
            name: "t".into(), url: "http://127.0.0.1:0".into(), api_key: None,
        };
        let client = EmbeddingClient::new(server, "m".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(client.embed(vec![])).unwrap().unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn model_accessor_returns_model() {
        let server = ServerConfig { name: "t".into(), url: "u".into(), api_key: None };
        let client = EmbeddingClient::new(server, "nomic-embed-text".into());
        assert_eq!(client.model(), "nomic-embed-text");
    }
}

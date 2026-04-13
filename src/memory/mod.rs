//! Long-term memory: curated facts + conversation archive.

pub mod types;

mod chunk;
mod commands;
mod embed;
mod extract;
mod retrieval;
mod schema;
mod store;

use std::path::Path;
use std::sync::Arc;

use rusqlite::params;

pub use types::{Kind, MemoryError, Paths, RetrievedItem, Scope, Source};

use crate::config::settings::{AppConfig, ServerConfig};
use crate::memory::embed::EmbeddingClient;
use crate::memory::store::{Store, default_paths};
use crate::memory::types::now;

/// Public facade. All app-side interactions go through this type.
pub struct MemoryService {
    global: Arc<Store>,
    project: Arc<Store>,
    embed: EmbeddingClient,
    top_n: usize,
    decay_half_life_days: u32,
    embedding_dim: i64,
}

impl MemoryService {
    pub async fn open(
        config: &AppConfig,
        project_dir: &Path,
    ) -> Result<Self, MemoryError> {
        if !config.memory.enabled {
            return Err(MemoryError::Disabled("memory.enabled = false".into()));
        }
        if config.memory.embedding_model.is_empty() {
            return Err(MemoryError::Disabled("memory.embedding_model not set".into()));
        }
        let server: ServerConfig = config
            .servers
            .get(&config.memory.embedding_server)
            .cloned()
            .ok_or_else(|| MemoryError::Disabled(format!(
                "memory.embedding_server '{}' not in [servers]",
                config.memory.embedding_server
            )))?;

        let embed = EmbeddingClient::new(server, config.memory.embedding_model.clone());

        // Probe the embedding dim once at startup by embedding a trivial string.
        // Failure here disables memory for the session.
        let probe = embed.embed(vec!["probe".into()]).await?;
        let dim = match probe {
            Some(vs) if !vs.is_empty() && !vs[0].is_empty() => vs[0].len() as i64,
            _ => return Err(MemoryError::Disabled("embedding probe failed".into())),
        };

        let paths: Paths = default_paths(project_dir);
        let global = Arc::new(Store::open(
            Scope::Global, paths.global_db, &config.memory.embedding_model, dim,
        )?);
        let project = Arc::new(Store::open(
            Scope::Project, paths.project_db, &config.memory.embedding_model, dim,
        )?);

        Ok(Self {
            global,
            project,
            embed,
            top_n: config.memory.top_n,
            decay_half_life_days: config.memory.decay_half_life_days,
            embedding_dim: dim,
        })
    }

    pub fn embedding_dim(&self) -> i64 { self.embedding_dim }

    /// Save a curated memory. Writes into the requested scope.
    pub async fn save(
        &self,
        content: String,
        kind: Kind,
        scope: Scope,
    ) -> Result<i64, MemoryError> {
        let emb = self
            .embed
            .embed(vec![content.clone()])
            .await?
            .and_then(|mut v| v.pop());

        let store = match scope {
            Scope::Global => Arc::clone(&self.global),
            Scope::Project => Arc::clone(&self.project),
        };

        let emb_json = emb.as_ref().map(|v| serde_json::to_string(v).unwrap());

        tokio::task::spawn_blocking(move || -> Result<i64, MemoryError> {
            let conn = store.conn();
            let mut guard = conn.lock().expect("poisoned");
            let tx = guard.transaction()?;
            let ts = now();
            tx.execute(
                "INSERT INTO memories(kind, content, source, created_at, updated_at,
                                       last_used_at, use_count)
                 VALUES (?, ?, 'user_command', ?, ?, ?, 0)",
                params![kind.as_str(), content, ts, ts, ts],
            )?;
            let id: i64 = tx.last_insert_rowid();
            if let Some(ref json) = emb_json {
                tx.execute(
                    "INSERT INTO memories_vec(rowid, vector)
                     VALUES (?, vector_from_json(?, 'float4'))",
                    params![id, json],
                )?;
            }
            tx.commit()?;
            Ok(id)
        })
        .await
        .map_err(|e| MemoryError::Http(format!("join error: {e}")))?
    }
}

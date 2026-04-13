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

    pub async fn forget(&self, id: i64, scope: Scope) -> Result<bool, MemoryError> {
        let store = match scope {
            Scope::Global => Arc::clone(&self.global),
            Scope::Project => Arc::clone(&self.project),
        };
        tokio::task::spawn_blocking(move || -> Result<bool, MemoryError> {
            let conn = store.conn();
            let mut guard = conn.lock().expect("poisoned");
            let tx = guard.transaction()?;
            // Delete from vec first; FTS is handled by triggers on memories.
            tx.execute("DELETE FROM memories_vec WHERE rowid = ?", params![id])?;
            let n = tx.execute("DELETE FROM memories WHERE id = ?", params![id])?;
            tx.commit()?;
            Ok(n > 0)
        }).await.map_err(|e| MemoryError::Http(format!("join error: {e}")))?
    }

    pub async fn list(&self, scope: Scope, limit: usize)
        -> Result<Vec<crate::memory::types::Memory>, MemoryError>
    {
        use crate::memory::types::{Memory, Kind, Source};
        let store = match scope {
            Scope::Global => Arc::clone(&self.global),
            Scope::Project => Arc::clone(&self.project),
        };
        tokio::task::spawn_blocking(move || -> Result<Vec<Memory>, MemoryError> {
            let conn = store.conn();
            let guard = conn.lock().expect("poisoned");
            let mut stmt = guard.prepare(
                "SELECT id, kind, content, source, created_at, updated_at, last_used_at, use_count
                 FROM memories ORDER BY last_used_at DESC LIMIT ?"
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| {
                Ok(Memory {
                    id: r.get(0)?,
                    kind: Kind::parse(&r.get::<_, String>(1)?).unwrap_or(Kind::Project),
                    content: r.get(2)?,
                    source: if r.get::<_, String>(3)? == "extracted"
                        { Source::Extracted } else { Source::UserCommand },
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                    last_used_at: r.get(6)?,
                    use_count: r.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows { out.push(r?); }
            Ok(out)
        }).await.map_err(|e| MemoryError::Http(format!("join error: {e}")))?
    }

    /// Starts a new session row in the project DB. Returns session_id.
    pub async fn begin_session(
        &self,
        server: Option<String>,
        model: Option<String>,
    ) -> Result<i64, MemoryError> {
        let store = Arc::clone(&self.project);
        tokio::task::spawn_blocking(move || -> Result<i64, MemoryError> {
            let conn = store.conn();
            let guard = conn.lock().expect("poisoned");
            guard.execute(
                "INSERT INTO sessions(started_at, server, model) VALUES (?, ?, ?)",
                params![now(), server, model],
            )?;
            Ok(guard.last_insert_rowid())
        }).await.map_err(|e| MemoryError::Http(format!("join error: {e}")))?
    }

    /// Archive one user or assistant message. Chunks the content, embeds each
    /// chunk, writes all rows in a single transaction. Fire-and-forget caller
    /// convention: errors log and are swallowed at the App level.
    pub async fn archive_turn(
        &self,
        session_id: i64,
        role: &str,
        content: String,
    ) -> Result<(), MemoryError> {
        let chunks = crate::memory::chunk::split(&content);
        if chunks.is_empty() { return Ok(()); }

        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embs = self.embed.embed(texts.clone()).await?;

        let store = Arc::clone(&self.project);
        let role = role.to_string();
        let token_counts: Vec<i64> = chunks.iter().map(|c| c.token_count as i64).collect();

        tokio::task::spawn_blocking(move || -> Result<(), MemoryError> {
            let conn = store.conn();
            let mut guard = conn.lock().expect("poisoned");
            let tx = guard.transaction()?;

            // Next seq within session.
            let next_seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(seq), -1) + 1 FROM chunks WHERE session_id = ?",
                params![session_id], |r| r.get(0),
            )?;

            for (i, text) in texts.iter().enumerate() {
                tx.execute(
                    "INSERT INTO chunks(session_id, seq, role, content, token_count, created_at)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    params![session_id, next_seq + i as i64, role, text, token_counts[i], now()],
                )?;
                let id = tx.last_insert_rowid();
                if let Some(ref vs) = embs {
                    if let Some(v) = vs.get(i) {
                        let json = serde_json::to_string(v).unwrap();
                        tx.execute(
                            "INSERT INTO chunks_vec(rowid, vector)
                             VALUES (?, vector_from_json(?, 'float4'))",
                            params![id, json],
                        )?;
                    }
                }
            }
            tx.commit()?;
            Ok(())
        }).await.map_err(|e| MemoryError::Http(format!("join error: {e}")))?
    }

    /// Mark a session ended. Called from end-of-session path before extraction.
    pub async fn end_session_mark(&self, session_id: i64, title: Option<String>)
        -> Result<(), MemoryError>
    {
        let store = Arc::clone(&self.project);
        tokio::task::spawn_blocking(move || -> Result<(), MemoryError> {
            let conn = store.conn();
            let guard = conn.lock().expect("poisoned");
            guard.execute(
                "UPDATE sessions SET ended_at = ?, title = COALESCE(?, title) WHERE id = ?",
                params![now(), title, session_id],
            )?;
            Ok(())
        }).await.map_err(|e| MemoryError::Http(format!("join error: {e}")))?
    }

    pub async fn recall(&self, query: &str) -> Result<Vec<RetrievedItem>, MemoryError> {
        use crate::memory::retrieval::retrieve_from;

        // Embed query (optional)
        let q_vec = self.embed.embed(vec![query.to_string()])
            .await?
            .and_then(|mut v| v.pop());

        let g = Arc::clone(&self.global);
        let p = Arc::clone(&self.project);
        let query = query.to_string();
        let top_n = self.top_n;
        let hl = self.decay_half_life_days;

        let (mut g_items, mut p_items) = tokio::task::spawn_blocking(
            move || -> Result<(Vec<RetrievedItem>, Vec<RetrievedItem>), MemoryError> {
                let g_items = retrieve_from(&g, &query, q_vec.as_deref(), top_n, hl)?;
                let p_items = retrieve_from(&p, &query, q_vec.as_deref(), top_n, hl)?;
                Ok((g_items, p_items))
            }
        ).await.map_err(|e| MemoryError::Http(format!("join error: {e}")))??;

        // Project boost on ties
        for it in &mut p_items { it.score *= 1.10; }
        g_items.append(&mut p_items);
        g_items.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        g_items.truncate(self.top_n);

        // Bump last_used_at on curated memories we returned. Chunks are not tracked.
        let store_g = Arc::clone(&self.global);
        let store_p = Arc::clone(&self.project);
        let returned = g_items.clone();
        tokio::task::spawn_blocking(move || -> Result<(), MemoryError> {
            let ts = now();
            for it in &returned {
                if it.kind.is_none() { continue; }
                let store = match it.scope { Scope::Global => &store_g, Scope::Project => &store_p };
                let conn = store.conn();
                let guard = conn.lock().expect("poisoned");
                // Match by content because RetrievedItem does not carry id upstream.
                // This is a minor inefficiency but keeps the public type simple.
                guard.execute(
                    "UPDATE memories SET last_used_at = ?, use_count = use_count + 1
                     WHERE content = ?",
                    params![ts, it.content],
                )?;
            }
            Ok(())
        }).await.ok();

        Ok(g_items)
    }

    /// Run end-of-session extraction. Blocking-caller usage: awaited.
    /// Fire-and-forget callers: wrap in tokio::spawn.
    pub async fn extract_session(
        &self,
        api: &crate::api::client::ApiClient,
        session_id: i64,
        model_name: String,
    ) -> Result<(), MemoryError> {
        let title = crate::memory::extract::run(
            api,
            &self.embed,
            Arc::clone(&self.project),
            Arc::clone(&self.global),
            session_id,
            model_name,
        ).await?;
        self.end_session_mark(session_id, title).await
    }
}

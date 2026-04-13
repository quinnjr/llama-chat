//! End-of-session extraction: ask the model to mine durable memories from
//! a transcript, dedupe against existing memories, insert the survivors.

use std::sync::Arc;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::api::client::ApiClient;
use crate::api::types::{ChatRequest, Message};
use crate::memory::embed::EmbeddingClient;
use crate::memory::store::Store;
use crate::memory::types::{Kind, MemoryError, Source, now};

pub const EXTRACTION_PROMPT: &str = "\
You analyse a chat transcript and extract DURABLE facts worth remembering \
across future conversations. Do NOT extract ephemeral task state, transient \
debugging details, or temporary context. Return STRICT JSON: a top-level \
object with two fields — `title` (short string, or null) and `memories` (array). \
Each memory has `content` (string) and `kind` (one of: user, feedback, project, \
reference). If nothing durable, return {\"title\": null, \"memories\": []}. \
Output ONLY the JSON — no prose.";

pub const DEDUP_THRESHOLD: f32 = 0.92;

#[derive(Deserialize, Serialize)]
pub struct ExtractedPayload {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub memories: Vec<Extracted>,
}

#[derive(Deserialize, Serialize)]
pub struct Extracted {
    pub content: String,
    pub kind: String,
}

pub fn parse_payload(raw: &str) -> Option<ExtractedPayload> {
    // Tolerate models that wrap JSON in ```json fences.
    let trimmed = raw.trim();
    let body = if let Some(stripped) = trimmed.strip_prefix("```json") {
        stripped.trim_start_matches('\n').trim_end_matches("```").trim()
    } else if let Some(stripped) = trimmed.strip_prefix("```") {
        stripped.trim_start_matches('\n').trim_end_matches("```").trim()
    } else {
        trimmed
    };
    serde_json::from_str::<ExtractedPayload>(body).ok()
}

/// Fetch the transcript for a session, concatenated role-prefixed for the LLM.
pub fn load_transcript(store: &Store, session_id: i64) -> Result<String, MemoryError> {
    let conn = store.conn();
    let guard = conn.lock().expect("poisoned");
    let mut stmt = guard.prepare(
        "SELECT role, content FROM chunks WHERE session_id = ? ORDER BY seq"
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![session_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = String::new();
    for (role, content) in rows {
        out.push_str(&role);
        out.push_str(": ");
        out.push_str(&content);
        out.push_str("\n\n");
    }
    Ok(out)
}

/// Dedup check: cosine similarity > threshold to any existing memory of same
/// kind counts as a duplicate. Returns Some(existing_id) if dup.
#[allow(dead_code)]
pub fn find_duplicate(
    store: &Store, kind: Kind, candidate_vec: &[f32],
) -> Result<Option<i64>, MemoryError> {
    let conn = store.conn();
    let guard = conn.lock().expect("poisoned");
    let qjson = serde_json::to_string(candidate_vec).unwrap();
    let mut stmt = guard.prepare(
        "SELECT v.rowid, v.distance
         FROM memories_vec v
         JOIN memories m ON m.id = v.rowid
         WHERE m.kind = ?
           AND knn_match(v.distance, vector_from_json(?, 'float4'))
         LIMIT 1"
    )?;
    let hit: Option<(i64, f64)> = stmt
        .query_row(params![kind.as_str(), qjson], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok();
    if let Some((id, distance)) = hit {
        // cosine distance in sqlite-vector-rs is 1 - similarity
        let similarity = 1.0 - distance as f32;
        if similarity >= DEDUP_THRESHOLD {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Insert or bump an extracted memory.
pub fn upsert_extracted(
    store: Arc<Store>, kind: Kind, content: String, emb: Option<Vec<f32>>,
) -> Result<(), MemoryError> {
    let conn = store.conn();
    let mut guard = conn.lock().expect("poisoned");
    let ts = now();

    let dup_id = if let Some(ref v) = emb {
        // Use the same connection for the dup probe — borrow the lock we hold.
        let qjson = serde_json::to_string(v).unwrap();
        let hit: Option<(i64, f64)> = guard.query_row(
            "SELECT v.rowid, v.distance
             FROM memories_vec v JOIN memories m ON m.id = v.rowid
             WHERE m.kind = ? AND knn_match(v.distance, vector_from_json(?, 'float4'))
             LIMIT 1",
            params![kind.as_str(), qjson],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).ok();
        hit.and_then(|(id, d)| if (1.0 - d as f32) >= DEDUP_THRESHOLD { Some(id) } else { None })
    } else {
        None
    };

    let tx = guard.transaction()?;
    if let Some(id) = dup_id {
        tx.execute(
            "UPDATE memories SET updated_at = ?, use_count = use_count + 1 WHERE id = ?",
            params![ts, id],
        )?;
    } else {
        tx.execute(
            "INSERT INTO memories(kind, content, source, created_at, updated_at,
                                   last_used_at, use_count)
             VALUES (?, ?, ?, ?, ?, ?, 0)",
            params![kind.as_str(), content, Source::Extracted.as_str(), ts, ts, ts],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(v) = emb {
            tx.execute(
                "INSERT INTO memories_vec(rowid, vector)
                 VALUES (?, vector_from_json(?, 'float4'))",
                params![id, serde_json::to_string(&v).unwrap()],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// High-level driver: load transcript, ask the model, parse, dedup, write.
/// Returns the extracted title (if any).
pub async fn run(
    api: &ApiClient,
    embed: &EmbeddingClient,
    store: Arc<Store>,
    global: Arc<Store>,
    session_id: i64,
    model_name: String,
) -> Result<Option<String>, MemoryError> {
    let transcript = {
        let s = Arc::clone(&store);
        tokio::task::spawn_blocking(move || load_transcript(&s, session_id))
            .await.map_err(|e| MemoryError::Http(format!("join: {e}")))??
    };
    if transcript.trim().is_empty() { return Ok(None); }

    let req = ChatRequest {
        model: model_name,
        messages: vec![
            Message { role: "system".into(), content: Some(EXTRACTION_PROMPT.into()),
                      tool_calls: None, tool_call_id: None },
            Message { role: "user".into(), content: Some(transcript),
                      tool_calls: None, tool_call_id: None },
        ],
        stream: true,  // we buffer tokens below
        tools: None,
        think: false,
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let api_clone = api.clone();
    let task = tokio::spawn(async move { api_clone.chat_stream(req, tx).await });

    let mut buf = String::new();
    while let Some(ev) = rx.recv().await {
        if let crate::api::client::StreamEvent::Token(t) = ev { buf.push_str(&t); }
    }
    task.await.ok();

    let Some(payload) = parse_payload(&buf) else {
        eprintln!("[memory] extraction returned malformed JSON, skipping");
        return Ok(None);
    };

    // Global-vs-project kind routing: user & feedback go global;
    // project & reference go project.
    for m in payload.memories {
        let Some(kind) = Kind::parse(&m.kind) else { continue };
        let target = match kind {
            Kind::User | Kind::Feedback => Arc::clone(&global),
            Kind::Project | Kind::Reference => Arc::clone(&store),
        };
        let emb = embed.embed(vec![m.content.clone()]).await?
            .and_then(|mut v| v.pop());
        let content = m.content;
        let t = Arc::clone(&target);
        let _ = tokio::task::spawn_blocking(move || {
            upsert_extracted(t, kind, content, emb)
        }).await.map_err(|e| MemoryError::Http(format!("join: {e}")))?;
    }

    Ok(payload.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_json() {
        let s = r#"{"title":"hello","memories":[{"content":"x","kind":"user"}]}"#;
        let p = parse_payload(s).unwrap();
        assert_eq!(p.title.as_deref(), Some("hello"));
        assert_eq!(p.memories.len(), 1);
    }

    #[test]
    fn parse_fenced_json() {
        let s = "```json\n{\"memories\":[]}\n```";
        let p = parse_payload(s).unwrap();
        assert!(p.memories.is_empty());
    }

    #[test]
    fn parse_malformed_returns_none() {
        assert!(parse_payload("{").is_none());
        assert!(parse_payload("hello world").is_none());
    }
}

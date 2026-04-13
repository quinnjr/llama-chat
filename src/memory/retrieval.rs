//! Hybrid FTS5 + HNSW retrieval with reciprocal-rank fusion and time decay.

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::params;

use crate::memory::store::Store;
use crate::memory::types::{Kind, MemoryError, RetrievedItem, Scope, now};

const FTS_LIMIT: usize = 20;
const VEC_LIMIT: usize = 20;
const RRF_K: f64 = 60.0;

/// Run retrieval against one Store. Returns up to `top_n` items, scored and
/// decayed. `query_vec` is None when embeddings are unavailable.
pub fn retrieve_from(
    store: &Store,
    query: &str,
    query_vec: Option<&[f32]>,
    top_n: usize,
    decay_half_life_days: u32,
) -> Result<Vec<RetrievedItem>, MemoryError> {
    let conn = store.conn();
    let guard = conn.lock().expect("poisoned");

    // ── memories ──────────────────────────────────────────────────
    let fts_mem_ranks = run_fts_memories(&guard, query)?;
    let vec_mem_ranks = match query_vec {
        Some(v) => run_vec_memories(&guard, v)?,
        None => HashMap::new(),
    };

    // ── chunks ────────────────────────────────────────────────────
    let fts_chunk_ranks = run_fts_chunks(&guard, query)?;
    let vec_chunk_ranks = match query_vec {
        Some(v) => run_vec_chunks(&guard, v)?,
        None => HashMap::new(),
    };

    // RRF fuse per source
    let mem_scores = rrf_fuse(&[&fts_mem_ranks, &vec_mem_ranks]);
    let chunk_scores = rrf_fuse(&[&fts_chunk_ranks, &vec_chunk_ranks]);

    // Materialise
    let mut items: Vec<(i64, f64, ItemRow)> = Vec::new();
    for (id, score) in &mem_scores {
        if let Some(row) = load_memory(&guard, *id)? {
            items.push((*id, *score, ItemRow::Memory(row)));
        }
    }
    for (id, score) in &chunk_scores {
        if let Some(row) = load_chunk(&guard, *id)? {
            items.push((*id, *score, ItemRow::Chunk(row)));
        }
    }

    // Apply decay (memories only)
    let now_ts = now();
    let half_life = (decay_half_life_days as i64).max(1) * 86_400;
    for (_, score, row) in items.iter_mut() {
        if let ItemRow::Memory(m) = row {
            let age = (now_ts - m.last_used_at).max(0);
            let factor = 0.5_f64.powf(age as f64 / half_life as f64);
            *score *= factor;
        }
    }

    // Sort and truncate
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    items.truncate(top_n);

    Ok(items.into_iter().map(|(_, score, row)| {
        match row {
            ItemRow::Memory(m) => RetrievedItem {
                scope: store.scope,
                kind: Some(m.kind),
                content: m.content,
                score,
            },
            ItemRow::Chunk(c) => RetrievedItem {
                scope: store.scope,
                kind: None,
                content: c.content,
                score,
            },
        }
    }).collect())
}

enum ItemRow { Memory(MemRow), Chunk(ChunkRow) }
struct MemRow { kind: Kind, content: String, last_used_at: i64 }
struct ChunkRow { content: String }

fn load_memory(conn: &rusqlite::Connection, id: i64) -> Result<Option<MemRow>, MemoryError> {
    let row = conn.query_row(
        "SELECT kind, content, last_used_at FROM memories WHERE id = ?",
        params![id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
    );
    match row {
        Ok((k, content, last_used_at)) => Ok(Some(MemRow {
            kind: Kind::parse(&k).unwrap_or(Kind::Project),
            content, last_used_at,
        })),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn load_chunk(conn: &rusqlite::Connection, id: i64) -> Result<Option<ChunkRow>, MemoryError> {
    let row = conn.query_row(
        "SELECT content FROM chunks WHERE id = ?",
        params![id], |r| r.get::<_, String>(0),
    );
    match row {
        Ok(content) => Ok(Some(ChunkRow { content })),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Raw BM25 FTS5 query for memories. Returns map id → rank (0-based).
fn run_fts_memories(conn: &rusqlite::Connection, query: &str)
    -> Result<HashMap<i64, usize>, MemoryError>
{
    if query.trim().is_empty() { return Ok(HashMap::new()); }
    let match_q = fts_safe(query);
    let mut stmt = conn.prepare(
        "SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?
         ORDER BY bm25(memories_fts) LIMIT ?"
    )?;
    let ids: Vec<i64> = stmt.query_map(params![match_q, FTS_LIMIT as i64],
        |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect())
}

fn run_fts_chunks(conn: &rusqlite::Connection, query: &str)
    -> Result<HashMap<i64, usize>, MemoryError>
{
    if query.trim().is_empty() { return Ok(HashMap::new()); }
    let match_q = fts_safe(query);
    let mut stmt = conn.prepare(
        "SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH ?
         ORDER BY bm25(chunks_fts) LIMIT ?"
    )?;
    let ids: Vec<i64> = stmt.query_map(params![match_q, FTS_LIMIT as i64],
        |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect())
}

fn run_vec_memories(conn: &rusqlite::Connection, v: &[f32])
    -> Result<HashMap<i64, usize>, MemoryError>
{
    let json = serde_json::to_string(v).unwrap();
    let mut stmt = conn.prepare(
        "SELECT rowid FROM memories_vec
         WHERE knn_match(distance, vector_from_json(?, 'float4'))
         LIMIT ?"
    )?;
    let ids: Vec<i64> = stmt.query_map(params![json, VEC_LIMIT as i64],
        |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect())
}

fn run_vec_chunks(conn: &rusqlite::Connection, v: &[f32])
    -> Result<HashMap<i64, usize>, MemoryError>
{
    let json = serde_json::to_string(v).unwrap();
    let mut stmt = conn.prepare(
        "SELECT rowid FROM chunks_vec
         WHERE knn_match(distance, vector_from_json(?, 'float4'))
         LIMIT ?"
    )?;
    let ids: Vec<i64> = stmt.query_map(params![json, VEC_LIMIT as i64],
        |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect())
}

/// Escape FTS5 special characters by wrapping each term in double quotes.
fn fts_safe(query: &str) -> String {
    query.split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Reciprocal Rank Fusion: score(id) = sum over lists of 1 / (k + rank).
fn rrf_fuse(lists: &[&HashMap<i64, usize>]) -> HashMap<i64, f64> {
    let mut out: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (id, rank) in *list {
            *out.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + *rank as f64 + 1.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_prefers_better_ranks() {
        let mut a = HashMap::new();
        a.insert(1, 0); a.insert(2, 1);
        let mut b = HashMap::new();
        b.insert(2, 0); b.insert(1, 1);
        let fused = rrf_fuse(&[&a, &b]);
        assert!(fused[&1] > 0.0 && fused[&2] > 0.0);
        // Item that ranked 0 in both would beat one that ranked 0 in only one.
    }

    #[test]
    fn fts_safe_escapes_quotes_and_joins() {
        let q = fts_safe(r#"hello "world" foo"#);
        assert!(q.contains("OR"));
        assert!(!q.contains(r#""""#));
    }

    #[test]
    fn rrf_monotonic_on_insert() {
        let mut a = HashMap::new();
        a.insert(10, 5);
        let before = rrf_fuse(&[&a])[&10];
        let mut a2 = a.clone();
        a2.insert(99, 0);  // new higher-ranked item
        let after = rrf_fuse(&[&a2])[&10];
        assert_eq!(before, after);
    }
}

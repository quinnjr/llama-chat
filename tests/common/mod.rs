//! Shared test infrastructure. Provides a deterministic memory harness
//! that avoids hitting a real embedding endpoint.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use tempfile::TempDir;

/// A deterministic "embedding" for testing: hash the input into a fixed-size
/// f32 vector. Lexically identical inputs produce identical vectors.
pub fn fake_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut out = vec![0.0_f32; dim];
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        out[i % dim] += (*b as f32) / 255.0;
    }
    // Normalise to unit length for cosine sanity.
    let n: f32 = out.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    out.iter().map(|v| v / n).collect()
}

/// Opens an in-memory-style DB (tempfile-backed), registers the vector
/// extension, and initialises the v1 schema.
pub fn open_test_db(dim: i64) -> (TempDir, PathBuf, Arc<Mutex<Connection>>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut conn = Connection::open(&path).unwrap();
    sqlite_vector_rs::register(&conn).unwrap();
    llama_chat::memory::__test::init_schema(&mut conn, "test-model", dim).unwrap();
    (dir, path, Arc::new(Mutex::new(conn)))
}

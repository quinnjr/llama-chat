mod common;

use rusqlite::params;

#[test]
fn round_trip_insert_and_select() {
    let (_dir, _path, conn) = common::open_test_db(4);
    let guard = conn.lock().unwrap();

    guard.execute(
        "INSERT INTO memories(kind, content, source, created_at, updated_at, last_used_at, use_count)
         VALUES ('user', 'Prefers tabs over spaces', 'user_command', 1000, 1000, 1000, 0)",
        [],
    ).unwrap();

    let content: String = guard.query_row(
        "SELECT content FROM memories WHERE id = 1",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(content, "Prefers tabs over spaces");
}

#[test]
fn fts_finds_inserted_memory() {
    let (_dir, _path, conn) = common::open_test_db(4);
    let guard = conn.lock().unwrap();

    guard.execute(
        "INSERT INTO memories(kind, content, source, created_at, updated_at, last_used_at, use_count)
         VALUES ('user', 'The user loves Rust macros', 'user_command', 1000, 1000, 1000, 0)",
        [],
    ).unwrap();

    let hit: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'rust'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(hit, 1);

    let hit2: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'macros'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(hit2, 1);
}

#[test]
fn knn_finds_nearest_vector() {
    let (_dir, _path, conn) = common::open_test_db(4);
    let guard = conn.lock().unwrap();

    // Insert 3 memories with known embeddings.
    guard.execute(
        "INSERT INTO memories(kind, content, source, created_at, updated_at, last_used_at, use_count)
         VALUES ('user', 'apple', 'user_command', 1000, 1000, 1000, 0)",
        [],
    ).unwrap();
    let id1 = guard.last_insert_rowid();
    let v1 = common::fake_embed("apple", 4);
    let j1 = serde_json::to_string(&v1).unwrap();
    guard.execute(
        "INSERT INTO memories_vec(rowid, vector) VALUES (?, vector_from_json(?, 'float4'))",
        params![id1, j1],
    ).unwrap();

    guard.execute(
        "INSERT INTO memories(kind, content, source, created_at, updated_at, last_used_at, use_count)
         VALUES ('user', 'banana', 'user_command', 1001, 1001, 1001, 0)",
        [],
    ).unwrap();
    let id2 = guard.last_insert_rowid();
    let v2 = common::fake_embed("banana", 4);
    let j2 = serde_json::to_string(&v2).unwrap();
    guard.execute(
        "INSERT INTO memories_vec(rowid, vector) VALUES (?, vector_from_json(?, 'float4'))",
        params![id2, j2],
    ).unwrap();

    guard.execute(
        "INSERT INTO memories(kind, content, source, created_at, updated_at, last_used_at, use_count)
         VALUES ('user', 'car', 'user_command', 1002, 1002, 1002, 0)",
        [],
    ).unwrap();
    let id3 = guard.last_insert_rowid();
    let v3 = common::fake_embed("car", 4);
    let j3 = serde_json::to_string(&v3).unwrap();
    guard.execute(
        "INSERT INTO memories_vec(rowid, vector) VALUES (?, vector_from_json(?, 'float4'))",
        params![id3, j3],
    ).unwrap();

    // Query with a vector similar to "apple"
    let q = common::fake_embed("apple", 4);
    let jq = serde_json::to_string(&q).unwrap();

    let nearest: i64 = guard.query_row(
        "SELECT rowid FROM memories_vec
         WHERE knn_match(distance, vector_from_json(?, 'float4'))
         LIMIT 1",
        params![jq],
        |r| r.get(0),
    ).unwrap();

    // The nearest to "apple" should be id1 (itself)
    assert_eq!(nearest, id1);
}

#[test]
fn null_embedding_row_is_still_fts_searchable() {
    let (_dir, _path, conn) = common::open_test_db(4);
    let guard = conn.lock().unwrap();

    // Insert a memory but don't add a vector row
    guard.execute(
        "INSERT INTO memories(kind, content, source, created_at, updated_at, last_used_at, use_count)
         VALUES ('user', 'Some content without embedding', 'user_command', 1000, 1000, 1000, 0)",
        [],
    ).unwrap();

    // FTS should still find it
    let hit: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'embedding'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(hit, 1);

    // Vector table should not have it
    let vec_count: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memories_vec",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(vec_count, 0);
}

#[test]
fn cascade_delete_session_removes_chunks() {
    let (_dir, _path, conn) = common::open_test_db(4);
    let guard = conn.lock().unwrap();

    // Create a session
    guard.execute(
        "INSERT INTO sessions(started_at, server, model) VALUES (1000, 'test', 'gpt-4')",
        [],
    ).unwrap();
    let session_id = guard.last_insert_rowid();

    // Add chunks
    guard.execute(
        "INSERT INTO chunks(session_id, seq, role, content, token_count, created_at)
         VALUES (?, 0, 'user', 'hello', 1, 1000)",
        params![session_id],
    ).unwrap();
    guard.execute(
        "INSERT INTO chunks(session_id, seq, role, content, token_count, created_at)
         VALUES (?, 1, 'assistant', 'hi', 1, 1001)",
        params![session_id],
    ).unwrap();

    // Verify chunks exist
    let chunk_count: i64 = guard.query_row(
        "SELECT COUNT(*) FROM chunks WHERE session_id = ?",
        params![session_id],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(chunk_count, 2);

    // Delete session
    guard.execute(
        "DELETE FROM sessions WHERE id = ?",
        params![session_id],
    ).unwrap();

    // Chunks should be cascade-deleted
    let chunk_count_after: i64 = guard.query_row(
        "SELECT COUNT(*) FROM chunks WHERE session_id = ?",
        params![session_id],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(chunk_count_after, 0);
}

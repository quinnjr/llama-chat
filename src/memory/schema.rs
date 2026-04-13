//! Schema migrations. Linear, forward-only. Every migration is idempotent
//! because we wrap them in a single transaction and rely on the
//! `meta.schema_version` row to decide what to apply.

use rusqlite::{Connection, params};

use crate::memory::types::{MemoryError, now};

/// Current schema version this build knows how to produce.
pub const CURRENT_VERSION: i64 = 1;

/// Initialise a fresh DB or migrate an older one.
///
/// Returns the final schema version. Errors if the DB was written by
/// a newer llama-chat build than this one — refusing to run is safer
/// than silently corrupting the user's data.
pub fn init(conn: &mut Connection, embedding_model: &str, embedding_dim: i64)
    -> Result<i64, MemoryError>
{
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS meta (
            k TEXT PRIMARY KEY,
            v TEXT NOT NULL
        );
    ")?;

    let found = read_version(conn)?;

    if found == 0 {
        let tx = conn.transaction()?;
        apply_v1(&tx, embedding_model, embedding_dim)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(k, v) VALUES('schema_version', ?)",
            params![CURRENT_VERSION.to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(k, v) VALUES('created_at', ?)",
            params![now().to_string()],
        )?;
        tx.commit()?;
        Ok(CURRENT_VERSION)
    } else if found == CURRENT_VERSION {
        Ok(found)
    } else if found > CURRENT_VERSION {
        Err(MemoryError::SchemaTooNew { found, supported: CURRENT_VERSION })
    } else {
        // Future migrations would run here, in order.
        // For v1 only, this branch is unreachable.
        Ok(found)
    }
}

fn read_version(conn: &Connection) -> Result<i64, MemoryError> {
    let v: Option<String> = conn
        .query_row(
            "SELECT v FROM meta WHERE k = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
}

fn apply_v1(tx: &rusqlite::Transaction, embedding_model: &str, embedding_dim: i64)
    -> Result<(), MemoryError>
{
    // Record the embedding model & dim so mismatches on later startups are detected.
    tx.execute(
        "INSERT OR REPLACE INTO meta(k, v) VALUES('embedding_model', ?)",
        params![embedding_model],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO meta(k, v) VALUES('embedding_dim', ?)",
        params![embedding_dim.to_string()],
    )?;

    // Curated memories
    tx.execute_batch("
        CREATE TABLE memories (
            id            INTEGER PRIMARY KEY,
            kind          TEXT NOT NULL CHECK (kind IN ('user','feedback','project','reference')),
            content       TEXT NOT NULL,
            source        TEXT NOT NULL CHECK (source IN ('extracted','user_command')),
            created_at    INTEGER NOT NULL,
            updated_at    INTEGER NOT NULL,
            last_used_at  INTEGER NOT NULL,
            use_count     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX memories_kind_idx     ON memories(kind);
        CREATE INDEX memories_last_used_ix ON memories(last_used_at);

        CREATE VIRTUAL TABLE memories_fts USING fts5(
            content,
            content='memories',
            content_rowid='id',
            tokenize='porter unicode61'
        );

        CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
        END;
        CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content)
                VALUES ('delete', old.id, old.content);
        END;
        CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content)
                VALUES ('delete', old.id, old.content);
            INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
        END;

        CREATE TABLE sessions (
            id         INTEGER PRIMARY KEY,
            started_at INTEGER NOT NULL,
            ended_at   INTEGER,
            server     TEXT,
            model      TEXT,
            title      TEXT
        );

        CREATE TABLE chunks (
            id          INTEGER PRIMARY KEY,
            session_id  INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            seq         INTEGER NOT NULL,
            role        TEXT NOT NULL CHECK (role IN ('user','assistant')),
            content     TEXT NOT NULL,
            token_count INTEGER NOT NULL,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX chunks_session_ix ON chunks(session_id, seq);

        CREATE VIRTUAL TABLE chunks_fts USING fts5(
            content,
            content='chunks',
            content_rowid='id',
            tokenize='porter unicode61'
        );

        CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
            INSERT INTO chunks_fts(rowid, content) VALUES (new.id, new.content);
        END;
        CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
            INSERT INTO chunks_fts(chunks_fts, rowid, content)
                VALUES ('delete', old.id, old.content);
        END;
        CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
            INSERT INTO chunks_fts(chunks_fts, rowid, content)
                VALUES ('delete', old.id, old.content);
            INSERT INTO chunks_fts(rowid, content) VALUES (new.id, new.content);
        END;
    ")?;

    // Virtual vector tables. Must register sqlite-vector-rs on the connection
    // before calling this function — the caller in store.rs does so.
    let vec_ddl = format!(
        "CREATE VIRTUAL TABLE memories_vec USING vector(dim={embedding_dim}, type=float4, metric=cosine);
         CREATE VIRTUAL TABLE chunks_vec   USING vector(dim={embedding_dim}, type=float4, metric=cosine);"
    );
    tx.execute_batch(&vec_ddl)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_with_extension() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_vector_rs::register(&conn).unwrap();
        conn
    }

    #[test]
    fn init_fresh_db_creates_tables() {
        let mut conn = open_with_extension();
        let v = init(&mut conn, "test-model", 4).unwrap();
        assert_eq!(v, CURRENT_VERSION);

        // memories table exists
        conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get::<_, i64>(0))
            .unwrap();
        // vector table exists
        conn.query_row("SELECT COUNT(*) FROM memories_vec", [], |r| r.get::<_, i64>(0))
            .unwrap();
    }

    #[test]
    fn init_idempotent() {
        let mut conn = open_with_extension();
        init(&mut conn, "test-model", 4).unwrap();
        let v = init(&mut conn, "test-model", 4).unwrap();
        assert_eq!(v, CURRENT_VERSION);
    }

    #[test]
    fn refuses_newer_schema() {
        let mut conn = open_with_extension();
        init(&mut conn, "test-model", 4).unwrap();
        conn.execute(
            "UPDATE meta SET v = '99' WHERE k = 'schema_version'",
            [],
        ).unwrap();
        let err = init(&mut conn, "test-model", 4).unwrap_err();
        match err {
            MemoryError::SchemaTooNew { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, CURRENT_VERSION);
            }
            _ => panic!("wrong error"),
        }
    }
}

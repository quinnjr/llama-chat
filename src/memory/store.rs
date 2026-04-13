//! A `Store` wraps a single `rusqlite::Connection` guarded by a Mutex.
//! Each `MemoryService` holds two `Store`s (global + project).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::memory::schema;
use crate::memory::types::{MemoryError, Scope};

pub struct Store {
    pub scope: Scope,
    pub path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (or create) the DB at `path`, register the vector extension,
    /// and run schema migrations.
    pub fn open(
        scope: Scope,
        path: PathBuf,
        embedding_model: &str,
        embedding_dim: i64,
    ) -> Result<Self, MemoryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut conn = Connection::open(&path)?;
        sqlite_vector_rs::register(&conn)
            .map_err(|e| MemoryError::Http(format!("vector extension register: {e}")))?;

        schema::init(&mut conn, embedding_model, embedding_dim)?;

        Ok(Self {
            scope,
            path,
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Shared handle for spawn_blocking callers.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }
}

/// Derive the two standard DB paths.
pub fn default_paths(project_dir: &Path) -> crate::memory::types::Paths {
    let global = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("llama-chat")
        .join("memory.db");
    let project = project_dir.join(".llama-chat").join("memory.db");
    crate::memory::types::Paths { global_db: global, project_db: project }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_file_and_parent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sub/dir/memory.db");
        let _store = Store::open(Scope::Project, path.clone(), "m", 4).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn open_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("memory.db");
        let _a = Store::open(Scope::Project, path.clone(), "m", 4).unwrap();
        let _b = Store::open(Scope::Project, path, "m", 4).unwrap();
    }

    #[test]
    fn default_paths_uses_project_dir() {
        let dir = tempdir().unwrap();
        let paths = default_paths(dir.path());
        assert!(paths.project_db.starts_with(dir.path()));
        assert!(paths.project_db.ends_with(".llama-chat/memory.db"));
    }
}

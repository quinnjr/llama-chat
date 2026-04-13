use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    User,
    Feedback,
    Project,
    Reference,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Feedback => "feedback",
            Kind::Project => "project",
            Kind::Reference => "reference",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "user" => Some(Kind::User),
            "feedback" => Some(Kind::Feedback),
            "project" => Some(Kind::Project),
            "reference" => Some(Kind::Reference),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Extracted,
    UserCommand,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Extracted => "extracted",
            Source::UserCommand => "user_command",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
}

#[derive(Debug, Clone)]
pub struct Memory {
    pub id: i64,
    pub kind: Kind,
    pub content: String,
    #[allow(dead_code)]
    pub source: Source,
    #[allow(dead_code)]
    pub created_at: i64,
    #[allow(dead_code)]
    pub updated_at: i64,
    #[allow(dead_code)]
    pub last_used_at: i64,
    #[allow(dead_code)]
    pub use_count: i64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Chunk {
    pub id: i64,
    pub session_id: i64,
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub token_count: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct RetrievedItem {
    pub scope: Scope,
    pub kind: Option<Kind>,  // None for chunks
    pub content: String,
    pub score: f64,
}

#[derive(Debug)]
pub enum MemoryError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Http(String),
    Json(serde_json::Error),
    SchemaTooNew { found: i64, supported: i64 },
    Disabled(String),
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::Io(e) => write!(f, "io: {e}"),
            MemoryError::Sqlite(e) => write!(f, "sqlite: {e}"),
            MemoryError::Http(e) => write!(f, "http: {e}"),
            MemoryError::Json(e) => write!(f, "json: {e}"),
            MemoryError::SchemaTooNew { found, supported } => {
                write!(f, "schema version {found} newer than supported {supported}")
            }
            MemoryError::Disabled(reason) => write!(f, "disabled: {reason}"),
        }
    }
}

impl std::error::Error for MemoryError {}

impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> Self { MemoryError::Io(e) }
}
impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self { MemoryError::Sqlite(e) }
}
impl From<serde_json::Error> for MemoryError {
    fn from(e: serde_json::Error) -> Self { MemoryError::Json(e) }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub global_db: PathBuf,
    pub project_db: PathBuf,
}

/// Helper: current unix timestamp in seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

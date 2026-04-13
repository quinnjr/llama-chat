//! Long-term memory: curated facts + conversation archive.
//!
//! Public API lives on [`MemoryService`]. All other modules are internal.

pub mod types;

mod chunk;
mod embed;
mod extract;
mod retrieval;
mod schema;
mod store;

pub use types::{Kind, MemoryError, RetrievedItem, Scope, Source};

// MemoryService is introduced in a later task.

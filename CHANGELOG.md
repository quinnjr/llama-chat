# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-04-14

### Added

#### Background Task Monitor
- **`bg_run` tool** — run any tool (shell, file ops, MCP, subagents) in the background with a user-assigned label
- **`bg_status` tool** — poll one task for detailed status and partial output, or get a summary of all tasks
- **`bg_cancel` tool** — abort a running background task with graceful SIGTERM → SIGKILL shutdown
- **Completion queuing** — background results are queued and injected as system messages when the LLM is idle
- **Timer-based polling** — configurable interval (default 30s) nudges the LLM with status updates on running tasks
- **Permission checking** — inner tool permissions are enforced at launch time
- **Recursive backgrounding prevention** — bg_run/bg_status/bg_cancel cannot be backgrounded

#### Background Subagent Runner
- **Self-contained async runner** for executing subagents in the background
- Full agent lifecycle (stream → tool calls → results → stream) runs inside the spawned task
- Completely independent of the foreground subagent state machine
- Tool calls auto-allowed (user approved at bg_run invocation)
- Concurrent execution of multiple agents within a single background task

#### Configuration
- New `[background]` section in config.toml:
  - `poll_interval` — seconds between automatic status nudges (default: 30)

#### UI Enhancements
- **Background task counter** in header: gear icon with active count (e.g., "⚙ 2 bg")
- Background tasks cleared on `/clear` command

### Dependencies Added
- `libc` 0.2 for SIGTERM signal delivery during task cancellation

## [0.3.0] - 2026-04-13

### Added

#### Memory System
- **Persistent long-term memory** with two-layer architecture (global user preferences + per-project knowledge)
- **Hybrid retrieval** combining FTS5 (BM25) full-text search with HNSW vector search
- **Reciprocal Rank Fusion (RRF)** for combining multiple ranking sources with k=60
- **Time decay** for curated memories based on last_used_at with configurable half-life
- **LLM-based extraction** pipeline that mines durable facts from conversation transcripts
- **Automatic context injection** - relevant memories injected before each user turn (no model tool calls required)
- **Rolling-window chunker** - 500-token windows with 50-token overlap for conversation archival
- **Session management** - tracks conversations with begin/end timestamps and automatic archival
- **Orphan recovery** - recovers incomplete sessions from crashes on next startup

#### Slash Commands
- `/remember [--global|--project] [--kind=K] <text>` - Save a memory manually
- `/remember-this [--global|--project] [--kind=K]` - Save the last assistant response
- `/forget [--global|--project] <id>` - Delete a memory by ID
- `/memory list [--scope=global|project]` - List stored memories
- `/memory disable` - Disable memory for the current session

#### Configuration
- New `[memory]` section in config.toml:
  - `enabled` - Enable/disable memory system
  - `embedding_model` - Model name for embeddings endpoint
  - `embedding_server` - Which server config to use
  - `top_n` - Number of memories to retrieve (default: 8)
  - `decay_half_life_days` - Time decay rate (default: 90)
  - `extraction_on_clear` - Extract memories on `/clear` (default: true)

#### UI Enhancements
- **Memory status indicator** in header: `[mem]` when active, `[mem:off]` when disabled
- Extraction progress feedback on `/clear` and `/exit`

#### Infrastructure
- SQLite with FTS5 full-text search and sqlite-vector-rs for HNSW vector index
- Graceful degradation when embeddings endpoint unavailable
- Comprehensive test suite (300+ tests) with deterministic test harness
- Integration tests for schema, FTS5, HNSW, and CASCADE behavior
- Property tests for chunker losslessness

### Changed
- Bumped version from 0.2.0 to 0.3.0
- Updated README.md with memory system documentation

### Technical Details
- **Database schema v1** with migrations framework for future upgrades
- **Transactional writes** for atomicity (memories + vectors inserted together)
- **Deduplication** via cosine similarity threshold (0.92) during extraction
- **Kind routing**: user/feedback → global DB, project/reference → project DB
- **Soft-fail embedding client** returns None on network errors for graceful degradation
- **Async/spawn_blocking** pattern for non-blocking SQLite operations

### Dependencies Added
- `rusqlite` 0.39 with bundled SQLite and FTS5
- `sqlite-vector-rs` 0.2 for HNSW vector index
- `tempfile` 3 for test isolation

## [0.2.0] - [Previous Release]

(Previous changelog entries would go here)

## [0.1.0] - [Initial Release]

(Initial release notes would go here)

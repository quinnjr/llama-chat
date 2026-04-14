use std::sync::Arc;

use crossterm::event::KeyEvent;
use tokio::sync::Mutex;

use crate::api::client::StreamEvent;
use crate::mcp::McpServer;

#[allow(dead_code)]
pub enum AppEvent {
    Key(KeyEvent),
    Stream(StreamEvent),
    ToolResult {
        tool_call_id: String,
        result: String,
        success: bool,
    },
    ToolOutputChunk {
        tool_call_id: String,
        chunk: String,
    },
    McpConnected {
        server_name: String,
        tool_count: usize,
        server: Arc<Mutex<McpServer>>,
    },
    McpError {
        server_name: String,
        error: String,
    },
    SubagentStream {
        index: usize,
        event: StreamEvent,
    },
    SubagentToolResult {
        index: usize,
        tool_call_id: String,
        result: String,
        success: bool,
    },
    ModelsLoaded(Vec<String>),
    HealthCheck(bool),
    Resize,
    Error(String),
    MemoryStatus {
        disabled: bool,
        reason: String,
    },
    MemoryExtractionDone {
        session_id: i64,
    },
    BackgroundTaskDone {
        label: String,
        result: String,
        success: bool,
    },
    BackgroundTaskOutput {
        label: String,
        chunk: String,
    },
    BackgroundTaskPoll,
}

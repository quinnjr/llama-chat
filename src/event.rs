use crossterm::event::KeyEvent;
use crate::api::client::StreamEvent;

pub enum AppEvent {
    Key(KeyEvent),
    Stream(StreamEvent),
    ToolResult {
        tool_call_id: String,
        result: String,
        success: bool,
    },
    McpReady {
        server_name: String,
        tool_count: usize,
    },
    McpError {
        server_name: String,
        error: String,
    },
    Error(String),
    Tick,
}

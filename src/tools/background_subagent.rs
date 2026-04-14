use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::api::client::{ApiClient, StreamEvent};
use crate::api::types::*;
use crate::config::settings::ServerConfig;
use crate::event::AppEvent;
use crate::mcp::McpServer;
use crate::subagent::AgentSpec;
use crate::tools::Tool;
use crate::tools::filesystem::{EditFileTool, ListFilesTool, ReadFileTool, WriteFileTool};
use crate::tools::shell::ShellTool;

/// Run one or more subagents to completion inside a background task.
///
/// Each agent gets its own conversation and streams from the LLM in a loop,
/// executing tool calls directly (no event-loop round-trip) until the model
/// produces a final text response.  Progress tokens are forwarded via
/// `BackgroundTaskOutput`; the combined result is sent as
/// `BackgroundTaskDone` when all agents finish.
#[cfg(not(tarpaulin_include))]
pub async fn run_background_subagents(
    agents: Vec<AgentSpec>,
    server: ServerConfig,
    model: String,
    tool_defs: Vec<ToolDefinition>,
    mcp_servers: HashMap<String, Arc<Mutex<McpServer>>>,
    mcp_tool_map: HashMap<String, (String, String)>,
    tx: mpsc::UnboundedSender<AppEvent>,
    label: String,
    mut abort_rx: oneshot::Receiver<()>,
) {
    let run_agents = async {
        let mcp_servers = Arc::new(mcp_servers);
        let mcp_tool_map = Arc::new(mcp_tool_map);

        let mut handles = Vec::new();

        for (i, agent) in agents.into_iter().enumerate() {
            let server = server.clone();
            let model = model.clone();
            let tool_defs = tool_defs.clone();
            let mcp_servers = Arc::clone(&mcp_servers);
            let mcp_tool_map = Arc::clone(&mcp_tool_map);
            let tx = tx.clone();
            let label = label.clone();

            handles.push(tokio::spawn(async move {
                run_single_agent(i, agent, server, model, tool_defs, mcp_servers, mcp_tool_map, tx, label).await
            }));
        }

        let mut agent_results: Vec<(usize, Vec<String>)> = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => agent_results.push(result),
                Err(e) => agent_results.push((usize::MAX, vec![format!("Agent panicked: {e}")])),
            }
        }

        // Sort by agent index so output is deterministic
        agent_results.sort_by_key(|(idx, _)| *idx);

        let mut combined = String::new();
        for (i, (_, parts)) in agent_results.iter().enumerate() {
            combined.push_str(&format!("[agent-{} result]\n", i));
            combined.push_str(&parts.join("\n"));
            combined.push_str("\n\n");
        }

        combined.trim().to_string()
    };

    tokio::select! {
        result = run_agents => {
            let _ = tx.send(AppEvent::BackgroundTaskDone {
                label,
                result,
                success: true,
            });
        }
        _ = &mut abort_rx => {
            let _ = tx.send(AppEvent::BackgroundTaskDone {
                label,
                result: "(cancelled)".to_string(),
                success: false,
            });
        }
    }
}

/// Run a single agent through the stream-tool-loop cycle until it produces
/// a final text response with no tool calls.
#[cfg(not(tarpaulin_include))]
async fn run_single_agent(
    index: usize,
    agent: AgentSpec,
    server: ServerConfig,
    model: String,
    tool_defs: Vec<ToolDefinition>,
    mcp_servers: Arc<HashMap<String, Arc<Mutex<McpServer>>>>,
    mcp_tool_map: Arc<HashMap<String, (String, String)>>,
    tx: mpsc::UnboundedSender<AppEvent>,
    label: String,
) -> (usize, Vec<String>) {
    let mut conversation = Vec::new();
    if let Some(ref sys) = agent.system {
        conversation.push(Message {
            role: "system".into(),
            content: Some(sys.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
    }
    conversation.push(Message {
        role: "user".into(),
        content: Some(agent.prompt.clone()),
        tool_calls: None,
        tool_call_id: None,
    });

    let mut result_parts: Vec<String> = Vec::new();

    loop {
        // --- (a) Stream from LLM ---
        let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
        let client = ApiClient::new(server.clone());
        let request = ChatRequest {
            model: model.clone(),
            messages: conversation.clone(),
            stream: true,
            tools: if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs.clone())
            },
            think: true,
        };

        tokio::spawn(async move {
            let _ = client.chat_stream(request, stream_tx).await;
        });

        // --- (b) Process stream events locally ---
        let mut streaming_buffer = String::new();
        let mut assembling_tool_calls: HashMap<u32, ToolCall> = HashMap::new();

        while let Some(event) = stream_rx.recv().await {
            match event {
                StreamEvent::Token(text) => {
                    streaming_buffer.push_str(&text);
                    let _ = tx.send(AppEvent::BackgroundTaskOutput {
                        label: label.clone(),
                        chunk: text,
                    });
                }
                StreamEvent::ToolCallDelta(delta) => {
                    let entry = assembling_tool_calls
                        .entry(delta.index)
                        .or_insert_with(|| ToolCall {
                            id: String::new(),
                            call_type: "function".into(),
                            function: FunctionCall {
                                name: String::new(),
                                arguments: String::new(),
                            },
                        });
                    if let Some(id) = delta.id {
                        entry.id = id;
                    }
                    if let Some(ref fc) = delta.function {
                        if let Some(ref name) = fc.name {
                            entry.function.name.push_str(name);
                        }
                        if let Some(ref args) = fc.arguments {
                            entry.function.arguments.push_str(args);
                        }
                    }
                }
                StreamEvent::Usage(_) => {}
                StreamEvent::Done => break,
            }
        }

        // --- (c) Finalize the stream turn ---
        if !streaming_buffer.is_empty() {
            conversation.push(Message {
                role: "assistant".into(),
                content: Some(streaming_buffer.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
            result_parts.push(streaming_buffer);
        }

        if assembling_tool_calls.is_empty() {
            // No tool calls — agent is done
            break;
        }

        // Sort by index for deterministic ordering
        let mut sorted: Vec<(u32, ToolCall)> = assembling_tool_calls.into_iter().collect();
        sorted.sort_by_key(|(idx, _)| *idx);
        let tool_calls: Vec<ToolCall> = sorted.into_iter().map(|(_, tc)| tc).collect();

        // Push assistant message with tool calls
        conversation.push(Message {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(tool_calls.clone()),
            tool_call_id: None,
        });

        // --- (d) Execute each tool call ---
        for tc in &tool_calls {
            let tool_name = &tc.function.name;
            let arguments = &tc.function.arguments;

            // Send progress summary
            let _ = tx.send(AppEvent::BackgroundTaskOutput {
                label: label.clone(),
                chunk: format!("\u{2699} {}: {}\n", tool_name, truncate_args(arguments, 120)),
            });

            let (output, _success) =
                execute_tool_directly(tool_name, arguments, &mcp_servers, &mcp_tool_map).await;

            conversation.push(Message {
                role: "tool".into(),
                content: Some(output),
                tool_calls: None,
                tool_call_id: Some(tc.id.clone()),
            });
        }

        // Loop back to step (a) for the next LLM turn
    }

    (index, result_parts)
}

/// Execute a tool directly without going through the event loop.
///
/// Returns `(output, success)`.
#[cfg(not(tarpaulin_include))]
async fn execute_tool_directly(
    tool_name: &str,
    arguments: &str,
    mcp_servers: &HashMap<String, Arc<Mutex<McpServer>>>,
    mcp_tool_map: &HashMap<String, (String, String)>,
) -> (String, bool) {
    match tool_name {
        "shell" => match ShellTool.execute(arguments).await {
            Ok(o) => (o, true),
            Err(e) => (e.to_string(), false),
        },
        "read_file" => match ReadFileTool.execute(arguments).await {
            Ok(o) => (o, true),
            Err(e) => (e.to_string(), false),
        },
        "write_file" => match WriteFileTool.execute(arguments).await {
            Ok(o) => (o, true),
            Err(e) => (e.to_string(), false),
        },
        "edit_file" => match EditFileTool.execute(arguments).await {
            Ok(o) => (o, true),
            Err(e) => (e.to_string(), false),
        },
        "list_files" => match ListFilesTool.execute(arguments).await {
            Ok(o) => (o, true),
            Err(e) => (e.to_string(), false),
        },
        name if name.starts_with("mcp_") => {
            if let Some((server_name, real_tool_name)) = mcp_tool_map.get(name) {
                if let Some(server) = mcp_servers.get(server_name) {
                    let mut srv = server.lock().await;
                    let args: serde_json::Value =
                        serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
                    match srv.call_tool(real_tool_name, args).await {
                        Ok(output) => (output, true),
                        Err(e) => (e.to_string(), false),
                    }
                } else {
                    (format!("MCP server not found: {}", server_name), false)
                }
            } else {
                (format!("Unknown MCP tool: {}", name), false)
            }
        }
        "todo" | "todo_complete" | "wipe_todo" => {
            ("(todo tools not available in background subagents)".into(), false)
        }
        "subagent" | "bg_run" | "bg_status" | "bg_cancel" => {
            (
                format!(
                    "Tool '{}' cannot be used inside background subagents",
                    tool_name
                ),
                false,
            )
        }
        _ => (format!("Unknown tool: {}", tool_name), false),
    }
}

/// Truncate a string for display in progress messages.
fn truncate_args(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_args_short_string() {
        assert_eq!(truncate_args("hello", 10), "hello");
    }

    #[test]
    fn truncate_args_exact_length() {
        assert_eq!(truncate_args("12345", 5), "12345");
    }

    #[test]
    fn truncate_args_long_string() {
        let result = truncate_args("hello world", 5);
        assert_eq!(result, "hello...");
    }
}

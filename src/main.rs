mod api;
mod app;
mod config;
mod event;
mod mcp;
mod memory;
mod skills;
mod subagent;
mod tools;
mod ui;

use std::io;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::{
    event::{self as ct_event, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::App;
use crate::config::mcp_config::McpConfig;
use crate::config::settings::AppConfig;
use crate::event::AppEvent;

#[cfg(not(tarpaulin_include))]
#[tokio::main]
async fn main() -> Result<()> {
    let yolo = std::env::args().any(|a| a == "--yolo");

    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("llama-chat");

    let config = AppConfig::load(&config_dir.join("config.toml"))?;
    let mcp_config = McpConfig::load(&config_dir.join("mcp.json"))?;

    let project_dir = std::env::current_dir()?;
    let (memory, memory_disabled_reason) = if config.memory.enabled {
        match crate::memory::MemoryService::open(&config, &project_dir).await {
            Ok(svc) => (Some(std::sync::Arc::new(svc)), None),
            Err(e) => {
                eprintln!("[memory] disabled: {e}");
                (None, Some(e.to_string()))
            }
        }
    } else {
        (None, None)
    };

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    let mut app = App::new(config, mcp_config.clone(), event_tx.clone(), memory)?;

    if let Some(reason) = memory_disabled_reason {
        app.memory_disabled_reason = Some(reason);
    }

    if yolo {
        app.yolo = true;
    }

    for (name, entry) in &mcp_config.mcp_servers {
        let name = name.clone();
        let entry = entry.clone();
        let tx = event_tx.clone();
        tokio::spawn(async move {
            match mcp::McpServer::connect(name.clone(), &entry).await {
                Ok(server) => {
                    let tool_count = server.tools.len();
                    let server = std::sync::Arc::new(tokio::sync::Mutex::new(server));
                    let _ = tx.send(AppEvent::McpConnected {
                        server_name: name,
                        tool_count,
                        server,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::McpError {
                        server_name: name,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let input_tx = event_tx.clone();
    tokio::spawn(async move {
        loop {
            if ct_event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                if let Ok(event) = ct_event::read() {
                    match event {
                        Event::Key(key) => {
                            let _ = input_tx.send(AppEvent::Key(key));
                        }
                        Event::Resize(_, _) => {
                            let _ = input_tx.send(AppEvent::Resize);
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    let health_tx = event_tx.clone();
    let health_server = app.api_client.server().clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        loop {
            let url = format!("{}/models", health_server.url);
            let mut req = client.get(&url);
            if let Some(ref key) = health_server.api_key {
                req = req.bearer_auth(key);
            }
            let healthy = req
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            let _ = health_tx.send(AppEvent::HealthCheck(healthy));
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    loop {
        terminal.draw(|f| {
            ui::draw(f, &app, &app.theme.clone());
        })?;

        app.anim_frame = app.anim_frame.wrapping_add(1);

        if let Some(event) = event_rx.recv().await {
            match event {
                AppEvent::Key(key) => {
                    if app.pattern_input.is_some() {
                        match key.code {
                            KeyCode::Enter => {
                                app.handle_pattern_submit();
                            }
                            KeyCode::Esc => {
                                app.pattern_input = None;
                            }
                            KeyCode::Char(c) => {
                                if let Some(ref mut buf) = app.pattern_input {
                                    buf.push(c);
                                }
                            }
                            KeyCode::Backspace => {
                                if let Some(ref mut buf) = app.pattern_input {
                                    buf.pop();
                                }
                            }
                            _ => {}
                        }
                    } else if app.pending_permission.is_some() {
                        match key.code {
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                app.handle_permission_response(true, false);
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                app.handle_permission_response(false, false);
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                app.handle_permission_response(true, true);
                            }
                            KeyCode::Char('p') | KeyCode::Char('P') => {
                                app.pattern_input = Some(String::new());
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if app.waiting_for_response {
                                    app.abort_streaming();
                                } else {
                                    app.should_quit = true;
                                }
                            }
                            KeyCode::Char(c @ ('t' | 'T')) => {
                                if app.waiting_for_response {
                                    app.toggle_thinking();
                                } else {
                                    app.input_buffer.push(c);
                                }
                            }
                            KeyCode::Char(' ') => {
                                if app.waiting_for_response {
                                    app.abort_streaming();
                                } else {
                                    app.input_buffer.push(' ');
                                }
                            }
                            KeyCode::Enter => {
                                app.submit_message();
                            }
                            KeyCode::Char(c) => {
                                app.input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                app.input_buffer.pop();
                            }
                            KeyCode::Esc => {
                                app.should_quit = true;
                            }
                            _ => {}
                        }
                    }
                }
                AppEvent::Stream(stream_event) => {
                    app.handle_stream_event(stream_event);
                }
                AppEvent::ToolResult {
                    tool_call_id,
                    result,
                    success,
                } => {
                    app.handle_tool_result(tool_call_id, result, success);
                }
                AppEvent::ToolOutputChunk {
                    tool_call_id: _,
                    chunk,
                } => {
                    app.tool_output_buffer.push_str(&chunk);
                }
                AppEvent::McpConnected {
                    server_name,
                    tool_count,
                    server,
                } => {
                    {
                        let srv = server.lock().await;
                        for tool in &srv.tools {
                            let full_name = format!("mcp_{}_{}", server_name, tool.name);
                            app.mcp_tool_map.insert(
                                full_name.clone(),
                                (server_name.clone(), tool.name.clone()),
                            );
                            app.mcp_tool_defs.push(crate::api::types::ToolDefinition {
                                tool_type: "function".into(),
                                function: crate::api::types::FunctionDefinition {
                                    name: full_name,
                                    description: tool.description.clone().unwrap_or_default(),
                                    parameters: tool.input_schema.clone(),
                                },
                            });
                        }
                    }
                    app.mcp_servers.insert(server_name.clone(), server);
                    app.tool_count += tool_count;
                    app.messages.push(app::ChatEntry::System(format!(
                        "MCP '{server_name}' connected ({tool_count} tools)"
                    )));
                }
                AppEvent::McpError { server_name, error } => {
                    app.messages.push(app::ChatEntry::System(format!(
                        "MCP '{server_name}' failed: {error}"
                    )));
                }
                AppEvent::Resize => {
                    // Redraw with new dimensions happens at top of loop
                }
                AppEvent::ModelsLoaded(models) => {
                    if models.is_empty() {
                        app.messages.push(app::ChatEntry::System(
                            "No models available on this server.".into(),
                        ));
                    } else {
                        let list: Vec<String> = models.iter().map(|m| format!("  {m}")).collect();
                        app.messages.push(app::ChatEntry::System(format!(
                            "Available models:\n{}",
                            list.join("\n")
                        )));
                    }
                }
                AppEvent::HealthCheck(healthy) => {
                    app.server_healthy = Some(healthy);
                }
                AppEvent::Error(e) => {
                    app.messages
                        .push(app::ChatEntry::System(format!("Error: {e}")));
                }
                AppEvent::SubagentStream { index, event } => {
                    app.handle_subagent_stream(index, event);
                }
                AppEvent::SubagentToolResult {
                    index,
                    tool_call_id,
                    result,
                    success,
                } => {
                    app.handle_subagent_tool_result(index, tool_call_id, result, success);
                }
                AppEvent::MemoryStatus { disabled, reason } => {
                    if disabled {
                        app.memory_disabled_reason = Some(reason);
                        app.memory = None;
                        app.memory_session_id = None;
                    } else if let Some(rest) = reason.strip_prefix("session:") {
                        if let Ok(id) = rest.parse::<i64>() {
                            app.memory_session_id = Some(id);
                        }
                    }
                }
                AppEvent::MemoryExtractionDone { session_id: _ } => {
                    app.memory_session_id = None;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

mod app;
mod config;
mod event;
mod api;
mod mcp;
mod tools;
mod ui;
mod skills;

use std::io;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::{
    event::{self as ct_event, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::app::App;
use crate::config::settings::AppConfig;
use crate::config::mcp_config::McpConfig;
use crate::event::AppEvent;

#[tokio::main]
async fn main() -> Result<()> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("ollama-chat");

    let config = AppConfig::load(&config_dir.join("config.toml"))?;
    let mcp_config = McpConfig::load(&config_dir.join("mcp.json"))?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    let mut app = App::new(config, mcp_config.clone(), event_tx.clone())?;

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
                if let Ok(Event::Key(key)) = ct_event::read() {
                    let _ = input_tx.send(AppEvent::Key(key));
                }
            }
        }
    });

    loop {
        terminal.draw(|f| {
            ui::draw(f, &app, &app.theme.clone());
        })?;

        if let Some(event) = event_rx.recv().await {
            match event {
                AppEvent::Key(key) => {
                    if app.pending_permission.is_some() {
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
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('c')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                app.should_quit = true;
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
                AppEvent::McpConnected {
                    server_name,
                    tool_count,
                    server,
                } => {
                    {
                        let srv = server.lock().await;
                        for tool in &srv.tools {
                            let full_name =
                                format!("mcp_{}_{}", server_name, tool.name);
                            app.mcp_tool_map.insert(
                                full_name.clone(),
                                (server_name.clone(), tool.name.clone()),
                            );
                            app.mcp_tool_defs.push(crate::api::types::ToolDefinition {
                                tool_type: "function".into(),
                                function: crate::api::types::FunctionDefinition {
                                    name: full_name,
                                    description: tool
                                        .description
                                        .clone()
                                        .unwrap_or_default(),
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
                AppEvent::Error(e) => {
                    app.messages.push(app::ChatEntry::System(format!("Error: {e}")));
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

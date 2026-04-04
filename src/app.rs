use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{mpsc, Mutex};

use crate::api::client::{ApiClient, StreamEvent};
use crate::api::types::*;
use crate::config::settings::AppConfig;
use crate::config::mcp_config::McpConfig;
use crate::config::theme::Theme;
use crate::event::AppEvent;
use crate::mcp::McpServer;
use crate::skills::{self, Skill};
use crate::tools::permissions::PermissionManager;
use crate::tools::shell::{self, ShellTool};
use crate::tools::filesystem::{ReadFileTool, WriteFileTool, ListFilesTool};
use crate::tools::{Tool, ToolRegistry};

pub struct App {
    pub messages: Vec<ChatEntry>,
    pub conversation: Vec<Message>,
    pub input_buffer: String,
    pub streaming_buffer: String,
    pub active_model: String,
    pub active_server_name: String,
    pub tool_count: usize,
    pub pending_permission: Option<PendingPermission>,
    pub should_quit: bool,

    pub config: AppConfig,
    pub theme: Theme,
    pub api_client: ApiClient,
    pub tool_registry: ToolRegistry,
    pub permissions: PermissionManager,
    pub skills: HashMap<String, Skill>,
    pub mcp_servers: HashMap<String, Arc<Mutex<McpServer>>>,
    pub mcp_tool_defs: Vec<ToolDefinition>,
    pub mcp_tool_map: HashMap<String, (String, String)>,
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    pub pending_tool_calls: Vec<ToolCall>,
    pub assembling_tool_calls: HashMap<u32, ToolCall>,
    #[allow(dead_code)]
    project_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
    ToolCall { name: String, command: String, status: String },
    ToolOutput(String),
    System(String),
}

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub tool_name: String,
    pub command: String,
    pub tool_call_id: String,
    pub arguments: String,
}

impl App {
    pub fn new(
        config: AppConfig,
        _mcp_config: McpConfig,
        event_tx: mpsc::UnboundedSender<AppEvent>,
    ) -> Result<Self> {
        let theme = Theme::from_config(&config.theme.preset, &config.theme.colors);
        let project_dir = std::env::current_dir()?;

        let server_key = &config.defaults.server;
        let server_config = config.servers.get(server_key)
            .cloned()
            .unwrap_or_else(|| crate::config::settings::ServerConfig {
                name: "Local Ollama".into(),
                url: "http://localhost:11434/v1".into(),
                api_key: None,
            });
        let server_name = server_config.name.clone();
        let api_client = ApiClient::new(server_config);

        let mut tool_registry = ToolRegistry::new();
        tool_registry.register(Box::new(ShellTool));
        tool_registry.register(Box::new(ReadFileTool));
        tool_registry.register(Box::new(WriteFileTool));
        tool_registry.register(Box::new(ListFilesTool));
        let tool_count = tool_registry.tool_count();

        let permissions = PermissionManager::load(&project_dir);

        let global_skills_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("ollama-chat/skills");
        let project_skills_dir = project_dir.join(".ollama-chat/skills");
        let skills = skills::load_all_skills(&global_skills_dir, &project_skills_dir)
            .unwrap_or_default();

        let mut conversation = Vec::new();
        let context_path = project_dir.join(".ollama-chat/context.md");
        if context_path.exists() {
            if let Ok(ctx) = std::fs::read_to_string(&context_path) {
                conversation.push(Message {
                    role: "system".into(),
                    content: Some(ctx),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        Ok(Self {
            messages: Vec::new(),
            conversation,
            input_buffer: String::new(),
            streaming_buffer: String::new(),
            active_model: config.defaults.model.clone(),
            active_server_name: server_name,
            tool_count,
            pending_permission: None,
            should_quit: false,
            config,
            theme,
            api_client,
            tool_registry,
            permissions,
            skills,
            mcp_servers: HashMap::new(),
            mcp_tool_defs: Vec::new(),
            mcp_tool_map: HashMap::new(),
            event_tx,
            pending_tool_calls: Vec::new(),
            assembling_tool_calls: HashMap::new(),
            project_dir,
        })
    }

    pub fn submit_message(&mut self) {
        let input = self.input_buffer.trim().to_string();
        if input.is_empty() {
            return;
        }
        self.input_buffer.clear();

        if input.starts_with('/') {
            self.handle_slash_command(&input);
            return;
        }

        self.messages.push(ChatEntry::User(input.clone()));
        self.conversation.push(Message {
            role: "user".into(),
            content: Some(input),
            tool_calls: None,
            tool_call_id: None,
        });

        self.start_streaming();
    }

    fn handle_slash_command(&mut self, input: &str) {
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim());

        match cmd {
            "/exit" | "/quit" => {
                self.should_quit = true;
            }
            "/clear" => {
                self.messages.clear();
                self.conversation.retain(|m| m.role == "system");
                self.messages.push(ChatEntry::System("Conversation cleared.".into()));
            }
            "/model" => {
                if let Some(model) = arg {
                    self.active_model = model.to_string();
                    self.messages.push(ChatEntry::System(
                        format!("Switched to model: {model}")
                    ));
                } else {
                    self.messages.push(ChatEntry::System(
                        format!("Current model: {}. Use /model <name> to switch.", self.active_model)
                    ));
                }
            }
            "/server" => {
                if let Some(name) = arg {
                    if let Some(server) = self.config.servers.get(name) {
                        self.api_client.set_server(server.clone());
                        self.active_server_name = server.name.clone();
                        self.messages.push(ChatEntry::System(
                            format!("Switched to server: {}", server.name)
                        ));
                    } else {
                        let available: Vec<&str> = self.config.servers.keys()
                            .map(|s| s.as_str()).collect();
                        self.messages.push(ChatEntry::System(
                            format!("Unknown server '{name}'. Available: {}", available.join(", "))
                        ));
                    }
                } else {
                    let list: Vec<String> = self.config.servers.iter()
                        .map(|(k, v)| format!("  {k} — {}", v.name))
                        .collect();
                    self.messages.push(ChatEntry::System(
                        format!("Servers:\n{}", list.join("\n"))
                    ));
                }
            }
            "/tools" => {
                let mut lines = vec![format!("Built-in tools: {}", self.tool_registry.tool_count())];
                if !self.mcp_tool_defs.is_empty() {
                    lines.push(format!("MCP tools: {}", self.mcp_tool_defs.len()));
                }
                self.messages.push(ChatEntry::System(lines.join("\n")));
            }
            "/skills" => {
                if self.skills.is_empty() {
                    self.messages.push(ChatEntry::System("No skills loaded.".into()));
                } else {
                    let list: Vec<String> = self.skills.values()
                        .map(|s| format!("  /{} — {}", s.name, s.description))
                        .collect();
                    self.messages.push(ChatEntry::System(
                        format!("Skills:\n{}", list.join("\n"))
                    ));
                }
            }
            "/help" => {
                self.messages.push(ChatEntry::System(
                    "Commands:\n  /model [name]  — switch model\n  /server [name] — switch server\n  /tools         — list tools\n  /skills        — list skills\n  /clear         — clear chat\n  /exit          — quit".into()
                ));
            }
            other => {
                let skill_name = other.strip_prefix('/').unwrap_or(other);
                if let Some(skill) = self.skills.get(skill_name) {
                    self.conversation.push(Message {
                        role: "system".into(),
                        content: Some(skill.content.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    self.messages.push(ChatEntry::System(
                        format!("Skill '{}' activated.", skill.name)
                    ));
                } else {
                    self.messages.push(ChatEntry::System(
                        format!("Unknown command: {other}")
                    ));
                }
            }
        }
    }

    fn start_streaming(&self) {
        let mut tool_defs = self.tool_registry.definitions();
        tool_defs.extend(self.mcp_tool_defs.clone());

        let request = ChatRequest {
            model: self.active_model.clone(),
            messages: self.conversation.clone(),
            stream: true,
            tools: if tool_defs.is_empty() { None } else { Some(tool_defs) },
        };

        let tx = self.event_tx.clone();
        let client_server = self.api_client.server().clone();
        let api_client = ApiClient::new(client_server);

        tokio::spawn(async move {
            let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
            let tx2 = tx.clone();

            tokio::spawn(async move {
                if let Err(e) = api_client.chat_stream(request, stream_tx).await {
                    let _ = tx2.send(AppEvent::Error(e.to_string()));
                }
            });

            while let Some(event) = stream_rx.recv().await {
                let _ = tx.send(AppEvent::Stream(event));
            }
        });
    }

    pub fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::Token(text) => {
                self.streaming_buffer.push_str(&text);
            }
            StreamEvent::ToolCallDelta(delta) => {
                let entry = self.assembling_tool_calls
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
            StreamEvent::Done => {
                self.finalize_response();
            }
            StreamEvent::Error(e) => {
                self.messages.push(ChatEntry::System(format!("Error: {e}")));
            }
        }
    }

    fn finalize_response(&mut self) {
        if !self.streaming_buffer.is_empty() {
            let text = std::mem::take(&mut self.streaming_buffer);
            self.messages.push(ChatEntry::Assistant(text.clone()));
            self.conversation.push(Message {
                role: "assistant".into(),
                content: Some(text),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        if !self.assembling_tool_calls.is_empty() {
            let mut calls: Vec<(u32, ToolCall)> = self.assembling_tool_calls.drain().collect();
            calls.sort_by_key(|(idx, _)| *idx);
            let tool_calls: Vec<ToolCall> = calls.into_iter().map(|(_, tc)| tc).collect();

            self.conversation.push(Message {
                role: "assistant".into(),
                content: None,
                tool_calls: Some(tool_calls.clone()),
                tool_call_id: None,
            });

            self.pending_tool_calls = tool_calls;
            self.process_next_tool_call();
        }
    }

    pub fn process_next_tool_call(&mut self) {
        let tc = match self.pending_tool_calls.first() {
            Some(tc) => tc.clone(),
            None => {
                self.start_streaming();
                return;
            }
        };

        let tool_name = &tc.function.name;
        let command_display = if tool_name == "shell" {
            shell::extract_command(&tc.function.arguments)
                .unwrap_or_else(|| tc.function.arguments.clone())
        } else {
            format!("{} {}", tool_name, tc.function.arguments)
        };

        // All built-in tool calls go through the permission system. Use the
        // extracted shell command for permission lookups so that saved rules
        // created against bare commands continue to match; for other tools use
        // the full "{tool} {args}" display string.
        let permission_key = if tool_name == "shell" {
            shell::extract_command(&tc.function.arguments)
                .unwrap_or_else(|| tc.function.arguments.clone())
        } else {
            command_display.clone()
        };

        if self.permissions.is_allowed(&permission_key) {
            self.messages.push(ChatEntry::ToolCall {
                name: tool_name.clone(),
                command: command_display,
                status: "allowed".into(),
            });
            self.execute_tool_call(tc);
        } else {
            self.messages.push(ChatEntry::ToolCall {
                name: tool_name.clone(),
                command: command_display.clone(),
                status: "pending".into(),
            });
            self.pending_permission = Some(PendingPermission {
                tool_name: tool_name.clone(),
                command: command_display,
                tool_call_id: tc.id.clone(),
                arguments: tc.function.arguments.clone(),
            });
        }
    }

    fn execute_tool_call(&mut self, tc: ToolCall) {
        self.pending_tool_calls.remove(0);

        let tool_name = tc.function.name.clone();
        let arguments = tc.function.arguments.clone();
        let call_id = tc.id.clone();
        let tx = self.event_tx.clone();

        if tool_name.starts_with("mcp_") {
            if let Some((server_name, real_tool_name)) = self.mcp_tool_map.get(&tool_name) {
                if let Some(server) = self.mcp_servers.get(server_name) {
                    let server = Arc::clone(server);
                    let real_name = real_tool_name.clone();
                    let args: serde_json::Value =
                        serde_json::from_str(&arguments).unwrap_or(serde_json::Value::Null);
                    tokio::spawn(async move {
                        let mut server = server.lock().await;
                        match server.call_tool(&real_name, args).await {
                            Ok(output) => {
                                let _ = tx.send(AppEvent::ToolResult {
                                    tool_call_id: call_id,
                                    result: output,
                                    success: true,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::ToolResult {
                                    tool_call_id: call_id,
                                    result: e.to_string(),
                                    success: false,
                                });
                            }
                        }
                    });
                } else {
                    let _ = tx.send(AppEvent::ToolResult {
                        tool_call_id: call_id,
                        result: format!("MCP server not found for tool: {tool_name}"),
                        success: false,
                    });
                }
            } else {
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result: format!("Unknown MCP tool: {tool_name}"),
                    success: false,
                });
            }
            return;
        }

        tokio::spawn(async move {
            let result = match tool_name.as_str() {
                "shell" => ShellTool.execute(&arguments).await,
                "read_file" => ReadFileTool.execute(&arguments).await,
                "write_file" => WriteFileTool.execute(&arguments).await,
                "list_files" => ListFilesTool.execute(&arguments).await,
                _ => Err(anyhow::anyhow!("unknown tool")),
            };
            match result {
                Ok(output) => {
                    let _ = tx.send(AppEvent::ToolResult {
                        tool_call_id: call_id,
                        result: output,
                        success: true,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::ToolResult {
                        tool_call_id: call_id,
                        result: e.to_string(),
                        success: false,
                    });
                }
            }
        });
    }

    pub fn handle_tool_result(&mut self, tool_call_id: String, result: String, success: bool) {
        let content = if success {
            result
        } else {
            format!("Error: {}", result)
        };
        self.messages.push(ChatEntry::ToolOutput(content.clone()));
        self.conversation.push(Message {
            role: "tool".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
        });
        self.process_next_tool_call();
    }

    pub fn handle_permission_response(&mut self, allow: bool, save: bool) {
        let perm = match self.pending_permission.take() {
            Some(p) => p,
            None => return,
        };

        if allow {
            if save {
                let _ = self.permissions.add_exact(&perm.command);
            }
            if let Some(ChatEntry::ToolCall { status, .. }) = self.messages.last_mut() {
                *status = "allowed".into();
            }
            let tc = ToolCall {
                id: perm.tool_call_id,
                call_type: "function".into(),
                function: FunctionCall {
                    name: perm.tool_name,
                    arguments: perm.arguments,
                },
            };
            self.execute_tool_call(tc);
        } else {
            if let Some(ChatEntry::ToolCall { status, .. }) = self.messages.last_mut() {
                *status = "denied".into();
            }
            if !self.pending_tool_calls.is_empty() {
                self.pending_tool_calls.remove(0);
            }
            self.conversation.push(Message {
                role: "tool".into(),
                content: Some("Permission denied by user.".into()),
                tool_calls: None,
                tool_call_id: Some(perm.tool_call_id),
            });
            self.process_next_tool_call();
        }
    }
}

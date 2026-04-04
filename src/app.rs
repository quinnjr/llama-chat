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
use crate::tools::filesystem::{ReadFileTool, WriteFileTool, EditFileTool, ListFilesTool};
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
    pub pattern_input: Option<String>,
    pub yolo: bool,
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
    pub session_allow: std::collections::HashSet<String>,
    pub tool_output_buffer: String,
    pub thinking_buffer: String,
    pub in_thinking: bool,
    #[allow(dead_code)]
    project_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
    Thinking(String),
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
        tool_registry.register(Box::new(EditFileTool));
        tool_registry.register(Box::new(ListFilesTool));
        let tool_count = tool_registry.tool_count();

        let permissions = PermissionManager::load(&project_dir);

        let global_skills_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("llama-chat/skills");
        let project_skills_dir = project_dir.join(".llama-chat/skills");
        let skills = skills::load_all_skills(&global_skills_dir, &project_skills_dir)
            .unwrap_or_default();

        let mut conversation = Vec::new();

        // Load repository rules from standard files, in priority order.
        // Each file's content is injected as a system message so the model
        // follows the project's conventions.
        let rule_files = [
            project_dir.join("CLAUDE.md"),
            project_dir.join("AGENTS.md"),
            project_dir.join(".llama-chat/context.md"),
        ];
        for path in &rule_files {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if !content.trim().is_empty() {
                        conversation.push(Message {
                            role: "system".into(),
                            content: Some(content),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
            }
        }

        // Load Cursor MDC rules from .cursor/rules/*.mdc
        let mdc_rules = load_mdc_rules(&project_dir);
        if !mdc_rules.is_empty() {
            let combined = mdc_rules.join("\n\n---\n\n");
            conversation.push(Message {
                role: "system".into(),
                content: Some(combined),
                tool_calls: None,
                tool_call_id: None,
            });
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
            pattern_input: None,
            yolo: false,
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
            session_allow: ["read_file", "write_file", "edit_file", "list_files"]
                .iter().map(|s| s.to_string()).collect(),
            event_tx,
            pending_tool_calls: Vec::new(),
            assembling_tool_calls: HashMap::new(),
            tool_output_buffer: String::new(),
            thinking_buffer: String::new(),
            in_thinking: false,
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
                        format!("Current model: {}. Fetching available models...", self.active_model)
                    ));
                    let tx = self.event_tx.clone();
                    let server = self.api_client.server().clone();
                    let client = ApiClient::new(server);
                    tokio::spawn(async move {
                        match client.list_models().await {
                            Ok(models) => {
                                let _ = tx.send(AppEvent::ModelsLoaded(models));
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::Error(
                                    format!("Failed to list models: {e}"),
                                ));
                            }
                        }
                    });
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
            "/init" => {
                let agents_path = self.project_dir.join("AGENTS.md");
                if agents_path.exists() {
                    self.messages.push(ChatEntry::System(
                        "AGENTS.md already exists. Edit it directly to update.".into()
                    ));
                } else {
                    // Ask the model to generate AGENTS.md by examining the project
                    self.messages.push(ChatEntry::System(
                        "Generating AGENTS.md for this project...".into()
                    ));
                    self.messages.push(ChatEntry::User("Examine this project's structure, languages, and conventions. Then create an AGENTS.md file in the project root that describes:\n\n1. What this project is and its purpose\n2. Key architecture decisions and patterns\n3. Coding conventions (naming, style, error handling)\n4. How to build, test, and run the project\n5. Important files and directories\n6. Any rules an AI agent should follow when working on this codebase\n\nUse the list_files and read_file tools to understand the project, then write_file to create AGENTS.md. Be specific to this project — no generic boilerplate.".into()));
                    self.conversation.push(Message {
                        role: "user".into(),
                        content: Some("Examine this project's structure, languages, and conventions. Then create an AGENTS.md file in the project root that describes:\n\n1. What this project is and its purpose\n2. Key architecture decisions and patterns\n3. Coding conventions (naming, style, error handling)\n4. How to build, test, and run the project\n5. Important files and directories\n6. Any rules an AI agent should follow when working on this codebase\n\nUse the list_files and read_file tools to understand the project, then write_file to create AGENTS.md. Be specific to this project — no generic boilerplate.".into()),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    self.start_streaming();
                }
            }
            "/help" => {
                self.messages.push(ChatEntry::System(
                    "Commands:\n  /model [name]  — switch model\n  /server [name] — switch server\n  /tools         — list tools\n  /skills        — list skills\n  /init          — generate AGENTS.md\n  /clear         — clear chat\n  /exit          — quit".into()
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
            think: true,
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
        }
    }

    fn finalize_response(&mut self) {
        // Reset thinking state in case stream ended mid-think
        self.in_thinking = false;
        self.thinking_buffer.clear();

        if !self.streaming_buffer.is_empty() {
            let text = std::mem::take(&mut self.streaming_buffer);

            // Parse think tags and split into Thinking + Assistant entries
            let mut remaining = text.as_str();
            let mut assistant_content = String::new();

            while !remaining.is_empty() {
                if let Some(think_start) = remaining.find("<think>") {
                    // Content before <think> is assistant text
                    let before = &remaining[..think_start];
                    if !before.trim().is_empty() {
                        assistant_content.push_str(before);
                    }

                    remaining = &remaining[think_start + "<think>".len()..];

                    if let Some(think_end) = remaining.find("</think>") {
                        let thinking = &remaining[..think_end];
                        if !thinking.trim().is_empty() {
                            self.messages.push(ChatEntry::Thinking(thinking.trim().to_string()));
                        }
                        remaining = &remaining[think_end + "</think>".len()..];
                    } else {
                        // Unclosed think tag — treat rest as thinking
                        if !remaining.trim().is_empty() {
                            self.messages.push(ChatEntry::Thinking(remaining.trim().to_string()));
                        }
                        remaining = "";
                    }
                } else {
                    assistant_content.push_str(remaining);
                    remaining = "";
                }
            }

            if !assistant_content.trim().is_empty() {
                self.messages.push(ChatEntry::Assistant(assistant_content.trim().to_string()));
            }

            // Preserve the full text including think tags in conversation history
            // so the model retains its reasoning context
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

        if self.yolo || self.session_allow.contains(tool_name.as_str()) || self.permissions.is_allowed(&permission_key) {
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

        // MCP tools
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

        // Shell tool: stream output line-by-line
        if tool_name == "shell" {
            let args: Result<serde_json::Value, _> = serde_json::from_str(&arguments);
            let command = args.ok()
                .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
                .unwrap_or(arguments.clone());

            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();

            match child {
                Ok(mut child) => {
                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();
                    let tx_out = tx.clone();
                    let tx_err = tx.clone();
                    let cid_out = call_id.clone();
                    let cid_err = call_id.clone();

                    // Stream stdout lines
                    if let Some(stdout) = stdout {
                        let tx = tx_out;
                        let cid = cid_out;
                        tokio::spawn(async move {
                            use tokio::io::AsyncBufReadExt;
                            let mut reader = tokio::io::BufReader::new(stdout);
                            let mut line = String::new();
                            while let Ok(n) = reader.read_line(&mut line).await {
                                if n == 0 { break; }
                                let _ = tx.send(AppEvent::ToolOutputChunk {
                                    tool_call_id: cid.clone(),
                                    chunk: line.clone(),
                                });
                                line.clear();
                            }
                        });
                    }

                    // Stream stderr lines
                    if let Some(stderr) = stderr {
                        let tx = tx_err;
                        let cid = cid_err;
                        tokio::spawn(async move {
                            use tokio::io::AsyncBufReadExt;
                            let mut reader = tokio::io::BufReader::new(stderr);
                            let mut line = String::new();
                            while let Ok(n) = reader.read_line(&mut line).await {
                                if n == 0 { break; }
                                let _ = tx.send(AppEvent::ToolOutputChunk {
                                    tool_call_id: cid.clone(),
                                    chunk: format!("stderr: {}", line),
                                });
                                line.clear();
                            }
                        });
                    }

                    // Wait for process to finish, then send final ToolResult
                    let tx_done = tx.clone();
                    tokio::spawn(async move {
                        let status = child.wait().await;
                        let (success, code) = match &status {
                            Ok(s) => (s.success(), s.code()),
                            Err(_) => (false, None),
                        };
                        let _ = tx_done.send(AppEvent::ToolResult {
                            tool_call_id: call_id,
                            result: if success {
                                "(command completed)".into()
                            } else {
                                format!("(command exited with {:?})", code)
                            },
                            success,
                        });
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
            return;
        }

        // Other built-in tools: non-streaming
        tokio::spawn(async move {
            let result = match tool_name.as_str() {
                "read_file" => ReadFileTool.execute(&arguments).await,
                "write_file" => WriteFileTool.execute(&arguments).await,
                "edit_file" => EditFileTool.execute(&arguments).await,
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
        // Flush any streaming output
        let mut full_output = std::mem::take(&mut self.tool_output_buffer);
        if !full_output.is_empty() && result != "(command completed)" {
            full_output.push('\n');
            full_output.push_str(&result);
        } else if full_output.is_empty() {
            full_output = result.clone();
        }

        let display = if success { full_output.clone() } else { format!("Error: {}", full_output) };
        self.messages.push(ChatEntry::ToolOutput(display));

        // Send the full output to the model
        let content = if success { full_output } else { format!("Error: {}", result) };
        self.conversation.push(Message {
            role: "tool".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
        });
        self.process_next_tool_call();
    }

    pub fn handle_pattern_submit(&mut self) {
        if let Some(pattern) = self.pattern_input.take() {
            if !pattern.is_empty() {
                let _ = self.permissions.add_pattern(&pattern);
            }
            self.handle_permission_response(true, false);
        }
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

/// Load Cursor MDC rule files from .cursor/rules/*.mdc.
///
/// Rules with `alwaysApply: true` are always included. Rules with globs
/// are included only if the project contains files matching those globs.
/// Rules with `alwaysApply: false` and no globs are skipped (they're
/// manual-trigger only in Cursor).
fn load_mdc_rules(project_dir: &std::path::Path) -> Vec<String> {
    let rules_dir = project_dir.join(".cursor/rules");
    if !rules_dir.exists() {
        return Vec::new();
    }

    let mut results = Vec::new();

    let entries = match std::fs::read_dir(&rules_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mdc") {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let (frontmatter, content) = match parse_mdc_frontmatter(&raw) {
            Some(pair) => pair,
            None => continue,
        };

        if content.trim().is_empty() {
            continue;
        }

        let always_apply = frontmatter
            .lines()
            .any(|l| l.trim().starts_with("alwaysApply:") && l.contains("true"));

        if always_apply {
            results.push(content);
            continue;
        }

        // Extract globs from frontmatter
        let globs = extract_mdc_globs(&frontmatter);
        if globs.is_empty() {
            // No globs and not alwaysApply — skip (manual-trigger only)
            continue;
        }

        // Check if any project files match the globs
        let has_match = globs.iter().any(|pattern| {
            let full_pattern = format!("{}/{}", project_dir.display(), pattern);
            glob::glob(&full_pattern)
                .map(|mut matches| matches.next().is_some())
                .unwrap_or(false)
        });

        if has_match {
            results.push(content);
        }
    }

    results
}

fn parse_mdc_frontmatter(text: &str) -> Option<(String, String)> {
    let text = text.trim_start();
    if !text.starts_with("---") {
        return Some((String::new(), text.to_string()));
    }
    let after_first = &text[3..];
    let end = after_first.find("---")?;
    let fm = after_first[..end].to_string();
    let content = after_first[end + 3..].to_string();
    Some((fm, content))
}

/// Extract glob patterns from MDC frontmatter.
/// Handles both inline (`globs: "*.rs"`) and list format:
/// ```yaml
/// globs:
///   - src/**/*.rs
///   - tests/*.rs
/// ```
fn extract_mdc_globs(frontmatter: &str) -> Vec<String> {
    let mut globs = Vec::new();
    let mut in_globs = false;

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("globs:") {
            let rest = trimmed.strip_prefix("globs:").unwrap().trim();
            if !rest.is_empty() {
                // Inline format: globs: *.rs or globs: "*.rs"
                // Could also be comma-separated
                for g in rest.split(',') {
                    let g = g.trim().trim_matches('"').trim_matches('\'').trim();
                    if !g.is_empty() {
                        globs.push(g.to_string());
                    }
                }
                in_globs = false;
            } else {
                // List format follows on next lines
                in_globs = true;
            }
            continue;
        }

        if in_globs {
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = item.trim().trim_matches('"').trim_matches('\'');
                if !item.is_empty() {
                    globs.push(item.to_string());
                }
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Non-list line means globs section ended
                in_globs = false;
            }
        }
    }

    globs
}

#[cfg(test)]
mod mdc_tests {
    use super::*;

    #[test]
    fn parse_always_apply_mdc() {
        let text = "---\nalwaysApply: true\n---\n\n# Rust Standards\n\nUse edition 2024.";
        let (fm, content) = parse_mdc_frontmatter(text).unwrap();
        assert!(fm.contains("alwaysApply: true"));
        assert!(content.contains("Rust Standards"));
    }

    #[test]
    fn extract_list_globs() {
        let fm = "description: test\nglobs:\n  - src/**/*.rs\n  - tests/*.rs\nalwaysApply: false";
        let globs = extract_mdc_globs(fm);
        assert_eq!(globs, vec!["src/**/*.rs", "tests/*.rs"]);
    }

    #[test]
    fn extract_inline_globs() {
        let fm = "globs: \"*.ts\", \"*.tsx\"\nalwaysApply: false";
        let globs = extract_mdc_globs(fm);
        assert_eq!(globs, vec!["*.ts", "*.tsx"]);
    }

    #[test]
    fn no_globs_no_always_apply_skipped() {
        let fm = "description: manual rule\nalwaysApply: false";
        let globs = extract_mdc_globs(fm);
        assert!(globs.is_empty());
    }
}

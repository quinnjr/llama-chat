use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, mpsc};

use crate::api::client::{ApiClient, StreamEvent};
use crate::api::types::*;
use crate::config::mcp_config::McpConfig;
use crate::config::settings::AppConfig;
use crate::config::theme::Theme;
use crate::event::AppEvent;
use crate::mcp::McpServer;
use crate::skills::{self, Skill};
use crate::tools::filesystem::{EditFileTool, ListFilesTool, ReadFileTool, WriteFileTool};
use crate::tools::permissions::PermissionManager;
use crate::tools::background::{
    BackgroundTaskManager, BgCancelTool, BgRunTool, BgStatusTool,
};
use crate::tools::shell::{self, ShellTool};
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
    pub show_thinking: bool,

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
    pub waiting_for_response: bool,
    pub abort_handle: Option<tokio::sync::oneshot::Sender<()>>,
    pub anim_frame: u8,
    pub todo_items: Vec<TodoItem>,
    pub server_healthy: Option<bool>,
    pub last_token_usage: Option<Usage>,
    pub streaming_token_count: u32,
    pub subagent_states: Vec<crate::subagent::SubagentState>,
    pub subagents_pending: usize,
    pub consecutive_errors: u32,
    pub subagent_call_id: Option<String>,
    pub bg_tasks: BackgroundTaskManager,
    #[allow(dead_code)]
    project_dir: PathBuf,
    pub memory: Option<Arc<crate::memory::MemoryService>>,
    pub memory_session_id: Option<i64>,
    pub memory_disabled_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
    Thinking {
        content: String,
        collapsed: bool,
    },
    ToolCall {
        name: String,
        command: String,
        status: String,
    },
    ToolOutput(String),
    System(String),
    SubagentOutput {
        index: usize,
        text: String,
    },
}

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub tool_name: String,
    pub command: String,
    pub tool_call_id: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

impl App {
    pub fn new(
        config: AppConfig,
        _mcp_config: McpConfig,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        memory: Option<Arc<crate::memory::MemoryService>>,
    ) -> Result<Self> {
        let theme = Theme::from_config(&config.theme.preset, &config.theme.colors);
        let project_dir = std::env::current_dir()?;

        let server_key = &config.defaults.server;
        let server_config = config.servers.get(server_key).cloned().unwrap_or_else(|| {
            crate::config::settings::ServerConfig {
                name: "Local Ollama".into(),
                url: "http://localhost:11434/v1".into(),
                api_key: None,
            }
        });
        let server_name = server_config.name.clone();
        let api_client = ApiClient::new(server_config);

        let mut tool_registry = ToolRegistry::new();
        tool_registry.register(Box::new(ShellTool));
        tool_registry.register(Box::new(ReadFileTool));
        tool_registry.register(Box::new(WriteFileTool));
        tool_registry.register(Box::new(EditFileTool));
        tool_registry.register(Box::new(ListFilesTool));
        tool_registry.register(Box::new(BgRunTool));
        tool_registry.register(Box::new(BgStatusTool));
        tool_registry.register(Box::new(BgCancelTool));
        let tool_count = tool_registry.tool_count() + 4; // +3 todo + 1 subagent

        let permissions = PermissionManager::load(&project_dir);

        let global_skills_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("llama-chat/skills");
        let project_skills_dir = project_dir.join(".llama-chat/skills");
        let skills =
            skills::load_all_skills(&global_skills_dir, &project_skills_dir).unwrap_or_default();

        let mut conversation = Vec::new();

        // Core system prompt: instruct the model to act as a coding agent
        // that uses its tools directly rather than explaining steps to the user.
        let cwd = project_dir.display();
        let system_prompt = format!(
            "You are an expert software engineer working as a CLI coding assistant. \
             You have access to tools that let you interact with the user's system: \
             shell (run commands), read_file, write_file, edit_file, list_files, \
             todo (track tasks), and subagent (spawn parallel workers).\n\n\
             CRITICAL RULES:\n\
             - ALWAYS use your tools to accomplish tasks. Do NOT tell the user to run \
               commands themselves — execute them directly with the shell tool.\n\
             - When asked to create files or projects, use shell and write_file to do it.\n\
             - When asked to modify code, use read_file to see it, then edit_file or \
               write_file to change it.\n\
             - For multi-step tasks, use the todo tool to create a plan, then execute \
               each step, marking items complete as you go.\n\
             - Be concise in your text responses. Let tool outputs speak for themselves.\n\
             - The working directory is: {cwd}\n\
             - Every shell command runs in a NEW sh -c subprocess from {cwd}.\n\
             - cd has no effect on subsequent commands — use 'cd /path && cmd' in ONE command.\n\
             - You can run multiple tool calls in sequence — after each tool result you \
               will get a chance to call more tools or respond to the user.\n\n\
             SHELL COMMANDS MUST BE NON-INTERACTIVE:\n\
             - stdin is /dev/null — commands cannot prompt for input.\n\
             - Always use flags that skip confirmation prompts:\n\
               npm: --yes or -y (e.g. 'npm init -y', 'npx --yes create-vite')\n\
               apt: -y\n\
               pip: --yes or use pip install directly\n\
               git: --no-edit where applicable\n\
               rm: use -f when deletion is intended\n\
             - For commands that might prompt, pipe 'yes |' or use CI=true.\n\
             - Never run interactive editors (vim, nano) — use write_file/edit_file instead."
        );
        conversation.push(Message {
            role: "system".into(),
            content: Some(system_prompt),
            tool_calls: None,
            tool_call_id: None,
        });

        // Load repository rules from standard files, in priority order.
        // Each file's content is injected as a system message so the model
        // follows the project's conventions.
        let rule_files = [
            project_dir.join("CLAUDE.md"),
            project_dir.join("AGENTS.md"),
            project_dir.join(".llama-chat/context.md"),
        ];
        for path in &rule_files {
            if path.exists()
                && let Ok(content) = std::fs::read_to_string(path)
                && !content.trim().is_empty()
            {
                conversation.push(Message {
                    role: "system".into(),
                    content: Some(content),
                    tool_calls: None,
                    tool_call_id: None,
                });
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
            show_thinking: config.defaults.show_thinking,
            config,
            theme,
            api_client,
            tool_registry,
            permissions,
            skills,
            mcp_servers: HashMap::new(),
            mcp_tool_defs: Vec::new(),
            mcp_tool_map: HashMap::new(),
            session_allow: ["read_file", "write_file", "edit_file", "list_files", "todo", "todo_complete", "wipe_todo", "subagent"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            event_tx,
            pending_tool_calls: Vec::new(),
            assembling_tool_calls: HashMap::new(),
            tool_output_buffer: String::new(),
            thinking_buffer: String::new(),
            in_thinking: false,
            waiting_for_response: false,
            abort_handle: None,
            anim_frame: 0,
            todo_items: Vec::new(),
            server_healthy: None,
            last_token_usage: None,
            subagent_states: Vec::new(),
            subagents_pending: 0,
            streaming_token_count: 0,
            consecutive_errors: 0,
            subagent_call_id: None,
            bg_tasks: BackgroundTaskManager::new(),
            project_dir,
            memory,
            memory_session_id: None,
            memory_disabled_reason: None,
        })
    }

    async fn build_memory_block(
        svc: std::sync::Arc<crate::memory::MemoryService>,
        query: String,
    ) -> Option<String> {
        let items = svc.recall(&query).await.ok()?;
        if items.is_empty() {
            return None;
        }
        let mut s = String::from("<memory>\n");
        for it in items {
            let scope = match it.scope {
                crate::memory::Scope::Global => "global",
                crate::memory::Scope::Project => "project",
            };
            let kind = it.kind.map(|k| k.as_str()).unwrap_or("chunk");
            s.push_str(&format!("- [{scope}/{kind}] {}\n", it.content));
        }
        s.push_str("</memory>\n");
        Some(s)
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

        if self.memory_session_id.is_none()
            && let Some(ref svc) = self.memory
        {
            let svc = svc.clone();
            let server = Some(self.active_server_name.clone());
            let model = Some(self.active_model.clone());
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match svc.begin_session(server, model).await {
                    Ok(id) => {
                        let _ = tx.send(crate::event::AppEvent::MemoryStatus {
                            disabled: false,
                            reason: format!("session:{id}"),
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(crate::event::AppEvent::MemoryStatus {
                            disabled: true,
                            reason: format!("begin_session: {e}"),
                        });
                    }
                }
            });
        }

        // Inject retrieved memories before the user turn
        if let Some(ref svc) = self.memory {
            let svc_cloned = svc.clone();
            let q = input.clone();
            // Blocking inline: recall is a hot-path operation that must complete
            // before start_streaming injects the system prompt. We use a short
            // timeout so a hung embeddings endpoint does not brick input.
            let block_opt = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        App::build_memory_block(svc_cloned, q),
                    )
                    .await
                    .ok()
                    .flatten()
                })
            });
            if let Some(block) = block_opt {
                self.conversation.push(crate::api::types::Message {
                    role: "system".into(),
                    content: Some(block),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        self.messages.push(ChatEntry::User(input.clone()));
        self.conversation.push(Message {
            role: "user".into(),
            content: Some(input.clone()),
            tool_calls: None,
            tool_call_id: None,
        });

        // Archive user turn immediately
        if let (Some(svc), Some(sid)) = (&self.memory, self.memory_session_id) {
            let svc = svc.clone();
            let content = input.clone();
            tokio::spawn(async move {
                if let Err(e) = svc.archive_turn(sid, "user", content).await {
                    eprintln!("[memory] archive user turn: {e}");
                }
            });
        }

        self.last_token_usage = None;
        self.start_streaming();
    }

    fn handle_slash_command(&mut self, input: &str) {
        // Try memory commands first. None means "not ours, fall through".
        if let Some(parsed) = crate::memory::parse_command(input) {
            match parsed {
                Ok(cmd) => self.dispatch_memory_command(cmd),
                Err(msg) => self.messages.push(crate::app::ChatEntry::System(msg)),
            }
            return;
        }

        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim());

        match cmd {
            "/exit" | "/quit" => {
                if let (Some(svc), Some(sid)) = (&self.memory, self.memory_session_id) {
                    let svc = svc.clone();
                    let api = self.api_client.clone();
                    let model = self.active_model.clone();
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            svc.extract_session(&api, sid, model),
                        ).await;
                    });
                }
                self.should_quit = true;
            }
            "/clear" => {
                if let (Some(svc), Some(sid)) = (&self.memory, self.memory_session_id)
                    && self.config.memory.extraction_on_clear
                {
                    let svc = svc.clone();
                    let api = self.api_client.clone();
                    let model = self.active_model.clone();
                    // Block (with a visible "extracting..." placeholder) up to 30s.
                    self.messages.push(crate::app::ChatEntry::System(
                        "[extracting memories…]".into()));
                    let result = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async move {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                svc.extract_session(&api, sid, model),
                            ).await
                        })
                    });
                    match result {
                        Ok(Ok(())) => {
                            self.messages.push(crate::app::ChatEntry::System(
                                "memories saved".into()));
                        }
                        Ok(Err(e)) => {
                            self.messages.push(crate::app::ChatEntry::System(
                                format!("extract error: {e}")));
                        }
                        Err(_) => {
                            self.messages.push(crate::app::ChatEntry::System(
                                "extraction timed out".into()));
                        }
                    }
                }
                self.memory_session_id = None;
                self.bg_tasks.clear_all();
                self.messages.clear();
                self.conversation.retain(|m| m.role == "system");
                self.messages
                    .push(ChatEntry::System("Conversation cleared.".into()));
            }
            "/model" => {
                if let Some(model) = arg {
                    self.active_model = model.to_string();
                    self.messages
                        .push(ChatEntry::System(format!("Switched to model: {model}")));
                } else {
                    self.messages.push(ChatEntry::System(format!(
                        "Current model: {}. Fetching available models...",
                        self.active_model
                    )));
                    let tx = self.event_tx.clone();
                    let server = self.api_client.server().clone();
                    let client = ApiClient::new(server);
                    tokio::spawn(async move {
                        match client.list_models().await {
                            Ok(models) => {
                                let _ = tx.send(AppEvent::ModelsLoaded(models));
                            }
                            Err(e) => {
                                let _ =
                                    tx.send(AppEvent::Error(format!("Failed to list models: {e}")));
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
                        self.server_healthy = None;
                        self.messages.push(ChatEntry::System(format!(
                            "Switched to server: {}",
                            server.name
                        )));
                    } else {
                        let available: Vec<&str> =
                            self.config.servers.keys().map(|s| s.as_str()).collect();
                        self.messages.push(ChatEntry::System(format!(
                            "Unknown server '{name}'. Available: {}",
                            available.join(", ")
                        )));
                    }
                } else {
                    let list: Vec<String> = self
                        .config
                        .servers
                        .iter()
                        .map(|(k, v)| format!("  {k} — {}", v.name))
                        .collect();
                    self.messages
                        .push(ChatEntry::System(format!("Servers:\n{}", list.join("\n"))));
                }
            }
            "/tools" => {
                let mut lines = vec![format!(
                    "Built-in tools: {}",
                    self.tool_registry.tool_count()
                )];
                if !self.mcp_tool_defs.is_empty() {
                    lines.push(format!("MCP tools: {}", self.mcp_tool_defs.len()));
                }
                lines.push("Todo tools: todo, todo_complete, wipe_todo".into());
                self.messages.push(ChatEntry::System(lines.join("\n")));
            }
            "/skills" => {
                if self.skills.is_empty() {
                    self.messages
                        .push(ChatEntry::System("No skills loaded.".into()));
                } else {
                    let list: Vec<String> = self
                        .skills
                        .values()
                        .map(|s| format!("  /{} — {}", s.name, s.description))
                        .collect();
                    self.messages
                        .push(ChatEntry::System(format!("Skills:\n{}", list.join("\n"))));
                }
            }
            "/init" => {
                let agents_path = self.project_dir.join("AGENTS.md");
                if agents_path.exists() {
                    self.messages.push(ChatEntry::System(
                        "AGENTS.md already exists. Edit it directly to update.".into(),
                    ));
                } else {
                    self.messages.push(ChatEntry::System(
                        "Generating AGENTS.md for this project...".into(),
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
            "/thinking" => {
                self.show_thinking = !self.show_thinking;
                let status = if self.show_thinking { "visible" } else { "hidden" };
                self.messages.push(ChatEntry::System(format!(
                    "Thinking display: {status}"
                )));
            }
            "/help" => {
                self.messages.push(ChatEntry::System(
                    "Commands:\n  /model [name]  — switch model\n  /server [name] — switch server\n  /tools         — list tools\n  /skills        — list skills\n  /init          — generate AGENTS.md\n  /clear         — clear chat\n  /thinking      — toggle thinking display\n  /remember <text> — save a memory\n  /forget <id>     — delete a memory\n  /memory list     — list memories\n  /exit          — quit\n\nKeybindings:\n  Ctrl+C          — stop generating\n  t               — toggle thinking".into()
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
                    self.messages.push(ChatEntry::System(format!(
                        "Skill '{}' activated.",
                        skill.name
                    )));
                } else {
                    self.messages
                        .push(ChatEntry::System(format!("Unknown command: {other}")));
                }
            }
        }
    }

    fn dispatch_memory_command(&mut self, cmd: crate::memory::Command) {
        use crate::memory::{Command, Scope, save_ack};
        let Some(ref svc) = self.memory else {
            self.messages.push(crate::app::ChatEntry::System(
                format!("memory disabled: {}",
                        self.memory_disabled_reason.as_deref().unwrap_or("not enabled"))
            ));
            return;
        };
        let svc = svc.clone();
        let tx = self.event_tx.clone();
        match cmd {
            Command::Remember { content, scope, kind } => {
                tokio::spawn(async move {
                    match svc.save(content, kind, scope).await {
                        Ok(id) => {
                            let _ = tx.send(crate::event::AppEvent::Error(save_ack(id, scope, kind)));
                        }
                        Err(e) => { let _ = tx.send(crate::event::AppEvent::Error(format!("save: {e}"))); }
                    }
                });
            }
            Command::RememberThis { scope, kind } => {
                // Find the last assistant turn from current conversation.
                let last_asst = self.conversation.iter().rev()
                    .find(|m| m.role == "assistant")
                    .and_then(|m| m.content.clone());
                let Some(content) = last_asst else {
                    self.messages.push(crate::app::ChatEntry::System(
                        "no assistant turn to remember".into()));
                    return;
                };
                tokio::spawn(async move {
                    match svc.save(content, kind, scope).await {
                        Ok(id) => { let _ = tx.send(crate::event::AppEvent::Error(save_ack(id, scope, kind))); }
                        Err(e) => { let _ = tx.send(crate::event::AppEvent::Error(format!("save: {e}"))); }
                    }
                });
            }
            Command::Forget { id, scope } => {
                tokio::spawn(async move {
                    match svc.forget(id, scope).await {
                        Ok(true)  => { let _ = tx.send(crate::event::AppEvent::Error(format!("forgot #{id}"))); }
                        Ok(false) => { let _ = tx.send(crate::event::AppEvent::Error(format!("no memory #{id}"))); }
                        Err(e)    => { let _ = tx.send(crate::event::AppEvent::Error(format!("forget: {e}"))); }
                    }
                });
            }
            Command::List { scope } => {
                let scopes: Vec<Scope> = match scope {
                    Some(s) => vec![s],
                    None => vec![Scope::Global, Scope::Project],
                };
                tokio::spawn(async move {
                    for s in scopes {
                        match svc.list(s, 50).await {
                            Ok(ms) => {
                                let label = match s { Scope::Global => "global", Scope::Project => "project" };
                                let header = format!("── {label} ({}) ──", ms.len());
                                let _ = tx.send(crate::event::AppEvent::Error(header));
                                for m in ms {
                                    let _ = tx.send(crate::event::AppEvent::Error(
                                        format!("#{} [{}] {}", m.id, m.kind.as_str(), m.content)
                                    ));
                                }
                            }
                            Err(e) => { let _ = tx.send(crate::event::AppEvent::Error(format!("list: {e}"))); }
                        }
                    }
                });
            }
            Command::Reindex | Command::Accept => {
                self.messages.push(crate::app::ChatEntry::System(
                    "reindex/accept not implemented yet".into()));
            }
            Command::Disable => {
                self.memory = None;
                self.memory_disabled_reason = Some("user /memory disable".into());
                self.messages.push(crate::app::ChatEntry::System(
                    "memory disabled for this session".into()));
            }
        }
    }

    pub fn toggle_thinking(&mut self) {
        self.show_thinking = !self.show_thinking;
    }

    pub fn abort_streaming(&mut self) {
        if let Some(tx) = self.abort_handle.take() {
            let _ = tx.send(());
        }
        self.waiting_for_response = false;
        if !self.streaming_buffer.is_empty() {
            self.finalize_response();
        }
        self.messages.push(ChatEntry::System("Generation stopped.".into()));
    }

    pub fn todo_tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "todo".into(),
                    description: "Create a todo list for tracking task progress. Replaces any existing list.".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "items": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "List of todo item descriptions"
                            }
                        },
                        "required": ["items"]
                    }),
                },
            },
            ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "todo_complete".into(),
                    description: "Mark a todo item as completed by its zero-based index.".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "index": {
                                "type": "integer",
                                "description": "Zero-based index of the todo item to mark complete"
                            }
                        },
                        "required": ["index"]
                    }),
                },
            },
            ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "wipe_todo".into(),
                    description: "Clear the entire todo list to start fresh.".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {}
                    }),
                },
            },
        ]
    }

    pub fn handle_todo_tool(&mut self, arguments: &str) -> String {
        #[derive(serde::Deserialize)]
        struct TodoArgs {
            items: Vec<String>,
        }
        match serde_json::from_str::<TodoArgs>(arguments) {
            Ok(args) => {
                let count = args.items.len();
                self.todo_items = args
                    .items
                    .into_iter()
                    .map(|text| TodoItem { text, done: false })
                    .collect();
                format!("Added {count} items")
            }
            Err(e) => format!("Invalid todo arguments: {e}"),
        }
    }

    pub fn handle_todo_complete(&mut self, arguments: &str) -> Result<String, String> {
        #[derive(serde::Deserialize)]
        struct CompleteArgs {
            index: usize,
        }
        match serde_json::from_str::<CompleteArgs>(arguments) {
            Ok(args) => {
                if let Some(item) = self.todo_items.get_mut(args.index) {
                    item.done = true;
                    Ok(format!("Completed: {}", item.text))
                } else {
                    Err(format!("Invalid todo index: {}", args.index))
                }
            }
            Err(e) => Err(format!("Invalid todo_complete arguments: {e}")),
        }
    }

    pub fn handle_wipe_todo(&mut self) -> String {
        self.todo_items.clear();
        "Todo list cleared".into()
    }

    #[cfg(not(tarpaulin_include))]
    pub(crate) fn start_streaming(&mut self) {
        self.waiting_for_response = true;
        self.streaming_token_count = 0;
        let mut tool_defs = self.tool_registry.definitions();
        tool_defs.extend(self.mcp_tool_defs.clone());
        tool_defs.extend(self.todo_tool_definitions());
        tool_defs.push(crate::subagent::tool_definition());

        let request = ChatRequest {
            model: self.active_model.clone(),
            messages: self.conversation.clone(),
            stream: true,
            tools: if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs)
            },
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
                self.waiting_for_response = false;
                self.streaming_token_count += 1;
                self.streaming_buffer.push_str(&text);
            }
            StreamEvent::ToolCallDelta(delta) => {
                self.waiting_for_response = false;
                let entry = self
                    .assembling_tool_calls
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
            StreamEvent::Usage(usage) => {
                self.last_token_usage = Some(usage);
            }
            StreamEvent::Done => {
                self.finalize_response();
            }
        }
    }

    fn finalize_response(&mut self) {
        self.waiting_for_response = false;
        // Reset thinking state in case stream ended mid-think
        self.in_thinking = false;
        self.thinking_buffer.clear();

        // If the server didn't provide usage stats (e.g. Ollama streaming),
        // build approximate usage from what we can observe.
        if self.last_token_usage.is_none() && self.streaming_token_count > 0 {
            // Rough estimate: ~4 chars per token for prompt context
            let prompt_estimate = self
                .conversation
                .iter()
                .filter_map(|m| m.content.as_ref())
                .map(|c| c.len() as u32 / 4)
                .sum::<u32>();
            let completion = self.streaming_token_count;
            self.last_token_usage = Some(Usage {
                prompt_tokens: prompt_estimate,
                completion_tokens: completion,
                total_tokens: prompt_estimate + completion,
            });
        }

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
                            self.messages
                                .push(ChatEntry::Thinking {
                                    content: thinking.trim().to_string(),
                                    collapsed: false,
                                });
                        }
                        remaining = &remaining[think_end + "</think>".len()..];
                    } else {
                        // Unclosed think tag — treat rest as thinking
                        if !remaining.trim().is_empty() {
                            self.messages
                                .push(ChatEntry::Thinking {
                                    content: remaining.trim().to_string(),
                                    collapsed: false,
                                });
                        }
                        remaining = "";
                    }
                } else {
                    assistant_content.push_str(remaining);
                    remaining = "";
                }
            }

            if !assistant_content.trim().is_empty() {
                self.messages
                    .push(ChatEntry::Assistant(assistant_content.trim().to_string()));
            }

            // Preserve the full text including think tags in conversation history
            // so the model retains its reasoning context
            self.conversation.push(Message {
                role: "assistant".into(),
                content: Some(text.clone()),
                tool_calls: None,
                tool_call_id: None,
            });

            // Archive assistant turn
            if let (Some(svc), Some(sid)) = (&self.memory, self.memory_session_id) {
                let final_text = text;
                if !final_text.trim().is_empty() {
                    let svc = svc.clone();
                    tokio::spawn(async move {
                        if let Err(e) = svc.archive_turn(sid, "assistant", final_text).await {
                            eprintln!("[memory] archive assistant turn: {e}");
                        }
                    });
                }
            }
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
            tc.function.arguments.clone()
        };

        // All built-in tool calls go through the permission system. Use the
        // extracted shell command for permission lookups so that saved rules
        // created against bare commands continue to match; for other tools use
        // the full "{tool} {args}" display string.
        let permission_key = if tool_name == "shell" {
            shell::extract_command(&tc.function.arguments)
                .unwrap_or_else(|| tc.function.arguments.clone())
        } else {
            format!("{} {}", tool_name, tc.function.arguments)
        };

        if self.yolo
            || self.session_allow.contains(tool_name.as_str())
            || self.permissions.is_allowed(&permission_key)
        {
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

    #[cfg(not(tarpaulin_include))]
    fn execute_tool_call(&mut self, tc: ToolCall) {
        self.pending_tool_calls.remove(0);

        let tool_name = tc.function.name.clone();
        let arguments = tc.function.arguments.clone();
        let call_id = tc.id.clone();
        let tx = self.event_tx.clone();

        // Todo tools: inline state mutation, no async needed
        match tool_name.as_str() {
            "todo" => {
                let result = self.handle_todo_tool(&arguments);
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result,
                    success: true,
                });
                return;
            }
            "todo_complete" => {
                let (result, success) = match self.handle_todo_complete(&arguments) {
                    Ok(msg) => (msg, true),
                    Err(msg) => (msg, false),
                };
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result,
                    success,
                });
                return;
            }
            "wipe_todo" => {
                let result = self.handle_wipe_todo();
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result,
                    success: true,
                });
                return;
            }
            "bg_status" => {
                let args: crate::tools::background::BgStatusArgs =
                    serde_json::from_str(&arguments).unwrap_or(
                        crate::tools::background::BgStatusArgs { label: None }
                    );
                let result = if let Some(label) = args.label {
                    match self.bg_tasks.status_one(&label) {
                        Ok(status) => status,
                        Err(e) => e,
                    }
                } else {
                    let summary = self.bg_tasks.summary();
                    if summary.is_empty() {
                        "No background tasks.".into()
                    } else {
                        format!("Background tasks:\n{summary}")
                    }
                };
                self.bg_tasks.clear_acknowledged();
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result,
                    success: true,
                });
                return;
            }
            "bg_cancel" => {
                let (result, success) = match serde_json::from_str::<crate::tools::background::BgCancelArgs>(&arguments) {
                    Ok(args) => match self.bg_tasks.cancel(&args.label) {
                        Ok(()) => (format!("Task '{}' cancelled.", args.label), true),
                        Err(e) => (e, false),
                    },
                    Err(e) => (format!("Invalid bg_cancel arguments: {e}"), false),
                };
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result,
                    success,
                });
                return;
            }
            "bg_run" => {
                let args = match crate::tools::background::BgRunTool::parse_args(&arguments) {
                    Ok(a) => a,
                    Err(e) => {
                        let _ = tx.send(AppEvent::ToolResult {
                            tool_call_id: call_id,
                            result: e,
                            success: false,
                        });
                        return;
                    }
                };

                let inner_tool = args.tool.clone();
                let inner_args_str = args.arguments.to_string();
                let label = args.label.clone();

                // Permission check for the inner tool
                let inner_permission_key = if inner_tool == "shell" {
                    crate::tools::shell::extract_command(&inner_args_str)
                        .unwrap_or_else(|| inner_args_str.clone())
                } else {
                    format!("{} {}", inner_tool, inner_args_str)
                };

                if !self.yolo
                    && !self.session_allow.contains(inner_tool.as_str())
                    && !self.permissions.is_allowed(&inner_permission_key)
                {
                    let _ = tx.send(AppEvent::ToolResult {
                        tool_call_id: call_id,
                        result: format!("Permission denied for {}: {}", inner_tool, inner_permission_key),
                        success: false,
                    });
                    return;
                }

                let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();

                let task = crate::tools::background::BackgroundTask {
                    label: label.clone(),
                    tool_name: inner_tool.clone(),
                    arguments: inner_args_str.clone(),
                    status: crate::tools::background::BackgroundTaskStatus::Running,
                    output_chunks: vec![],
                    result: None,
                    success: None,
                    started_at: std::time::Instant::now(),
                    finished_at: None,
                    abort_tx: Some(abort_tx),
                    acknowledged: false,
                };

                if let Err(e) = self.bg_tasks.insert(task) {
                    let _ = tx.send(AppEvent::ToolResult {
                        tool_call_id: call_id,
                        result: e,
                        success: false,
                    });
                    return;
                }

                // Spawn background execution
                self.spawn_background_tool(label.clone(), inner_tool.clone(), inner_args_str.clone(), tx.clone(), abort_rx);

                let display = crate::tools::shell::extract_command(&inner_args_str)
                    .unwrap_or_else(|| inner_args_str.clone());
                let _ = tx.send(AppEvent::ToolResult {
                    tool_call_id: call_id,
                    result: format!("Background task '{}' started ({}: {})", label, inner_tool, display),
                    success: true,
                });
                return;
            }
            "subagent" => {
                match crate::subagent::parse_args(&arguments) {
                    Ok(args) => {
                        self.subagent_call_id = Some(call_id);
                        self.subagents_pending = args.agents.len();
                        self.subagent_states = args.agents.iter().enumerate().map(|(i, spec)| {
                            crate::subagent::SubagentState::new(i, spec.system.as_deref(), &spec.prompt)
                        }).collect();

                        let mut tool_defs = self.tool_registry.definitions();
                        tool_defs.extend(self.mcp_tool_defs.clone());
                        tool_defs.extend(self.todo_tool_definitions());
                        tool_defs.push(crate::subagent::tool_definition());

                        for state in &self.subagent_states {
                            let index = state.index;
                            let request = ChatRequest {
                                model: self.active_model.clone(),
                                messages: state.conversation.clone(),
                                stream: true,
                                tools: if tool_defs.is_empty() { None } else { Some(tool_defs.clone()) },
                                think: true,
                            };
                            let tx = self.event_tx.clone();
                            let server = self.api_client.server().clone();
                            tokio::spawn(async move {
                                let client = ApiClient::new(server);
                                let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = client.chat_stream(request, stream_tx).await {
                                        let _ = tx2.send(AppEvent::Error(format!("[agent-{index}] {e}")));
                                    }
                                });
                                while let Some(event) = stream_rx.recv().await {
                                    let _ = tx.send(AppEvent::SubagentStream { index, event });
                                }
                            });
                        }

                        for state in &self.subagent_states {
                            let prompt_preview = state
                                .conversation
                                .last()
                                .and_then(|m| m.content.as_deref())
                                .unwrap_or("");
                            self.messages.push(ChatEntry::SubagentOutput {
                                index: state.index,
                                text: format!("Started ({})", prompt_preview),
                            });
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::ToolResult {
                            tool_call_id: call_id,
                            result: e,
                            success: false,
                        });
                    }
                }
                return;
            }
            _ => {}
        }

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
            let command = args
                .ok()
                .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
                .unwrap_or(arguments.clone());

            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .env("CI", "true")
                .env("DEBIAN_FRONTEND", "noninteractive")
                .env("npm_config_yes", "true")
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
                    let stdout_handle = stdout.map(|stdout| {
                        let tx = tx_out;
                        let cid = cid_out;
                        tokio::spawn(async move {
                            use tokio::io::AsyncBufReadExt;
                            let mut reader = tokio::io::BufReader::new(stdout);
                            let mut line = String::new();
                            while let Ok(n) = reader.read_line(&mut line).await {
                                if n == 0 {
                                    break;
                                }
                                let _ = tx.send(AppEvent::ToolOutputChunk {
                                    tool_call_id: cid.clone(),
                                    chunk: line.clone(),
                                });
                                line.clear();
                            }
                        })
                    });

                    // Stream stderr lines
                    let stderr_handle = stderr.map(|stderr| {
                        let tx = tx_err;
                        let cid = cid_err;
                        tokio::spawn(async move {
                            use tokio::io::AsyncBufReadExt;
                            let mut reader = tokio::io::BufReader::new(stderr);
                            let mut line = String::new();
                            while let Ok(n) = reader.read_line(&mut line).await {
                                if n == 0 {
                                    break;
                                }
                                let _ = tx.send(AppEvent::ToolOutputChunk {
                                    tool_call_id: cid.clone(),
                                    chunk: format!("stderr: {}", line),
                                });
                                line.clear();
                            }
                        })
                    });

                    // Wait for process exit with timeout, then drain readers
                    // before sending ToolResult so all output chunks are queued
                    // in the channel ahead of the result event.
                    let tx_done = tx.clone();
                    tokio::spawn(async move {
                        let timeout = tokio::time::timeout(
                            std::time::Duration::from_secs(120),
                            child.wait(),
                        );
                        let (success, code, timed_out) = match timeout.await {
                            Ok(Ok(s)) => (s.success(), s.code(), false),
                            Ok(Err(_)) => (false, None, false),
                            Err(_) => {
                                // Timeout — kill the process
                                let _ = child.kill().await;
                                (false, None, true)
                            }
                        };
                        if let Some(h) = stdout_handle {
                            let _ = h.await;
                        }
                        if let Some(h) = stderr_handle {
                            let _ = h.await;
                        }
                        let _ = tx_done.send(AppEvent::ToolResult {
                            tool_call_id: call_id,
                            result: if timed_out {
                                "(command timed out after 120s)".into()
                            } else if success {
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

    #[cfg(not(tarpaulin_include))]
    fn spawn_background_tool(
        &self,
        label: String,
        tool_name: String,
        arguments: String,
        tx: mpsc::UnboundedSender<AppEvent>,
        abort_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        match tool_name.as_str() {
            "shell" => {
                let args: Result<serde_json::Value, _> = serde_json::from_str(&arguments);
                let command = args
                    .ok()
                    .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
                    .unwrap_or(arguments.clone());

                let child = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .env("CI", "true")
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .env("npm_config_yes", "true")
                    .spawn();

                match child {
                    Ok(mut child) => {
                        let stdout = child.stdout.take();
                        let stderr = child.stderr.take();
                        let tx_out = tx.clone();
                        let tx_err = tx.clone();
                        let label_out = label.clone();
                        let label_err = label.clone();

                        let stdout_handle = stdout.map(|stdout| {
                            tokio::spawn(async move {
                                use tokio::io::AsyncBufReadExt;
                                let mut reader = tokio::io::BufReader::new(stdout);
                                let mut line = String::new();
                                while let Ok(n) = reader.read_line(&mut line).await {
                                    if n == 0 { break; }
                                    let _ = tx_out.send(AppEvent::BackgroundTaskOutput {
                                        label: label_out.clone(),
                                        chunk: line.clone(),
                                    });
                                    line.clear();
                                }
                            })
                        });

                        let stderr_handle = stderr.map(|stderr| {
                            tokio::spawn(async move {
                                use tokio::io::AsyncBufReadExt;
                                let mut reader = tokio::io::BufReader::new(stderr);
                                let mut line = String::new();
                                while let Ok(n) = reader.read_line(&mut line).await {
                                    if n == 0 { break; }
                                    let _ = tx_err.send(AppEvent::BackgroundTaskOutput {
                                        label: label_err.clone(),
                                        chunk: format!("stderr: {}", line),
                                    });
                                    line.clear();
                                }
                            })
                        });

                        tokio::spawn(async move {
                            tokio::select! {
                                result = async {
                                    let wait_result = tokio::time::timeout(
                                        std::time::Duration::from_secs(120),
                                        child.wait(),
                                    ).await;
                                    let (success, code, timed_out) = match wait_result {
                                        Ok(Ok(s)) => (s.success(), s.code(), false),
                                        Ok(Err(_)) => (false, None, false),
                                        Err(_) => {
                                            let _ = child.kill().await;
                                            (false, None, true)
                                        }
                                    };
                                    if let Some(h) = stdout_handle { let _ = h.await; }
                                    if let Some(h) = stderr_handle { let _ = h.await; }
                                    (success, code, timed_out)
                                } => {
                                    let (success, code, timed_out) = result;
                                    let msg = if timed_out {
                                        "(command timed out after 120s)".into()
                                    } else if success {
                                        "(command completed)".into()
                                    } else {
                                        format!("(command exited with {:?})", code)
                                    };
                                    let _ = tx.send(AppEvent::BackgroundTaskDone {
                                        label,
                                        result: msg,
                                        success,
                                    });
                                }
                                _ = abort_rx => {
                                    // Send SIGTERM for graceful shutdown
                                    if let Some(pid) = child.id() {
                                        unsafe { libc::kill(pid as i32, libc::SIGTERM); }
                                    }
                                    // Grace period before force kill
                                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                    let _ = child.kill().await;
                                    let _ = tx.send(AppEvent::BackgroundTaskDone {
                                        label,
                                        result: "(cancelled)".into(),
                                        success: false,
                                    });
                                }
                            }
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::BackgroundTaskDone {
                            label,
                            result: e.to_string(),
                            success: false,
                        });
                    }
                }
            }
            name if name.starts_with("mcp_") => {
                if let Some((server_name, real_tool_name)) = self.mcp_tool_map.get(name) {
                    if let Some(server) = self.mcp_servers.get(server_name) {
                        let server = Arc::clone(server);
                        let real_name = real_tool_name.clone();
                        let args: serde_json::Value =
                            serde_json::from_str(&arguments).unwrap_or(serde_json::Value::Null);
                        tokio::spawn(async move {
                            let result = tokio::select! {
                                res = async {
                                    let mut srv = server.lock().await;
                                    srv.call_tool(&real_name, args).await
                                } => {
                                    match res {
                                        Ok(output) => (output, true),
                                        Err(e) => (e.to_string(), false),
                                    }
                                }
                                _ = abort_rx => {
                                    ("(cancelled)".to_string(), false)
                                }
                            };
                            let _ = tx.send(AppEvent::BackgroundTaskDone {
                                label,
                                result: result.0,
                                success: result.1,
                            });
                        });
                    } else {
                        let _ = tx.send(AppEvent::BackgroundTaskDone {
                            label,
                            result: format!("MCP server not found for tool: {}", name),
                            success: false,
                        });
                    }
                } else {
                    let _ = tx.send(AppEvent::BackgroundTaskDone {
                        label,
                        result: format!("Unknown MCP tool: {}", name),
                        success: false,
                    });
                }
            }
            // Non-streaming tools (file ops, etc.)
            _ => {
                let tool_name_clone = tool_name.clone();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        res = async {
                            match tool_name_clone.as_str() {
                                "read_file" => ReadFileTool.execute(&arguments).await,
                                "write_file" => WriteFileTool.execute(&arguments).await,
                                "edit_file" => EditFileTool.execute(&arguments).await,
                                "list_files" => ListFilesTool.execute(&arguments).await,
                                _ => Err(anyhow::anyhow!("Unknown tool '{}' for background execution", tool_name_clone)),
                            }
                        } => {
                            match res {
                                Ok(output) => (output, true),
                                Err(e) => (e.to_string(), false),
                            }
                        }
                        _ = abort_rx => {
                            ("(cancelled)".to_string(), false)
                        }
                    };
                    let _ = tx.send(AppEvent::BackgroundTaskDone {
                        label,
                        result: result.0,
                        success: result.1,
                    });
                });
            }
        }
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

        let display = if success {
            full_output.clone()
        } else {
            format!("Error: {}", full_output)
        };
        self.messages.push(ChatEntry::ToolOutput(display));

        // Track consecutive errors to break infinite retry loops
        if success {
            self.consecutive_errors = 0;
        } else {
            self.consecutive_errors += 1;
        }

        // Send the full output to the model
        let mut content = if success {
            full_output
        } else {
            format!("Error: {}", result)
        };

        // After repeated failures, nudge the model to change strategy
        if self.consecutive_errors >= 3 {
            content.push_str(&format!(
                "\n\n[SYSTEM: {} consecutive tool errors. \
                 Stop repeating the same approach — analyze the error messages \
                 and try a fundamentally different strategy.]",
                self.consecutive_errors
            ));
        }

        self.conversation.push(Message {
            role: "tool".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
        });
        // If no more pending foreground tools, drain completed background results
        if self.pending_tool_calls.is_empty() {
            let completed = self.bg_tasks.drain_completed();
            for result in completed {
                let status = if result.success { "completed" } else { "failed" };
                let content = format!(
                    "[Background task '{}' ({}) {} after {:.1}s]\n{}",
                    result.label, result.tool_name, status,
                    result.elapsed.as_secs_f64(), result.result
                );
                self.conversation.push(Message {
                    role: "system".into(),
                    content: Some(content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });
                self.messages.push(ChatEntry::System(content));
            }
        }
        self.process_next_tool_call();
    }

    pub fn handle_subagent_stream(&mut self, index: usize, event: StreamEvent) {
        if index >= self.subagent_states.len() || self.subagent_states[index].done {
            return;
        }
        match event {
            StreamEvent::Token(text) => {
                self.subagent_states[index].streaming_buffer.push_str(&text);
                self.messages.push(ChatEntry::SubagentOutput { index, text });
            }
            StreamEvent::ToolCallDelta(delta) => {
                let state = &mut self.subagent_states[index];
                let entry = state.assembling_tool_calls
                    .entry(delta.index)
                    .or_insert_with(|| ToolCall {
                        id: String::new(),
                        call_type: "function".into(),
                        function: FunctionCall { name: String::new(), arguments: String::new() },
                    });
                if let Some(id) = delta.id { entry.id = id; }
                if let Some(ref fc) = delta.function {
                    if let Some(ref name) = fc.name { entry.function.name.push_str(name); }
                    if let Some(ref args) = fc.arguments { entry.function.arguments.push_str(args); }
                }
            }
            StreamEvent::Usage(_) => {}
            StreamEvent::Done => {
                self.finalize_subagent(index);
            }
        }
    }

    fn finalize_subagent(&mut self, index: usize) {
        let state = &mut self.subagent_states[index];
        if !state.streaming_buffer.is_empty() {
            let text = std::mem::take(&mut state.streaming_buffer);
            state.result_parts.push(text.clone());
            state.conversation.push(Message {
                role: "assistant".into(),
                content: Some(text),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        if !state.assembling_tool_calls.is_empty() {
            let mut calls: Vec<(u32, ToolCall)> = state.assembling_tool_calls.drain().collect();
            calls.sort_by_key(|(idx, _)| *idx);
            let tool_calls: Vec<ToolCall> = calls.into_iter().map(|(_, tc)| tc).collect();
            state.conversation.push(Message {
                role: "assistant".into(),
                content: None,
                tool_calls: Some(tool_calls.clone()),
                tool_call_id: None,
            });
            state.pending_tool_calls = tool_calls;
            self.process_subagent_tool_call(index);
        } else {
            self.subagent_states[index].done = true;
            self.subagents_pending = self.subagents_pending.saturating_sub(1);
            self.check_all_subagents_done();
        }
    }

    fn check_all_subagents_done(&mut self) {
        if self.subagents_pending > 0 { return; }
        let mut combined = String::new();
        for state in &self.subagent_states {
            combined.push_str(&format!("[agent-{} result]\n", state.index));
            combined.push_str(&state.result_parts.join("\n"));
            combined.push_str("\n\n");
        }
        let combined = combined.trim().to_string();
        if let Some(call_id) = self.subagent_call_id.take() {
            let _ = self.event_tx.send(AppEvent::ToolResult {
                tool_call_id: call_id,
                result: combined,
                success: true,
            });
        }
        // States remain accessible until the next subagent dispatch overwrites
        // them; cleared in execute_tool_call when a new subagent invocation
        // begins so that concurrent test assertions can still read done flags.
    }

    #[cfg(not(tarpaulin_include))]
    fn process_subagent_tool_call(&mut self, index: usize) {
        let state = &mut self.subagent_states[index];
        let tc = match state.pending_tool_calls.first() {
            Some(tc) => tc.clone(),
            None => {
                // No more tool calls — continue streaming
                self.start_subagent_streaming(index);
                return;
            }
        };

        let tool_name = &tc.function.name;
        let command_display = if tool_name == "shell" {
            shell::extract_command(&tc.function.arguments)
                .unwrap_or_else(|| tc.function.arguments.clone())
        } else {
            tc.function.arguments.clone()
        };

        let permission_key = if tool_name == "shell" {
            shell::extract_command(&tc.function.arguments)
                .unwrap_or_else(|| tc.function.arguments.clone())
        } else {
            format!("{} {}", tool_name, tc.function.arguments)
        };

        self.messages.push(ChatEntry::SubagentOutput {
            index,
            text: format!("⚙ {} {}", tool_name, command_display),
        });

        if self.yolo
            || self.session_allow.contains(tool_name.as_str())
            || self.permissions.is_allowed(&permission_key)
        {
            self.execute_subagent_tool_call(index, tc);
        } else {
            self.messages.push(ChatEntry::ToolCall {
                name: format!("[agent-{}] {}", index, tool_name),
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

    #[cfg(not(tarpaulin_include))]
    fn execute_subagent_tool_call(&mut self, index: usize, tc: ToolCall) {
        let state = &mut self.subagent_states[index];
        state.pending_tool_calls.remove(0);

        let tool_name = tc.function.name.clone();
        let arguments = tc.function.arguments.clone();
        let call_id = tc.id.clone();
        let tx = self.event_tx.clone();

        // Todo tools — inline state mutation
        match tool_name.as_str() {
            "todo" => {
                let result = self.handle_todo_tool(&arguments);
                let _ = tx.send(AppEvent::SubagentToolResult {
                    index,
                    tool_call_id: call_id,
                    result,
                    success: true,
                });
                return;
            }
            "todo_complete" => {
                let (result, success) = match self.handle_todo_complete(&arguments) {
                    Ok(msg) => (msg, true),
                    Err(msg) => (msg, false),
                };
                let _ = tx.send(AppEvent::SubagentToolResult {
                    index,
                    tool_call_id: call_id,
                    result,
                    success,
                });
                return;
            }
            "wipe_todo" => {
                let result = self.handle_wipe_todo();
                let _ = tx.send(AppEvent::SubagentToolResult {
                    index,
                    tool_call_id: call_id,
                    result,
                    success: true,
                });
                return;
            }
            _ => {}
        }

        // Shell tool — use simple output capture to avoid interleaving
        if tool_name == "shell" {
            let args: Result<serde_json::Value, _> = serde_json::from_str(&arguments);
            let command = args
                .ok()
                .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
                .unwrap_or(arguments.clone());

            tokio::spawn(async move {
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .output()
                    .await;
                let result = match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        let mut r = String::new();
                        if !stdout.is_empty() { r.push_str(&stdout); }
                        if !stderr.is_empty() {
                            if !r.is_empty() { r.push('\n'); }
                            r.push_str("stderr: ");
                            r.push_str(&stderr);
                        }
                        if r.is_empty() { r.push_str("(no output)"); }
                        r
                    }
                    Err(e) => e.to_string(),
                };
                let _ = tx.send(AppEvent::SubagentToolResult {
                    index,
                    tool_call_id: call_id,
                    result,
                    success: true,
                });
            });
            return;
        }

        // Other built-in tools
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
                    let _ = tx.send(AppEvent::SubagentToolResult {
                        index,
                        tool_call_id: call_id,
                        result: output,
                        success: true,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::SubagentToolResult {
                        index,
                        tool_call_id: call_id,
                        result: e.to_string(),
                        success: false,
                    });
                }
            }
        });
    }

    pub fn handle_subagent_tool_result(
        &mut self,
        index: usize,
        tool_call_id: String,
        result: String,
        success: bool,
    ) {
        if index >= self.subagent_states.len() {
            return;
        }

        let display = if success {
            result.clone()
        } else {
            format!("Error: {result}")
        };
        self.messages.push(ChatEntry::SubagentOutput { index, text: display });

        let content = if success { result } else { format!("Error: {result}") };
        let state = &mut self.subagent_states[index];
        state.conversation.push(Message {
            role: "tool".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
        });

        self.process_subagent_tool_call(index);
    }

    #[cfg(not(tarpaulin_include))]
    fn start_subagent_streaming(&mut self, index: usize) {
        let state = &self.subagent_states[index];
        let mut tool_defs = self.tool_registry.definitions();
        tool_defs.extend(self.mcp_tool_defs.clone());
        tool_defs.extend(self.todo_tool_definitions());
        tool_defs.push(crate::subagent::tool_definition());

        let request = ChatRequest {
            model: self.active_model.clone(),
            messages: state.conversation.clone(),
            stream: true,
            tools: if tool_defs.is_empty() { None } else { Some(tool_defs) },
            think: true,
        };

        let tx = self.event_tx.clone();
        let server = self.api_client.server().clone();
        tokio::spawn(async move {
            let client = ApiClient::new(server);
            let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = client.chat_stream(request, stream_tx).await {
                    let _ = tx2.send(AppEvent::Error(format!("[agent-{index}] {e}")));
                }
            });
            while let Some(event) = stream_rx.recv().await {
                let _ = tx.send(AppEvent::SubagentStream { index, event });
            }
        });
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
mod app_tests {
    use super::*;
    use crate::api::client::StreamEvent;
    use tokio::sync::mpsc;

    fn test_app() -> App {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = AppConfig::default();
        let mcp_config = McpConfig::default();
        App::new(config, mcp_config, tx, None).unwrap()
    }

    // --- submit_message ---

    #[test]
    fn submit_empty_input_does_nothing() {
        let mut app = test_app();
        app.input_buffer = "   ".into();
        app.submit_message();
        assert!(app.messages.is_empty());
        // Buffer is NOT cleared when input is all whitespace because the
        // early return happens after trimming but before clearing.
        // The trim check returns early, leaving the original buffer intact.
    }

    #[test]
    fn submit_truly_empty_input_does_nothing() {
        let mut app = test_app();
        app.input_buffer = "".into();
        app.submit_message();
        assert!(app.messages.is_empty());
    }

    #[test]
    fn submit_slash_command_routes_to_handler() {
        let mut app = test_app();
        app.input_buffer = "/help".into();
        app.submit_message();
        assert!(app.input_buffer.is_empty());
        // Should have a system message with help text
        assert!(
            matches!(app.messages.last(), Some(ChatEntry::System(s)) if s.contains("Commands:"))
        );
    }

    // --- handle_slash_command ---

    #[test]
    fn slash_help() {
        let mut app = test_app();
        app.handle_slash_command("/help");
        match &app.messages[0] {
            ChatEntry::System(s) => {
                assert!(s.contains("/model"));
                assert!(s.contains("/server"));
                assert!(s.contains("/tools"));
                assert!(s.contains("/exit"));
                assert!(s.contains("/clear"));
            }
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn slash_exit() {
        let mut app = test_app();
        app.handle_slash_command("/exit");
        assert!(app.should_quit);
    }

    #[test]
    fn slash_quit() {
        let mut app = test_app();
        app.handle_slash_command("/quit");
        assert!(app.should_quit);
    }

    #[test]
    fn slash_clear() {
        let mut app = test_app();
        // Add some messages and conversation entries
        app.messages.push(ChatEntry::User("hello".into()));
        app.messages.push(ChatEntry::Assistant("hi".into()));
        app.conversation.push(Message {
            role: "system".into(),
            content: Some("system prompt".into()),
            tool_calls: None,
            tool_call_id: None,
        });
        app.conversation.push(Message {
            role: "user".into(),
            content: Some("hello".into()),
            tool_calls: None,
            tool_call_id: None,
        });

        app.handle_slash_command("/clear");

        // Messages should have only the "Conversation cleared." system message
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("cleared")));

        // Conversation should retain only system messages
        assert!(app.conversation.iter().all(|m| m.role == "system"));
    }

    #[test]
    fn slash_model_with_arg() {
        let mut app = test_app();
        app.handle_slash_command("/model codellama:13b");
        assert_eq!(app.active_model, "codellama:13b");
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("codellama:13b")));
    }

    #[tokio::test]
    async fn slash_model_without_arg() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("Current model:")));
    }

    #[test]
    fn slash_server_with_known_name() {
        let mut app = test_app();
        app.handle_slash_command("/server local");
        assert_eq!(app.active_server_name, "Local Ollama");
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("Local Ollama")));
    }

    #[test]
    fn slash_server_with_unknown_name() {
        let mut app = test_app();
        app.handle_slash_command("/server nonexistent");
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("Unknown server")));
    }

    #[test]
    fn slash_server_without_arg() {
        let mut app = test_app();
        app.handle_slash_command("/server");
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("Servers:")));
    }

    #[test]
    fn server_switch_resets_health() {
        let mut app = test_app();
        app.server_healthy = Some(true);
        app.handle_slash_command("/server local");
        assert!(app.server_healthy.is_none());
    }

    #[test]
    fn slash_tools() {
        let mut app = test_app();
        app.handle_slash_command("/tools");
        assert!(
            matches!(&app.messages[0], ChatEntry::System(s) if s.contains("Built-in tools: 8") && s.contains("Todo tools:"))
        );
    }

    #[test]
    fn slash_tools_with_mcp() {
        let mut app = test_app();
        app.mcp_tool_defs.push(ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "mcp_test".into(),
                description: "test".into(),
                parameters: serde_json::json!({}),
            },
        });
        app.handle_slash_command("/tools");
        match &app.messages[0] {
            ChatEntry::System(s) => {
                assert!(s.contains("Built-in tools:"));
                assert!(s.contains("MCP tools: 1"));
            }
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn slash_skills_empty() {
        let mut app = test_app();
        app.skills.clear();
        app.handle_slash_command("/skills");
        assert!(matches!(&app.messages[0], ChatEntry::System(s) if s.contains("No skills loaded")));
    }

    #[test]
    fn slash_skills_with_entries() {
        let mut app = test_app();
        app.skills.insert(
            "review".into(),
            crate::skills::Skill {
                name: "review".into(),
                description: "Review code".into(),
                content: "Review content".into(),
            },
        );
        app.handle_slash_command("/skills");
        match &app.messages[0] {
            ChatEntry::System(s) => {
                assert!(s.contains("Skills:"));
                assert!(s.contains("/review"));
            }
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn slash_unknown_command() {
        let mut app = test_app();
        app.handle_slash_command("/foobar");
        assert!(
            matches!(&app.messages[0], ChatEntry::System(s) if s.contains("Unknown command: /foobar"))
        );
    }

    #[test]
    fn slash_skill_activation() {
        let mut app = test_app();
        app.skills.insert(
            "review".into(),
            crate::skills::Skill {
                name: "review".into(),
                description: "Review code".into(),
                content: "Review the code carefully.".into(),
            },
        );
        app.handle_slash_command("/review");

        // Skill content should be added to conversation as system message
        let last_conv = app.conversation.last().unwrap();
        assert_eq!(last_conv.role, "system");
        assert_eq!(
            last_conv.content.as_deref(),
            Some("Review the code carefully.")
        );

        // User should see activation message
        assert!(
            matches!(&app.messages.last().unwrap(), ChatEntry::System(s) if s.contains("activated"))
        );
    }

    // --- server_healthy and last_token_usage defaults ---

    #[test]
    fn app_new_health_and_usage_defaults() {
        let app = test_app();
        assert!(app.server_healthy.is_none());
        assert!(app.last_token_usage.is_none());
    }

    #[test]
    fn handle_stream_usage_sets_last_token_usage() {
        let mut app = test_app();
        let usage = crate::api::types::Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        };
        app.handle_stream_event(StreamEvent::Usage(usage));
        let u = app.last_token_usage.as_ref().unwrap();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 50);
        assert_eq!(u.total_tokens, 150);
    }

    // --- handle_stream_event ---

    #[test]
    fn stream_event_token_appends() {
        let mut app = test_app();
        app.handle_stream_event(StreamEvent::Token("Hello".into()));
        assert_eq!(app.streaming_buffer, "Hello");
        app.handle_stream_event(StreamEvent::Token(" World".into()));
        assert_eq!(app.streaming_buffer, "Hello World");
    }

    #[test]
    fn waiting_for_response_cleared_by_token() {
        let mut app = test_app();
        app.waiting_for_response = true;
        app.handle_stream_event(StreamEvent::Token("hi".into()));
        assert!(!app.waiting_for_response);
    }

    #[test]
    fn waiting_for_response_cleared_by_tool_call_delta() {
        let mut app = test_app();
        app.waiting_for_response = true;
        let delta = DeltaToolCall {
            index: 0,
            id: Some("call_1".into()),
            call_type: Some("function".into()),
            function: Some(DeltaFunctionCall {
                name: Some("shell".into()),
                arguments: Some("{}".into()),
            }),
        };
        app.handle_stream_event(StreamEvent::ToolCallDelta(delta));
        assert!(!app.waiting_for_response);
    }

    #[test]
    fn waiting_for_response_cleared_by_finalize() {
        let mut app = test_app();
        app.waiting_for_response = true;
        app.streaming_buffer = "answer".into();
        app.handle_stream_event(StreamEvent::Done);
        assert!(!app.waiting_for_response);
    }

    #[test]
    fn stream_event_tool_call_delta_assembles() {
        let mut app = test_app();
        let delta = DeltaToolCall {
            index: 0,
            id: Some("call_123".into()),
            call_type: Some("function".into()),
            function: Some(DeltaFunctionCall {
                name: Some("shell".into()),
                arguments: Some(r#"{"comm"#.into()),
            }),
        };
        app.handle_stream_event(StreamEvent::ToolCallDelta(delta));

        let delta2 = DeltaToolCall {
            index: 0,
            id: None,
            call_type: None,
            function: Some(DeltaFunctionCall {
                name: None,
                arguments: Some(r#"and":"ls"}"#.into()),
            }),
        };
        app.handle_stream_event(StreamEvent::ToolCallDelta(delta2));

        let assembled = &app.assembling_tool_calls[&0];
        assert_eq!(assembled.id, "call_123");
        assert_eq!(assembled.function.name, "shell");
        assert_eq!(assembled.function.arguments, r#"{"command":"ls"}"#);
    }

    // --- finalize_response ---

    #[test]
    fn finalize_simple_text() {
        let mut app = test_app();
        app.streaming_buffer = "Hello there!".into();
        app.finalize_response();

        assert!(app.streaming_buffer.is_empty());
        assert!(matches!(&app.messages[0], ChatEntry::Assistant(s) if s == "Hello there!"));
        // Should be added to conversation
        let last = app.conversation.last().unwrap();
        assert_eq!(last.role, "assistant");
    }

    #[test]
    fn finalize_with_think_tags() {
        let mut app = test_app();
        app.streaming_buffer = "<think>Let me reason about this.</think>Here is my answer.".into();
        app.finalize_response();

        // Should have a Thinking entry and an Assistant entry
        let mut has_thinking = false;
        let mut has_assistant = false;
        for entry in &app.messages {
            match entry {
                ChatEntry::Thinking { content, .. } => {
                    assert_eq!(content, "Let me reason about this.");
                    has_thinking = true;
                }
                ChatEntry::Assistant(s) => {
                    assert_eq!(s, "Here is my answer.");
                    has_assistant = true;
                }
                _ => {}
            }
        }
        assert!(has_thinking);
        assert!(has_assistant);
    }

    #[test]
    fn finalize_with_unclosed_think_tag() {
        let mut app = test_app();
        app.streaming_buffer = "<think>Still thinking...".into();
        app.finalize_response();

        assert!(matches!(&app.messages[0], ChatEntry::Thinking { content, .. } if content == "Still thinking..."));
    }

    #[test]
    fn finalize_empty_streaming_buffer() {
        let mut app = test_app();
        app.streaming_buffer.clear();
        let msg_count = app.messages.len();
        app.finalize_response();
        // No messages should be added
        assert_eq!(app.messages.len(), msg_count);
    }

    #[test]
    fn finalize_only_whitespace_think_tag() {
        let mut app = test_app();
        app.streaming_buffer = "<think>   </think>Actual answer.".into();
        app.finalize_response();

        // Whitespace-only thinking should not produce a Thinking entry
        for entry in &app.messages {
            assert!(!matches!(entry, ChatEntry::Thinking { .. }));
        }
        assert!(
            matches!(app.messages.last(), Some(ChatEntry::Assistant(s)) if s == "Actual answer.")
        );
    }

    #[test]
    fn finalize_text_before_and_after_think() {
        let mut app = test_app();
        app.streaming_buffer = "Prefix <think>reasoning</think> Suffix".into();
        app.finalize_response();

        let mut found_thinking = false;
        let mut found_assistant = false;
        for entry in &app.messages {
            match entry {
                ChatEntry::Thinking { content, .. } if content == "reasoning" => found_thinking = true,
                ChatEntry::Assistant(s) if s.contains("Prefix") && s.contains("Suffix") => {
                    found_assistant = true
                }
                _ => {}
            }
        }
        assert!(found_thinking);
        assert!(found_assistant);
    }

    #[test]
    fn finalize_resets_thinking_state() {
        let mut app = test_app();
        app.in_thinking = true;
        app.thinking_buffer = "leftover".into();
        app.streaming_buffer = "response".into();
        app.finalize_response();

        assert!(!app.in_thinking);
        assert!(app.thinking_buffer.is_empty());
    }

    // --- handle_tool_result ---

    #[tokio::test]
    async fn handle_tool_result_success() {
        let mut app = test_app();
        app.handle_tool_result("call_1".into(), "output text".into(), true);

        assert!(matches!(&app.messages[0], ChatEntry::ToolOutput(s) if s == "output text"));
        let conv_last = app.conversation.last().unwrap();
        assert_eq!(conv_last.role, "tool");
        assert_eq!(conv_last.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(conv_last.content.as_deref(), Some("output text"));
    }

    #[tokio::test]
    async fn handle_tool_result_failure() {
        let mut app = test_app();
        app.handle_tool_result("call_2".into(), "some error".into(), false);

        assert!(matches!(&app.messages[0], ChatEntry::ToolOutput(s) if s.contains("Error:")));
        let conv_last = app.conversation.last().unwrap();
        assert!(conv_last.content.as_deref().unwrap().contains("Error:"));
    }

    #[tokio::test]
    async fn handle_tool_result_with_buffered_output() {
        let mut app = test_app();
        app.tool_output_buffer = "line1\nline2\n".into();
        app.handle_tool_result("call_3".into(), "(command completed)".into(), true);

        // Buffered output should be used directly since result is the sentinel
        assert!(matches!(&app.messages[0], ChatEntry::ToolOutput(s) if s.contains("line1")));
    }

    #[tokio::test]
    async fn handle_tool_result_buffer_plus_extra() {
        let mut app = test_app();
        app.tool_output_buffer = "buffered".into();
        app.handle_tool_result("call_4".into(), "extra output".into(), true);

        // Both buffered and extra should be combined
        assert!(
            matches!(&app.messages[0], ChatEntry::ToolOutput(s) if s.contains("buffered") && s.contains("extra output"))
        );
    }

    // --- handle_permission_response ---

    #[tokio::test]
    async fn permission_response_deny() {
        let mut app = test_app();
        app.messages.push(ChatEntry::ToolCall {
            name: "shell".into(),
            command: "rm -rf /".into(),
            status: "pending".into(),
        });
        app.pending_permission = Some(PendingPermission {
            tool_name: "shell".into(),
            command: "rm -rf /".into(),
            tool_call_id: "call_deny".into(),
            arguments: r#"{"command":"rm -rf /"}"#.into(),
        });

        app.handle_permission_response(false, false);

        assert!(app.pending_permission.is_none());
        // The last ToolCall entry should be "denied" (messages may have more
        // entries appended by process_next_tool_call, so search backwards)
        let denied = app
            .messages
            .iter()
            .any(|m| matches!(m, ChatEntry::ToolCall { status, .. } if status == "denied"));
        assert!(denied);
        // Should have added a permission denied tool message
        let tool_msg = app.conversation.iter().find(|m| m.role == "tool").unwrap();
        assert!(
            tool_msg
                .content
                .as_deref()
                .unwrap()
                .contains("Permission denied")
        );
    }

    #[test]
    fn permission_response_no_pending() {
        let mut app = test_app();
        let msg_count = app.messages.len();
        app.handle_permission_response(true, false);
        // Should be a no-op
        assert_eq!(app.messages.len(), msg_count);
    }

    // --- handle_pattern_submit ---

    #[tokio::test]
    async fn pattern_submit_empty_pattern() {
        let mut app = test_app();
        app.pattern_input = Some(String::new());
        app.messages.push(ChatEntry::ToolCall {
            name: "shell".into(),
            command: "echo hi".into(),
            status: "pending".into(),
        });
        // The tool call needs to be in pending_tool_calls because
        // handle_permission_response(allow=true) calls execute_tool_call
        // which removes from pending_tool_calls.
        app.pending_tool_calls.push(ToolCall {
            id: "call_p".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command":"echo hi"}"#.into(),
            },
        });
        app.pending_permission = Some(PendingPermission {
            tool_name: "shell".into(),
            command: "echo hi".into(),
            tool_call_id: "call_p".into(),
            arguments: r#"{"command":"echo hi"}"#.into(),
        });

        app.handle_pattern_submit();
        assert!(app.pattern_input.is_none());
    }

    #[tokio::test]
    async fn pattern_submit_with_pattern() {
        let mut app = test_app();
        app.pattern_input = Some("echo *".into());
        app.messages.push(ChatEntry::ToolCall {
            name: "shell".into(),
            command: "echo hi".into(),
            status: "pending".into(),
        });
        app.pending_tool_calls.push(ToolCall {
            id: "call_pp".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command":"echo hi"}"#.into(),
            },
        });
        app.pending_permission = Some(PendingPermission {
            tool_name: "shell".into(),
            command: "echo hi".into(),
            tool_call_id: "call_pp".into(),
            arguments: r#"{"command":"echo hi"}"#.into(),
        });

        app.handle_pattern_submit();
        assert!(app.pattern_input.is_none());
        // The pattern should have been saved to permissions
        assert!(app.permissions.is_allowed("echo hello"));
    }

    // --- init command ---

    #[tokio::test]
    async fn slash_init_agents_already_exists() {
        let mut app = test_app();
        // /init either reports "already exists" or starts generating.
        // In the test working dir (project root), AGENTS.md may or may not
        // exist. Either outcome is valid.
        app.handle_slash_command("/init");
        let has_relevant_msg = app.messages.iter().any(|m| {
            matches!(m,
                ChatEntry::System(s) if s.contains("AGENTS.md already exists") || s.contains("Generating"))
            || matches!(m, ChatEntry::User(s) if s.contains("Examine this project"))
        });
        assert!(has_relevant_msg);
    }

    // --- process_next_tool_call ---

    #[tokio::test]
    async fn process_next_tool_call_session_allowed() {
        let mut app = test_app();
        // read_file is in session_allow by default
        app.pending_tool_calls.push(ToolCall {
            id: "call_read".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "read_file".into(),
                arguments: r#"{"path": "/nonexistent"}"#.into(),
            },
        });

        app.process_next_tool_call();

        // Should have been allowed (status "allowed") because read_file is in session_allow
        let has_allowed = app
            .messages
            .iter()
            .any(|m| matches!(m, ChatEntry::ToolCall { status, .. } if status == "allowed"));
        assert!(has_allowed);
    }

    #[test]
    fn process_next_tool_call_shell_needs_permission() {
        let mut app = test_app();
        app.pending_tool_calls.push(ToolCall {
            id: "call_shell".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command": "rm -rf /"}"#.into(),
            },
        });

        app.process_next_tool_call();

        // Should be pending permission
        assert!(app.pending_permission.is_some());
        let has_pending = app
            .messages
            .iter()
            .any(|m| matches!(m, ChatEntry::ToolCall { status, .. } if status == "pending"));
        assert!(has_pending);
    }

    #[tokio::test]
    async fn process_next_tool_call_yolo_bypasses_permission() {
        let mut app = test_app();
        app.yolo = true;
        app.pending_tool_calls.push(ToolCall {
            id: "call_yolo".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command": "echo yolo"}"#.into(),
            },
        });

        app.process_next_tool_call();

        // Should be auto-allowed
        let has_allowed = app
            .messages
            .iter()
            .any(|m| matches!(m, ChatEntry::ToolCall { status, .. } if status == "allowed"));
        assert!(has_allowed);
        assert!(app.pending_permission.is_none());
    }

    // --- finalize_response with tool calls ---

    #[tokio::test]
    async fn finalize_response_with_assembled_tool_calls() {
        let mut app = test_app();
        // Simulate assembled tool calls (as if streamed in via ToolCallDelta)
        app.assembling_tool_calls.insert(
            0,
            ToolCall {
                id: "call_assembled".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: r#"{"path": "/tmp/test"}"#.into(),
                },
            },
        );

        app.finalize_response();

        // Should have pushed the assistant message with tool_calls
        let assistant_msg = app
            .conversation
            .iter()
            .find(|m| m.role == "assistant" && m.tool_calls.is_some());
        assert!(assistant_msg.is_some());
        let tool_calls = assistant_msg.unwrap().tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "read_file");
    }

    // --- StreamEvent::Done triggers finalize ---

    #[tokio::test]
    async fn stream_event_done_triggers_finalize() {
        let mut app = test_app();
        app.streaming_buffer = "final answer".into();
        app.handle_stream_event(StreamEvent::Done);

        // finalize_response should have been called
        assert!(app.streaming_buffer.is_empty());
        assert!(matches!(&app.messages[0], ChatEntry::Assistant(s) if s == "final answer"));
    }

    // --- handle_permission_response with save ---

    #[tokio::test]
    async fn permission_response_allow_with_save() {
        let mut app = test_app();
        app.messages.push(ChatEntry::ToolCall {
            name: "shell".into(),
            command: "git status".into(),
            status: "pending".into(),
        });
        app.pending_tool_calls.push(ToolCall {
            id: "call_save".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command":"git status"}"#.into(),
            },
        });
        app.pending_permission = Some(PendingPermission {
            tool_name: "shell".into(),
            command: "git status".into(),
            tool_call_id: "call_save".into(),
            arguments: r#"{"command":"git status"}"#.into(),
        });

        app.handle_permission_response(true, true);

        // The command should now be permanently allowed
        assert!(app.permissions.is_allowed("git status"));
    }

    // --- permission_response deny with pending_tool_calls ---

    #[tokio::test]
    async fn permission_response_deny_removes_from_pending() {
        let mut app = test_app();
        app.messages.push(ChatEntry::ToolCall {
            name: "shell".into(),
            command: "rm -rf".into(),
            status: "pending".into(),
        });
        app.pending_tool_calls.push(ToolCall {
            id: "call_deny2".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"command":"rm -rf"}"#.into(),
            },
        });
        app.pending_permission = Some(PendingPermission {
            tool_name: "shell".into(),
            command: "rm -rf".into(),
            tool_call_id: "call_deny2".into(),
            arguments: r#"{"command":"rm -rf"}"#.into(),
        });

        app.handle_permission_response(false, false);

        // pending_tool_calls should be empty now (the one entry was removed)
        assert!(app.pending_tool_calls.is_empty());
    }

    // --- submit_message regular (non-slash) ---

    #[tokio::test]
    async fn submit_regular_message() {
        let mut app = test_app();
        app.input_buffer = "Hello, how are you?".into();
        app.submit_message();

        assert!(app.input_buffer.is_empty());
        assert!(matches!(&app.messages[0], ChatEntry::User(s) if s == "Hello, how are you?"));
        let last_conv = app.conversation.last().unwrap();
        assert_eq!(last_conv.role, "user");
        assert_eq!(last_conv.content.as_deref(), Some("Hello, how are you?"));
    }

    // --- App::new with missing server config ---

    #[test]
    fn app_new_with_missing_server_config() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut config = AppConfig::default();
        // Remove the "local" server so the fallback kicks in
        config.servers.clear();
        config.defaults.server = "nonexistent".into();
        let mcp_config = McpConfig::default();
        let app = App::new(config, mcp_config, tx, None).unwrap();

        // Should fall back to default "Local Ollama"
        assert_eq!(app.active_server_name, "Local Ollama");
    }

    // --- App::new ---

    #[test]
    fn app_new_default_state() {
        let app = test_app();
        assert!(app.messages.is_empty());
        assert!(app.input_buffer.is_empty());
        assert!(app.streaming_buffer.is_empty());
        assert_eq!(app.active_model, "llama3:8b");
        assert_eq!(app.active_server_name, "Local Ollama");
        assert_eq!(app.tool_count, 12); // 8 built-in + 3 todo + 1 subagent
        assert!(!app.should_quit);
        assert!(!app.yolo);
        assert!(app.pending_permission.is_none());
        assert!(app.pattern_input.is_none());
        assert!(app.pending_tool_calls.is_empty());
    }

    #[test]
    fn app_new_session_allow_defaults() {
        let app = test_app();
        assert!(app.session_allow.contains("read_file"));
        assert!(app.session_allow.contains("write_file"));
        assert!(app.session_allow.contains("edit_file"));
        assert!(app.session_allow.contains("list_files"));
        assert!(!app.session_allow.contains("shell"));
    }

    #[test]
    fn todo_items_start_empty() {
        let app = test_app();
        assert!(app.todo_items.is_empty());
    }

    #[test]
    fn todo_tool_definitions_are_present() {
        let app = test_app();
        let defs = app.todo_tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"todo"));
        assert!(names.contains(&"todo_complete"));
        assert!(names.contains(&"wipe_todo"));
        assert_eq!(defs.len(), 3);
    }

    #[test]
    fn handle_todo_tool_sets_items() {
        let mut app = test_app();
        app.handle_todo_tool(r#"{"items":["first","second","third"]}"#);
        assert_eq!(app.todo_items.len(), 3);
        assert_eq!(app.todo_items[0].text, "first");
        assert_eq!(app.todo_items[1].text, "second");
        assert_eq!(app.todo_items[2].text, "third");
        assert!(app.todo_items.iter().all(|t| !t.done));
    }

    #[test]
    fn handle_todo_tool_replaces_existing() {
        let mut app = test_app();
        app.handle_todo_tool(r#"{"items":["old"]}"#);
        app.handle_todo_tool(r#"{"items":["new1","new2"]}"#);
        assert_eq!(app.todo_items.len(), 2);
        assert_eq!(app.todo_items[0].text, "new1");
    }

    #[test]
    fn handle_todo_complete_marks_done() {
        let mut app = test_app();
        app.handle_todo_tool(r#"{"items":["a","b","c"]}"#);
        let result = app.handle_todo_complete(r#"{"index":1}"#);
        assert!(app.todo_items[1].done);
        assert!(!app.todo_items[0].done);
        assert!(result.unwrap().contains("Completed"));
    }

    #[test]
    fn handle_todo_complete_out_of_bounds() {
        let mut app = test_app();
        app.handle_todo_tool(r#"{"items":["a"]}"#);
        let result = app.handle_todo_complete(r#"{"index":5}"#);
        assert!(result.unwrap_err().contains("Invalid"));
    }

    #[test]
    fn handle_todo_tool_invalid_json() {
        let mut app = test_app();
        let result = app.handle_todo_tool("not json");
        assert!(result.contains("Invalid todo arguments"));
        assert!(app.todo_items.is_empty());
    }

    #[test]
    fn handle_todo_complete_invalid_json() {
        let mut app = test_app();
        let result = app.handle_todo_complete(r#"{"index": "not a number"}"#);
        assert!(result.unwrap_err().contains("Invalid todo_complete arguments"));
    }

    #[test]
    fn handle_wipe_todo_clears_list() {
        let mut app = test_app();
        app.handle_todo_tool(r#"{"items":["a","b"]}"#);
        app.handle_wipe_todo();
        assert!(app.todo_items.is_empty());
    }

    #[test]
    fn subagent_state_defaults() {
        let app = test_app();
        assert!(app.subagent_states.is_empty());
        assert_eq!(app.subagents_pending, 0);
        assert!(app.subagent_call_id.is_none());
    }

    #[test]
    fn handle_subagent_token_appends_to_buffer() {
        let mut app = test_app();
        app.subagent_states.push(crate::subagent::SubagentState::new(0, None, "test"));
        app.subagents_pending = 1;
        app.handle_subagent_stream(0, StreamEvent::Token("hello".into()));
        assert_eq!(app.subagent_states[0].streaming_buffer, "hello");
    }

    #[test]
    fn handle_subagent_token_adds_chat_entry() {
        let mut app = test_app();
        app.subagent_states.push(crate::subagent::SubagentState::new(0, None, "test"));
        app.subagents_pending = 1;
        app.handle_subagent_stream(0, StreamEvent::Token("hello".into()));
        assert!(matches!(
            app.messages.last(),
            Some(ChatEntry::SubagentOutput { index: 0, text }) if text == "hello"
        ));
    }

    #[tokio::test]
    async fn handle_subagent_tool_result_adds_to_conversation() {
        let mut app = test_app();
        let mut state = crate::subagent::SubagentState::new(0, None, "test");
        state.pending_tool_calls.push(ToolCall {
            id: "tc_1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "read_file".into(),
                arguments: "{}".into(),
            },
        });
        app.subagent_states.push(state);
        app.subagents_pending = 1;
        app.subagent_call_id = Some("parent_call".into());

        app.handle_subagent_tool_result(0, "tc_1".into(), "file contents".into(), true);
        let last_msg = app.subagent_states[0].conversation.last().unwrap();
        assert_eq!(last_msg.role, "tool");
        assert_eq!(last_msg.content.as_deref(), Some("file contents"));
        assert_eq!(last_msg.tool_call_id.as_deref(), Some("tc_1"));
    }

    #[test]
    fn subagent_done_without_tools_marks_complete() {
        let mut app = test_app();
        app.subagent_states.push(crate::subagent::SubagentState::new(0, None, "test"));
        app.subagents_pending = 1;
        app.subagent_call_id = Some("call_1".into());
        app.subagent_states[0].streaming_buffer = "result text".into();
        app.handle_subagent_stream(0, StreamEvent::Done);
        assert!(app.subagent_states[0].done);
        assert_eq!(app.subagents_pending, 0);
    }
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

    #[test]
    fn parse_mdc_no_frontmatter() {
        let text = "Just content, no frontmatter.";
        let (fm, content) = parse_mdc_frontmatter(text).unwrap();
        assert!(fm.is_empty());
        assert_eq!(content, text);
    }

    #[test]
    fn parse_mdc_unclosed_frontmatter() {
        let text = "---\nalwaysApply: true\nContent without closing.";
        let result = parse_mdc_frontmatter(text);
        // Unclosed frontmatter returns None
        assert!(result.is_none());
    }

    #[test]
    fn extract_mdc_globs_comment_in_list() {
        let fm = "globs:\n  - src/**/*.rs\n  # comment\n  - tests/*.rs";
        let globs = extract_mdc_globs(fm);
        assert_eq!(globs, vec!["src/**/*.rs", "tests/*.rs"]);
    }

    #[test]
    fn extract_mdc_globs_ends_on_non_list_line() {
        let fm = "globs:\n  - *.rs\ndescription: some rule";
        let globs = extract_mdc_globs(fm);
        assert_eq!(globs, vec!["*.rs"]);
    }

    #[test]
    fn extract_mdc_globs_single_inline_glob() {
        let fm = "globs: *.py\nalwaysApply: false";
        let globs = extract_mdc_globs(fm);
        assert_eq!(globs, vec!["*.py"]);
    }

    #[test]
    fn extract_mdc_globs_empty_inline() {
        let fm = "globs:\nalwaysApply: false";
        let globs = extract_mdc_globs(fm);
        // Empty globs line, no list follows (next line is a different key)
        assert!(globs.is_empty());
    }

    #[test]
    fn load_mdc_rules_nonexistent_dir() {
        let dir = std::path::PathBuf::from("/tmp/llama-chat-test-mdc-nonexistent-xyz");
        let rules = load_mdc_rules(&dir);
        assert!(rules.is_empty());
    }

    #[test]
    fn load_mdc_rules_always_apply() {
        let dir = std::env::temp_dir().join("llama-chat-test-mdc-always");
        let rules_dir = dir.join(".cursor/rules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&rules_dir).unwrap();

        std::fs::write(
            rules_dir.join("style.mdc"),
            "---\nalwaysApply: true\n---\n\nUse consistent formatting.",
        )
        .unwrap();

        let rules = load_mdc_rules(&dir);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].contains("Use consistent formatting."));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_mdc_rules_empty_content_skipped() {
        let dir = std::env::temp_dir().join("llama-chat-test-mdc-empty");
        let rules_dir = dir.join(".cursor/rules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&rules_dir).unwrap();

        std::fs::write(
            rules_dir.join("empty.mdc"),
            "---\nalwaysApply: true\n---\n   ",
        )
        .unwrap();

        let rules = load_mdc_rules(&dir);
        assert!(rules.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_mdc_rules_non_mdc_files_skipped() {
        let dir = std::env::temp_dir().join("llama-chat-test-mdc-nonmdc");
        let rules_dir = dir.join(".cursor/rules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&rules_dir).unwrap();

        std::fs::write(rules_dir.join("readme.md"), "# Not an MDC file").unwrap();
        std::fs::write(
            rules_dir.join("actual.mdc"),
            "---\nalwaysApply: true\n---\n\nReal rule.",
        )
        .unwrap();

        let rules = load_mdc_rules(&dir);
        assert_eq!(rules.len(), 1);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_mdc_rules_manual_trigger_skipped() {
        let dir = std::env::temp_dir().join("llama-chat-test-mdc-manual");
        let rules_dir = dir.join(".cursor/rules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&rules_dir).unwrap();

        // No globs, not alwaysApply — this is manual-trigger only
        std::fs::write(
            rules_dir.join("manual.mdc"),
            "---\ndescription: manual\nalwaysApply: false\n---\n\nManual rule content.",
        )
        .unwrap();

        let rules = load_mdc_rules(&dir);
        assert!(rules.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_mdc_rules_with_matching_glob() {
        let dir = std::env::temp_dir().join("llama-chat-test-mdc-glob");
        let rules_dir = dir.join(".cursor/rules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&rules_dir).unwrap();

        // Create a file that matches the glob
        std::fs::write(dir.join("test.rs"), "fn main() {}").unwrap();

        std::fs::write(
            rules_dir.join("rust.mdc"),
            "---\nglobs: *.rs\nalwaysApply: false\n---\n\nRust style rules.",
        )
        .unwrap();

        let rules = load_mdc_rules(&dir);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].contains("Rust style rules."));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_mdc_rules_with_non_matching_glob() {
        let dir = std::env::temp_dir().join("llama-chat-test-mdc-noglob");
        let rules_dir = dir.join(".cursor/rules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&rules_dir).unwrap();

        // No .py files in this temp dir
        std::fs::write(
            rules_dir.join("python.mdc"),
            "---\nglobs: *.py\nalwaysApply: false\n---\n\nPython rules.",
        )
        .unwrap();

        let rules = load_mdc_rules(&dir);
        assert!(rules.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }
}

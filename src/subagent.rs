use std::collections::HashMap;

use crate::api::types::*;

#[derive(Debug, Clone)]
pub struct SubagentState {
    pub index: usize,
    pub conversation: Vec<Message>,
    pub streaming_buffer: String,
    pub assembling_tool_calls: HashMap<u32, ToolCall>,
    pub pending_tool_calls: Vec<ToolCall>,
    pub result_parts: Vec<String>,
    pub done: bool,
}

impl SubagentState {
    pub fn new(index: usize, system: Option<&str>, prompt: &str) -> Self {
        let mut conversation = Vec::new();
        if let Some(sys) = system {
            conversation.push(Message {
                role: "system".into(),
                content: Some(sys.to_string()),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        conversation.push(Message {
            role: "user".into(),
            content: Some(prompt.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });
        Self {
            index,
            conversation,
            streaming_buffer: String::new(),
            assembling_tool_calls: HashMap::new(),
            pending_tool_calls: Vec::new(),
            result_parts: Vec::new(),
            done: false,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct SubagentArgs {
    pub agents: Vec<AgentSpec>,
}

#[derive(Debug, serde::Deserialize)]
pub struct AgentSpec {
    pub prompt: String,
    pub system: Option<String>,
}

pub fn parse_args(arguments: &str) -> Result<SubagentArgs, String> {
    serde_json::from_str::<SubagentArgs>(arguments)
        .map_err(|e| format!("Invalid subagent arguments: {e}"))
}

pub fn tool_definition() -> ToolDefinition {
    ToolDefinition {
        tool_type: "function".into(),
        function: FunctionDefinition {
            name: "subagent".into(),
            description: "Spawn one or more subagents to handle tasks concurrently. Each subagent gets its own conversation and can use all available tools.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agents": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "prompt": {
                                    "type": "string",
                                    "description": "The task prompt for this subagent"
                                },
                                "system": {
                                    "type": "string",
                                    "description": "Optional system instruction for this subagent"
                                }
                            },
                            "required": ["prompt"]
                        },
                        "description": "List of subagents to spawn. All run concurrently."
                    }
                },
                "required": ["agents"]
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_agent() {
        let json = r#"{"agents":[{"prompt":"do something"}]}"#;
        let args = parse_args(json).unwrap();
        assert_eq!(args.agents.len(), 1);
        assert_eq!(args.agents[0].prompt, "do something");
        assert!(args.agents[0].system.is_none());
    }

    #[test]
    fn parse_multiple_agents_with_system() {
        let json = r#"{"agents":[{"prompt":"task A","system":"you are A"},{"prompt":"task B"}]}"#;
        let args = parse_args(json).unwrap();
        assert_eq!(args.agents.len(), 2);
        assert_eq!(args.agents[0].system.as_deref(), Some("you are A"));
        assert!(args.agents[1].system.is_none());
    }

    #[test]
    fn parse_invalid_json() {
        let result = parse_args("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid subagent arguments"));
    }

    #[test]
    fn subagent_state_new_with_system() {
        let state = SubagentState::new(0, Some("be helpful"), "do X");
        assert_eq!(state.index, 0);
        assert_eq!(state.conversation.len(), 2);
        assert_eq!(state.conversation[0].role, "system");
        assert_eq!(state.conversation[0].content.as_deref(), Some("be helpful"));
        assert_eq!(state.conversation[1].role, "user");
        assert_eq!(state.conversation[1].content.as_deref(), Some("do X"));
        assert!(!state.done);
    }

    #[test]
    fn subagent_state_new_without_system() {
        let state = SubagentState::new(1, None, "do Y");
        assert_eq!(state.conversation.len(), 1);
        assert_eq!(state.conversation[0].role, "user");
    }

    #[test]
    fn tool_definition_has_correct_name() {
        let def = tool_definition();
        assert_eq!(def.function.name, "subagent");
        assert_eq!(def.tool_type, "function");
    }
}

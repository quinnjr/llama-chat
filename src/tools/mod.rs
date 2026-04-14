pub mod background;
pub mod background_subagent;
pub mod filesystem;
pub mod permissions;
pub mod shell;

use anyhow::Result;
use std::collections::HashMap;

use crate::api::types::{FunctionDefinition, ToolDefinition};

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, arguments: &str) -> Result<String>;

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: self.name().into(),
                description: self.description().into(),
                parameters: self.parameters_schema(),
            },
        }
    }
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().into(), tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTool;
    #[async_trait::async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            "mock"
        }
        fn description(&self) -> &str {
            "A mock tool"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: &str) -> anyhow::Result<String> {
            Ok("mock result".into())
        }
    }

    struct AnotherMockTool;
    #[async_trait::async_trait]
    impl Tool for AnotherMockTool {
        fn name(&self) -> &str {
            "another"
        }
        fn description(&self) -> &str {
            "Another mock tool"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: &str) -> anyhow::Result<String> {
            Ok("another result".into())
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.tool_count(), 0);
        assert!(registry.definitions().is_empty());
    }

    #[test]
    fn register_and_count() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        assert_eq!(registry.tool_count(), 1);
        registry.register(Box::new(AnotherMockTool));
        assert_eq!(registry.tool_count(), 2);
    }

    #[test]
    fn definitions_reflect_registered_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        registry.register(Box::new(AnotherMockTool));

        let defs = registry.definitions();
        assert_eq!(defs.len(), 2);

        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"mock"));
        assert!(names.contains(&"another"));

        for def in &defs {
            assert_eq!(def.tool_type, "function");
        }
    }

    #[test]
    fn to_definition_includes_description_and_schema() {
        let tool = MockTool;
        let def = tool.to_definition();
        assert_eq!(def.function.name, "mock");
        assert_eq!(def.function.description, "A mock tool");
        assert_eq!(
            def.function.parameters,
            serde_json::json!({"type": "object"})
        );
    }

    #[test]
    fn register_same_name_overwrites() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        registry.register(Box::new(MockTool));
        assert_eq!(registry.tool_count(), 1);
    }
}

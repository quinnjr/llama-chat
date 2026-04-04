use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use crate::tools::Tool;

pub struct ShellTool;

#[derive(Deserialize)]
struct ShellArgs {
    command: String,
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use for running programs, checking files, git operations, etc."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, arguments: &str) -> Result<String> {
        let args: ShellArgs = serde_json::from_str(arguments)?;
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("stderr: ");
            result.push_str(&stderr);
        }
        if result.is_empty() {
            result.push_str("(no output)");
        }
        Ok(result)
    }
}

pub fn extract_command(arguments: &str) -> Option<String> {
    let args: ShellArgs = serde_json::from_str(arguments).ok()?;
    Some(args.command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;

    #[test]
    fn extract_command_valid_json() {
        let result = extract_command(r#"{"command": "echo hello"}"#);
        assert_eq!(result, Some("echo hello".into()));
    }

    #[test]
    fn extract_command_invalid_json() {
        assert_eq!(extract_command("not json"), None);
    }

    #[test]
    fn extract_command_missing_field() {
        assert_eq!(extract_command(r#"{"other": "value"}"#), None);
    }

    #[test]
    fn shell_tool_metadata() {
        let tool = ShellTool;
        assert_eq!(tool.name(), "shell");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
    }

    #[tokio::test]
    async fn shell_execute_echo() {
        let tool = ShellTool;
        let result = tool.execute(r#"{"command": "echo hello"}"#).await.unwrap();
        assert_eq!(result.trim(), "hello");
    }

    #[tokio::test]
    async fn shell_execute_stderr() {
        let tool = ShellTool;
        let result = tool
            .execute(r#"{"command": "echo oops >&2"}"#)
            .await
            .unwrap();
        assert!(result.contains("stderr:"));
        assert!(result.contains("oops"));
    }

    #[tokio::test]
    async fn shell_execute_no_output() {
        let tool = ShellTool;
        let result = tool.execute(r#"{"command": "true"}"#).await.unwrap();
        assert_eq!(result, "(no output)");
    }

    #[tokio::test]
    async fn shell_execute_both_stdout_and_stderr() {
        let tool = ShellTool;
        let result = tool
            .execute(r#"{"command": "echo out; echo err >&2"}"#)
            .await
            .unwrap();
        assert!(result.contains("out"));
        assert!(result.contains("stderr:"));
        assert!(result.contains("err"));
    }

    #[tokio::test]
    async fn shell_execute_invalid_json() {
        let tool = ShellTool;
        let result = tool.execute("not json").await;
        assert!(result.is_err());
    }
}

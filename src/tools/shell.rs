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
    fn name(&self) -> &str { "shell" }

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

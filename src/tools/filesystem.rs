use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use crate::tools::Tool;

pub struct ReadFileTool;

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the contents of a file at the given path." }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "File path to read" } },
            "required": ["path"]
        })
    }
    async fn execute(&self, arguments: &str) -> Result<String> {
        let args: ReadFileArgs = serde_json::from_str(arguments)?;
        let contents = tokio::fs::read_to_string(&args.path).await?;
        Ok(contents)
    }
}

pub struct WriteFileTool;

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file at the given path. Creates the file if it doesn't exist, overwrites if it does." }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }
    async fn execute(&self, arguments: &str) -> Result<String> {
        let args: WriteFileArgs = serde_json::from_str(arguments)?;
        if let Some(parent) = std::path::Path::new(&args.path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&args.path, &args.content).await?;
        Ok(format!("Wrote {} bytes to {}", args.content.len(), args.path))
    }
}

pub struct ListFilesTool;

#[derive(Deserialize)]
struct ListFilesArgs {
    path: String,
    #[serde(default)]
    pattern: Option<String>,
}

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str { "list_files" }
    fn description(&self) -> &str { "List files in a directory. Optionally filter with a glob pattern." }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path to list" },
                "pattern": { "type": "string", "description": "Optional glob pattern (e.g. '*.rs')" }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, arguments: &str) -> Result<String> {
        let args: ListFilesArgs = serde_json::from_str(arguments)?;
        let glob_pattern = if let Some(ref pat) = args.pattern {
            format!("{}/{}", args.path, pat)
        } else {
            format!("{}/*", args.path)
        };
        let mut entries = Vec::new();
        for entry in glob::glob(&glob_pattern)? {
            if let Ok(path) = entry {
                entries.push(path.display().to_string());
            }
        }
        entries.sort();
        Ok(entries.join("\n"))
    }
}

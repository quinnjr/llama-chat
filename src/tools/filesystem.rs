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

pub struct EditFileTool;

#[derive(Deserialize)]
struct EditFileArgs {
    path: String,
    old_string: String,
    new_string: String,
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str { "edit_file" }
    fn description(&self) -> &str { "Edit a file by replacing an exact string match with new content. The old_string must match exactly once in the file." }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to edit" },
                "old_string": { "type": "string", "description": "The exact string to find and replace (must be unique in the file)" },
                "new_string": { "type": "string", "description": "The replacement string" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }
    async fn execute(&self, arguments: &str) -> Result<String> {
        let args: EditFileArgs = serde_json::from_str(arguments)?;
        let contents = tokio::fs::read_to_string(&args.path).await?;
        let count = contents.matches(&args.old_string).count();
        if count == 0 {
            anyhow::bail!("old_string not found in {}", args.path);
        }
        if count > 1 {
            anyhow::bail!("old_string matches {} times in {} — must be unique", count, args.path);
        }
        let new_contents = contents.replacen(&args.old_string, &args.new_string, 1);
        tokio::fs::write(&args.path, &new_contents).await?;
        Ok(format!("Edited {} — replaced 1 occurrence", args.path))
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
        for path in glob::glob(&glob_pattern)?.flatten() {
            entries.push(path.display().to_string());
        }
        entries.sort();
        Ok(entries.join("\n"))
    }
}

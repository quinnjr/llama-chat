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
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a file at the given path."
    }
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
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file at the given path. Creates the file if it doesn't exist, overwrites if it does."
    }
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
        Ok(format!(
            "Wrote {} bytes to {}",
            args.content.len(),
            args.path
        ))
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
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match with new content. The old_string must match exactly once in the file."
    }
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
            anyhow::bail!(
                "old_string matches {} times in {} — must be unique",
                count,
                args.path
            );
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
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "List files in a directory. Optionally filter with a glob pattern."
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("llama-chat-fs-test-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- ReadFileTool ---

    #[test]
    fn read_file_tool_metadata() {
        let tool = ReadFileTool;
        assert_eq!(tool.name(), "read_file");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[tokio::test]
    async fn read_file_existing() {
        let dir = temp_dir("read-existing");
        let file = dir.join("hello.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = ReadFileTool;
        let result = tool
            .execute(&format!(r#"{{"path": "{}"}}"#, file.display()))
            .await
            .unwrap();
        assert_eq!(result, "hello world");

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn read_file_nonexistent() {
        let tool = ReadFileTool;
        let result = tool
            .execute(r#"{"path": "/tmp/llama-chat-fs-test-nonexistent-file-xyz"}"#)
            .await;
        assert!(result.is_err());
    }

    // --- WriteFileTool ---

    #[test]
    fn write_file_tool_metadata() {
        let tool = WriteFileTool;
        assert_eq!(tool.name(), "write_file");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn write_file_new() {
        let dir = temp_dir("write-new");
        let file = dir.join("output.txt");

        let tool = WriteFileTool;
        let result = tool
            .execute(&format!(
                r#"{{"path": "{}", "content": "test content"}}"#,
                file.display()
            ))
            .await
            .unwrap();

        assert!(result.contains("12 bytes"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "test content");

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn write_file_overwrite() {
        let dir = temp_dir("write-overwrite");
        let file = dir.join("existing.txt");
        std::fs::write(&file, "old content").unwrap();

        let tool = WriteFileTool;
        tool.execute(&format!(
            r#"{{"path": "{}", "content": "new content"}}"#,
            file.display()
        ))
        .await
        .unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new content");

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn write_file_creates_parent_dirs() {
        let dir = temp_dir("write-parent");
        let file = dir.join("sub/dir/deep.txt");

        let tool = WriteFileTool;
        tool.execute(&format!(
            r#"{{"path": "{}", "content": "deep"}}"#,
            file.display()
        ))
        .await
        .unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "deep");

        std::fs::remove_dir_all(dir).ok();
    }

    // --- EditFileTool ---

    #[test]
    fn edit_file_tool_metadata() {
        let tool = EditFileTool;
        assert_eq!(tool.name(), "edit_file");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn edit_file_successful() {
        let dir = temp_dir("edit-success");
        let file = dir.join("code.rs");
        std::fs::write(&file, "fn main() {\n    println!(\"old\");\n}\n").unwrap();

        let tool = EditFileTool;
        let result = tool.execute(&format!(
            r#"{{"path": "{}", "old_string": "println!(\"old\")", "new_string": "println!(\"new\")"}}"#,
            file.display()
        )).await.unwrap();

        assert!(result.contains("Edited"));
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("println!(\"new\")"));
        assert!(!content.contains("println!(\"old\")"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn edit_file_old_string_not_found() {
        let dir = temp_dir("edit-not-found");
        let file = dir.join("test.txt");
        std::fs::write(&file, "abc def").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(&format!(
                r#"{{"path": "{}", "old_string": "xyz", "new_string": "new"}}"#,
                file.display()
            ))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn edit_file_multiple_matches() {
        let dir = temp_dir("edit-multi");
        let file = dir.join("test.txt");
        std::fs::write(&file, "aaa bbb aaa").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(&format!(
                r#"{{"path": "{}", "old_string": "aaa", "new_string": "ccc"}}"#,
                file.display()
            ))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("2 times"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn edit_file_nonexistent() {
        let tool = EditFileTool;
        let result = tool.execute(
            r#"{"path": "/tmp/llama-chat-fs-test-nonexistent-edit.txt", "old_string": "a", "new_string": "b"}"#
        ).await;
        assert!(result.is_err());
    }

    // --- ListFilesTool ---

    #[test]
    fn list_files_tool_metadata() {
        let tool = ListFilesTool;
        assert_eq!(tool.name(), "list_files");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn list_files_directory() {
        let dir = temp_dir("list-dir");
        std::fs::write(dir.join("alpha.txt"), "").unwrap();
        std::fs::write(dir.join("beta.txt"), "").unwrap();
        std::fs::write(dir.join("gamma.rs"), "").unwrap();

        let tool = ListFilesTool;
        let result = tool
            .execute(&format!(r#"{{"path": "{}"}}"#, dir.display()))
            .await
            .unwrap();

        assert!(result.contains("alpha.txt"));
        assert!(result.contains("beta.txt"));
        assert!(result.contains("gamma.rs"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn list_files_with_pattern() {
        let dir = temp_dir("list-pattern");
        std::fs::write(dir.join("foo.rs"), "").unwrap();
        std::fs::write(dir.join("bar.rs"), "").unwrap();
        std::fs::write(dir.join("baz.txt"), "").unwrap();

        let tool = ListFilesTool;
        let result = tool
            .execute(&format!(
                r#"{{"path": "{}", "pattern": "*.rs"}}"#,
                dir.display()
            ))
            .await
            .unwrap();

        assert!(result.contains("foo.rs"));
        assert!(result.contains("bar.rs"));
        assert!(!result.contains("baz.txt"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn list_files_empty_directory() {
        let dir = temp_dir("list-empty");

        let tool = ListFilesTool;
        let result = tool
            .execute(&format!(r#"{{"path": "{}"}}"#, dir.display()))
            .await
            .unwrap();
        assert!(result.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }
}

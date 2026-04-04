use std::collections::HashMap;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, Default)]
pub struct McpConfig {
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerEntry>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpServerEntry {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub transport: Option<String>,
}

impl McpServerEntry {
    pub fn detected_transport(&self) -> &str {
        if let Some(ref t) = self.transport {
            return t.as_str();
        }
        if self.command.is_some() {
            return "stdio";
        }
        if self.url.is_some() {
            return "streamable-http";
        }
        "unknown"
    }
}

impl McpConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&contents)?)
        } else {
            Ok(Self::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mcp_json() {
        let json = r#"{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]
    },
    "remote-db": {
      "url": "http://mcp-server.internal:3001/sse"
    },
    "explicit": {
      "url": "http://example.com/mcp",
      "transport": "sse"
    }
  }
}"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 3);

        let fs = &config.mcp_servers["filesystem"];
        assert_eq!(fs.detected_transport(), "stdio");
        assert_eq!(fs.command.as_deref(), Some("npx"));

        let remote = &config.mcp_servers["remote-db"];
        assert_eq!(remote.detected_transport(), "streamable-http");

        let explicit = &config.mcp_servers["explicit"];
        assert_eq!(explicit.detected_transport(), "sse");
    }

    #[test]
    fn empty_json_is_ok() {
        let config: McpConfig = serde_json::from_str("{}").unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn load_nonexistent_path_returns_empty() {
        let path = std::path::Path::new("/tmp/llama-chat-test-mcp-nonexistent.json");
        let config = McpConfig::load(path).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn load_valid_file() {
        let dir = std::env::temp_dir().join("llama-chat-test-mcp-load");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        std::fs::write(&path, r#"{"mcpServers": {"test": {"command": "echo"}}}"#).unwrap();

        let config = McpConfig::load(&path).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers["test"].command.as_deref(), Some("echo"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detected_transport_with_explicit_override() {
        let entry = McpServerEntry {
            command: Some("npx".into()),
            args: None,
            url: Some("http://example.com".into()),
            transport: Some("sse".into()),
        };
        // Explicit transport overrides auto-detection
        assert_eq!(entry.detected_transport(), "sse");
    }

    #[test]
    fn detected_transport_command_is_stdio() {
        let entry = McpServerEntry {
            command: Some("npx".into()),
            args: Some(vec!["-y".into(), "server".into()]),
            url: None,
            transport: None,
        };
        assert_eq!(entry.detected_transport(), "stdio");
    }

    #[test]
    fn detected_transport_url_is_streamable_http() {
        let entry = McpServerEntry {
            command: None,
            args: None,
            url: Some("http://example.com/mcp".into()),
            transport: None,
        };
        assert_eq!(entry.detected_transport(), "streamable-http");
    }

    #[test]
    fn detected_transport_nothing_is_unknown() {
        let entry = McpServerEntry {
            command: None,
            args: None,
            url: None,
            transport: None,
        };
        assert_eq!(entry.detected_transport(), "unknown");
    }
}

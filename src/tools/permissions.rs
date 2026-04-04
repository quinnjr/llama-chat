use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
}

pub struct PermissionManager {
    config: PermissionsConfig,
    path: PathBuf,
}

impl PermissionManager {
    pub fn load(project_dir: &Path) -> Self {
        let path = project_dir.join(".llama-chat/permissions.json");
        let config = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            PermissionsConfig::default()
        };
        Self { config, path }
    }

    pub fn is_allowed(&self, command: &str) -> bool {
        self.config.allow.iter().any(|pattern| glob_match(pattern, command))
    }

    pub fn add_exact(&mut self, command: &str) -> Result<()> {
        self.config.allow.push(command.to_string());
        self.save()
    }

    pub fn add_pattern(&mut self, pattern: &str) -> Result<()> {
        self.config.allow.push(pattern.to_string());
        self.save()
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.config)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }
}

/// Simple glob matching: `*` matches any sequence of characters.
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }

    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !text.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            if !text[pos..].ends_with(part) {
                return false;
            }
            pos = text.len();
        } else {
            if let Some(found) = text[pos..].find(part) {
                pos += found + part.len();
            } else {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(glob_match("git status", "git status"));
        assert!(!glob_match("git status", "git diff"));
    }

    #[test]
    fn wildcard_suffix() {
        assert!(glob_match("git *", "git status"));
        assert!(glob_match("git *", "git diff --cached"));
        assert!(!glob_match("git *", "cargo build"));
    }

    #[test]
    fn wildcard_middle() {
        assert!(glob_match("ls * src/", "ls -la src/"));
        assert!(!glob_match("ls * src/", "ls -la tests/"));
    }

    #[test]
    fn wildcard_prefix() {
        assert!(glob_match("* --help", "cargo --help"));
        assert!(!glob_match("* --help", "cargo build"));
    }

    #[test]
    fn bare_star() {
        assert!(glob_match("*", "anything at all"));
    }

    #[test]
    fn permission_check() {
        let config = PermissionsConfig {
            allow: vec!["ls *".into(), "git status".into(), "cargo *".into()],
        };
        let mgr = PermissionManager {
            config,
            path: PathBuf::from("/tmp/test-perms.json"),
        };
        assert!(mgr.is_allowed("ls -la"));
        assert!(mgr.is_allowed("git status"));
        assert!(mgr.is_allowed("cargo build"));
        assert!(!mgr.is_allowed("rm -rf /"));
    }
}

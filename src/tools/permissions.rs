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
        self.config
            .allow
            .iter()
            .any(|pattern| glob_match(pattern, command))
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

    #[test]
    fn load_from_nonexistent_dir() {
        let dir = PathBuf::from("/tmp/llama-chat-test-perms-nonexistent-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        let mgr = PermissionManager::load(&dir);
        assert!(!mgr.is_allowed("anything"));
        assert!(mgr.config.allow.is_empty());
    }

    #[test]
    fn add_exact_save_reload() {
        let dir = std::env::temp_dir().join("llama-chat-test-perms-save");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut mgr = PermissionManager::load(&dir);
        assert!(!mgr.is_allowed("git status"));

        mgr.add_exact("git status").unwrap();
        assert!(mgr.is_allowed("git status"));

        // Reload from disk and verify persistence
        let mgr2 = PermissionManager::load(&dir);
        assert!(mgr2.is_allowed("git status"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn add_pattern_and_match() {
        let dir = std::env::temp_dir().join("llama-chat-test-perms-pattern");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut mgr = PermissionManager::load(&dir);
        mgr.add_pattern("cargo *").unwrap();

        assert!(mgr.is_allowed("cargo build"));
        assert!(mgr.is_allowed("cargo test"));
        assert!(!mgr.is_allowed("git push"));

        // Reload and verify
        let mgr2 = PermissionManager::load(&dir);
        assert!(mgr2.is_allowed("cargo build"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn empty_allow_list_denies_all() {
        let mgr = PermissionManager {
            config: PermissionsConfig::default(),
            path: PathBuf::from("/tmp/test.json"),
        };
        assert!(!mgr.is_allowed("ls"));
        assert!(!mgr.is_allowed("echo hi"));
    }

    #[test]
    fn multiple_wildcards() {
        // Pattern: a*b*c requires "a" prefix, "b" in middle, "c" suffix
        assert!(glob_match("a*b*c", "axbxc"));
        assert!(glob_match("a*b*c", "abbbc"));
        assert!(!glob_match("a*b*c", "axbxd")); // ends with d, not c
        assert!(!glob_match("a*b*c", "xbxc")); // doesn't start with a
    }

    #[test]
    fn middle_part_not_found() {
        // Pattern: a*xyz*c — "xyz" not found in text
        assert!(!glob_match("a*xyz*c", "abc"));
    }

    #[test]
    fn double_wildcard() {
        assert!(glob_match("**", "anything"));
        assert!(glob_match("a**b", "aXb"));
    }
}

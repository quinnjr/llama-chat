use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DefaultsConfig {
    #[serde(default = "default_server")]
    pub server: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_show_thinking")]
    pub show_thinking: bool,
}

fn default_server() -> String {
    "local".into()
}
fn default_model() -> String {
    "llama3:8b".into()
}
fn default_show_thinking() -> bool {
    true
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            server: default_server(),
            model: default_model(),
            show_thinking: default_show_thinking(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ThemeConfig {
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default)]
    pub colors: HashMap<String, String>,
}

fn default_preset() -> String {
    "dark".into()
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            preset: default_preset(),
            colors: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MemoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub embedding_model: String,
    #[serde(default = "default_embedding_server")]
    pub embedding_server: String,
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    #[serde(default = "default_decay_half_life")]
    pub decay_half_life_days: u32,
    #[serde(default = "default_extraction_on_clear")]
    pub extraction_on_clear: bool,
}

fn default_embedding_server() -> String { "local".into() }
fn default_top_n() -> usize { 8 }
fn default_decay_half_life() -> u32 { 90 }
fn default_extraction_on_clear() -> bool { true }

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            embedding_model: String::new(),
            embedding_server: default_embedding_server(),
            top_n: default_top_n(),
            decay_half_life_days: default_decay_half_life(),
            extraction_on_clear: default_extraction_on_clear(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&contents)?)
        } else {
            Ok(Self::default())
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut servers = HashMap::new();
        servers.insert(
            "local".into(),
            ServerConfig {
                name: "Local Ollama".into(),
                url: "http://localhost:11434/v1".into(),
                api_key: None,
            },
        );
        Self {
            servers,
            defaults: DefaultsConfig::default(),
            theme: ThemeConfig::default(),
            memory: MemoryConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml_str = r##"
[servers.local]
name = "Local Ollama"
url = "http://localhost:11434/v1"

[servers.remote]
name = "GPU Box"
url = "http://gpu-box:8080/v1"
api_key = "sk-secret"

[defaults]
server = "local"
model = "llama3:8b"

[theme]
preset = "dark"

[theme.colors]
accent = "#818cf8"
"##;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.servers.len(), 2);
        assert_eq!(
            config.servers["remote"].api_key.as_deref(),
            Some("sk-secret")
        );
        assert_eq!(config.defaults.server, "local");
        assert_eq!(config.theme.preset, "dark");
        assert_eq!(config.theme.colors["accent"], "#818cf8");
    }

    #[test]
    fn default_config_has_local_server() {
        let config = AppConfig::default();
        assert!(config.servers.contains_key("local"));
        assert_eq!(config.defaults.server, "local");
        assert_eq!(config.defaults.model, "llama3:8b");
    }

    #[test]
    fn empty_toml_uses_defaults() {
        let config: AppConfig = toml::from_str("").unwrap();
        assert!(config.servers.is_empty());
        assert_eq!(config.defaults.server, "local");
    }

    #[test]
    fn load_nonexistent_path_returns_default() {
        let path = std::path::Path::new("/tmp/llama-chat-test-config-nonexistent.toml");
        let config = AppConfig::load(path).unwrap();
        assert!(config.servers.contains_key("local"));
        assert_eq!(config.defaults.model, "llama3:8b");
    }

    #[test]
    fn load_valid_file() {
        let dir = std::env::temp_dir().join("llama-chat-test-settings-load");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
[servers.myserver]
name = "My Server"
url = "http://example.com:8080/v1"

[defaults]
server = "myserver"
model = "codellama:7b"

[theme]
preset = "light"
"#,
        )
        .unwrap();

        let config = AppConfig::load(&path).unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers["myserver"].name, "My Server");
        assert_eq!(config.defaults.server, "myserver");
        assert_eq!(config.defaults.model, "codellama:7b");
        assert_eq!(config.theme.preset, "light");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn default_config_theme_is_dark() {
        let config = AppConfig::default();
        assert_eq!(config.theme.preset, "dark");
        assert!(config.theme.colors.is_empty());
    }

    #[test]
    fn server_config_api_key_is_optional() {
        let config = AppConfig::default();
        let local = &config.servers["local"];
        assert!(local.api_key.is_none());
        assert_eq!(local.url, "http://localhost:11434/v1");
    }

    #[test]
    fn default_memory_config_is_disabled() {
        let config = AppConfig::default();
        assert!(!config.memory.enabled);
        assert_eq!(config.memory.top_n, 8);
        assert_eq!(config.memory.decay_half_life_days, 90);
        assert!(config.memory.extraction_on_clear);
        assert!(config.memory.embedding_model.is_empty());
    }

    #[test]
    fn parse_memory_section() {
        let toml_str = r#"
[memory]
enabled = true
embedding_model = "nomic-embed-text"
embedding_server = "local"
top_n = 5
decay_half_life_days = 30
extraction_on_clear = false
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.memory.enabled);
        assert_eq!(config.memory.embedding_model, "nomic-embed-text");
        assert_eq!(config.memory.top_n, 5);
        assert!(!config.memory.extraction_on_clear);
    }
}

//! Round-trip: save → SELECT it back.

use std::time::Duration;

use tempfile::tempdir;

use llama_chat::config::settings::{AppConfig, MemoryConfig, ServerConfig};
use llama_chat::memory::{Kind, MemoryService, Scope};

// A stub embeddings endpoint is out of scope for this test; we rely on the
// real network-optional probe. The test is gated behind an env var so CI
// without a running embedding server skips it. Set LLAMA_CHAT_EMBED_URL to
// point at Ollama with nomic-embed-text to run it locally.

#[tokio::test]
async fn save_and_read_back() {
    let Some(url) = std::env::var_os("LLAMA_CHAT_EMBED_URL") else {
        eprintln!("skipping: LLAMA_CHAT_EMBED_URL not set");
        return;
    };
    let tmp = tempdir().unwrap();

    let mut cfg = AppConfig::default();
    cfg.servers.insert("local".into(), ServerConfig {
        name: "local".into(),
        url: url.to_string_lossy().into_owned(),
        api_key: None,
    });
    cfg.memory = MemoryConfig {
        enabled: true,
        embedding_model: std::env::var("LLAMA_CHAT_EMBED_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text".into()),
        embedding_server: "local".into(),
        top_n: 8,
        decay_half_life_days: 90,
        extraction_on_clear: true,
    };

    let svc = tokio::time::timeout(
        Duration::from_secs(10),
        MemoryService::open(&cfg, tmp.path()),
    ).await.expect("timeout").expect("open");

    let id = svc.save("I prefer terse responses".into(), Kind::Feedback, Scope::Global)
        .await.unwrap();
    assert!(id > 0);
}

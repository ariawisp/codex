#![cfg(target_os = "macos")]

use codex_core::client::ModelClient;
use codex_core::client_common::Prompt;
use codex_core::config::{Config, ConfigOverrides, ConfigToml};
use codex_protocol::models::{ContentItem, ResponseItem};

/// macOS-only integration smoke test for the CodexPC provider via XPC.
///
/// Requires:
/// - codexpc daemon running (install via ../codexpc/packaging/install-agent.sh)
/// - CODEXPC_CHECKPOINT env var pointing at a local GPT-OSS checkpoint
#[tokio::test(flavor = "multi_thread")] 
#[ignore]
async fn codexpc_stream_smoke() {
    if std::env::var("CODEXPC_CHECKPOINT").is_err() && std::env::var("CODEXPC_CHECKPOINT_PATH").is_err() {
        eprintln!("skipping: CODEXPC_CHECKPOINT not set");
        return;
    }

    // Minimal config pointing at the built-in codexpc provider
    let mut cfg = ConfigToml::default();
    cfg.model = Some("gpt-oss:20b".into());
    cfg.model_provider = Some("codexpc".into());

    let config = Config::load_from_base_config_with_overrides(cfg, ConfigOverrides::default(), std::env::temp_dir())
        .expect("load config");

    let conversation_id = codex_core::util::random_uuid();
    let client = ModelClient::new(std::sync::Arc::new(config), None, codex_core::built_in_model_providers()["codexpc"].clone(), None, codex_protocol::config_types::ReasoningSummary::Auto, conversation_id);

    let mut prompt = Prompt::default();
    prompt.input = vec![ResponseItem::Message {
        id: None,
        role: "user".into(),
        content: vec![ContentItem::InputText { text: "Hello".into() }],
    }];

    // Optional: start daemon if CODEXPCD_BIN set
    let mut child: Option<std::process::Child> = None;
    if let Ok(bin) = std::env::var("CODEXPCD_BIN") {
        if !bin.trim().is_empty() {
            let mut cmd = std::process::Command::new(bin);
            cmd.env("CODEXPC_ALLOW_TOOLS", "1");
            // Force a deterministic tool call for this test
            cmd.env("CODEXPC_TEST_FORCE_TOOL", "echo:{\"msg\":\"hello from codexpc test\"}");
            let spawned = cmd
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            child = spawned;
            // wait a moment for service to register
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    let mut stream = client.stream(&prompt).await.expect("stream start");
    let mut saw_created = false;
    let mut saw_completed = false;
    let mut delta_count = 0u32;
    let mut saw_tool_call = false;
    let mut saw_tool_output = false;
    use futures::StreamExt;
    use tokio::time::{timeout, Duration};
    let deadline = Duration::from_secs(10);
    while let Ok(Some(ev)) = timeout(deadline, stream.next()).await {
        match ev.expect("event ok") {
            codex_core::client_common::ResponseEvent::Created => saw_created = true,
            codex_core::client_common::ResponseEvent::OutputTextDelta(s) => { if !s.is_empty() { delta_count += 1; } }
            codex_core::client_common::ResponseEvent::OutputItemDone(item) => {
                match item {
                    codex_protocol::models::ResponseItem::CustomToolCall { name, input, .. } => {
                        if name.contains("echo") { saw_tool_call = true; assert!(input.contains("hello")); }
                    }
                    codex_protocol::models::ResponseItem::CustomToolCallOutput { call_id, output } => {
                        if call_id.contains("echo") { saw_tool_output = true; assert!(output.contains("hello")); }
                    }
                    _ => {}
                }
            }
            codex_core::client_common::ResponseEvent::Completed { response_id: _, token_usage } => { 
                saw_completed = true; 
                // token_usage may be None depending on daemon; best-effort check
                if let Some(u) = token_usage { assert!(u.total_tokens > 0); }
                break; 
            }
            _ => {}
        }
    }
    assert!(saw_created, "expected created");
    assert!(saw_completed, "expected completed");
    assert!(delta_count >= 0);
    assert!(saw_tool_call, "expected tool call event");
    assert!(saw_tool_output, "expected tool output event");
    // Cleanup daemon child if started
    if let Some(mut c) = child { let _ = c.kill(); }
}

use crate::client::ModelClient;
use crate::client_common::{Prompt, ResponseEvent, ResponseStream};
use crate::error::{CodexErr, EnvVarError, Result};
use crate::protocol::TokenUsage;
use codex_protocol::models::ResponseItem;
use tokio::sync::mpsc;

impl ModelClient {
    #[cfg(target_os = "macos")]
    pub(crate) async fn stream_via_codexpc_xpc(&self, prompt: &Prompt) -> Result<ResponseStream> {
        use tokio::task;
        let checkpoint = std::env::var("CODEXPC_CHECKPOINT")
            .or_else(|_| std::env::var("CODEXPC_CHECKPOINT_PATH"))
            .map_err(|_| {
                CodexErr::EnvVar(EnvVarError {
                    var: "CODEXPC_CHECKPOINT".into(),
                    instructions: Some(
                        "Set CODEXPC_CHECKPOINT to your GPT-OSS checkpoint path".into(),
                    ),
                })
            })?;
        let service =
            std::env::var("CODEXPC_SERVICE").unwrap_or_else(|_| "com.yourorg.codexpc".into());
        // Optional debug handshake for diagnostics
        if std::env::var("CODEXPC_DEBUG_HANDSHAKE").is_ok() {
            if let Some(hs) = codexpc_xpc::handshake(&service) {
                tracing::info!(target: "codexpc", "handshake encoding_name={:?} special_tokens_count={} stop_tokens_for_assistant_actions_count={}",
                    hs.encoding_name, hs.special_tokens.len(), hs.stop_tokens_for_assistant_actions.len());
            }
        }
        // Keep instructions empty for CodexPC XPC; daemon injects minimal Harmony scaffold for JSON path.
        let instructions = String::new();
        // Always request unlimited tokens; the daemon will stop on Harmony stop tokens.
        let max_tokens = 0u64;
        let temperature = 0.0f64; // TODO: plumb sampling
        // Build typed Harmony messages and render prefill tokens via Harmony
        let formatted = prompt.get_formatted_input();
        let harmony_tools_json = Self::build_harmony_tools_json(prompt);
        // Include minimal reasoning flags when configured
        let reasoning_json = {
            let effort = self.get_reasoning_effort();
            let summary = self.get_reasoning_summary();
            let mut fields: Vec<String> = Vec::new();
            if let Some(e) = effort {
                fields.push(format!("\"effort\":\"{}\"", e.to_string().to_lowercase()));
            }
            fields.push(format!(
                "\"summary\":\"{}\"",
                summary.to_string().to_lowercase()
            ));
            if fields.is_empty() {
                None
            } else {
                Some(format!("{{{}}}", fields.join(",")))
            }
        };

        // Render tokens with Harmony and stream via tokens-over-XPC by default
        use openai_harmony::chat::{Conversation, DeveloperContent, Message, Role};
        use openai_harmony::{load_harmony_encoding, HarmonyEncodingName};
        let mut messages: Vec<Message> = Vec::new();
        // System message with default content; optionally include instructions as text
        let mut sys = Message::from_role_and_content(Role::System, openai_harmony::chat::SystemContent::new());
        // Historical behavior: add instructions (or default system text) as plain text in system message
        let sys_text = if !instructions.is_empty() {
            instructions.clone()
        } else {
            String::from(
                "# Valid channels: analysis, commentary, final.\nAlways write user-facing responses in the final channel; use analysis only for internal reasoning.",
            )
        };
        sys = sys.adding_content(sys_text);
        messages.push(sys);
        if let Some(tools_root) = Self::tools_for_harmony(prompt) {
            let dev = DeveloperContent::new().with_tools(tools_root);
            messages.push(Message::from_role_and_content(Role::Developer, dev));
        }
        // Add user/assistant history
        for item in formatted.iter() {
            if let ResponseItem::Message { role, content, .. } = item {
                let parsed_role = match role.as_str() {
                    "system" => Role::System,
                    "developer" => Role::Developer,
                    "assistant" => Role::Assistant,
                    "tool" => Role::Tool,
                    _ => Role::User,
                };
                let mut msg = Message::from_role_and_content(parsed_role, "");
                for part in content.iter() {
                    match part {
                        codex_protocol::models::ContentItem::InputText { text }
                        | codex_protocol::models::ContentItem::OutputText { text } => {
                            msg = msg.adding_content(text.clone());
                        }
                        codex_protocol::models::ContentItem::InputImage { .. } => {
                            // images unsupported in tokens path prefill for now
                        }
                    }
                }
                messages.push(msg);
            }
        }
        let conversation = Conversation::from_messages(messages);
        let enc = load_harmony_encoding(HarmonyEncodingName::HARMONY_GPT_OSS)
            .map_err(|e| CodexErr::Other(format!("harmony load failed: {e}")))?;
        let prefill: Vec<u32> = enc
            .render_conversation_for_completion(&conversation, Role::Assistant, None)
            .map_err(|e| CodexErr::Other(format!("harmony render failed: {e}")))?
            .into_iter()
            .collect();
        let prime_final = true;
        let (handle, mut rx) = codexpc_xpc::stream_from_tokens(
            &service,
            &checkpoint,
            &prefill,
            prime_final,
            harmony_tools_json.as_deref(),
            reasoning_json.as_deref(),
            temperature,
            max_tokens,
        );
        let (tx, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);
        task::spawn(async move {
            let mut assistant_buf = String::new();
            while let Some(ev) = rx.recv().await {
                let send = match ev {
                    codexpc_xpc::Event::Created => tx.send(Ok(ResponseEvent::Created)).await,
                    codexpc_xpc::Event::OutputTextDelta(s) => {
                        assistant_buf.push_str(&s);
                        tx.send(Ok(ResponseEvent::OutputTextDelta(s))).await
                    }
                    codexpc_xpc::Event::Completed {
                        response_id,
                        input_tokens,
                        output_tokens,
                        total_tokens,
                    } => {
                        // Emit a final assistant message so history is preserved for next turns
                        if !assistant_buf.is_empty() {
                            let item = codex_protocol::models::ResponseItem::Message {
                                id: None,
                                role: "assistant".into(),
                                content: vec![codex_protocol::models::ContentItem::OutputText {
                                    text: std::mem::take(&mut assistant_buf),
                                }],
                            };
                            // Reuse OutputItemDone carrier to inject the final message
                            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                        }
                        let usage = Some(TokenUsage {
                            input_tokens,
                            cached_input_tokens: 0,
                            output_tokens,
                            reasoning_output_tokens: 0,
                            total_tokens,
                        });
                        tx.send(Ok(ResponseEvent::Completed {
                            response_id,
                            token_usage: usage,
                        }))
                        .await
                    }
                    codexpc_xpc::Event::Metrics { ttfb_ms, tokens_per_sec, delta_count, tool_calls } => {
                        tracing::info!(target: "codexpc", "metrics ttfb_ms={} tokens_per_sec={} delta_count={} tool_calls={}", ttfb_ms, tokens_per_sec, delta_count, tool_calls);
                        continue;
                    }
                    codexpc_xpc::Event::OutputItemDone {
                        item_type,
                        status,
                        name,
                        input,
                        call_id,
                    } => {
                        let call_name = if name.is_empty() {
                            if item_type.is_empty() { "tool".into() } else { item_type }
                        } else {
                            name
                        };
                        let call_id_final = call_id.unwrap_or_else(|| call_name.clone());
                        let item = codex_protocol::models::ResponseItem::CustomToolCall {
                            id: None,
                            status: if status.is_empty() { None } else { Some(status) },
                            call_id: call_id_final,
                            name: call_name,
                            input,
                        };
                        tx.send(Ok(ResponseEvent::OutputItemDone(item))).await
                    }
                    codexpc_xpc::Event::OutputItemOutput { name, output, call_id } => {
                        let call_id_final = call_id.unwrap_or(name);
                        let item = codex_protocol::models::ResponseItem::CustomToolCallOutput {
                            call_id: call_id_final,
                            output,
                        };
                        tx.send(Ok(ResponseEvent::OutputItemDone(item))).await
                    }
                    codexpc_xpc::Event::Error { code, message } => {
                        tx.send(Err(CodexErr::Stream(format!("{code}: {message}"), None)))
                            .await
                    }
                };
                if send.is_err() {
                    break;
                }
            }
            drop(handle);
        });
        Ok(ResponseStream { rx_event })
    }

    #[cfg(target_os = "macos")]
    // json_escape and string-based content renderers were removed in favor of serde_json

    #[cfg(target_os = "macos")]
    fn build_harmony_conversation_json(
        instructions: &str,
        items: &[ResponseItem],
        include_dev_tools_msg: bool,
    ) -> Option<String> {
        use serde_json::json;
        let mut messages: Vec<serde_json::Value> = Vec::new();
        if !instructions.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": [{"type": "text", "text": instructions}],
            }));
        } else {
            let sys = "# Valid channels: analysis, commentary, final.
Always write user-facing responses in the final channel; use analysis only for internal reasoning.";
            messages.push(json!({
                "role": "system",
                "content": [{"type": "text", "text": sys}],
            }));
        }
        if include_dev_tools_msg {
            if let Some(desc) = Self::build_tool_description() {
                messages.push(json!({
                    "role": "developer",
                    "content": [{"type": "text", "text": desc}],
                }));
            }
        }
        for item in items.iter() {
            if let ResponseItem::Message { role, content, .. } = item {
                let mut parts: Vec<serde_json::Value> = Vec::new();
                for part in content.iter() {
                    match part {
                        codex_protocol::models::ContentItem::InputText { text }
                        | codex_protocol::models::ContentItem::OutputText { text } => {
                            parts.push(json!({"type": "text", "text": text}));
                        }
                        codex_protocol::models::ContentItem::InputImage { image_url } => {
                            parts.push(json!({"type": "image", "image_url": image_url}));
                        }
                    }
                }
                if !parts.is_empty() {
                    messages.push(json!({
                        "role": role,
                        "content": parts,
                    }));
                }
            }
        }
        if messages.is_empty() { None } else { Some(json!({"messages": messages}).to_string()) }
    }

    #[cfg(target_os = "macos")]
    fn tools_for_harmony(prompt: &Prompt) -> Option<openai_harmony::chat::ToolNamespaceConfig> {
        use crate::openai_tools::OpenAiTool;
        use openai_harmony::chat::{ToolDescription, ToolNamespaceConfig};
        let mut tools: Vec<ToolDescription> = Vec::new();
        for t in prompt.tools.iter() {
            if let OpenAiTool::Function(f) = t {
                let params = serde_json::to_value(&f.parameters).ok();
                tools.push(ToolDescription::new(&f.name, &f.description, params));
            }
        }
        if tools.is_empty() { None } else { Some(ToolNamespaceConfig::new("functions", None, tools)) }
    }

    #[cfg(target_os = "macos")]
    fn build_harmony_tools_json(prompt: &Prompt) -> Option<String> {
        use crate::openai_tools::OpenAiTool;
        let mut arr: Vec<serde_json::Value> = Vec::new();
        for t in prompt.tools.iter() {
            if let OpenAiTool::Function(f) = t {
                if let Ok(params) = serde_json::to_value(&f.parameters) {
                    arr.push(serde_json::json!({
                        "name": f.name,
                        "json_schema": params,
                    }));
                }
            }
        }
        if arr.is_empty() {
            None
        } else {
            Some(serde_json::json!({
                "version": 1,
                "namespace": "functions",
                "tools": arr,
            }).to_string())
        }
    }

    #[cfg(target_os = "macos")]
    fn build_tool_description() -> Option<String> {
        let supported = [
            (
                "echo",
                "{\\\"type\\\":\\\"object\\\",\\\"properties\\\":{\\\"msg\\\":{\\\"type\\\":\\\"string\\\"}}}",
            ),
            (
                "upper",
                "{\\\"type\\\":\\\"object\\\",\\\"properties\\\":{\\\"msg\\\":{\\\"type\\\":\\\"string\\\"}}}",
            ),
        ];
        if supported.is_empty() { return None; }
        let list = supported.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
        let schemas = supported
            .iter()
            .map(|(n, sch)| format!("{}: {}", n, sch))
            .collect::<Vec<_>>()
            .join("; ");
        Some(format!(
            "Available tools: {}. Schemas: {}. To call a tool, set the recipient to the tool name and provide JSON arguments in commentary channel.",
            list, schemas
        ))
    }

}

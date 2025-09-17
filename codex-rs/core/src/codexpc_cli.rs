use crate::client::ModelClient;
use crate::client_common::{Prompt, ResponseEvent, ResponseStream};
use crate::error::{CodexErr, EnvVarError, Result};
use tokio::sync::mpsc;

impl ModelClient {
    #[cfg(not(target_os = "macos"))]
    pub(crate) async fn stream_via_codexpc_cli(&self, prompt: &Prompt) -> Result<ResponseStream> {
        use tokio::io::{AsyncReadExt, BufReader};
        use tokio::process::Command;

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

        let instructions = prompt
            .get_full_instructions(&self.get_model_family())
            .to_string();
        let max_tokens = self.get_max_output_tokens();
        let mut cmd = Command::new("codexpc-cli");
        cmd.arg("--service")
            .arg(service)
            .arg("--checkpoint")
            .arg(checkpoint)
            .arg("--prompt")
            .arg(instructions)
            .arg("--max-tokens")
            .arg(max_tokens.to_string());

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(CodexErr::Io)?;

        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let mut reader = BufReader::new(stdout);
        let mut err_reader = BufReader::new(stderr);

        let (tx, rx) = mpsc::channel::<Result<ResponseEvent>>(1600);
        let tx_stdout = tx.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let s = String::from_utf8_lossy(&buf[..n]).to_string();
                        if s.trim() == "[created]" {
                            let _ = tx_stdout.send(Ok(ResponseEvent::Created)).await;
                            continue;
                        }
                        if s.contains("[completed]") {
                            let _ = tx_stdout
                                .send(Ok(ResponseEvent::Completed {
                                    response_id: "codexpc".into(),
                                    token_usage: None,
                                }))
                                .await;
                            break;
                        }
                        let _ = tx_stdout.send(Ok(ResponseEvent::OutputTextDelta(s))).await;
                    }
                    Err(e) => {
                        let _ = tx_stdout
                            .send(Err(CodexErr::Stream(
                                format!("read stdout error: {e}"),
                                None,
                            )))
                            .await;
                        break;
                    }
                }
            }
        });

        let tx_err = tx.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            if err_reader.read_to_end(&mut buf).await.is_ok() {
                if !buf.is_empty() {
                    let _ = tx_err
                        .send(Err(CodexErr::Stream(
                            String::from_utf8_lossy(&buf).to_string(),
                            None,
                        )))
                        .await;
                }
            }
        });

        let tx_done = tx.clone();
        tokio::spawn(async move {
            if let Ok(status) = child.wait().await {
                if !status.success() {
                    let _ = tx_done
                        .send(Err(CodexErr::Stream(
                            format!("codexpc-cli exited with status {status}"),
                            None,
                        )))
                        .await;
                }
            }
        });

        Ok(ResponseStream { rx_event: rx })
    }
}


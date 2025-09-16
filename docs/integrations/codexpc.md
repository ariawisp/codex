# CodexPC (macOS XPC) integration

Codex uses HTTP model providers by default (OpenAI Responses, Chat Completions, or compatible). `codexpc` is a macOS‑only XPC daemon that runs GPT‑OSS locally with low latency and does not expose an HTTP API.

Until a native provider is added to Codex, the recommended setup is:

- Use `codexpc` directly for local development flows and smoke tests (streaming over XPC).
- Keep Codex configured with your preferred HTTP provider (e.g., OpenAI or a local Ollama instance) for day‑to‑day use.

Status:

- The `codexpc` daemon and client library are available at `../codexpc`.
- A native provider for Codex is available (macOS-only, initial integration via the `codexpc-cli` helper).

Getting started with `codexpc`:

1. Build and install the daemon

```
cd ../codexpc
./packaging/install-agent.sh
```

2. Health check and a quick stream using the bundled Swift CLI

```
cd ../codexpc/cli-swift
swift run -c release codexpc-cli --health
swift run -c release codexpc-cli --checkpoint /path/to/model.bin --prompt "hello" --temperature 0.0 --max-tokens 32
```

3. Kotlin/Native sample (optional)

See `../codexpc/client-kotlin-native/README.md` for a Kotlin sample client.

Roadmap:

- Configure Codex to use CodexPC:

```
# ~/.codex/config.toml
model_provider = "codexpc"
model = "gpt-oss:20b"  # any string; used for display
```

Set environment variables so Codex can locate your model and service:

```
export CODEXPC_CHECKPOINT=/path/to/gpt-oss/model.bin
# optional (defaults to com.yourorg.codexpc)
export CODEXPC_SERVICE=com.yourorg.codexpc
```

Notes:
- This first version uses the `codexpc-cli` helper to stream output.
- Instructions (system prompt) are sent; richer inputs and tools will be added in a future update.

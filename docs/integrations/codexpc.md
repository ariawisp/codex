# CodexPC (macOS XPC) integration

Codex uses HTTP model providers by default (OpenAI Responses, Chat Completions, or compatible). `codexpc` is a macOS‑only XPC daemon that runs GPT‑OSS locally with low latency and does not expose an HTTP API. A native Codex provider integrates directly via XPC on macOS.

Status:

- The `codexpc` daemon and client library are available at `../codexpc`.
- The native provider for Codex (macOS-only) uses XPC directly; on non‑macOS, a CLI shim is used for smoke tests.
- Prefill is rendered to tokens using Harmony in-process (Rust) and sent over XPC. Streaming uses Harmony parser events (final-only deltas by default) with tool call parity.

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

On macOS, when `CODEXPC_CHECKPOINT` (or `CODEXPC_CHECKPOINT_PATH`) is set, Codex automatically prefers the native XPC provider regardless of the configured provider.

Notes:
- On macOS, Codex streams via XPC and produces final‑only deltas. Commentary is suppressed.
- Tokens-over-XPC is the default path. Typed messages and conversation JSON remain as compatibility fallbacks in the daemon.
- Handshake endpoint (daemon): send `{type: "handshake"}` to retrieve `encoding_name`, `special_tokens`, and `stop_tokens_for_assistant_actions` for diagnostics.
- A macOS‑only integration smoke test is available (ignored by default): `codex-rs/core/tests/mac_codexpc_integration.rs`.
  - Set `CODEXPC_CHECKPOINT` to your local GPT‑OSS checkpoint.
  - Optionally set `CODEXPCD_BIN` to the installed `codexpcd` path to spawn the daemon for the test.
  - The test asserts `Created` → final `delta(s)` → `Completed`.

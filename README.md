<div align="center">
  <img src="public/femtobot.svg" height="128">
  <h1>lightclaw: The Real Lightweight AI Assistant</h1>
  <p>
    <img src="https://img.shields.io/badge/language-Rust-orange" alt="Rust">
    <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
    <img src="https://img.shields.io/badge/binary_size-~15MB-success" alt="Size">
    <a href="https://rig.rs/"><img src="https://img.shields.io/badge/powered%20by-Rig-dca282" alt="Powered by Rig"></a>
  </p>
</div>

`LightClaw` is a fast, local-first AI assistant inspired by [OpenClaw](https://github.com/openclaw/openclaw) and [nanobot](https://github.com/HKUDS/nanobot), packaged as a single Rust binary.

If you want agentic tooling, memory, and Telegram/Discord integration without a heavy runtime, lightclaw is built for that.

## Why lightclaw

| Metric | OpenClaw | Nanobot | LightClaw |
|--------|----------|---------|----------|
| Distribution | Complex repo | Python + venv | **Single binary** |
| Disk overhead | Heavy | ~350MB env | **~15MB total** |
| Runtime footprint | High | ~100MB+ | **Low footprint** |
| Startup | Slow | ~0.5s | **Near-instant** |

## 60-Second Quickstart

```bash
curl -fsSL https://lightclaw.dev/install.sh | bash
lightclaw configure
```

Supported platforms:
- Linux x86_64
- Linux ARM64 (Raspberry Pi 4/5, ARM servers)
- Linux ARMv7 (e.g. older ARM devices / e-readers)
- macOS x86_64
- macOS ARM64 (Apple Silicon)

Note: A Windows binary exists, but it is currently less stable and not as well-supported as the others.

## What You Get

- Single-binary deploy: ship one executable, no Python runtime.
- Tool-capable agent: file, shell, web, and scheduling actions.
- Telegram/Discord-native interface: high-performance polling built in.
- Local-first memory: vectors + metadata stored locally with SQLite.
- Rust reliability: strong typing, memory safety, and concurrency.
- Skills support: OpenClaw-style `SKILL.md` skills via `activate_skill`.

## Memory System

LightClaw includes long-term memory:

- Short-term chat history per session.
- Periodic summarization of recent conversation chunks.
- Semantic retrieval over stored memories.
- Privacy-first local storage (no external vector DB required).

## Configuration

Create `~/.lightclaw/config.json`:

```json
{
  "agents": {
    "defaults": {
      "provider": "openrouter",
      "model": "anthropic/claude-opus-4-5",
      "model_fallbacks": [
        "openai/gpt-4o-mini",
        "ollama/llama3.2"
      ]
    }
  },
  "providers": {
    "openrouter": {
      "apiKey": "sk-or-..."
    },
    "openai": {
      "apiKey": "sk-..."
    },
    "ollama": {
      "apiBase": "http://127.0.0.1:11434/v1"
    },
    "mistral": {
      "apiKey": "..."
    }
  },
  "channels": {
    "telegram": {
      "token": "YOUR_BOT_TOKEN",
      "allow_from": ["123456789"],
      "transcription": {
        "enabled": true,
        "provider": "openai",
        "model": "whisper-1",
        "language": "en",
        "max_bytes": 20971520,
        "diarize": false,
        "context_bias": "",
        "timestamp_granularities": ["segment"]
      }
    },
    "discord": {
      "token": "YOUR_DISCORD_BOT_TOKEN",
      "allow_from": ["123456789012345678"],
      "allowed_channels": ["123456789012345678"]
    }
  },
  "tools": {
    "web": {
      "search": {
        "provider": "firecrawl",
        "firecrawlApiKey": "fc-..."
      }
    }
  }
}
```

## Build From Source

```bash
cargo build --release
./target/release/lightclaw
```

Cross-platform build script:

```bash
./scripts/build.sh
```

## Architecture

LightClaw uses an actor-like model with a central `MessageBus`:

- `Agent`: context handling and LLM orchestration.
- `Telegram`: chat input/output transport.
- `Discord`: chat input/output transport.
- `Tools`: executable capability modules.
- `Memory`: summary ingestion + retrieval loop.

All components run on a single async Tokio runtime.

## Skills

lightclaw can discover and activate OpenClaw-style skills from:

- `~/.lightclaw/workspace/skills/*/SKILL.md`
- `~/.lightclaw/workspace/.agents/skills/*/SKILL.md`
- `~/.agents/skills/*/SKILL.md`

When relevant, the model can call `activate_skill` to load the full instructions for a skill.

### Skills CLI

LightClaw includes a native `skills` command group backed by Rust APIs for ClawHub and skills.sh/source installs.

```bash
# Search across all backends (default: ClawHub + skills.sh)
lightclaw skills search "calendar"

# Search on skills.sh only
lightclaw skills search react --from skills

# Search on ClawHub only
lightclaw skills search react --from clawhub

# Install from ClawHub
lightclaw skills install weather --from clawhub

# Install from source (OpenClaw-compatible project layout)
lightclaw skills install vercel-labs/agent-skills --from skills
```

For `--from skills`, installs land in `./skills` under the workspace.

### Service CLI

LightClaw also includes a native `service` command group to manage a background daemon and stream logs with the same commands across platforms.

```bash
# Install and start a user-level service
lightclaw service install

# Check service state
lightclaw service status

# Restart/stop/start
lightclaw service restart
lightclaw service stop
lightclaw service start

# Stream runtime logs
lightclaw service logs -f

# Remove the service
lightclaw service uninstall

# Full uninstall wizard (service + optional files + optional binary)
lightclaw uninstall
```

By default, service commands target user-level services. Use `--system` for system-level service operations.

## Project Structure

```text
scripts/
  build.sh
  release.sh
  install.sh
  count_loc.sh
src/
  lib.rs          # Library crate root (app wiring / CLI runner)
  agent/          # Agent orchestration and core reasoning flow
  channels/       # Channel adapters (Telegram, Discord)
  cron/           # Scheduling types and persistent schedule storage
  memory/         # Summary, vector/file stores, retrieval logic
  skills/         # Skill manager, installer hub, and skills CLI commands
  tools/          # Tool implementations (fs, shell, web, send, cron)
  bus.rs          # Message bus for component coordination
  config.rs       # Config schema and loading
  configure.rs    # CLI setup flow for local configuration
  main.rs         # Thin binary entrypoint
  transcription.rs # Audio transcription integration
```

## Powered by Rig

LightClaw is built on [Rig](https://rig.rs/), which provides:

- Provider abstraction across OpenAI/OpenRouter-style backends
- Structured tool calling
- Retrieval-friendly agent primitives

## Contributing

Contributions are welcome.

- Open an issue for bugs, regressions, or feature ideas.
- Open a PR with focused changes and a clear description.
- Keep changes lightweight and production-oriented.

---
<p align="center">
  <sub>run lighter, run faster, run everywhere</sub>
</p>

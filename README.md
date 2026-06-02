# wecode

[中文文档](README_CN.md)

Turn WeChat or Feishu into a remote console for your local [Codex CLI](https://github.com/openai/codex) — dispatch coding tasks from your phone while your machine does the real work.

## Why wecode

AI coding assistants are powerful, but they're tied to your desk. You think of a fix on the commute, want to check a diff at lunch, or need to approve a risky command from your phone. Opening a laptop isn't always an option.

**wecode bridges that gap.** It connects your mobile chat (WeChat or Feishu) to a Codex instance running on your machine, with full project context, session continuity, and approval controls.

| What you get | How it works |
|---|---|
| Code from anywhere | Send a message on WeChat/Feishu, Codex runs in your repo |
| Session continuity | Same chat thread resumes the same Codex session |
| Real project context | Codex sees your actual repo, config, and sandbox |
| Safety by default | Sandboxed execution, chat-based approval for risky commands |
| Zero infrastructure | Runs on your machine, no servers to deploy |

## Key Advantages

- **Not a chatbot** — WeChat/Feishu is just the input layer. Codex runs locally with full repo access.
- **Not a one-shot relay** — Sessions persist across messages via `thread_id` binding.
- **Not full-trust remote exec** — Default sandbox is `workspace-write`; dangerous commands require explicit chat approval.
- **Not vendor-locked** — Prefers Codex app-server remote API, falls back to `codex exec` automatically.
- **Auditable** — Prompt flow logs, `doctor` diagnostics, dry-run mode, and comprehensive integration tests.

## Architecture

```text
┌─────────────────┐
│  WeChat / Feishu │  ← You, on your phone
└────────┬────────┘
         │
┌────────▼────────┐
│  OpenClaw Gateway│  ← Message routing & session binding
│  port 19789     │
└────────┬────────┘
         │
┌────────▼────────┐
│  wecode          │  ← Command parsing, approval, config
│  codex-backend   │
└────────┬────────┘
         │
┌────────▼────────┐
│  Codex CLI       │  ← Code understanding & execution
│  (app-server)    │
└────────┬────────┘
         │
┌────────▼────────┐
│  Your Project    │  ← Real repo, real files
└─────────────────┘
```

**Responsibility boundaries:**

| Layer | Owns |
|---|---|
| OpenClaw | WeChat/Feishu login, message delivery, Gateway, session binding |
| wecode | Local config, command routing, approval queue, Codex invocation, session scanning |
| Codex CLI | Model calls, tool execution, code modification, context management |

**Transport strategy:** wecode prefers the Codex app-server remote API (managed proxy). If unavailable, it falls back to `codex app-server --listen stdio://`, then to `codex exec`. This layered approach insulates you from upstream protocol changes.

## Supported Platforms

| Platform | Status |
|---|---|
| macOS (ARM64 / x86_64) | Full support, including `caffeinate` sleep prevention |
| Linux (x86_64 / ARM64) | Full support |
| Windows (x86_64) | Full support |

Pre-built binaries for all platforms are available on the [Releases](https://github.com/zayfen/wecode/releases) page.

## Getting Started

### Prerequisites

- **Node.js** 24 (or >= 22.19.0)
- **Codex CLI** installed and logged in
- A WeChat or Feishu account for the target OpenClaw channel

### Install

Download a pre-built binary from [Releases](https://github.com/zayfen/wecode/releases), or build from source:

```bash
cargo install --path .
```

### Bootstrap

```bash
# WeChat channel
wecode bootstrap --weixin

# Feishu channel
wecode bootstrap --feishu

# Preview without executing
wecode bootstrap --dry-run --weixin
```

Bootstrap installs a private OpenClaw runtime under `~/.wecode/`, configures the Gateway, registers the Codex backend, and connects the chat channel. You may be prompted to scan a QR code or complete a login flow.

### First Message

Send any message to your connected WeChat/Feishu account. OpenClaw routes it to wecode, which invokes Codex in your configured project directory. The response flows back to your chat.

## Usage

Chat commands use a `:` prefix to avoid conflicts with OpenClaw's own slash commands:

### Codex Commands (forwarded as `/...` prompts)

| Command | Description |
|---|---|
| `:init [description]` | Initialize project context |
| `:compact [instruction]` | Compact conversation context |
| `:plan [task]` | Create an execution plan |
| `:goal [objective]` | Set a goal for Codex |
| `:agent [task]` | Dispatch an agent task |
| `:side [question]` | Side-channel query without affecting main thread |
| `:report [note]` | Check progress on a long-running task |
| `:new [prompt]` | Start a new Codex thread (within OpenClaw session) |

### Local Commands (handled by wecode directly)

| Command | Description |
|---|---|
| `:help` | Show available commands |
| `:status` | Backend, session, approval, and git status |
| `:diff` | Show current `git diff` |
| `:pwd` | Print Codex working directory |
| `:ls [dir]` | List files in directory |
| `:cat <file>` | Read file contents |
| `:cd <dir>` | Switch Codex project directory |
| `:shell <cmd>` | Execute shell command in project directory |
| `:model [name]` | Get or set Codex model |
| `:models` | List available models |
| `:yolo [true\|false]` | Toggle full-auto mode (no approval prompts) |
| `:stop` | Terminate running Codex task |
| `:resume [session_id]` | Bind to latest or specified Codex session |
| `:fresh [prompt]` | Force a new Codex thread |

### Approval Commands

| Command | Description |
|---|---|
| `:yes [id]` / `yes` | Approve pending command |
| `:no [id]` / `no` | Deny pending command |

When only one approval is pending, bare `yes` / `no` works. With multiple pending items, specify the approval ID.

## Configuration

wecode searches for config in this order:

```text
$WECODE_CONFIG
$XDG_CONFIG_HOME/wecode/config.json    (Linux/macOS)
~/.config/wecode/config.json           (Linux/macOS)
%APPDATA%/wecode/config.json           (Windows)
```

Minimal configuration example:

```json
{
  "openclaw": {
    "profile": "wecode",
    "gatewayPort": 19789,
    "preventSleep": "ac"
  },
  "codex": {
    "cwd": "/path/to/your/project",
    "sandbox": "workspace-write",
    "transport": "remote",
    "models": ["default", "gpt-5.4"]
  }
}
```

Run `wecode sample-config` to see the full configuration with defaults.

## Diagnostics

```bash
wecode doctor              # Check Node.js, npm, OpenClaw, Codex availability
wecode runtime-status      # Inspect Gateway, LaunchAgent, and process state
```

Prompt flow logs are written to `~/.wecode/openclaw-state/logs/prompt-flow.log.YYYY-MM-DD` (daily rotation via `tracing`).

## Development

```bash
cargo build                # Build
cargo test                 # Run all tests (118 integration tests)
cargo run -- --help        # CLI help

# Gateway smoke test
node scripts/openclaw-agent-smoke.mjs
```

## Project Structure

```text
src/
├── main.rs              CLI entry point
├── app.rs               Core application logic, Codex invocation, local commands
├── codex_remote.rs      Codex app-server remote JSON-RPC adapter
├── commands.rs          Chat command parsing and prompt conversion
├── openclaw.rs          OpenClaw bootstrap plans and runtime patching
├── platform.rs          Cross-platform abstractions (paths, process management)
├── config.rs            Configuration structures and defaults
├── run_lock.rs          PID-based run lock for serializing Codex turns
├── sessions.rs          Codex session scanning and title lookup
├── backend.rs           Backend trait and command spec builder
├── native_approval.rs   File-based approval persistence
├── diagnostics.rs       External tool detection
├── paths.rs             Path expansion utilities
├── cli.rs               CLI argument parsing
└── yolo.rs              Per-project yolo mode state
```

## License

[MIT](LICENSE)

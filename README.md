# wecode

`wecode` is a personal Rust CLI that wires OpenClaw's Weixin channel to the
local Codex CLI. This version does not implement the Weixin protocol itself; it
manages the local tools that already own those surfaces:

- Weixin login and message delivery: OpenClaw + `@tencent-weixin/openclaw-weixin`
- OpenClaw agent runtime: session-aware custom CLI backend `wecode-codex/default`
- Codex execution: `wecode codex-backend` calling your already logged-in `codex exec`

By default, `wecode` installs OpenClaw into a private runtime under
`~/.wecode/openclaw-runtime` and keeps OpenClaw state/config/workspace under
`~/.wecode/`. It does not install `openclaw` globally. The private Gateway uses
OpenClaw profile `wecode` and port `19789`, leaving the global OpenClaw default
profile and port `18789` untouched.

## Commands

```bash
cargo run -- doctor
cargo run -- sample-config
cargo run -- bootstrap --dry-run --install-openclaw
cargo run -- bootstrap --install-openclaw
cargo run -- configure-codex
cargo run -- install-weixin
cargo run -- codex "say hello from wecode"
cargo run -- codex-backend "say hello from wecode"
cargo run -- codex-backend --jsonl "say hello from wecode"
cargo run -- render "/codex explain this repo"
node scripts/openclaw-agent-smoke.mjs
```

## First Setup

1. Install Node 24, or at least Node `>=22.19.0`.
   `wecode` prefers a supported system Node and can also auto-detect mise, nvm,
   or Volta installs. If your default shell still points at an old Node, set
   `openclaw.nodeBinDir` in the config.
2. Make sure Codex CLI is installed and logged in. This is normal Codex CLI
   login, not OpenClaw Codex OAuth.
3. Run:

```bash
cargo run -- bootstrap --install-openclaw
```

The bootstrap command runs these steps in order. If `wecode` detects that the
current `node` is too old, the dry-run and actual execution prepend the detected
Node bin directory to `PATH` before `npm`, `npx`, and `openclaw` commands:

```bash
npm install --prefix ~/.wecode/openclaw-runtime openclaw@latest
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json ~/.wecode/openclaw-runtime/node_modules/.bin/openclaw config set gateway.port 19789
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json ~/.wecode/openclaw-runtime/node_modules/.bin/openclaw config set agents.defaults.workspace ~/.wecode/workspace
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json ~/.wecode/openclaw-runtime/node_modules/.bin/openclaw config set agents.defaults.cliBackends '{"wecode-codex":{"args":["codex-backend","--jsonl"],"command":"<path-to-wecode>","input":"stdin","output":"jsonl","resumeArgs":["codex-backend","--jsonl","--resume","{sessionId}"],"resumeOutput":"jsonl","serialize":true,"sessionIdFields":["thread_id"]}}' --strict-json --merge
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json ~/.wecode/openclaw-runtime/node_modules/.bin/openclaw config set agents.defaults.models '{"wecode-codex/default":{"alias":"Wecode Codex"}}' --strict-json --merge
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json ~/.wecode/openclaw-runtime/node_modules/.bin/openclaw config set agents.defaults.model '"wecode-codex/default"' --strict-json
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json PATH=~/.wecode/openclaw-runtime/node_modules/.bin:$PATH npx -y @tencent-weixin/openclaw-weixin-cli@latest install
OPENCLAW_PROFILE=wecode OPENCLAW_STATE_DIR=~/.wecode/openclaw-state OPENCLAW_CONFIG_PATH=~/.wecode/openclaw-state/openclaw.json ~/.wecode/openclaw-runtime/node_modules/.bin/openclaw gateway install --force --port 19789
```

Only the Weixin login is interactive during bootstrap. The Weixin installer
shows a QR code through OpenClaw; scan it with Weixin and confirm on the phone.

After setup, send a message to the connected Weixin account. OpenClaw routes the
message to the `wecode-codex/default` CLI backend, which calls
`wecode codex-backend --jsonl`, then `codex exec`. The first Codex run emits a
`thread_id`; OpenClaw stores that id on the Weixin session and later calls
`wecode codex-backend --jsonl --resume <thread_id>` so the same chat continues
the same Codex exec thread.

## Config

By default, `wecode` looks for:

```text
$WECODE_CONFIG
$XDG_CONFIG_HOME/wecode/config.json
~/.config/wecode/config.json
```

If no file exists, built-in defaults are used. See
[`examples/wecode.config.json`](examples/wecode.config.json).

`openclaw.nodeBinDir` is optional. Leave it as `null` for auto-detection, or set
it when you want to pin OpenClaw commands to a specific Node install:

```json
{
  "openclaw": {
    "nodeBinDir": "~/.local/share/mise/installs/node/24.16.0/bin"
  }
}
```

This is useful when `/usr/local/bin/node` or an old Homebrew `node@22` appears
first on `PATH`.

Custom commands are prefix-based. They are currently used by
`wecode render` and by `wecode codex-backend` before invoking Codex:

```bash
cargo run -- render "/review src/main.rs"
printf '/review src/main.rs' | cargo run -- codex-backend
```

Commands marked `requireConfirm: true` are rejected by `codex-backend` in this
version because the Weixin bridge does not yet implement a confirmation
round-trip.

## Verification

Run the local checks first:

```bash
cargo fmt -- --check
cargo check
cargo test
cargo run -- config validate examples/wecode.config.json
```

After `cargo run -- configure-codex` and a running private Gateway on port
`19789`, run the Gateway smoke test:

```bash
node scripts/openclaw-agent-smoke.mjs
```

The script simulates the part after Weixin has delivered a message to OpenClaw:
it opens a local WebSocket to the private Gateway, sends two `agent` requests
with the same `sessionKey`, and checks that both turns use the same Codex
`thread_id` through OpenClaw's `cliSessionBinding`.

Expected output shape:

```text
sessionKey: agent:main:wecode-smoke-...
firstReply: WECODE_GATEWAY_DIRECT_OK
secondReply: WECODE_GATEWAY_RESUME_OK
cliSessionId: 019e...
resumeVerified: true
```

## Context And Approval

`wecode` no longer treats every Weixin message as an unrelated Codex task when
it is launched through OpenClaw. The configured backend is JSONL/session-aware:
OpenClaw parses Codex's `thread_id`, stores it with the Weixin session, and
passes it back through `--resume` on later turns. The working directory, git
state, files, and Codex exec conversation therefore continue for the same
Weixin session.

Direct local calls without OpenClaw are different. `wecode codex-backend "..."` starts
a fresh Codex exec thread unless you pass `--resume <thread_id>` yourself.

Tool approval is deliberately conservative in this version. The Weixin path is a
background CLI call, not the interactive Codex TUI, so native Codex approval
prompts are not exposed as Weixin buttons yet. For personal use, keep
`codex.sandbox` at `workspace-write` and let Codex fail/adjust when it cannot
run a command. Commands that need an explicit `wecode` confirmation can be
marked `requireConfirm: true`; the backend rejects them until a Weixin approval
round-trip is implemented.

The next architecture step is a `wecode daemon` that talks to Codex app-server
or `codex --remote <ADDR>`. That would let Weixin messages join a live Codex
thread and route command/file approval requests back through a terminal or a
Weixin `/approve <id>` flow. This version stays on `codex exec` because it is
smaller, reproducible, and already supports session resume.

## Project Structure

The tests are split by responsibility:

```text
tests/bootstrap.rs    OpenClaw setup plan and private Gateway wiring
tests/cli.rs          CLI argument parsing
tests/commands.rs     custom command rendering and confirmation boundary
tests/config.rs       default and JSON config parsing
tests/diagnostics.rs  local tool diagnostics
tests/common/         shared test helpers
```

OpenClaw owns channel delivery, Weixin session storage, CLI session binding,
and replies. `wecode` owns setup, diagnostics, JSON command expansion, the
Codex CLI backend process, and repeatable local verification scripts.

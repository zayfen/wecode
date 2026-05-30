# AGENTS.md

## 仓库定位

`wecode` 是一个个人使用的 Rust CLI，用来把 OpenClaw 的微信消息接到本机
Codex CLI。默认场景是单用户、本机运行、可审计，不要把它当成通用 Web
服务、公开 SDK 或多租户平台。

## 默认工作目录

- 仓库根目录：`/Users/riven/Github/wecode`
- 默认在仓库根目录执行搜索、测试和代码修改。
- 项目运行时还会涉及 `~/.wecode/openclaw-runtime`、`~/.wecode/openclaw-state`
  和本机 `~/.codex/sessions/`。没有明确要求时，不要修改这些用户状态目录。

## 关键文件职责

- `src/lib.rs`：配置结构、命令解析、OpenClaw 配置计划、Codex session 扫描。
- `src/main.rs`：CLI 入口、Codex 调用、微信本地命令、审批队列、model override。
- `examples/wecode.config.json`：示例配置。
- `scripts/openclaw-agent-smoke.mjs`：Gateway session 续接 smoke test。
- `tests/*.rs`：按行为拆分的集成测试。
- `README.md`：用户文档，默认保持中文。

## 改动边界

- 先看 `git status --short --branch`，不要覆盖或回滚用户未提交改动。
- 优先复用现有 Rust、`serde`、JSONL、CLI 模式；不要为了小功能引入新框架或
  做无关重构。
- 新增或修改微信 slash 命令时，同时检查：
  - 是本地直接处理，还是转换成 Codex prompt
  - `--resume` 行为是否正确
  - JSONL 输出是否兼容 OpenClaw
  - 对应测试是否补齐
- 微信侧输出保持简洁，不要直接回传大段本地状态、敏感路径或完整 session 内容。

## 验证规则

常规 Rust 改动默认运行：

```bash
cargo fmt --check
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test
```

如果改到配置解析、bootstrap 计划或示例配置，再补：

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo run -- config validate examples/wecode.config.json
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo run -- bootstrap --dry-run --install-openclaw
```

只有在用户已经配置并启动私有 OpenClaw Gateway 时，才运行：

```bash
node scripts/openclaw-agent-smoke.mjs
```

## 安全与外部状态

- 不要默认执行会修改用户机器状态的命令，例如 `bootstrap --install-openclaw`、
  `install-weixin`、`openclaw gateway install`。
- 不要默认清理 `~/.wecode/` 或 `~/.codex/sessions/` 下的文件。
- `/resume` 相关输出只列最小必要信息：session id、时间、cwd、originator、
  title。
- 默认保持保守 sandbox；除非用户明确要求，不要把默认行为改成更宽权限。

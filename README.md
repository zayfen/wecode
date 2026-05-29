# wecode

`wecode` 是一个个人用的 Rust CLI，用来把微信或飞书消息接入本机 Codex CLI。它不重新实现通信协议，也不替代 Codex，而是把现有工具组合成一个可维护的个人编程助手。

GitHub 默认展示的文档就是本文件 `README.md`，因此这里使用中文作为主 README。

## 它解决什么问题

在手机微信或飞书里给自己的账号发消息，就可以让本机 Codex 在指定项目目录里工作：

- 微信或飞书负责入口和通知。
- OpenClaw 负责通信通道、Gateway、会话保存和 CLI 后端调用。
- `wecode` 负责本地配置、命令转换、Codex 调用、会话恢复和审批。
- Codex CLI 负责实际代码理解、修改、评审和调试。

目标是让你在不打开电脑的情况下，也能从微信发起代码任务、查看 diff、切换 Codex session、审批高风险命令，并保持同一个项目上下文。

## 核心能力

- 微信/飞书接入本机 Codex：通过 OpenClaw 通道转发消息到 `wecode codex-backend`。
- Codex session 续接：OpenClaw 保存 Codex 返回的 `thread_id`，后续消息自动用 `--resume` 继续同一会话。
- Codex 内置命令适配：支持 `/init`、`/diff`、`/status`、`/model`、`/review`、`/new`、`/compact`、`/plan`、`/goal`、`/agent`、`/side`。
- 聊天中列出和绑定 session：`/resume` 列出当前项目的本机 Codex session，`/resume <session_id>` 绑定到当前聊天会话。
- 微信审批：配置了 `requireConfirm: true` 的命令会先生成审批 id，微信发送 `/approve <id>` 后才执行。
- 私有 OpenClaw 运行时：默认安装到 `~/.wecode/openclaw-runtime`，不污染全局 OpenClaw 配置。

## 架构

```text
个人微信或飞书
  |
  v
OpenClaw Weixin/Feishu 通道
  |
  v
OpenClaw Gateway, profile=wecode, port=19789
  |
  v
OpenClaw agent runtime
  |
  v
wecode codex-backend --jsonl --cwd <project_dir> [--model wecode-codex/<model>] [--resume <thread_id>]
  |
  v
codex exec / codex exec resume / codex exec review
  |
  v
当前项目目录
```

职责边界：

- OpenClaw 管理微信/飞书登录、消息投递、Gateway、会话和 CLI session binding。
- `wecode` 管理本地安装计划、配置、诊断、命令模板、审批队列、Codex session 扫描和 Codex CLI 调用。
- Codex CLI 管理模型调用、工具执行、代码修改、上下文和本机 session 文件。

## 优势

- 不锁死在一个聊天窗口：微信只是入口，真正上下文仍在 Codex 本机 session 中。
- 可恢复：同一个聊天会话会继续同一个 Codex `thread_id`，不会每条消息都变成新任务。
- 可审计：本地审批文件存放在 `openclaw.stateDir/approvals`，高风险命令不会静默执行。
- 可控：默认 sandbox 是 `workspace-write`，不默认开启危险全权限。
- 可迁移：OpenClaw、Codex、微信通道都是独立组件，后续可以把 `codex exec` 换成 `codex app-server` 或 remote-control。
- 不污染全局环境：OpenClaw 默认安装在 `~/.wecode` 下，使用独立 profile 和端口。

## 快速开始

前置条件：

- Node 24，或者至少 Node `>=22.19.0`。
- 已安装并登录 Codex CLI。
- 当前机器能使用目标 OpenClaw 通道登录。

初始化：

```bash
cargo run -- bootstrap --weixin
# 或
cargo run -- bootstrap --feishu
```

`bootstrap` 一定会安装私有 OpenClaw，不再需要 `--install-openclaw`。这个命令会执行 OpenClaw 安装、私有运行时补丁、Gateway 配置、Codex CLI 后端配置、通信通道安装/登录和 Gateway 安装。通道登录过程中可能会出现二维码或登录提示，需要按目标通道完成确认。

`wecode` 会把 OpenClaw 的文本内置命令处理关闭，让聊天里的 `/help`、`/status`、`/pwd`、`/ls`、`/cat`、`/cd`、`/shell` 等 slash 命令先进入 `wecode codex-backend`。OpenClaw 默认会在微信和飞书这类非 native slash command 通道上继续 fallback 处理文本命令，即使 `commands.text=false`；因此 `bootstrap` 会补丁私有 OpenClaw 运行时，让这个配置真正生效。这样 `/help` 返回的是 wecode 帮助，而不是 OpenClaw 默认帮助。

已有安装如果遇到 `/help` 被 OpenClaw 拦截，可以重新执行 `wecode bootstrap --weixin`，或只执行补丁后重启 Gateway：

```bash
wecode patch-openclaw-runtime --runtime-dir ~/.wecode/openclaw-runtime
```

完成后，给已连接的微信或飞书账号发消息。OpenClaw 会把消息路由到 `wecode-codex/default`，实际调用：

```bash
wecode codex-backend --jsonl --cwd /Users/riven/Github/wecode --model wecode-codex/default
```

第一次 Codex 运行会返回 `thread_id`，OpenClaw 会把它存到聊天会话里。后续同一聊天会话会调用：

```bash
wecode codex-backend --jsonl --cwd /Users/riven/Github/wecode --model wecode-codex/default --resume <thread_id>
```

切换 Codex 模型用 wecode 的项目级模型命令。默认配置会把 `default` 和 `gpt-5.4` 加入候选列表，因此聊天里可以发送：

```text
/models
/model gpt-5.4
```

`/model <model>` 会把模型保存到当前项目自己的状态文件中，不同项目互不影响。后续 `wecode` 调用 Codex 时会使用 Codex CLI 原生参数 `codex exec -m <model> -C <project_dir>`，同时把 Codex 子进程的工作目录切到该项目，确保 `workspace-write` 沙箱能写当前项目。

## 常用本地命令

```bash
cargo run -- doctor
cargo run -- sample-config
cargo run -- bootstrap --dry-run --weixin
cargo run -- bootstrap --weixin
cargo run -- bootstrap --feishu
cargo run -- patch-openclaw-runtime --runtime-dir ~/.wecode/openclaw-runtime
cargo run -- configure-codex
cargo run -- codex "say hello from wecode"
cargo run -- codex-backend "say hello from wecode"
cargo run -- codex-backend --jsonl "say hello from wecode"
cargo run -- render "/codex explain this repo"
node scripts/openclaw-agent-smoke.mjs
```

## 聊天命令

`wecode codex-backend` 会识别适合微信和飞书使用的 Codex 风格 slash 命令：

```text
/help                     显示命令列表
/init [说明]              让 Codex 创建或更新 AGENTS.md
/diff                     显示当前 git diff
/pwd                      显示当前 Codex 项目目录的绝对路径
/ls [目录]                本地列出目录下的文件和目录，返回绝对路径
/cat <文件>               本地读取文件内容并返回
/cd <目录>                切换 Codex 项目根目录，后续 Codex 任务使用该目录
/shell <命令>             在当前 Codex 项目目录执行任意 shell 命令并返回输出
/status                   显示后端、session、审批和 git 状态
/model                    显示当前项目使用的 Codex 模型
/models                   列出配置中的 Codex 模型候选
/model <model>            设置当前项目后续 Codex 调用使用的模型
/model default            清除当前项目的模型覆盖，回到 Codex 默认模型
/review [说明]            执行 codex exec review --uncommitted
/new [prompt]             开启新的 Codex thread，并让聊天会话改绑新 thread_id
/compact [说明]           让 Codex 压缩当前上下文为 handoff summary
/plan [任务]              让 Codex 先写计划，不直接改文件
/goal [目标]              让 Codex 汇报或更新当前目标
/agent [任务]             让 Codex 在适合时使用 subagent
/side [问题]              作为旁路分析回答，不主动改文件
/report [说明]            等价于用 /side 查询“任务状态”，适合长任务中查看进展
/resume                   列出当前项目的本机 Codex sessions
/sessions                 等同于 /resume
/resume <session_id>      把当前聊天会话绑定到指定 Codex session
/approve <id>             批准待执行命令
/deny <id>                拒绝待执行命令
```

处理方式：

- `/help`、`/diff`、`/pwd`、`/ls`、`/cat`、`/cd`、`/shell`、`/status`、`/resume`、`/approve`、`/deny` 由 `wecode` 本地直接回答，不会进入 Codex。
- `/cd <目录>` 会把目标目录写入 `openclaw.stateDir/codex-cwd.txt`，并让下一次 Codex 请求新开 thread，避免续接旧项目 session。
- `/shell <命令>` 会在当前 Codex 项目目录执行命令；Unix/macOS 使用 `sh -lc`，Windows 使用 `cmd /C`，返回 exit code、stdout 和 stderr。
- `/model` 和 `/models` 由 `wecode` 本地处理；模型状态按项目目录保存到 `openclaw.stateDir/codex-models/`。
- `/review` 调用 Codex 的非交互评审子命令。
- `/new` 忽略当前 `--resume`，强制开启新 Codex thread。
- `/report` 会转换成 `/side 任务状态` 语义的 Codex prompt，用来在长任务中旁路查询进展。
- `/init`、`/compact`、`/plan`、`/goal`、`/agent`、`/side` 会转换成明确的 `codex exec` prompt。

## 配置

`wecode` 按以下顺序寻找配置：

```text
$WECODE_CONFIG
$XDG_CONFIG_HOME/wecode/config.json
~/.config/wecode/config.json
```

没有配置文件时使用内置默认值。示例见 [examples/wecode.config.json](examples/wecode.config.json)。

关键配置：

```json
{
  "openclaw": {
    "profile": "wecode",
    "runtimeDir": "~/.wecode/openclaw-runtime",
    "stateDir": "~/.wecode/openclaw-state",
    "configPath": "~/.wecode/openclaw-state/openclaw.json",
    "workspaceDir": "~/.wecode/workspace",
    "gatewayPort": 19789,
    "nodeBinDir": null
  },
  "codex": {
    "sandbox": "workspace-write",
    "cwd": ".",
    "model": null,
    "models": ["default", "gpt-5.4"]
  }
}
```

`openclaw.workspaceDir` 是 OpenClaw 自己的工作区，用来保存 OpenClaw agent 的 workspace 文件；不要把它设置成你的代码项目目录，否则 OpenClaw 可能会在项目里生成 `SOUL.md`、`IDENTITY.md`、`.openclaw/` 等文件。

要切换 Codex 处理的项目，只设置 `codex.cwd`，或在聊天里发送 `/cd <项目目录>`：

```json
{
  "openclaw": {
    "workspaceDir": "~/.wecode/workspace"
  },
  "codex": {
    "cwd": "/Users/riven/Github/wecode",
    "sandbox": "workspace-write",
    "model": null,
    "models": ["default", "gpt-5.4"]
  }
}
```

这样 OpenClaw 的工作区保持固定，`wecode` 会把 `codex.cwd` 作为 `--cwd` 传给后端，并在调用 Codex 时使用 `codex exec -C <project_dir>`。`/resume` 扫描 `~/.codex/sessions/**/rollout-*.jsonl` 时，也会按该项目目录过滤。

`codex.models` 会生成 OpenClaw 的 `agents.defaults.models` 白名单。想在微信中使用更多 Codex 模型时，把模型名追加到这里，然后重新执行：

```bash
cargo run -- configure-codex
```

例如：

```json
{
  "codex": {
    "models": ["default", "gpt-5.4", "your-next-model"]
  }
}
```

## 自定义命令和审批

自定义命令按前缀匹配，把微信消息转换为 Codex prompt：

```json
{
  "name": "deploy",
  "prefix": "/deploy ",
  "prompt": "Deploy request: {{message}}",
  "requireConfirm": true
}
```

如果 `requireConfirm` 是 `true`，聊天里第一次发送：

```text
/deploy production
```

`wecode` 不会立即调用 Codex，而是返回类似：

```text
Command `deploy` requires approval.
Approve: /approve appr-...
Deny: /deny appr-...
```

发送 `/approve <id>` 后才会执行保存的 prompt；发送 `/deny <id>` 会删除待审批请求。

## 验证

本地检查：

```bash
cargo fmt -- --check
cargo check
cargo test
cargo run -- config validate examples/wecode.config.json
```

Gateway smoke test：

```bash
node scripts/openclaw-agent-smoke.mjs
```

该脚本模拟微信消息已进入 OpenClaw 之后的流程：它连接私有 Gateway，用同一个 `sessionKey` 发送两次 agent 请求，并检查第二次是否复用了同一个 Codex `thread_id`。

期望输出形状：

```text
sessionKey: agent:main:wecode-smoke-...
firstReply: WECODE_GATEWAY_DIRECT_OK
secondReply: WECODE_GATEWAY_RESUME_OK
cliSessionId: 019e...
resumeVerified: true
```

## 当前限制

- 当前后端基于 `codex exec`，不是 Codex TUI。
- Codex 原生工具审批弹窗暂时不能直接变成微信按钮。
- 现在的微信审批是 `wecode` 自己的确认队列，适合保护自定义高风险命令。
- 后续更完整的方案是接入 `codex app-server` 或 `codex --remote <ADDR>`，让微信加入实时 Codex 会话并承接原生审批事件。

## 项目结构

```text
src/lib.rs                         配置、命令解析、OpenClaw 配置计划、私有运行时补丁、session 扫描
src/main.rs                        CLI 入口、Codex 调用、微信本地命令、审批队列
scripts/openclaw-agent-smoke.mjs   Gateway session 续接 smoke test
examples/wecode.config.json        示例配置
tests/bootstrap.rs                 OpenClaw setup plan 和私有 Gateway 配置
tests/cli.rs                       CLI 参数解析
tests/codex_backend.rs             Codex 后端、resume、审批、model、/new 行为
tests/commands.rs                  命令模板和微信 slash 命令解析
tests/config.rs                    默认配置和 JSON 配置解析
tests/diagnostics.rs               本地工具诊断
tests/resume_sessions.rs           Codex session 扫描和项目过滤
```

## 设计取舍

这个版本优先选择可复现、可测试、可渐进升级的实现：

- 用 `codex exec` 保持后端简单。
- 用 JSONL 与 OpenClaw 对接，方便 OpenClaw 提取 `thread_id`。
- 用本机 Codex session 文件实现 `/resume` 列表。
- 用 `openclaw.stateDir` 保存 `model` override 和待审批请求。
- 把微信命令分成本地控制命令和 Codex prompt 命令，避免把 TUI-only 行为硬塞进非交互后端。
- 对私有 OpenClaw runtime 做最小补丁，只改变 `commands.text=false` 在非 native 通道上的文本命令 fallback，避免 `/help`、`/status`、`/model` 等命令被 OpenClaw 抢走。

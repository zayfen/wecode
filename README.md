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
- Codex 内置命令透传：聊天里发送 `:init`、`:new`、`:compact`、`:plan`、`:goal`、`:agent`、`:side`，`wecode` 会转成 Codex 原生 `/...` prompt，也可用自定义命令覆盖 prompt。
- 聊天中管理 session：`:resume [session_id]` 绑定到最近或指定的本机 Codex session，`:fresh [prompt]` 硬新开 Codex thread。
- 微信审批：配置了 `requireConfirm: true` 的命令会先生成审批 id，微信发送 `:approve <id>` 后才执行。
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

代码模块边界：

- `config`、`cli`、`command_step`、`diagnostics` 负责通用配置、命令行解析、命令步骤和本机工具诊断。
- `commands` 负责聊天文本命令解析和 Codex prompt 转换，不直接执行本地命令或后端。
- `openclaw` 负责 OpenClaw bootstrap/config 计划和通信渠道安装描述，新增渠道优先从这里接入。
- `backend` 负责后端执行器的内部接口和 Codex 命令规格，后续接入 Claude Code 等 backend 时应复用这个边界。
- `sessions` 和 `paths` 负责 Codex session 扫描与共享路径处理。

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

`bootstrap` 一定会安装私有 OpenClaw，不再需要 `--install-openclaw`。这个命令会执行 OpenClaw 安装、Gateway 配置、Codex CLI 后端配置、通信通道安装/登录和 Gateway 安装。通道登录过程中可能会出现二维码或登录提示，需要按目标通道完成确认。

聊天里的 `wecode` 命令统一使用 `:` 前缀，例如 `:help`、`:status`、`:compact`。这样可以避开 OpenClaw 自己的 slash 命令路由，不需要补丁 OpenClaw，也不需要关闭 `commands.text`。当命令需要交给 Codex 原生处理时，`wecode` 只把开头的 `:` 转成 `/`，例如 `:compact keep decisions` 会作为 `/compact keep decisions` 发给 Codex。

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
:models
:model gpt-5.4
```

`:model <model>` 会把模型保存到当前项目自己的状态文件中，不同项目互不影响。后续 `wecode` 调用 Codex 时会使用 Codex CLI 原生参数 `codex exec -m <model> -C <project_dir>`，同时把 Codex 子进程的工作目录切到该项目，确保 `workspace-write` 沙箱能写当前项目。

## 常用本地命令

```bash
cargo run -- doctor
cargo run -- sample-config
cargo run -- bootstrap --dry-run --weixin
cargo run -- bootstrap --weixin
cargo run -- bootstrap --feishu
cargo run -- configure-codex
cargo run -- codex "say hello from wecode"
cargo run -- codex-backend "say hello from wecode"
cargo run -- codex-backend --jsonl "say hello from wecode"
cargo run -- render ":codex explain this repo"
node scripts/openclaw-agent-smoke.mjs
```

## 聊天命令

`wecode codex-backend` 会识别适合微信和飞书使用的 `:` 命令。需要交给 Codex 的命令会在后端转成 Codex 原生 `/...` prompt：

```text
:help                     显示命令列表
:init [说明]              转成 /init 后发送给 Codex 原生命令
:diff                     显示当前 git diff
:pwd                      显示当前 Codex 项目目录的绝对路径
:ls [目录]                本地列出目录下的文件和目录，返回绝对路径
:cat <文件>               本地读取文件内容并返回
:cd <目录>                切换 Codex 项目根目录，后续 Codex 任务使用该目录
:shell <命令>             在当前 Codex 项目目录执行任意 shell 命令并返回输出
:status                   显示后端、session、审批和 git 状态
:model                    显示当前项目使用的 Codex 模型
:models                   列出配置中的 Codex 模型候选
:model <model>            设置当前项目后续 Codex 调用使用的模型
:model default            清除当前项目的模型覆盖，回到 Codex 默认模型
:review [说明]            执行 codex exec review --uncommitted
:new [prompt]             转成 /new 后发送给 Codex 原生命令，不保证切换 Wecode 绑定的 session
:compact [说明]           转成 /compact 后发送给 Codex 原生命令
:plan [任务]              转成 /plan 后发送给 Codex 原生命令
:goal [目标]              转成 /goal 后发送给 Codex 原生命令
:agent [任务]             转成 /agent 后发送给 Codex 原生命令
:side [问题]              转成 /side 后发送给 Codex 原生命令
:report [说明]            等价于旁路查询“任务状态”，适合长任务中查看进展
:resume [session_id]      绑定到最近或指定的 Codex session，不请求 Codex
:fresh [prompt]           硬新开 Codex thread；带 prompt 时立即执行，不带 prompt 时作用于下一条请求
:approve <id>             批准待执行命令
:deny <id>                拒绝待执行命令
```

处理方式：

- `:help`、`:diff`、`:pwd`、`:ls`、`:cat`、`:cd`、`:shell`、`:status`、`:model`、`:models`、`:resume`、`:fresh`、`:approve`、`:deny` 由 `wecode` 本地直接回答或执行，不会作为 prompt 进入 Codex。普通本地命令结果会以 markdown code block 返回；`:resume` 是 session 控制命令，会返回 `thread_id` 让 OpenClaw 绑定 session。
- `:cd <目录>` 会把目标目录写入 `openclaw.stateDir/codex-cwd.txt`，并让下一次 Codex 请求新开 thread，避免续接旧项目 session。
- `:fresh` 会让下一次 Codex 请求不带 `--resume`；`:fresh <prompt>` 会立刻以新 Codex thread 执行 `<prompt>`。
- `:shell <命令>` 会在当前 Codex 项目目录执行命令；Unix/macOS 使用 `sh -lc`，Windows 使用 `cmd /C`，返回命令真实 stdout/stderr 内容。
- `:model` 和 `:models` 由 `wecode` 本地处理；模型状态按项目目录保存到 `openclaw.stateDir/codex-models/`。
- `:review` 调用 Codex 的非交互评审子命令。
- `:init`、`:new`、`:compact`、`:plan`、`:goal`、`:agent`、`:side` 默认会先把开头的 `:` 转成 `/`，再作为原始输入发送给 Codex；如果用户在配置中添加同前缀的自定义命令，会按自定义模板覆盖。注意 `:new` 在 `codex exec resume <old-session> -- "<prompt>"` 场景里只是 prompt，不保证改变 OpenClaw/Wecode 当前绑定的 session；需要硬新开 thread 时使用 `:fresh`。
- `:report` 会转换成类似 `/side 任务状态` 语义的 Codex prompt，用来在长任务中旁路查询进展。

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

要切换 Codex 处理的项目，只设置 `codex.cwd`，或在聊天里发送 `:cd <项目目录>`：

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

这样 OpenClaw 的工作区保持固定，`wecode` 会把 `codex.cwd` 作为 `--cwd` 传给后端，并在调用 Codex 时使用 `codex exec -C <project_dir>`。`:resume` 不带参数时扫描 `~/.codex/sessions/**/rollout-*.jsonl`，选择最近的 Codex session 并返回 `thread_id`；带参数时直接绑定指定 session id。

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
  "prefix": ":deploy ",
  "prompt": "Deploy request: {{message}}",
  "requireConfirm": true
}
```

如果 `requireConfirm` 是 `true`，聊天里第一次发送：

```text
:deploy production
```

`wecode` 不会立即调用 Codex，而是返回类似：

```text
Command `deploy` requires approval.
Approve: :approve appr-...
Deny: :deny appr-...
```

发送 `:approve <id>` 后才会执行保存的 prompt；发送 `:deny <id>` 会删除待审批请求。

## 调试日志

`wecode codex-backend` 会把用户输入的 prompt 流转写入：

```text
~/.wecode/openclaw-state/logs/prompt-flow.log.YYYY-MM-DD
```

日志使用 `tracing` 和 `tracing-appender` 写入，按天轮转，不是 JSONL。同一次请求共享
`run_id`。常见事件包括：

- `backend_input_received`：OpenClaw 传给 Wecode 的原始输入。
- `backend_input_prepared`：从飞书/微信包装消息中提取后的命令输入，以及 Wecode 识别出的本地命令或最终 prompt。
- `codex_prompt_dispatch`：准备发送给 Codex 的 prompt、model、resume/fresh 决策。
- `codex_exec_command`：实际执行的 `codex exec` 参数和 working root。
- `codex_exec_result`：Codex 进程退出状态和输出大小。

该日志会保留完整用户 prompt，适合本机调试，不要上传到公开 issue 或共享仓库。

## 验证

本地检查：

```bash
cargo fmt --check
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
src/lib.rs                         模块出口和公共 API
src/main.rs                        CLI 入口
src/app.rs                         应用流程、Codex 调用、本地命令、审批队列
scripts/openclaw-agent-smoke.mjs   Gateway session 续接 smoke test
examples/wecode.config.json        示例配置
tests/bootstrap.rs                 OpenClaw setup plan 和私有 Gateway 配置
tests/cli.rs                       CLI 参数解析
tests/codex_backend.rs             Codex 后端、resume、审批、model、:new、:fresh 行为
tests/commands.rs                  命令模板和聊天 `:` 命令解析
tests/config.rs                    默认配置和 JSON 配置解析
tests/diagnostics.rs               本地工具诊断
tests/resume_sessions.rs           Codex session 扫描和项目过滤
```

## 设计取舍

这个版本优先选择可复现、可测试、可渐进升级的实现：

- 用 `codex exec` 保持后端简单。
- 用 JSONL 与 OpenClaw 对接，方便 OpenClaw 提取 `thread_id`。
- 用本机 Codex session 文件实现 `:resume` 最近 session 绑定。
- 用 `openclaw.stateDir` 保存 `model` override 和待审批请求。
- 把聊天命令分成本地控制命令和 Codex prompt 命令，避免把 TUI-only 行为硬塞进非交互后端。
- 用 `:` 作为 Wecode 聊天命令命名空间，避开 OpenClaw 的 slash 命令；需要交给 Codex 的命令再转成 `/...` prompt。

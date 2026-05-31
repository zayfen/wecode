# wecode

`wecode` 是一个个人用的 Rust CLI，用来把微信或飞书消息接入本机 Codex CLI。它把你已经在本机使用的 Codex、项目目录、session、sandbox 和审批流程，延伸到手机聊天入口里，让代码任务可以从微信或飞书发起、续接和检查。

它不重新实现通信协议，也不替代 Codex，而是把 OpenClaw 和 Codex CLI 组合成一个可维护、可审计、可本机运行的个人编程助手。

GitHub 默认展示的文档就是本文件 `README.md`，因此这里使用中文作为主 README。

## 一句话介绍

`wecode` 让微信或飞书变成本机 Codex 的远程控制台：人在外面也能让电脑里的 Codex 在指定仓库里继续工作，同时保留本机执行、本机状态、本机审批和可回退的 CLI 路径。

## 项目价值

很多 AI 编程工作并不一定要坐在电脑前才能发起：想到一个改动、需要看一眼 diff、想让 Codex 继续排查失败测试、临时审批一个高风险命令，手机聊天入口已经足够。`wecode` 的价值是把这些碎片化时刻接到本机开发环境里，而不是再做一个脱离仓库上下文的聊天机器人。

它的核心取向是“个人、本机、可控”：

- **把手机变成开发入口**：微信或飞书负责输入和通知，Codex 仍在你的机器、你的仓库、你的配置里运行。
- **保留真实项目上下文**：同一聊天会话续接同一个 Codex `thread_id`，避免每条消息都变成孤立任务。
- **适合长任务和异步工作**：可以在路上发起实现、回到电脑前查看改动，也可以用 `:report`、`:diff`、`:review` 跟进进展。
- **默认安全边界清晰**：默认 sandbox 是 `workspace-write`，自定义高风险命令可要求微信审批，本地状态目录可检查。
- **工程上可维护**：OpenClaw 管通信，`wecode` 管本地配置和命令转换，Codex 管代码理解和执行；每层职责明确，出了问题容易定位。
- **不绑定单一路径**：优先使用 Codex app-server remote API，managed remote 不可用时可走 stdio app-server，再必要时回退 `codex exec`。

## 适合场景

- 个人开发者希望在微信或飞书里调度本机 Codex，而不是维护一套公开 Bot 服务。
- 已经习惯 Codex CLI，希望远程发起任务但仍复用本机 session、配置、sandbox 和仓库权限。
- 经常需要在手机上查看项目状态、diff、模型配置、最近 session 或审批命令。
- 需要一个可审计、可回退、容易调试的私人工具，而不是多租户平台或通用 SDK。

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
- 微信审批：配置了 `requireConfirm: true` 的命令会先生成审批 id，微信发送 `:approve <id>` 或 `:yes <id>` 后才执行；Codex remote 原生审批也会转成同样的审批提示。
- 私有 OpenClaw 运行时：默认安装到 `~/.wecode/openclaw-runtime`，不污染全局 OpenClaw 配置。
- macOS AC 防睡眠：默认用 `caffeinate -s` 包装 OpenClaw Gateway LaunchAgent，避免外接电源下息屏后系统睡眠导致消息不处理。

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
Codex app-server remote API
  |-- managed: codex remote-control daemon + codex app-server proxy
  |-- compatible: codex app-server --listen stdio://
  |
  v
fallback: codex exec / codex exec resume / codex exec review
  |
  v
当前项目目录
```

职责边界：

- OpenClaw 管理微信/飞书登录、消息投递、Gateway、会话和 CLI session binding。
- `wecode` 管理本地安装计划、配置、诊断、命令模板、审批队列、Codex remote/exec 调用和 Codex session 扫描。
- Codex CLI 管理模型调用、工具执行、代码修改、上下文和本机 session 文件。

代码模块边界：

- `config`、`cli`、`command_step`、`diagnostics` 负责通用配置、命令行解析、命令步骤和本机工具诊断。
- `commands` 负责聊天文本命令解析和 Codex prompt 转换，不直接执行本地命令或后端。
- `openclaw` 负责 OpenClaw bootstrap/config 计划和通信渠道安装描述，新增渠道优先从这里接入。
- `backend` 负责后端执行器的内部接口和 Codex 命令规格，后续接入 Claude Code 等 backend 时应复用这个边界。
- `sessions` 和 `paths` 负责 Codex session 扫描与共享路径处理。

## 优势

- **不是普通聊天 Bot**：微信或飞书只是入口，真正的上下文仍在 Codex 本机 session 和当前项目目录里。
- **不是一次性 prompt 转发器**：OpenClaw 保存 Codex 返回的 `thread_id`，后续消息会自动续接同一条 Codex 会话。
- **不是全权限远程执行器**：默认 sandbox 是 `workspace-write`，高风险自定义命令可以走微信审批。
- **不是侵入式平台**：OpenClaw 默认安装在 `~/.wecode` 下，使用独立 profile、端口和状态目录，不污染全局 OpenClaw 配置。
- **不是绑定单一实验接口**：默认优先使用 Codex app-server remote API，失败时可回退 `codex exec`，降低上游协议变化带来的风险。
- **不是只能靠猜日志排障**：`wecode` 提供 `doctor`、`--dry-run`、prompt-flow 日志、Gateway smoke test 和覆盖主要行为的集成测试。

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

`:model <model>` 会把模型保存到当前项目自己的状态文件中，不同项目互不影响。后续 `wecode` 调用 Codex 时会优先通过 Codex app-server remote API 设置模型和工作目录；如果 remote 不可用，会回退到 Codex CLI 原生参数 `codex exec --yolo -m <model> -C <project_dir>`。

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
:approve <id> / :yes <id> 批准待执行命令
:deny <id> / :no <id>     拒绝待执行命令
```

处理方式：

- `:help`、`:diff`、`:pwd`、`:ls`、`:cat`、`:cd`、`:shell`、`:status`、`:model`、`:models`、`:resume`、`:fresh`、`:approve`、`:yes`、`:deny`、`:no` 由 `wecode` 本地直接回答或执行，不会作为 prompt 进入 Codex。普通本地命令结果会以 markdown code block 返回；`:resume` 是 session 控制命令，会返回 `thread_id` 让 OpenClaw 绑定 session。
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
    "timeoutSeconds": 1200,
    "cliNoOutputTimeoutMs": 900000,
    "preventSleep": "ac",
    "nodeBinDir": null
  },
  "codex": {
    "sandbox": "workspace-write",
    "transport": "remote",
    "remote": {
      "autoStart": true,
      "proxyCommand": "codex app-server proxy",
      "startCommand": "codex remote-control start --json",
      "fallbackProxyCommand": "codex app-server --listen stdio://"
    },
    "cwd": ".",
    "model": null,
    "models": ["default", "gpt-5.4"]
  }
}
```

`openclaw.workspaceDir` 是 OpenClaw 自己的工作区，用来保存 OpenClaw agent 的 workspace 文件；不要把它设置成你的代码项目目录，否则 OpenClaw 可能会在项目里生成 `SOUL.md`、`IDENTITY.md`、`.openclaw/` 等文件。

`openclaw.timeoutSeconds` 会写入 OpenClaw 的 `agents.defaults.timeoutSeconds`，控制一次 agent turn 的整体超时。`openclaw.cliNoOutputTimeoutMs` 会写入 `agents.defaults.cliBackends.wecode-codex.reliability.watchdog.fresh/resume.noOutputTimeoutMs`，控制 Codex CLI 子进程多长时间没有 stdout/stderr 输出后才被 OpenClaw watchdog 终止。默认值分别是 1200 秒和 900000 毫秒。

`openclaw.preventSleep` 控制 wecode 是否改写 OpenClaw Gateway 的 macOS LaunchAgent。默认值 `"ac"` 会把 Gateway 启动命令包装成 `/usr/bin/caffeinate -s ...`，只在外接电源时阻止系统睡眠，不阻止显示器息屏；设置为 `"off"` 会移除 wecode 添加的 `caffeinate` 包装。修改这个配置后运行 `wecode configure-codex` 或重新 `bootstrap`，LaunchAgent 才会被更新。

`codex.transport` 控制 wecode 如何调用 Codex。默认 `"remote"` 会先通过 `codex remote-control start --json` + `codex app-server proxy` 使用 managed remote；如果本机 Codex 不是 standalone 安装，或者 managed proxy 不可用，会改用 `codex.remote.fallbackProxyCommand`，默认是 `codex app-server --listen stdio://`。只有两条 remote 路径都失败时，`"remote"` 才自动回退 `codex exec`；`"remote-strict"` 不会回退 exec，但仍会尝试这个 stdio app-server 兼容路径；`"exec"` 会强制使用 `codex exec --yolo --json` 路径。

`wecode codex-backend --jsonl` 会输出 OpenClaw 可识别的 JSONL。remote 模式会把 app-server turn 结果转换成兼容 JSONL，并在 app-server 返回 assistant/message JSON 外壳时提取内部文本，避免微信侧看到二次包装的响应 JSON；当 Codex remote 在 turn 中途完成一段非 final agent message 时，`wecode` 会立即把这段阶段性回复写入 JSONL 并 flush，让微信侧先收到进展，而不是一直等最终答案。exec fallback 模式会实时转发 `codex exec --yolo --json` 的 stdout/stderr，避免因为 Wecode 缓存输出导致 OpenClaw watchdog 超时。

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

这样 OpenClaw 的工作区保持固定，`wecode` 会把 `codex.cwd` 作为 `--cwd` 传给后端，并在调用 Codex 时把它作为 remote thread/turn 的 `cwd`；exec fallback 时使用 `codex exec --yolo -C <project_dir>`。`:resume` 不带参数时扫描 `~/.codex/sessions/**/rollout-*.jsonl`，选择最近的 Codex session 并返回 `thread_id`；带参数时直接绑定指定 session id。

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
Approve: :approve appr-... or :yes appr-...
Deny: :deny appr-... or :no appr-...
```

发送 `:approve <id>` 或 `:yes <id>` 后才会执行保存的 prompt；发送 `:deny <id>` 或 `:no <id>` 会删除待审批请求。

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
- `codex_remote_dispatch`：准备通过 Codex app-server remote API 启动或续接 thread。
- `codex_remote_turn_completed`：remote turn 完成后的 thread id 和最终消息大小。
- `codex_remote_fallback_to_exec`：remote 不可用并回退到 `codex exec` 的原因。
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

- 当前后端优先使用 Codex app-server remote API，不启动 Codex TUI；standalone Codex 可走 managed remote，非 standalone Codex 会走 `codex app-server --listen stdio://` 兼容路径。`codex --remote <ADDR>` 仍是 TUI 连接 remote app-server 的入口。
- remote API 仍是 Codex 实验接口；默认 `"remote"` 会自动回退 `codex exec`，需要强校验时使用 `"remote-strict"`。
- remote 模式会把 Codex app-server 原生审批请求转成微信/飞书可见的 `appr-...` 审批提示。发送 `:approve appr-...` 或 `:yes appr-...` 会批准当前 Codex turn 的这一次请求；发送 `:deny appr-...` 或 `:no appr-...` 会拒绝。这个能力只覆盖 remote/app-server transport；`codex exec` fallback 使用 `--yolo`，不做交互式审批桥接。
- 如果你已经运行过旧版 `wecode configure-codex`，升级后重新运行一次 `wecode configure-codex`，让 OpenClaw backend 配置从 `serialize: true` 更新为 `serialize: false`。Wecode 会用自己的运行锁串行化 Codex turn，并允许 `:approve` / `:deny` / `:yes` / `:no` 在等待审批时进入。

## 项目结构

```text
src/lib.rs                         模块出口和公共 API
src/main.rs                        CLI 入口
src/app.rs                         应用流程、Codex 调用、本地命令、审批队列
src/codex_remote.rs                Codex app-server remote JSON-RPC 适配
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

- 保留 `codex exec` fallback 来对冲实验协议变化。
- 优先走 Codex app-server remote API，让 wecode 能直接启动/续接 thread；managed remote 不可用时使用 stdio app-server，避免要求所有用户安装 standalone Codex。
- 用 JSONL 与 OpenClaw 对接，方便 OpenClaw 提取 `thread_id`。
- 用本机 Codex session 文件实现 `:resume` 最近 session 绑定。
- 用 `openclaw.stateDir` 保存 `model` override 和待审批请求。
- 把聊天命令分成本地控制命令和 Codex prompt 命令，避免把 TUI-only 行为硬塞进非交互后端。
- 用 `:` 作为 Wecode 聊天命令命名空间，避开 OpenClaw 的 slash 命令；需要交给 Codex 的命令再转成 `/...` prompt。

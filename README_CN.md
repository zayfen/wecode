# wecode

把微信或飞书变成本机 [Codex CLI](https://github.com/openai/codex) 的远程控制台 — 手机发条消息，电脑里的 AI 就开始在你的仓库里干活。

## 为什么选 wecode

市面上连接聊天和 AI 的工具不少，但绝大多数只是把云端 API 包了一层聊天界面。wecode 做的事情完全不同：

| | wecode | chatgpt-on-wechat (45k⭐) | wechat-chatgpt | slack_codex_bot | telegram-claude-bot |
|---|---|---|---|---|---|
| 本地代码执行 | ✅ Codex CLI 本机运行 | ❌ 仅云端 API | ❌ 仅云端 API | ✅ 但极早期 | ✅ 但无审批 |
| 会话连续性 | ✅ thread_id 绑定 | ⚠️ 仅上下文窗口 | ❌ | ❌ | ⚠️ 部分 |
| 权限审批 | ✅ 聊天审批 + sandbox | ❌ | ❌ | ❌ | ❌ |
| 项目上下文 | ✅ 真实仓库 + 目录 | ❌ | ❌ | ⚠️ | ⚠️ |
| 零配置上手 | ✅ 一键 bootstrap | ❌ 需配置多个环境变量 | ❌ 需 Docker + wechaty | ❌ | ❌ |
| 多通道支持 | ✅ 微信 + 飞书 | ✅ 多通道 | ⚠️ 仅微信 | ⚠️ 仅 Slack | ⚠️ 仅 Telegram |

## 核心优势

### 开箱即用，零用户配置

```bash
wecode bootstrap --weixin   # 一条命令，从安装到可用
```

不需要手动配置 API Key、不需要 Docker、不需要搭建服务器。bootstrap 自动完成：
- 私有 OpenClaw 运行时安装
- Gateway 配置和启动
- Codex 后端注册
- 聊天通道连接和登录

整个过程只需要扫一次码。

### 优雅的权限控制

wecode 不是"全权代理"。它有三层安全边界：

1. **Sandbox 隔离** — 默认 `workspace-write`，Codex 只能修改项目目录内的文件
2. **聊天审批** — 高风险命令需要你在微信里明确回复 `yes` 才会执行
3. **Yolo 模式可控** — 需要全自动时按项目开启，不影响其他项目

```text
你: :deploy production
wecode: ⚠️ Command `deploy` requires approval.
        Approve: yes | Deny: no
你: yes
wecode: ✅ Executing...
```

### 真正的本地执行

不是把代码发到云端处理，Codex 就跑在你自己的机器上：
- 完整的仓库访问：git 历史、分支、配置文件一个不少
- 真实的开发环境：你的 Node 版本、你的依赖、你的测试
- 私密安全：代码不离开你的电脑

### 会话从不中断

同一个微信聊天窗口 = 同一个 Codex session。上午发的任务、下午续接、晚上查看结果 — 全程同一个上下文。

```text
你: 帮我把 user 模块的错误处理重构一下
wecode: [Codex 开始工作...]
--- 两小时后 ---
你: :diff
wecode: [显示 Codex 已完成的改动]
你: :report
wecode: 已完成 3 个文件重构，测试全部通过
```

### 智能降级，永不掉线

wecode 有三层传输策略，自动选择最优路径：

```text
Codex app-server remote API (首选)
  ↓ 不可用时
codex app-server --listen stdio:// (兼容模式)
  ↓ 仍不可用时
codex exec (最终回退)
```

上游协议变化、网络波动都不会让你的工作流中断。

## 架构

```text
┌─────────────────┐
│  微信 / 飞书     │  ← 你，在手机上
└────────┬────────┘
         │
┌────────▼────────┐
│  OpenClaw Gateway│  ← 消息路由 & 会话绑定
│  port 19789     │
└────────┬────────┘
         │
┌────────▼────────┐
│  wecode          │  ← 命令解析、审批、配置
│  codex-backend   │
└────────┬────────┘
         │
┌────────▼────────┐
│  Codex CLI       │  ← 代码理解 & 执行
│  (app-server)    │
└────────┬────────┘
         │
┌────────▼────────┐
│  你的项目仓库    │  ← 真实代码，真实文件
└─────────────────┘
```

**职责边界：**

| 层 | 负责 |
|---|---|
| OpenClaw | 微信/飞书登录、消息投递、Gateway、会话绑定 |
| wecode | 本地配置、命令路由、审批队列、Codex 调用、session 扫描 |
| Codex CLI | 模型调用、工具执行、代码修改、上下文管理 |

## 支持平台

| 平台 | 状态 |
|---|---|
| macOS (ARM64 / x86_64) | 完整支持，包含 `caffeinate` 防睡眠 |
| Linux (x86_64 / ARM64) | 完整支持 |
| Windows (x86_64) | 完整支持 |

所有平台的预编译二进制文件可在 [Releases](https://github.com/zayfen/wecode/releases) 页面下载。

## 快速开始

### 前置条件

- **Node.js** 24（或 >= 22.19.0）
- **Codex CLI** 已安装并登录
- 一个可用的微信或飞书账号

### 安装

从 [Releases](https://github.com/zayfen/wecode/releases) 下载预编译二进制，或从源码构建：

```bash
cargo install --path .
```

### 初始化

```bash
# 微信通道
wecode bootstrap --weixin

# 飞书通道
wecode bootstrap --feishu

# 预览模式（不实际执行）
wecode bootstrap --dry-run --weixin
```

### 发送第一条消息

给已连接的微信/飞书账号发任意消息。OpenClaw 会把消息路由到 wecode，wecode 调用 Codex 在你配置的项目目录中执行，结果返回到聊天窗口。

## 使用方式

聊天命令使用 `:` 前缀，避免与 OpenClaw 的 slash 命令冲突：

### Codex 命令（转发为 `/...` prompt）

| 命令 | 说明 |
|---|---|
| `:init [描述]` | 初始化项目上下文 |
| `:compact [说明]` | 压缩对话上下文 |
| `:plan [任务]` | 创建执行计划 |
| `:goal [目标]` | 设定 Codex 目标 |
| `:agent [任务]` | 派发 agent 任务 |
| `:side [问题]` | 旁路查询，不影响主线程 |
| `:report [说明]` | 查看长任务进展 |
| `:new [prompt]` | 新开 Codex thread |

### 本地命令（wecode 直接处理）

| 命令 | 说明 |
|---|---|
| `:help` | 显示可用命令 |
| `:status` | 后端、session、审批和 git 状态 |
| `:diff` | 显示当前 git diff |
| `:pwd` | 显示 Codex 工作目录 |
| `:ls [目录]` | 列出目录文件 |
| `:cat <文件>` | 读取文件内容 |
| `:cd <目录>` | 切换 Codex 项目目录 |
| `:shell <命令>` | 在项目目录执行 shell 命令 |
| `:model [名称]` | 查看或设置 Codex 模型 |
| `:models` | 列出可用模型 |
| `:yolo [true\|false]` | 切换全自动模式 |
| `:stop` | 终止正在运行的 Codex 任务 |
| `:resume [session_id]` | 绑定到最近或指定的 session |
| `:fresh [prompt]` | 强制新开 Codex thread |

### 审批命令

| 命令 | 说明 |
|---|---|
| `:yes [id]` / `yes` | 批准待执行命令 |
| `:no [id]` / `no` | 拒绝待执行命令 |

只有一个待审批项时，直接回复 `yes` / `no` 即可。

## 配置

wecode 按以下顺序查找配置文件：

```text
$WECODE_CONFIG
$XDG_CONFIG_HOME/wecode/config.json    (Linux/macOS)
~/.config/wecode/config.json           (Linux/macOS)
%APPDATA%/wecode/config.json           (Windows)
```

最简配置示例：

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

运行 `wecode sample-config` 查看完整配置和默认值。

## 诊断

```bash
wecode doctor              # 检查 Node.js、npm、OpenClaw、Codex 可用性
wecode runtime-status      # 检查 Gateway、LaunchAgent、进程状态
```

## 开发

```bash
cargo build                # 构建
cargo test                 # 运行所有测试（118 个集成测试）
cargo run -- --help        # CLI 帮助

# Gateway 冒烟测试
node scripts/openclaw-agent-smoke.mjs
```

## 许可证

[MIT](LICENSE)
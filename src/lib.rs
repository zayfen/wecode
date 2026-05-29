use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fmt, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

const MIN_NODE_VERSION: (u64, u64, u64) = (22, 19, 0);
pub const WECODE_CLI_BACKEND_ID: &str = "wecode-codex";
const WECODE_CLI_BACKEND_MODEL: &str = "default";
const WECODE_CLI_BACKEND_ALIAS: &str = "Wecode Codex";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WecodeConfig {
    #[serde(default)]
    pub openclaw: OpenclawConfig,
    #[serde(default)]
    pub codex: CodexConfig,
    #[serde(default = "default_commands")]
    pub commands: Vec<CustomCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenclawConfig {
    #[serde(default = "default_openclaw_model")]
    pub model: String,
    #[serde(default, rename = "autoInstallOpenclaw")]
    pub auto_install_openclaw: bool,
    #[serde(default = "default_openclaw_profile")]
    pub profile: String,
    #[serde(default = "default_openclaw_runtime_dir", rename = "runtimeDir")]
    pub runtime_dir: String,
    #[serde(default = "default_openclaw_state_dir", rename = "stateDir")]
    pub state_dir: String,
    #[serde(default = "default_openclaw_config_path", rename = "configPath")]
    pub config_path: String,
    #[serde(default = "default_openclaw_workspace_dir", rename = "workspaceDir")]
    pub workspace_dir: String,
    #[serde(default = "default_gateway_port", rename = "gatewayPort")]
    pub gateway_port: u16,
    #[serde(default, rename = "nodeBinDir")]
    pub node_bin_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexConfig {
    #[serde(default = "default_codex_sandbox")]
    pub sandbox: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_codex_models")]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomCommand {
    pub name: String,
    pub prefix: String,
    pub prompt: String,
    #[serde(default, rename = "requireConfirm")]
    pub require_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedCommand {
    pub command_name: String,
    pub prompt: String,
    pub require_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendInput {
    Prompt(String),
    FreshPrompt(String),
    Review {
        instructions: Option<String>,
    },
    Help,
    Status,
    Diff,
    Pwd,
    Ls {
        path: String,
    },
    Cat {
        path: String,
    },
    Cd {
        path: String,
    },
    ModelShow,
    ModelsList,
    ModelSet {
        model: String,
    },
    ResumeList,
    ResumeBind {
        session_id: String,
    },
    Approve {
        approval_id: String,
    },
    Deny {
        approval_id: String,
    },
    ApprovalRequired {
        command_name: String,
        prompt: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSessionSummary {
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    pub originator: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStep {
    pub program: String,
    pub args: Vec<String>,
    pub path_prepend: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl CommandStep {
    pub fn new<const N: usize>(program: &str, args: [&str; N]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            path_prepend: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn from_vec(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            path_prepend: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn with_path_prepend(mut self, path: impl Into<String>) -> Self {
        self.path_prepend.push(path.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env.extend(
            envs.into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    pub fn display_shell(&self) -> String {
        let mut parts = Vec::new();
        parts.extend(
            self.env
                .iter()
                .map(|(key, value)| format!("{key}={}", shell_quote(value))),
        );
        if !self.path_prepend.is_empty() {
            parts.push(format!(
                "PATH={}:$PATH",
                self.path_prepend
                    .iter()
                    .map(|path| shell_quote(path))
                    .collect::<Vec<_>>()
                    .join(":")
            ));
        }
        parts.push(shell_quote(&self.program));
        parts.extend(self.args.iter().map(|arg| shell_quote(arg)));
        parts.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSnapshot {
    pub node_version: Option<String>,
    pub npm_found: bool,
    pub npx_found: bool,
    pub openclaw_version: Option<String>,
    pub codex_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReport {
    pub ok: bool,
    pub items: Vec<ToolCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCheck {
    pub name: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Doctor,
    SampleConfig,
    ConfigValidate {
        path: Option<String>,
    },
    Bootstrap {
        config_path: Option<String>,
        dry_run: bool,
        install_openclaw: bool,
    },
    InstallWeixin,
    ConfigureCodex {
        config_path: Option<String>,
    },
    Codex {
        config_path: Option<String>,
        prompt: String,
    },
    CodexBackend {
        config_path: Option<String>,
        jsonl: bool,
        model: Option<String>,
        cwd: Option<String>,
        prompt: Option<String>,
        resume_session_id: Option<String>,
    },
    Render {
        config_path: Option<String>,
        input: String,
    },
}

pub fn default_config() -> WecodeConfig {
    WecodeConfig {
        openclaw: OpenclawConfig::default(),
        codex: CodexConfig::default(),
        commands: default_commands(),
    }
}

pub fn read_config_str(input: &str) -> Result<WecodeConfig, serde_json::Error> {
    serde_json::from_str(input)
}

pub fn render_command_input(config: &WecodeConfig, input: &str) -> Option<RenderedCommand> {
    config.commands.iter().find_map(|command| {
        input
            .strip_prefix(&command.prefix)
            .map(|message| RenderedCommand {
                command_name: command.name.clone(),
                prompt: command.prompt.replace("{{message}}", message.trim()),
                require_confirm: command.require_confirm,
            })
    })
}

pub fn prepare_backend_prompt(config: &WecodeConfig, input: &str) -> Result<String, String> {
    match render_command_input(config, input) {
        Some(rendered) if rendered.require_confirm => Err(format!(
            "command `{}` requires confirmation, which is not available in codex-backend",
            rendered.command_name
        )),
        Some(rendered) => Ok(rendered.prompt),
        None => Ok(input.to_string()),
    }
}

pub fn prepare_backend_input(config: &WecodeConfig, input: &str) -> Result<BackendInput, String> {
    match parse_control_command(input)? {
        Some(input) => Ok(input),
        None => match render_command_input(config, input) {
            Some(rendered) if rendered.require_confirm => Ok(BackendInput::ApprovalRequired {
                command_name: rendered.command_name,
                prompt: rendered.prompt,
            }),
            Some(rendered) => Ok(BackendInput::Prompt(rendered.prompt)),
            None => Ok(BackendInput::Prompt(input.to_string())),
        },
    }
}

pub fn list_codex_sessions(
    sessions_root: &Path,
    target_cwd: &Path,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }

    let target_cwd = comparable_path(target_cwd);
    let mut files = Vec::new();
    collect_session_files(sessions_root, &mut files)
        .map_err(|err| format!("failed to scan Codex sessions: {err}"))?;

    let mut sessions = files
        .iter()
        .filter_map(|path| read_codex_session(path).ok().flatten())
        .filter(|session| comparable_path(Path::new(&session.cwd)) == target_cwd)
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| right.id.cmp(&left.id))
    });
    sessions.truncate(limit);
    Ok(sessions)
}

pub fn parse_node_version(output: &str) -> Option<(u64, u64, u64)> {
    let mut numbers = Vec::new();
    let mut current = String::new();

    for ch in output.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' {
            if current.is_empty() {
                return None;
            }
            numbers.push(current.parse().ok()?);
            current.clear();
        } else if !current.is_empty() {
            numbers.push(current.parse().ok()?);
            break;
        }
    }

    if !current.is_empty() && numbers.len() < 3 {
        numbers.push(current.parse().ok()?);
    }

    match numbers.as_slice() {
        [major, minor, patch, ..] => Some((*major, *minor, *patch)),
        _ => None,
    }
}

pub fn diagnose_tools(snapshot: &ToolSnapshot) -> ToolReport {
    let mut items = Vec::new();

    match snapshot
        .node_version
        .as_deref()
        .and_then(parse_node_version)
    {
        Some(version) if version >= MIN_NODE_VERSION => items.push(ToolCheck::ok(
            "node",
            format!(
                "found {}",
                snapshot.node_version.as_deref().unwrap_or("node")
            ),
        )),
        Some(version) => items.push(ToolCheck::fail(
            "node",
            format!(
                "found {}. wecode requires Node >=22.19.0 for OpenClaw/latest",
                format_version(version)
            ),
        )),
        None => items.push(ToolCheck::fail(
            "node",
            "not found. Install Node 24 or Node >=22.19.0",
        )),
    }

    items.push(if snapshot.npm_found {
        ToolCheck::ok("npm", "found npm")
    } else {
        ToolCheck::fail("npm", "not found. Install Node with npm")
    });

    items.push(if snapshot.npx_found {
        ToolCheck::ok("npx", "found npx")
    } else {
        ToolCheck::fail("npx", "not found. Install Node with npx")
    });

    items.push(match snapshot.openclaw_version.as_deref() {
        Some(version) => ToolCheck::ok("openclaw", format!("found {version}")),
        None => ToolCheck::fail(
            "openclaw",
            "not found in wecode private runtime. Run `wecode bootstrap --install-openclaw`",
        ),
    });

    items.push(match snapshot.codex_version.as_deref() {
        Some(version) => ToolCheck::ok("codex", format!("found {version}")),
        None => ToolCheck::fail(
            "codex",
            "not found. Install and log in to Codex CLI before using local smoke tests",
        ),
    });

    ToolReport {
        ok: items.iter().all(|item| item.ok),
        items,
    }
}

pub fn bootstrap_plan(config: &WecodeConfig, install_openclaw: bool) -> Vec<CommandStep> {
    bootstrap_plan_with_backend_command(config, install_openclaw, "wecode")
}

pub fn bootstrap_plan_with_backend_command(
    config: &WecodeConfig,
    install_openclaw: bool,
    backend_command: &str,
) -> Vec<CommandStep> {
    let mut steps = Vec::new();

    if install_openclaw || config.openclaw.auto_install_openclaw {
        let mut npm_install = CommandStep::from_vec(
            "npm",
            vec![
                "install".to_string(),
                "--prefix".to_string(),
                config.openclaw.runtime_dir.clone(),
                "openclaw@latest".to_string(),
            ],
        );
        for path in openclaw_node_path_prepend(config) {
            npm_install = npm_install.with_path_prepend(path);
        }
        steps.push(npm_install);
    }

    steps.extend(codex_bridge_config_plan(config, backend_command));
    steps.push(weixin_install_step(config));
    steps.push(gateway_install_step(config));

    steps
}

pub fn codex_config_plan(config: &WecodeConfig) -> Vec<CommandStep> {
    codex_config_plan_with_backend_command(config, "wecode")
}

pub fn codex_config_plan_with_backend_command(
    config: &WecodeConfig,
    backend_command: &str,
) -> Vec<CommandStep> {
    let mut steps = codex_bridge_config_plan(config, backend_command);
    steps.push(gateway_install_step(config));
    steps
}

pub fn gateway_install_step(config: &WecodeConfig) -> CommandStep {
    openclaw_step(
        config,
        openclaw_bin_path(config),
        vec![
            "gateway".to_string(),
            "install".to_string(),
            "--force".to_string(),
            "--port".to_string(),
            config.openclaw.gateway_port.to_string(),
        ],
    )
}

pub fn weixin_install_step(config: &WecodeConfig) -> CommandStep {
    let mut step = CommandStep::new(
        "npx",
        [
            "-y",
            "@tencent-weixin/openclaw-weixin-cli@latest",
            "install",
        ],
    );
    for path in openclaw_node_path_prepend(config) {
        step = step.with_path_prepend(path);
    }
    step.with_path_prepend(openclaw_bin_dir(config))
        .with_envs(openclaw_env(config))
}

pub fn openclaw_bin_path(config: &WecodeConfig) -> String {
    format!(
        "{}/node_modules/.bin/openclaw",
        trim_trailing_slashes(&config.openclaw.runtime_dir)
    )
}

pub fn openclaw_bin_dir(config: &WecodeConfig) -> String {
    format!(
        "{}/node_modules/.bin",
        trim_trailing_slashes(&config.openclaw.runtime_dir)
    )
}

pub fn parse_cli_args<I, S>(args: I) -> Result<CliCommand, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args: Vec<String> = args.into_iter().map(Into::into).collect();
    if !args.is_empty() {
        args.remove(0);
    }

    let Some(command) = args.first().map(String::as_str) else {
        return Ok(CliCommand::Help);
    };

    match command {
        "help" | "--help" | "-h" => Ok(CliCommand::Help),
        "doctor" => Ok(CliCommand::Doctor),
        "sample-config" => Ok(CliCommand::SampleConfig),
        "install-weixin" => Ok(CliCommand::InstallWeixin),
        "config" => parse_config_command(&args[1..]),
        "bootstrap" => parse_bootstrap_command(&args[1..]),
        "configure-codex" => {
            let (config_path, rest) = parse_optional_config(&args[1..])?;
            if !rest.is_empty() {
                return Err(format!("unexpected configure-codex argument: {}", rest[0]));
            }
            Ok(CliCommand::ConfigureCodex { config_path })
        }
        "codex" | "ask" => {
            let (config_path, rest) = parse_optional_config(&args[1..])?;
            if rest.is_empty() {
                return Err("codex requires a prompt".to_string());
            }
            Ok(CliCommand::Codex {
                config_path,
                prompt: rest.join(" "),
            })
        }
        "codex-backend" => parse_codex_backend_command(&args[1..]),
        "render" => {
            let (config_path, rest) = parse_optional_config(&args[1..])?;
            if rest.is_empty() {
                return Err("render requires an input message".to_string());
            }
            Ok(CliCommand::Render {
                config_path,
                input: rest.join(" "),
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

impl Default for OpenclawConfig {
    fn default() -> Self {
        Self {
            model: default_openclaw_model(),
            auto_install_openclaw: false,
            profile: default_openclaw_profile(),
            runtime_dir: default_openclaw_runtime_dir(),
            state_dir: default_openclaw_state_dir(),
            config_path: default_openclaw_config_path(),
            workspace_dir: default_openclaw_workspace_dir(),
            gateway_port: default_gateway_port(),
            node_bin_dir: None,
        }
    }
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            sandbox: default_codex_sandbox(),
            cwd: None,
            model: None,
            models: default_codex_models(),
        }
    }
}

impl ToolCheck {
    fn ok(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            message: message.into(),
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: false,
            message: message.into(),
        }
    }
}

impl BackendInput {
    pub fn is_resume_list(&self) -> bool {
        matches!(self, BackendInput::ResumeList)
    }

    pub fn resume_session_id(&self) -> Option<&str> {
        match self {
            BackendInput::ResumeBind { session_id } => Some(session_id),
            _ => None,
        }
    }
}

impl fmt::Display for CommandStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_shell())
    }
}

fn default_openclaw_model() -> String {
    format!("{WECODE_CLI_BACKEND_ID}/{WECODE_CLI_BACKEND_MODEL}")
}

fn default_openclaw_runtime_dir() -> String {
    "~/.wecode/openclaw-runtime".to_string()
}

fn default_openclaw_profile() -> String {
    "wecode".to_string()
}

fn default_openclaw_state_dir() -> String {
    "~/.wecode/openclaw-state".to_string()
}

fn default_openclaw_config_path() -> String {
    "~/.wecode/openclaw-state/openclaw.json".to_string()
}

fn default_openclaw_workspace_dir() -> String {
    "~/.wecode/workspace".to_string()
}

fn default_gateway_port() -> u16 {
    19789
}

fn default_codex_sandbox() -> String {
    "workspace-write".to_string()
}

fn default_codex_models() -> Vec<String> {
    vec![WECODE_CLI_BACKEND_MODEL.to_string(), "gpt-5.4".to_string()]
}

pub fn codex_model_from_openclaw_model(model: &str) -> Option<String> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return None;
    }
    let backend_prefix = format!("{WECODE_CLI_BACKEND_ID}/");
    let codex_model = trimmed.strip_prefix(&backend_prefix).unwrap_or(trimmed);
    if codex_model == WECODE_CLI_BACKEND_MODEL {
        None
    } else {
        Some(codex_model.to_string())
    }
}

fn default_commands() -> Vec<CustomCommand> {
    vec![
        CustomCommand {
            name: "ask".to_string(),
            prefix: "/codex ".to_string(),
            prompt: "{{message}}".to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "explain".to_string(),
            prefix: "/explain ".to_string(),
            prompt: "Explain this code, file, error, or concept clearly: {{message}}".to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "fix".to_string(),
            prefix: "/fix ".to_string(),
            prompt: "Find and implement a focused fix for this problem. Verify the result: {{message}}"
                .to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "test".to_string(),
            prefix: "/test ".to_string(),
            prompt: "Run, add, or repair focused tests for this target: {{message}}".to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "debug".to_string(),
            prefix: "/debug ".to_string(),
            prompt: "Debug this systematically, identify the root cause, and fix it when appropriate: {{message}}"
                .to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "refactor".to_string(),
            prefix: "/refactor ".to_string(),
            prompt: "Refactor this while preserving behavior. Keep the change focused and verify it: {{message}}"
                .to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "docs".to_string(),
            prefix: "/docs ".to_string(),
            prompt: "Write or update clear project documentation for: {{message}}".to_string(),
            require_confirm: false,
        },
    ]
}

fn format_version(version: (u64, u64, u64)) -> String {
    format!("v{}.{}.{}", version.0, version.1, version.2)
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=@~".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn parse_control_command(input: &str) -> Result<Option<BackendInput>, String> {
    let trimmed = input.trim();
    if matches!(trimmed, "/help" | "/commands") {
        return Ok(Some(BackendInput::Help));
    }
    if trimmed == "/status" {
        return Ok(Some(BackendInput::Status));
    }
    if trimmed == "/diff" {
        return Ok(Some(BackendInput::Diff));
    }
    if trimmed == "/pwd" {
        return Ok(Some(BackendInput::Pwd));
    }
    if trimmed == "/ls" {
        return Ok(Some(BackendInput::Ls {
            path: ".".to_string(),
        }));
    }
    if let Some(rest) = trimmed.strip_prefix("/ls ") {
        let path = parse_path_arg(rest, "/ls")?;
        return Ok(Some(BackendInput::Ls { path }));
    }
    if let Some(rest) = trimmed.strip_prefix("/cat ") {
        let path = parse_path_arg(rest, "/cat")?;
        return Ok(Some(BackendInput::Cat { path }));
    }
    if let Some(rest) = trimmed.strip_prefix("/cd ") {
        let path = parse_path_arg(rest, "/cd")?;
        return Ok(Some(BackendInput::Cd { path }));
    }
    if trimmed == "/model" {
        return Ok(Some(BackendInput::ModelShow));
    }
    if trimmed == "/models" {
        return Ok(Some(BackendInput::ModelsList));
    }
    if let Some(rest) = trimmed.strip_prefix("/model ") {
        let model = parse_single_arg(rest, "/model")?;
        return Ok(Some(BackendInput::ModelSet { model }));
    }
    if trimmed == "/review" {
        return Ok(Some(BackendInput::Review { instructions: None }));
    }
    if let Some(rest) = trimmed.strip_prefix("/review ") {
        return Ok(Some(BackendInput::Review {
            instructions: Some(rest.trim().to_string()),
        }));
    }
    if trimmed == "/init" || trimmed.starts_with("/init ") {
        let details = trimmed.strip_prefix("/init").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(init_prompt(details))));
    }
    if trimmed == "/new" || trimmed.starts_with("/new ") {
        let details = trimmed.strip_prefix("/new").unwrap_or("").trim();
        return Ok(Some(BackendInput::FreshPrompt(new_prompt(details))));
    }
    if trimmed == "/compact" || trimmed.starts_with("/compact ") {
        let details = trimmed.strip_prefix("/compact").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(compact_prompt(details))));
    }
    if trimmed == "/plan" || trimmed.starts_with("/plan ") {
        let details = trimmed.strip_prefix("/plan").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(plan_prompt(details))));
    }
    if trimmed == "/goal" || trimmed.starts_with("/goal ") {
        let details = trimmed.strip_prefix("/goal").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(goal_prompt(details))));
    }
    if trimmed == "/agent" || trimmed.starts_with("/agent ") {
        let details = trimmed.strip_prefix("/agent").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(agent_prompt(details))));
    }
    if trimmed == "/side" || trimmed.starts_with("/side ") {
        let details = trimmed.strip_prefix("/side").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(side_prompt(details))));
    }
    if matches!(trimmed, "/resume" | "/sessions") {
        return Ok(Some(BackendInput::ResumeList));
    }

    if let Some(rest) = trimmed.strip_prefix("/approve ") {
        return parse_approval_id(rest, "/approve")
            .map(|approval_id| Some(BackendInput::Approve { approval_id }));
    }

    if let Some(rest) = trimmed.strip_prefix("/deny ") {
        return parse_approval_id(rest, "/deny")
            .map(|approval_id| Some(BackendInput::Deny { approval_id }));
    }

    let Some(rest) = trimmed.strip_prefix("/resume ") else {
        return Ok(None);
    };
    let session_id = rest.trim();
    if session_id.is_empty() {
        return Ok(Some(BackendInput::ResumeList));
    }
    if session_id.split_whitespace().count() != 1 {
        return Err("/resume expects at most one Codex session id".to_string());
    }

    Ok(Some(BackendInput::ResumeBind {
        session_id: session_id.to_string(),
    }))
}

fn parse_single_arg(input: &str, command: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() || value.split_whitespace().count() != 1 {
        return Err(format!("{command} expects one value"));
    }
    Ok(value.to_string())
}

fn parse_path_arg(input: &str, command: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() {
        return Err(format!("{command} expects a path"));
    }
    Ok(value.to_string())
}

fn init_prompt(details: &str) -> String {
    append_details(
        "Initialize Codex project instructions for this repository. Inspect the codebase and create or update AGENTS.md with concise guidance for future coding agents. Keep the change focused.",
        details,
    )
}

fn new_prompt(details: &str) -> String {
    if details.is_empty() {
        "Start a new Codex session for this project. Reply with a concise confirmation and wait for the next instruction.".to_string()
    } else {
        details.to_string()
    }
}

fn compact_prompt(details: &str) -> String {
    append_details(
        "Compact the current session context into a concise handoff summary with the current goal, important decisions, changed files, verification status, and next actions.",
        details,
    )
}

fn plan_prompt(details: &str) -> String {
    append_details(
        "Create a concrete implementation plan. Do not edit files or run long tasks unless explicitly asked after the plan.",
        details,
    )
}

fn goal_prompt(details: &str) -> String {
    if details.is_empty() {
        "Report the current active goal if one exists, then summarize the next concrete step."
            .to_string()
    } else {
        format!("Set or update the active goal to this objective, then report the resulting plan: {details}")
    }
}

fn agent_prompt(details: &str) -> String {
    append_details(
        "Use available subagents for independent work where appropriate, then synthesize the result clearly.",
        details,
    )
}

fn side_prompt(details: &str) -> String {
    append_details(
        "Handle this as a side analysis: answer without changing files unless explicitly requested, and keep it separate from the main implementation path.",
        details,
    )
}

fn append_details(base: &str, details: &str) -> String {
    if details.is_empty() {
        base.to_string()
    } else {
        format!("{base}\n\nUser request: {details}")
    }
}

fn parse_approval_id(input: &str, command: &str) -> Result<String, String> {
    let approval_id = input.trim();
    if approval_id.is_empty() || approval_id.split_whitespace().count() != 1 {
        return Err(format!("{command} expects one approval id"));
    }
    if !approval_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(format!("{command} approval id contains invalid characters"));
    }
    Ok(approval_id.to_string())
}

fn collect_session_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn read_codex_session(path: &Path) -> Result<Option<CodexSessionSummary>, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open Codex session {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut meta = None;
    let mut title = String::new();

    for (line_idx, line) in reader.lines().enumerate() {
        if line_idx >= 200 {
            break;
        }
        let line = line.map_err(|err| {
            format!(
                "failed to read Codex session line from {}: {err}",
                path.display()
            )
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if meta.is_none() {
            meta = extract_session_meta(&value);
        }
        if title.is_empty() {
            title = extract_session_title(&value);
        }
        if meta.is_some() && !title.is_empty() {
            break;
        }
    }

    let Some((id, timestamp, cwd, originator)) = meta else {
        return Ok(None);
    };

    Ok(Some(CodexSessionSummary {
        id,
        timestamp,
        cwd,
        originator,
        title,
    }))
}

fn extract_session_meta(value: &Value) -> Option<(String, String, String, String)> {
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = value.get("payload")?;
    Some((
        payload.get("id")?.as_str()?.to_string(),
        payload.get("timestamp")?.as_str()?.to_string(),
        payload.get("cwd")?.as_str()?.to_string(),
        payload
            .get("originator")
            .and_then(Value::as_str)
            .unwrap_or("codex")
            .to_string(),
    ))
}

fn extract_session_title(value: &Value) -> String {
    if value.get("type").and_then(Value::as_str) == Some("event_msg") {
        if let Some(message) = value
            .get("payload")
            .and_then(|payload| payload.get("message"))
            .and_then(Value::as_str)
        {
            return sanitize_title(message);
        }
    }

    if value.get("type").and_then(Value::as_str) == Some("response_item") {
        let Some(payload) = value.get("payload") else {
            return String::new();
        };
        if payload.get("role").and_then(Value::as_str) != Some("user") {
            return String::new();
        }
        if let Some(content) = payload.get("content").and_then(Value::as_array) {
            for item in content {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    return sanitize_title(text);
                }
            }
        }
    }

    String::new()
}

fn sanitize_title(input: &str) -> String {
    let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(80).collect()
}

fn comparable_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.canonicalize().unwrap_or(absolute)
}

fn trim_trailing_slashes(value: &str) -> &str {
    value.trim_end_matches('/')
}

fn openclaw_node_path_prepend(config: &WecodeConfig) -> Vec<String> {
    config.openclaw.node_bin_dir.iter().cloned().collect()
}

fn openclaw_env(config: &WecodeConfig) -> Vec<(String, String)> {
    vec![
        (
            "OPENCLAW_PROFILE".to_string(),
            config.openclaw.profile.clone(),
        ),
        (
            "OPENCLAW_STATE_DIR".to_string(),
            config.openclaw.state_dir.clone(),
        ),
        (
            "OPENCLAW_CONFIG_PATH".to_string(),
            config.openclaw.config_path.clone(),
        ),
    ]
}

fn openclaw_step(
    config: &WecodeConfig,
    program: impl Into<String>,
    args: Vec<String>,
) -> CommandStep {
    let mut step = CommandStep::from_vec(program, args).with_envs(openclaw_env(config));
    for path in openclaw_node_path_prepend(config) {
        step = step.with_path_prepend(path);
    }
    step
}

fn codex_bridge_config_plan(config: &WecodeConfig, backend_command: &str) -> Vec<CommandStep> {
    let openclaw = openclaw_bin_path(config);
    vec![
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "gateway.port".to_string(),
                config.openclaw.gateway_port.to_string(),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "agents.defaults.workspace".to_string(),
                effective_openclaw_workspace_dir(config),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "commands.text".to_string(),
                "false".to_string(),
                "--strict-json".to_string(),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "agents.defaults.cliBackends".to_string(),
                cli_backend_config_json(config, backend_command),
                "--strict-json".to_string(),
                "--merge".to_string(),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "agents.defaults.models".to_string(),
                model_allowlist_json(config),
                "--strict-json".to_string(),
                "--merge".to_string(),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "agents.defaults.model".to_string(),
                serde_json::to_string(&config.openclaw.model)
                    .expect("serializing model string cannot fail"),
                "--strict-json".to_string(),
            ],
        ),
    ]
}

fn cli_backend_config_json(config: &WecodeConfig, backend_command: &str) -> String {
    let project_cwd = effective_project_cwd(config);
    serde_json::json!({
        WECODE_CLI_BACKEND_ID: {
            "args": ["codex-backend", "--jsonl", "--cwd", project_cwd],
            "command": backend_command,
            "input": "stdin",
            "modelArg": "--model",
            "output": "jsonl",
            "resumeArgs": ["codex-backend", "--jsonl", "--cwd", project_cwd, "--resume", "{sessionId}"],
            "resumeOutput": "jsonl",
            "serialize": true,
            "sessionIdFields": ["thread_id"]
        }
    })
    .to_string()
}

fn effective_openclaw_workspace_dir(config: &WecodeConfig) -> String {
    canonical_path_string(PathBuf::from(expand_tilde(&config.openclaw.workspace_dir)))
}

fn effective_project_cwd(config: &WecodeConfig) -> String {
    let path = config
        .codex
        .cwd
        .as_deref()
        .map(expand_tilde)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    canonical_path_string(path)
}

fn canonical_path_string(path: PathBuf) -> String {
    let absolute = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute
        .canonicalize()
        .unwrap_or(absolute)
        .display()
        .to_string()
}

fn expand_tilde(value: &str) -> String {
    if value == "~" {
        return env::var("HOME").unwrap_or_else(|_| value.to_string());
    }

    if let Some(rest) = value.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(rest).display().to_string();
        }
    }

    value.to_string()
}

fn model_allowlist_json(config: &WecodeConfig) -> String {
    let mut models = serde_json::Map::new();
    insert_openclaw_model(&mut models, &config.openclaw.model);
    for model in &config.codex.models {
        insert_openclaw_model(&mut models, &openclaw_model_id(model));
    }
    if let Some(model) = config.codex.model.as_deref() {
        insert_openclaw_model(&mut models, &openclaw_model_id(model));
    }
    serde_json::Value::Object(models).to_string()
}

fn insert_openclaw_model(models: &mut serde_json::Map<String, serde_json::Value>, model_id: &str) {
    let model_id = model_id.trim();
    if model_id.is_empty() || models.contains_key(model_id) {
        return;
    }
    models.insert(
        model_id.to_string(),
        serde_json::json!({
            "alias": openclaw_model_alias(model_id)
        }),
    );
}

fn openclaw_model_id(model: &str) -> String {
    let model = model.trim();
    if model.contains('/') {
        model.to_string()
    } else {
        format!("{WECODE_CLI_BACKEND_ID}/{model}")
    }
}

fn openclaw_model_alias(model_id: &str) -> String {
    if model_id == default_openclaw_model() {
        WECODE_CLI_BACKEND_ALIAS.to_string()
    } else if let Some(model) = model_id.strip_prefix(&format!("{WECODE_CLI_BACKEND_ID}/")) {
        format!("{WECODE_CLI_BACKEND_ALIAS} {model}")
    } else {
        format!("{WECODE_CLI_BACKEND_ALIAS} {model_id}")
    }
}

fn parse_config_command(args: &[String]) -> Result<CliCommand, String> {
    match args.first().map(String::as_str) {
        Some("validate") => Ok(CliCommand::ConfigValidate {
            path: args.get(1).cloned(),
        }),
        Some(other) => Err(format!("unknown config command: {other}")),
        None => Err("config requires a subcommand".to_string()),
    }
}

fn parse_bootstrap_command(args: &[String]) -> Result<CliCommand, String> {
    let mut config_path = None;
    let mut dry_run = false;
    let mut install_openclaw = false;
    let mut idx = 0;

    while idx < args.len() {
        match args[idx].as_str() {
            "--dry-run" => dry_run = true,
            "--install-openclaw" => install_openclaw = true,
            "--config" => {
                idx += 1;
                config_path = Some(
                    args.get(idx)
                        .ok_or_else(|| "--config requires a path".to_string())?
                        .clone(),
                );
            }
            other => return Err(format!("unexpected bootstrap argument: {other}")),
        }
        idx += 1;
    }

    Ok(CliCommand::Bootstrap {
        config_path,
        dry_run,
        install_openclaw,
    })
}

fn parse_codex_backend_command(args: &[String]) -> Result<CliCommand, String> {
    let mut config_path = None;
    let mut jsonl = false;
    let mut model = None;
    let mut cwd = None;
    let mut resume_session_id = None;
    let mut prompt = Vec::new();
    let mut idx = 0;

    while idx < args.len() {
        match args[idx].as_str() {
            "--config" => {
                idx += 1;
                config_path = Some(
                    args.get(idx)
                        .ok_or_else(|| "--config requires a path".to_string())?
                        .clone(),
                );
            }
            "--jsonl" => jsonl = true,
            "--model" => {
                idx += 1;
                model = Some(
                    args.get(idx)
                        .ok_or_else(|| "--model requires an OpenClaw model id".to_string())?
                        .clone(),
                );
            }
            "--cwd" => {
                idx += 1;
                cwd = Some(
                    args.get(idx)
                        .ok_or_else(|| "--cwd requires a project directory".to_string())?
                        .clone(),
                );
            }
            "--resume" => {
                idx += 1;
                resume_session_id = Some(
                    args.get(idx)
                        .ok_or_else(|| "--resume requires a Codex thread id".to_string())?
                        .clone(),
                );
            }
            "--" => {
                prompt.extend(args[idx + 1..].iter().cloned());
                break;
            }
            value => prompt.push(value.to_string()),
        }
        idx += 1;
    }

    Ok(CliCommand::CodexBackend {
        config_path,
        jsonl,
        model,
        cwd,
        prompt: (!prompt.is_empty()).then(|| prompt.join(" ")),
        resume_session_id,
    })
}

fn parse_optional_config(args: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    let mut config_path = None;
    let mut rest = Vec::new();
    let mut idx = 0;

    while idx < args.len() {
        match args[idx].as_str() {
            "--config" => {
                idx += 1;
                config_path = Some(
                    args.get(idx)
                        .ok_or_else(|| "--config requires a path".to_string())?
                        .clone(),
                );
            }
            value => rest.push(value.to_string()),
        }
        idx += 1;
    }

    Ok((config_path, rest))
}

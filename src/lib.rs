use serde::{Deserialize, Serialize};
use std::fmt;

const MIN_NODE_VERSION: (u64, u64, u64) = (22, 19, 0);
const WECODE_CLI_BACKEND_ID: &str = "wecode-codex";
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexConfig {
    #[serde(default = "default_codex_sandbox")]
    pub sandbox: String,
    #[serde(default)]
    pub cwd: Option<String>,
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
        steps.push(CommandStep::from_vec(
            "npm",
            vec![
                "install".to_string(),
                "--prefix".to_string(),
                config.openclaw.runtime_dir.clone(),
                "openclaw@latest".to_string(),
            ],
        ));
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
    CommandStep::new(
        "npx",
        [
            "-y",
            "@tencent-weixin/openclaw-weixin-cli@latest",
            "install",
        ],
    )
    .with_path_prepend(openclaw_bin_dir(config))
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
        }
    }
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            sandbox: default_codex_sandbox(),
            cwd: None,
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

fn default_commands() -> Vec<CustomCommand> {
    vec![CustomCommand {
        name: "ask".to_string(),
        prefix: "/codex ".to_string(),
        prompt: "{{message}}".to_string(),
        require_confirm: false,
    }]
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

fn trim_trailing_slashes(value: &str) -> &str {
    value.trim_end_matches('/')
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
    CommandStep::from_vec(program, args).with_envs(openclaw_env(config))
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
                config.openclaw.workspace_dir.clone(),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "config".to_string(),
                "set".to_string(),
                "agents.defaults.cliBackends".to_string(),
                cli_backend_config_json(backend_command),
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

fn cli_backend_config_json(backend_command: &str) -> String {
    serde_json::json!({
        WECODE_CLI_BACKEND_ID: {
            "args": ["codex-backend", "--jsonl"],
            "command": backend_command,
            "input": "stdin",
            "output": "jsonl",
            "resumeArgs": ["codex-backend", "--jsonl", "--resume", "{sessionId}"],
            "resumeOutput": "jsonl",
            "serialize": true,
            "sessionIdFields": ["thread_id"]
        }
    })
    .to_string()
}

fn model_allowlist_json(config: &WecodeConfig) -> String {
    serde_json::json!({
        config.openclaw.model.clone(): {
            "alias": WECODE_CLI_BACKEND_ALIAS
        }
    })
    .to_string()
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

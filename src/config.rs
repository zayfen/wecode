use serde::{Deserialize, Serialize};

pub const WECODE_CLI_BACKEND_ID: &str = "wecode-codex";
pub(crate) const WECODE_CLI_BACKEND_MODEL: &str = "default";
pub(crate) const WECODE_CLI_BACKEND_ALIAS: &str = "Wecode Codex";

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
            prefix: ":codex ".to_string(),
            prompt: "{{message}}".to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "explain".to_string(),
            prefix: ":explain ".to_string(),
            prompt: "Explain this code, file, error, or concept clearly: {{message}}".to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "fix".to_string(),
            prefix: ":fix ".to_string(),
            prompt: "Find and implement a focused fix for this problem. Verify the result: {{message}}"
                .to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "test".to_string(),
            prefix: ":test ".to_string(),
            prompt: "Run, add, or repair focused tests for this target: {{message}}".to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "debug".to_string(),
            prefix: ":debug ".to_string(),
            prompt: "Debug this systematically, identify the root cause, and fix it when appropriate: {{message}}"
                .to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "refactor".to_string(),
            prefix: ":refactor ".to_string(),
            prompt: "Refactor this while preserving behavior. Keep the change focused and verify it: {{message}}"
                .to_string(),
            require_confirm: false,
        },
        CustomCommand {
            name: "docs".to_string(),
            prefix: ":docs ".to_string(),
            prompt: "Write or update clear project documentation for: {{message}}".to_string(),
            require_confirm: false,
        },
    ]
}

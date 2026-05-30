use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    cli::BootstrapChannel,
    command_step::CommandStep,
    config::{
        PreventSleepMode, WecodeConfig, WECODE_CLI_BACKEND_ALIAS, WECODE_CLI_BACKEND_ID,
        WECODE_CLI_BACKEND_MODEL,
    },
    paths::{canonical_path_string, expand_tilde, trim_trailing_slashes},
};

const CAFFEINATE_PATH: &str = "/usr/bin/caffeinate";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Weixin,
    Feishu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSpec {
    pub kind: ChannelKind,
    pub name: &'static str,
    pub requires_runtime_patch: bool,
    pub visible_install_spinner: Option<&'static str>,
}

pub fn channel_spec(channel: BootstrapChannel) -> ChannelSpec {
    match channel {
        BootstrapChannel::Weixin => ChannelSpec {
            kind: ChannelKind::Weixin,
            name: "weixin",
            requires_runtime_patch: false,
            visible_install_spinner: Some("Wecode 启动中 · 安装微信"),
        },
        BootstrapChannel::Feishu => ChannelSpec {
            kind: ChannelKind::Feishu,
            name: "feishu",
            requires_runtime_patch: false,
            visible_install_spinner: None,
        },
    }
}

pub fn bootstrap_plan(config: &WecodeConfig, channel: BootstrapChannel) -> Vec<CommandStep> {
    bootstrap_plan_with_backend_command(config, channel, "wecode")
}

pub fn bootstrap_plan_with_backend_command(
    config: &WecodeConfig,
    channel: BootstrapChannel,
    backend_command: &str,
) -> Vec<CommandStep> {
    let mut steps = Vec::new();

    steps.push(openclaw_install_step(config));
    steps.extend(codex_bridge_config_plan(config, backend_command));
    steps.extend(channel_install_steps(config, channel));
    steps.push(gateway_install_step(config));

    steps
}

pub fn openclaw_install_step(config: &WecodeConfig) -> CommandStep {
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
    npm_install
}

pub fn openclaw_runtime_patch_step(config: &WecodeConfig, backend_command: &str) -> CommandStep {
    CommandStep::from_vec(
        backend_command,
        vec![
            "patch-openclaw-runtime".to_string(),
            "--runtime-dir".to_string(),
            config.openclaw.runtime_dir.clone(),
        ],
    )
}

pub fn channel_install_steps(config: &WecodeConfig, channel: BootstrapChannel) -> Vec<CommandStep> {
    match channel {
        BootstrapChannel::Weixin => vec![weixin_install_step(config)],
        BootstrapChannel::Feishu => feishu_install_steps(config),
    }
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

pub fn gateway_launch_agent_path(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde("~/Library/LaunchAgents"))
        .join(format!("ai.openclaw.{}.plist", config.openclaw.profile))
}

pub fn prevent_sleep_program_arguments(args: &[String], mode: PreventSleepMode) -> Vec<String> {
    let stripped = strip_wecode_caffeinate_wrapper(args);
    match mode {
        PreventSleepMode::Off => stripped,
        PreventSleepMode::Ac => {
            let mut wrapped = Vec::with_capacity(stripped.len() + 2);
            wrapped.push(CAFFEINATE_PATH.to_string());
            wrapped.push("-s".to_string());
            wrapped.extend(stripped);
            wrapped
        }
    }
}

pub fn patch_gateway_launch_agent_prevent_sleep(config: &WecodeConfig) -> Result<(), String> {
    patch_gateway_launch_agent_prevent_sleep_at(
        &gateway_launch_agent_path(config),
        config.openclaw.prevent_sleep,
    )
}

pub fn patch_gateway_launch_agent_prevent_sleep_at(
    path: &Path,
    mode: PreventSleepMode,
) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    if !path.exists() {
        if mode == PreventSleepMode::Off {
            return Ok(());
        }
        return Err(format!(
            "OpenClaw LaunchAgent plist not found at {}; cannot apply openclaw.preventSleep",
            path.display()
        ));
    }

    let args = read_launch_agent_program_arguments(path)?;
    let patched = prevent_sleep_program_arguments(&args, mode);
    if patched == args {
        return Ok(());
    }
    write_launch_agent_program_arguments(path, &patched)
}

fn strip_wecode_caffeinate_wrapper(args: &[String]) -> Vec<String> {
    if args.len() >= 2 && args[0] == CAFFEINATE_PATH && args[1] == "-s" {
        args[2..].to_vec()
    } else {
        args.to_vec()
    }
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

fn read_launch_agent_program_arguments(path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "ProgramArguments", "json", "-o", "-"])
        .arg(path)
        .output()
        .map_err(|err| format!("failed to run plutil for {}: {err}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "failed to read LaunchAgent ProgramArguments from {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|err| {
        format!(
            "failed to parse LaunchAgent ProgramArguments from {}: {err}",
            path.display()
        )
    })
}

fn write_launch_agent_program_arguments(path: &Path, args: &[String]) -> Result<(), String> {
    let json = serde_json::to_string(args)
        .map_err(|err| format!("failed to serialize LaunchAgent ProgramArguments: {err}"))?;
    let output = Command::new("/usr/bin/plutil")
        .args(["-replace", "ProgramArguments", "-json", &json])
        .arg(path)
        .output()
        .map_err(|err| format!("failed to run plutil for {}: {err}", path.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to update LaunchAgent ProgramArguments in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn feishu_install_steps(config: &WecodeConfig) -> Vec<CommandStep> {
    let openclaw = openclaw_bin_path(config);
    vec![
        openclaw_step(
            config,
            &openclaw,
            vec![
                "plugins".to_string(),
                "install".to_string(),
                "@openclaw/feishu".to_string(),
                "--force".to_string(),
            ],
        ),
        openclaw_step(
            config,
            &openclaw,
            vec![
                "channels".to_string(),
                "login".to_string(),
                "--channel".to_string(),
                "feishu".to_string(),
            ],
        ),
    ]
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

pub fn patch_openclaw_text_command_routing(runtime_dir: &Path) -> Result<(), String> {
    let dist_dir = runtime_dir
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    patch_text_command_routing(&dist_dir)?;
    patch_builtin_compact_command(&dist_dir)?;
    Ok(())
}

fn patch_text_command_routing(dist_dir: &Path) -> Result<(), String> {
    const ORIGINAL: &str = r#"function shouldHandleTextCommands(params) {
	if (params.commandSource === "native") return true;
	if (params.cfg.commands?.text !== false) return true;
	return !isNativeCommandSurface(params.surface);
}"#;
    const LEGACY_PATCHED: &str = r#"function shouldHandleTextCommands(params) {
	if (params.commandSource === "native") return true;
	return params.cfg.commands?.text !== false;
}"#;
    const PATCHED: &str = r#"function shouldHandleTextCommands(params) {
	return params.cfg.commands?.text !== false;
}"#;

    let candidates = openclaw_dist_candidates(dist_dir, |name| {
        name.starts_with("commands-text-routing-") && name.ends_with(".js")
    })?;

    if candidates.is_empty() {
        return Err(format!(
            "OpenClaw commands text routing file not found in {}",
            dist_dir.display()
        ));
    }

    for path in candidates {
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if source.contains(PATCHED) {
            return Ok(());
        }
        if source.contains(ORIGINAL) {
            let patched = source.replace(ORIGINAL, PATCHED);
            fs::write(&path, patched)
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
            return Ok(());
        }
        if source.contains(LEGACY_PATCHED) {
            let patched = source.replace(LEGACY_PATCHED, PATCHED);
            fs::write(&path, patched)
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
            return Ok(());
        }
    }

    Err("unsupported OpenClaw commands text routing format; expected text command fallback was not found".to_string())
}

fn patch_builtin_compact_command(dist_dir: &Path) -> Result<(), String> {
    const ORIGINAL: &str = r#"const handleCompactCommand = async (params) => {
	if (!(params.command.commandBodyNormalized === "/compact" || params.command.commandBodyNormalized.startsWith("/compact "))) return null;"#;
    const PATCHED: &str = r#"const handleCompactCommand = async (params) => {
	if (params.cfg.commands?.text === false) return null;
	if (!(params.command.commandBodyNormalized === "/compact" || params.command.commandBodyNormalized.startsWith("/compact "))) return null;"#;

    let candidates = openclaw_dist_candidates(dist_dir, |name| {
        name.starts_with("commands-handlers.runtime-") && name.ends_with(".js")
    })?;

    if candidates.is_empty() {
        return Ok(());
    }

    for path in candidates {
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if source.contains(PATCHED) {
            return Ok(());
        }
        if source.contains(ORIGINAL) {
            let patched = source.replace(ORIGINAL, PATCHED);
            fs::write(&path, patched)
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
            return Ok(());
        }
    }

    Err("unsupported OpenClaw commands handler format; expected /compact command handler was not found".to_string())
}

fn openclaw_dist_candidates(
    dist_dir: &Path,
    predicate: impl Fn(&str) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(dist_dir).map_err(|err| {
        format!(
            "failed to read OpenClaw dist dir {}: {err}",
            dist_dir.display()
        )
    })?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(&predicate)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    Ok(candidates)
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
                "agents.defaults.timeoutSeconds".to_string(),
                config.openclaw.timeout_seconds.to_string(),
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
            "reliability": {
                "watchdog": {
                    "fresh": {
                        "noOutputTimeoutMs": config.openclaw.cli_no_output_timeout_ms
                    },
                    "resume": {
                        "noOutputTimeoutMs": config.openclaw.cli_no_output_timeout_ms
                    }
                }
            },
            "resumeArgs": ["codex-backend", "--jsonl", "--cwd", project_cwd, "--resume", "{sessionId}"],
            "resumeOutput": "jsonl",
            "serialize": true,
            "sessionIdFields": ["thread_id"]
        }
    })
    .to_string()
}

fn effective_openclaw_workspace_dir(config: &WecodeConfig) -> String {
    canonical_path_string(std::path::PathBuf::from(expand_tilde(
        &config.openclaw.workspace_dir,
    )))
}

fn effective_project_cwd(config: &WecodeConfig) -> String {
    let path = config
        .codex
        .cwd
        .as_deref()
        .map(expand_tilde)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
    canonical_path_string(path)
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
    if model_id == format!("{WECODE_CLI_BACKEND_ID}/{WECODE_CLI_BACKEND_MODEL}") {
        WECODE_CLI_BACKEND_ALIAS.to_string()
    } else if let Some(model) = model_id.strip_prefix(&format!("{WECODE_CLI_BACKEND_ID}/")) {
        format!("{WECODE_CLI_BACKEND_ALIAS} {model}")
    } else {
        format!("{WECODE_CLI_BACKEND_ALIAS} {model_id}")
    }
}

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
pub const WECODE_WEIXIN_PLUGIN_NPM_SPEC: &str = "@tencent-weixin/openclaw-weixin@2.4.4";
const WECODE_WEIXIN_CHANNEL_ID: &str = "openclaw-weixin";

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
    steps.push(openclaw_runtime_patch_step(config, backend_command));
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
            "--state-dir".to_string(),
            config.openclaw.state_dir.clone(),
        ],
    )
}

pub fn channel_install_steps(config: &WecodeConfig, channel: BootstrapChannel) -> Vec<CommandStep> {
    match channel {
        BootstrapChannel::Weixin => weixin_install_steps(config),
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
    let flag = match mode {
        PreventSleepMode::Off => return stripped,
        PreventSleepMode::Ac => "-s",
        PreventSleepMode::Always => "-i",
    };
    let mut wrapped = Vec::with_capacity(stripped.len() + 2);
    wrapped.push(CAFFEINATE_PATH.to_string());
    wrapped.push(flag.to_string());
    wrapped.extend(stripped);
    wrapped
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
    if args.len() >= 2 && args[0] == CAFFEINATE_PATH && matches!(args[1].as_str(), "-s" | "-i") {
        args[2..].to_vec()
    } else {
        args.to_vec()
    }
}

pub fn weixin_install_steps(config: &WecodeConfig) -> Vec<CommandStep> {
    let openclaw = openclaw_bin_path(config);
    vec![
        openclaw_step(
            config,
            &openclaw,
            vec![
                "plugins".to_string(),
                "install".to_string(),
                WECODE_WEIXIN_PLUGIN_NPM_SPEC.to_string(),
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
                WECODE_WEIXIN_CHANNEL_ID.to_string(),
            ],
        ),
    ]
}

pub fn weixin_install_step(config: &WecodeConfig) -> CommandStep {
    weixin_install_steps(config)
        .into_iter()
        .next()
        .expect("weixin install steps must include plugin install")
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
    let mut steps = vec![
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
    ];
    steps.extend(feishu_streaming_config_steps(config, &openclaw));
    steps
}

fn feishu_streaming_config_steps(config: &WecodeConfig, openclaw: &str) -> Vec<CommandStep> {
    vec![
        openclaw_config_set_strict_json_step(
            config,
            openclaw,
            "channels.feishu.streaming",
            "true".to_string(),
        ),
        openclaw_config_set_strict_json_step(
            config,
            openclaw,
            "channels.feishu.renderMode",
            serde_json::to_string("card").expect("serializing static string cannot fail"),
        ),
        openclaw_config_set_strict_json_step(
            config,
            openclaw,
            "channels.feishu.blockStreaming",
            "true".to_string(),
        ),
    ]
}

fn openclaw_config_set_strict_json_step(
    config: &WecodeConfig,
    openclaw: &str,
    key: &str,
    json_value: String,
) -> CommandStep {
    openclaw_step(
        config,
        openclaw,
        vec![
            "config".to_string(),
            "set".to_string(),
            key.to_string(),
            json_value,
            "--strict-json".to_string(),
        ],
    )
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

pub fn patch_openclaw_runtime(runtime_dir: &Path, state_dir: &Path) -> Result<(), String> {
    patch_openclaw_text_command_routing(runtime_dir)?;
    patch_cli_stdout_progress(runtime_dir)?;
    patch_weixin_runtime(state_dir)?;
    patch_feishu_runtime(state_dir)?;
    Ok(())
}

fn patch_cli_stdout_progress(runtime_dir: &Path) -> Result<(), String> {
    let dist_dir = runtime_dir
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    let candidates = openclaw_dist_candidates(&dist_dir, |name| {
        name.starts_with("execute.runtime-") && name.ends_with(".js")
    })?;
    if candidates.is_empty() {
        return Ok(());
    }
    for path in candidates {
        patch_cli_stdout_progress_file(&path)?;
    }
    Ok(())
}

fn patch_cli_stdout_progress_file(path: &Path) -> Result<(), String> {
    const AGENT_EVENTS_IMPORT: &str =
        r#"import { i as emitAgentEvent } from "./agent-events-BuYtWSh4.js";"#;
    const DIAGNOSTIC_IMPORT: &str =
        r#"import { a as emitTrustedDiagnosticEvent } from "./diagnostic-events-CNGydBWO.js";"#;
    const ORIGINAL_STDOUT: &str = r#"onStdout: (chunk) => {
						stdoutTail = appendCliOutputTail(stdoutTail, chunk);
						if (!stdoutParseExceeded) {
							const nextStdoutParse = appendCliOutputParseBuffer(stdoutParseBuffer, chunk);
							stdoutParseBuffer = nextStdoutParse.buffer;
							stdoutParseExceeded = nextStdoutParse.exceeded;
						}
						streamingParser?.push(chunk);
					},"#;
    const PATCHED_STDOUT: &str = r#"onStdout: (chunk) => {
						emitTrustedDiagnosticEvent({
							type: "run.progress",
							runId: params.runId,
							sessionId: params.sessionId,
							...(params.sessionKey ? { sessionKey: params.sessionKey } : {}),
							reason: "cli:stdout"
						});
						stdoutTail = appendCliOutputTail(stdoutTail, chunk);
						if (!stdoutParseExceeded) {
							const nextStdoutParse = appendCliOutputParseBuffer(stdoutParseBuffer, chunk);
							stdoutParseBuffer = nextStdoutParse.buffer;
							stdoutParseExceeded = nextStdoutParse.exceeded;
						}
						streamingParser?.push(chunk);
					},"#;

    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut patched = source.clone();
    if !patched.contains(DIAGNOSTIC_IMPORT) {
        if patched.contains(AGENT_EVENTS_IMPORT) {
            patched = patched.replace(
                AGENT_EVENTS_IMPORT,
                &format!("{AGENT_EVENTS_IMPORT}\n{DIAGNOSTIC_IMPORT}"),
            );
        } else {
            return Err(format!(
                "unsupported OpenClaw CLI runtime format; agent event import not found in {}",
                path.display()
            ));
        }
    }
    if !patched.contains(r#"reason: "cli:stdout""#) {
        let original_stdout_spaces = ORIGINAL_STDOUT.replace('\t', "    ");
        let patched_stdout_spaces = PATCHED_STDOUT.replace('\t', "    ");
        if patched.contains(ORIGINAL_STDOUT) {
            patched = patched.replace(ORIGINAL_STDOUT, PATCHED_STDOUT);
        } else if patched.contains(&original_stdout_spaces) {
            patched = patched.replace(&original_stdout_spaces, &patched_stdout_spaces);
        } else {
            return Err(format!(
                "unsupported OpenClaw CLI runtime format; stdout handler not found in {}",
                path.display()
            ));
        }
    }
    if patched != source {
        fs::write(path, patched)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
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

fn patch_weixin_runtime(state_dir: &Path) -> Result<(), String> {
    let files = files_named_under(state_dir, "process-message.js")?;
    for path in files
        .iter()
        .filter(|path| path.to_string_lossy().contains("openclaw-weixin"))
    {
        patch_weixin_process_message(path)?;
    }

    let files = files_named_under(state_dir, "monitor.js")?;
    for path in files
        .iter()
        .filter(|path| path.to_string_lossy().contains("openclaw-weixin"))
    {
        patch_weixin_monitor(path)?;
    }

    let files = files_named_under(state_dir, "lane-key.js")?;
    for path in files
        .iter()
        .filter(|path| path.to_string_lossy().contains("openclaw-weixin"))
    {
        patch_weixin_lane_key(path)?;
    }
    Ok(())
}

fn patch_weixin_process_message(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut patched = source.clone();
    if patched.contains("disableBlockStreaming: true") {
        patched = patched.replace(
            "disableBlockStreaming: true",
            "disableBlockStreaming: false",
        );
    } else if !patched.contains("disableBlockStreaming: false") {
        return Err(format!(
            "unsupported Weixin process-message format; disableBlockStreaming flag not found in {}",
            path.display()
        ));
    }
    patched = patch_weixin_native_approval_control(&patched, path)?;
    patched = patch_weixin_approval_partial_sender(&patched, path)?;
    if patched != source {
        fs::write(path, patched)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn patch_weixin_native_approval_control(source: &str, path: &Path) -> Result<String, String> {
    let mut patched = source.to_string();
    const PATH_IMPORT: &str = r#"import path from "node:path";"#;
    const FS_IMPORT: &str = r#"import fs from "node:fs";"#;
    if !patched.contains(FS_IMPORT) {
        if patched.contains(PATH_IMPORT) {
            patched = patched.replace(PATH_IMPORT, &format!("{FS_IMPORT}\n{PATH_IMPORT}"));
        } else {
            return Err(format!(
                "unsupported Weixin process-message format; path import not found in {}",
                path.display()
            ));
        }
    }

    if !patched.contains("function parseWecodeApprovalControlText") {
        const HELPER_ANCHOR: &str =
            r#"/** Extract text body from item_list (for slash command detection). */"#;
        const HELPER: &str = r#"function parseWecodeApprovalControlText(text) {
    const match = String(text ?? "").trim().match(/^(?::?(yes|approve|no|deny))(?:\s+([\w-]+))?$/i);
    if (!match) return null;
    const action = match[1].toLowerCase();
    return {
        decision: action === "yes" || action === "approve" ? "approve" : "deny",
        approvalId: match[2],
    };
}
function resolveWecodeOpenClawStateDir() {
    const configured = process.env.OPENCLAW_STATE_DIR?.trim();
    const stateDir = configured && configured.length > 0 ? configured : "~/.wecode/openclaw-state";
    if (stateDir === "~") return process.env.HOME || ".";
    if (stateDir.startsWith("~/")) return path.join(process.env.HOME || ".", stateDir.slice(2));
    return path.resolve(stateDir);
}
function wecodeNativeApprovalsDir() {
    return path.join(resolveWecodeOpenClawStateDir(), "approvals", "native");
}
function readWecodeNativeApproval(approvalId) {
    const approvalPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.json`);
    try {
        const record = JSON.parse(fs.readFileSync(approvalPath, "utf8"));
        const expiresAt = Number(record?.expires_at_millis ?? 0);
        if (expiresAt > 0 && expiresAt < Date.now()) return null;
        return { approvalId, approvalPath, record };
    }
    catch (err) {
        if (err?.code !== "ENOENT") {
            logger.warn(`wecode approval control read failed approvalId=${approvalId} err=${String(err)}`);
        }
        return null;
    }
}
function listWecodePendingNativeApprovalIds() {
    const dir = wecodeNativeApprovalsDir();
    try {
        return fs.readdirSync(dir, { withFileTypes: true })
            .filter((entry) => entry.isFile() && entry.name.endsWith(".json") && !entry.name.endsWith(".decision.json"))
            .map((entry) => entry.name.slice(0, -".json".length))
            .filter((approvalId) => Boolean(readWecodeNativeApproval(approvalId)))
            .sort();
    }
    catch (err) {
        if (err?.code !== "ENOENT") {
            logger.warn(`wecode approval control list failed dir=${dir} err=${String(err)}`);
        }
        return [];
    }
}
function resolveWecodeNativeApprovalId(requestedApprovalId) {
    const approvalId = requestedApprovalId?.trim();
    if (approvalId) {
        return readWecodeNativeApproval(approvalId)
            ? { status: "ok", approvalId }
            : { status: "not_found", approvalId };
    }
    const ids = listWecodePendingNativeApprovalIds();
    if (ids.length === 1) return { status: "ok", approvalId: ids[0] };
    if (ids.length === 0) return { status: "none" };
    return { status: "multiple", ids };
}
async function sendWecodeApprovalControlMessage(params, text) {
    try {
        await sendMessageWeixin({
            to: params.to,
            text,
            opts: {
                baseUrl: params.baseUrl,
                token: params.token,
                contextToken: params.contextToken,
                runId: params.runId,
            },
        });
    }
    catch (err) {
        logger.error(`wecode approval control ack failed err=${String(err)}`);
    }
}
async function handleWecodeNativeApprovalControl(params) {
    const approval = parseWecodeApprovalControlText(params.text);
    if (!approval) return false;
    const resolved = resolveWecodeNativeApprovalId(approval.approvalId);
    if (resolved.status === "none") return false;
    if (resolved.status === "not_found") {
        await sendWecodeApprovalControlMessage(params, `Approval ${resolved.approvalId} was not found.`);
        return true;
    }
    if (resolved.status === "multiple") {
        await sendWecodeApprovalControlMessage(params, `Multiple pending approvals: ${resolved.ids.join(", ")}. Reply :yes <id> or :no <id>.`);
        return true;
    }
    const approvalId = resolved.approvalId;
    const decisionPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.decision.json`);
    try {
        fs.mkdirSync(wecodeNativeApprovalsDir(), { recursive: true });
        fs.writeFileSync(decisionPath, JSON.stringify({
            approval_id: approvalId,
            decision: approval.decision,
            decided_at_millis: Date.now(),
        }, null, 2));
        logger.info(`wecode approval control wrote decision approvalId=${approvalId} decision=${approval.decision}`);
    }
    catch (err) {
        logger.error(`wecode approval control write failed approvalId=${approvalId} err=${String(err)}`);
        await sendWecodeApprovalControlMessage(params, `Failed to update Codex approval ${approvalId}: ${String(err)}`);
        return true;
    }
    return true;
}

"#;
        if patched.contains(HELPER_ANCHOR) {
            patched = patched.replace(HELPER_ANCHOR, &format!("{HELPER}{HELPER_ANCHOR}"));
        } else if patched.contains("function extractTextBody") {
            patched = patched.replace(
                "function extractTextBody",
                &format!("{HELPER}function extractTextBody"),
            );
        } else {
            return Err(format!(
                "unsupported Weixin process-message format; text body helper anchor not found in {}",
                path.display()
            ));
        }
    }

    patched = ensure_wecode_stop_control_helpers(&patched);
    patched = ensure_wecode_stop_control_branch_weixin(&patched);
    patched = remove_legacy_wecode_native_approval_ack(&patched);

    const CONTEXT_TOKEN_BLOCK: &str = r#"    const contextToken = getContextTokenFromMsgContext(ctx);
    if (contextToken) {
        setContextToken(deps.accountId, full.from_user_id ?? "", contextToken);
    }"#;
    const CONTEXT_TOKEN_BLOCK_WITH_APPROVAL: &str = r#"    const contextToken = getContextTokenFromMsgContext(ctx);
    if (contextToken) {
        setContextToken(deps.accountId, full.from_user_id ?? "", contextToken);
    }
    if (await handleWecodeNativeApprovalControl({
        text: finalized.Body ?? textBody,
        to: ctx.To,
        contextToken,
        baseUrl: deps.baseUrl,
        token: deps.token,
        runId: randomUUID(),
    })) {
        return;
    }"#;
    if !patched.contains("if (await handleWecodeNativeApprovalControl({") {
        if patched.contains(CONTEXT_TOKEN_BLOCK) {
            patched = patched.replace(CONTEXT_TOKEN_BLOCK, CONTEXT_TOKEN_BLOCK_WITH_APPROVAL);
        } else {
            return Err(format!(
                "unsupported Weixin process-message format; context token block not found in {}",
                path.display()
            ));
        }
    }
    Ok(patched)
}

fn patch_weixin_approval_partial_sender(source: &str, path: &Path) -> Result<String, String> {
    const LEGACY_APPROVAL_PROMPT_DETECTION: &str = r#"                            const text = String(payload?.text ?? "");
                            if (!text.includes("Codex requests permission") || !text.includes("Approval: appr-")) return;
                            const match = text.match(/Approval:\s*(appr-[\w-]+)/);
                            const approvalId = match?.[1];"#;
    const APPROVAL_PROMPT_DETECTION: &str = r#"                            const text = String(payload?.text ?? "");
                            const approvalId = text.match(/审批 ID\**:\s*`?(appr-[\w-]+)`?/)?.[1]
                                ?? text.match(/Approval:\s*(appr-[\w-]+)/)?.[1];"#;
    const ORIGINAL: &str = r#"...(replyProgressSender?.replyOptions ?? {}),
                    disableBlockStreaming: false,"#;
    const OLD_PATCHED: &str = r#"...(replyProgressSender?.replyOptions ?? {}),
                    onPartialReply: (() => {
                        const sentApprovalIds = new Set();
                        return async (payload) => {
                            const text = String(payload?.text ?? "");
                            if (!text.includes("Codex requests permission") || !text.includes("Approval: appr-")) return;
                            const match = text.match(/Approval:\s*(appr-[\w-]+)/);
                            const approvalId = match?.[1];
                            if (!approvalId || sentApprovalIds.has(approvalId)) return;
                            sentApprovalIds.add(approvalId);
                            await sendMessageWeixin({
                                to: ctx.To,
                                text,
                                opts: { baseUrl: deps.baseUrl, token: deps.token, contextToken, runId },
                            });
                        };
                    })(),
                    disableBlockStreaming: false,"#;
    if source.contains("sendWecodeApprovalPromptFromReplyPayload") {
        if source.contains(APPROVAL_PROMPT_DETECTION) {
            return Ok(source.to_string());
        }
        if source.contains(LEGACY_APPROVAL_PROMPT_DETECTION) {
            return Ok(source.replace(LEGACY_APPROVAL_PROMPT_DETECTION, APPROVAL_PROMPT_DETECTION));
        }
        return Ok(source.to_string());
    }
    const PATCHED: &str = r#"...(replyProgressSender?.replyOptions ?? {}),
                    ...(() => {
                        const wecodeBaseReplyOptions = {
                            ...replyOptions,
                            ...(replyProgressSender?.replyOptions ?? {}),
                        };
                        const wecodeSentApprovalIds = new Set();
                        const sendWecodeApprovalPromptFromReplyPayload = async (payload) => {
                            const text = String(payload?.text ?? "");
                            const approvalId = text.match(/审批 ID\**:\s*`?(appr-[\w-]+)`?/)?.[1]
                                ?? text.match(/Approval:\s*(appr-[\w-]+)/)?.[1];
                            if (!approvalId || wecodeSentApprovalIds.has(approvalId)) return;
                            wecodeSentApprovalIds.add(approvalId);
                            logger.info(`wecode approval prompt detected approvalId=${approvalId} textLen=${text.length}`);
                            try {
                                await sendMessageWeixin({
                                    to: ctx.To,
                                    text,
                                    opts: { baseUrl: deps.baseUrl, token: deps.token, contextToken, runId },
                                });
                                logger.info(`wecode approval prompt sent approvalId=${approvalId}`);
                            }
                            catch (err) {
                                wecodeSentApprovalIds.delete(approvalId);
                                logger.error(`wecode approval prompt send failed approvalId=${approvalId} err=${String(err)}`);
                            }
                        };
                        const callWecodePreviousReplyOption = async (name, payload, context) => {
                            const previous = wecodeBaseReplyOptions?.[name];
                            if (typeof previous !== "function") return;
                            try {
                                await previous(payload, context);
                            }
                            catch (err) {
                                logger.warn(`wecode approval prompt previous ${name} failed err=${String(err)}`);
                            }
                        };
                        return {
                            onPartialReply: async (payload) => {
                                await callWecodePreviousReplyOption("onPartialReply", payload);
                                await sendWecodeApprovalPromptFromReplyPayload(payload);
                            },
                            onBlockReplyQueued: async (payload, context) => {
                                await callWecodePreviousReplyOption("onBlockReplyQueued", payload, context);
                                await sendWecodeApprovalPromptFromReplyPayload(payload);
                            },
                        };
                    })(),
                    disableBlockStreaming: false,"#;
    if source.contains(ORIGINAL) {
        return Ok(source.replace(ORIGINAL, PATCHED));
    }
    if source.contains(OLD_PATCHED) {
        return Ok(source.replace(OLD_PATCHED, PATCHED));
    }
    Err(format!(
        "unsupported Weixin process-message format; replyOptions block not found in {}",
        path.display()
    ))
}

fn patch_weixin_monitor(path: &Path) -> Result<(), String> {
    const PROCESS_IMPORT: &str =
        r#"import { processOneMessage } from "../messaging/process-message.js";"#;
    const PATCHED_IMPORTS: &str = r#"import { getWeixinLaneKey } from "../messaging/lane-key.js";
import { processOneMessage } from "../messaging/process-message.js";
import { createLaneScheduler } from "./lane-scheduler.js";"#;
    const CONFIG_MANAGER: &str =
        r#"    const configManager = new WeixinConfigManager({ baseUrl, token }, log);"#;
    const PATCHED_CONFIG_MANAGER: &str = r#"    const configManager = new WeixinConfigManager({ baseUrl, token }, log);
    const wecodeLaneScheduler = createLaneScheduler();"#;
    const ORIGINAL_PROCESS: &str = r#"                const fromUserId = full.from_user_id ?? "";
                const cachedConfig = await configManager.getForUser(fromUserId, full.context_token);
                await processOneMessage(full, {
                    accountId,
                    config,
                    channelRuntime,
                    baseUrl,
                    cdnBaseUrl,
                    token,
                    typingTicket: cachedConfig.typingTicket,
                    log: opts.runtime?.log ?? (() => { }),
                    errLog,
                });"#;
    const PATCHED_PROCESS: &str = r#"                const laneKey = getWeixinLaneKey({ accountId, msg: full });
                void wecodeLaneScheduler.enqueue(laneKey, async () => {
                    const fromUserId = full.from_user_id ?? "";
                    const cachedConfig = await configManager.getForUser(fromUserId, full.context_token);
                    await processOneMessage(full, {
                        accountId,
                        config,
                        channelRuntime,
                        baseUrl,
                        cdnBaseUrl,
                        token,
                        typingTicket: cachedConfig.typingTicket,
                        log: opts.runtime?.log ?? (() => { }),
                        errLog,
                    });
                }).catch((err) => {
                    errLog(`weixin processOneMessage error lane=${laneKey}: ${String(err)}`);
                    aLog.error(`processOneMessage lane=${laneKey} error: ${String(err)}, stack=${err?.stack ?? ""}`);
                });"#;

    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut patched = source.clone();
    if !patched.contains(r#"import { createLaneScheduler } from "./lane-scheduler.js";"#) {
        if patched.contains(PROCESS_IMPORT) {
            patched = patched.replace(PROCESS_IMPORT, PATCHED_IMPORTS);
        } else {
            return Err(format!(
                "unsupported Weixin monitor format; processOneMessage import not found in {}",
                path.display()
            ));
        }
    }
    if !patched.contains("const wecodeLaneScheduler = createLaneScheduler();") {
        if patched.contains(CONFIG_MANAGER) {
            patched = patched.replace(CONFIG_MANAGER, PATCHED_CONFIG_MANAGER);
        } else {
            return Err(format!(
                "unsupported Weixin monitor format; config manager initialization not found in {}",
                path.display()
            ));
        }
    }
    if !patched.contains("wecodeLaneScheduler.enqueue") {
        if patched.contains(ORIGINAL_PROCESS) {
            patched = patched.replace(ORIGINAL_PROCESS, PATCHED_PROCESS);
        } else {
            return Err(format!(
                "unsupported Weixin monitor format; processOneMessage call not found in {}",
                path.display()
            ));
        }
    }
    if patched != source {
        fs::write(path, patched)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn patch_weixin_lane_key(path: &Path) -> Result<(), String> {
    const ORIGINAL: &str = r#"const baseKey = `wx:${ctx.accountId}:${userId}`;
    if (isWeixinAbortMessage(ctx.msg)) {
        return `${baseKey}:control`;
    }
    return baseKey;"#;
    const PATCHED: &str = r#"const baseKey = `wx:${ctx.accountId}:${userId}`;
    const textBody = extractTextBodyForLane(ctx.msg).trim();
    if (isWeixinAbortMessage(ctx.msg) || isWecodeApprovalControlText(textBody)) {
        return `${baseKey}:control`;
    }
    return baseKey;"#;
    const HELPER_ANCHOR: &str = r#"/**
 * Compute the lane key used by the per-key scheduler. Abort requests get the
 * `:control` suffix to bypass the main lane.
 *
 * Note: the fallback `unknown` user happens for malformed payloads. Keep them
 * on a shared lane so they cannot fan out and exhaust resources.
 */"#;

    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let with_helper = ensure_wecode_approval_control_helper(&source, HELPER_ANCHOR)?;
    let patched = if with_helper.contains(PATCHED) {
        with_helper
    } else if with_helper.contains(ORIGINAL) {
        with_helper.replace(ORIGINAL, PATCHED)
    } else {
        return Err(format!(
            "unsupported Weixin lane-key format; control lane branch not found in {}",
            path.display()
        ));
    };
    if patched != source {
        fs::write(path, patched)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn patch_feishu_runtime(state_dir: &Path) -> Result<(), String> {
    let files = files_named_under(state_dir, "monitor.account-vUWzYlT_.js")?
        .into_iter()
        .chain(files_with_prefix_under(
            state_dir,
            "monitor.account-",
            ".js",
        )?)
        .chain(files_named_under(state_dir, "monitor.account-test.js")?)
        .collect::<Vec<_>>();

    for path in files
        .iter()
        .filter(|path| path.to_string_lossy().contains("@openclaw/feishu"))
    {
        patch_feishu_monitor_account(path)?;
    }
    Ok(())
}

fn patch_feishu_monitor_account(path: &Path) -> Result<(), String> {
    const ORIGINAL: &str = r#"if (isAbortRequestText(text)) return `${baseKey}:control`;"#;
    const PATCHED: &str = r#"if (isAbortRequestText(text) || isWecodeApprovalControlText(text)) return `${baseKey}:control`;"#;
    const HELPER_ANCHOR: &str = r#"//#region extensions/feishu/src/sequential-key.ts"#;

    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if !source.contains("function getFeishuSequentialKey") {
        return Ok(());
    }
    let with_timeout = patch_feishu_message_queue_timeout(&source, path)?;
    let with_helper = ensure_wecode_approval_control_helper(&with_timeout, HELPER_ANCHOR)?;
    let patched = if with_helper.contains(PATCHED) {
        with_helper
    } else if with_helper.contains(ORIGINAL) {
        with_helper.replace(ORIGINAL, PATCHED)
    } else {
        return Err(format!(
            "unsupported Feishu monitor.account format; sequential control branch not found in {}",
            path.display()
        ));
    };
    let patched = patch_feishu_native_approval_control(&patched, path)?;
    let patched = patch_feishu_approval_partial_sender(&patched, path)?;
    if patched != source {
        fs::write(path, patched)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn patch_feishu_message_queue_timeout(source: &str, path: &Path) -> Result<String, String> {
    if source.contains("wecodeFeishuTaskTimeoutMs") {
        return Ok(source.to_string());
    }
    if !source.contains("function createFeishuMessageReceiveHandler") {
        return Ok(source.to_string());
    }
    const ORIGINAL: &str = r#"	const log = runtime?.log ?? console.log;
	const error = runtime?.error ?? console.error;
	const enqueue = createSequentialQueue({ onTaskTimeout: (key, timeoutMs) => {
		log(`feishu[${accountId}]: per-chat task exceeded ${timeoutMs}ms cap (key=${key}); evicting from queue so later same-key messages can proceed (#70133)`);
	} });"#;
    const PATCHED: &str = r#"	const log = runtime?.log ?? console.log;
	const error = runtime?.error ?? console.error;
	const wecodeFeishuAgentTimeoutSeconds = Number(cfg?.agents?.defaults?.timeoutSeconds);
	const wecodeFeishuTaskTimeoutMs = Number.isFinite(wecodeFeishuAgentTimeoutSeconds) && wecodeFeishuAgentTimeoutSeconds > 0
		? Math.max(DEFAULT_TASK_TIMEOUT_MS, Math.ceil(wecodeFeishuAgentTimeoutSeconds * 1e3) + 60 * 1e3)
		: DEFAULT_TASK_TIMEOUT_MS;
	const enqueue = createSequentialQueue({ taskTimeoutMs: wecodeFeishuTaskTimeoutMs, onTaskTimeout: (key, timeoutMs) => {
		log(`feishu[${accountId}]: per-chat task exceeded ${timeoutMs}ms cap (key=${key}); evicting from queue so later same-key messages can proceed (#70133)`);
	} });"#;

    if source.contains(ORIGINAL) {
        return Ok(source.replace(ORIGINAL, PATCHED));
    }
    Err(format!(
        "unsupported Feishu monitor.account format; message queue timeout block not found in {}",
        path.display()
    ))
}

fn patch_feishu_native_approval_control(source: &str, path: &Path) -> Result<String, String> {
    let mut patched = source.to_string();
    if !patched.contains("async function handleFeishuMessage(params) {") {
        return Ok(patched);
    }
    if !patched.contains("async function handleWecodeNativeApprovalControl") {
        const HELPER_ANCHOR: &str = r#"async function handleFeishuMessage(params) {"#;
        const HELPER: &str = r#"function parseWecodeApprovalControlText(text) {
	const match = String(text ?? "").trim().match(/^(?::?(yes|approve|no|deny))(?:\s+([\w-]+))?$/i);
	if (!match) return null;
	const action = match[1].toLowerCase();
	return {
		decision: action === "yes" || action === "approve" ? "approve" : "deny",
		approvalId: match[2],
	};
}
function resolveWecodeOpenClawStateDir() {
	const configured = process.env.OPENCLAW_STATE_DIR?.trim();
	const stateDir = configured && configured.length > 0 ? configured : "~/.wecode/openclaw-state";
	if (stateDir === "~") return process.env.HOME || ".";
	if (stateDir.startsWith("~/")) return path.join(process.env.HOME || ".", stateDir.slice(2));
	return path.resolve(stateDir);
}
function wecodeNativeApprovalsDir() {
	return path.join(resolveWecodeOpenClawStateDir(), "approvals", "native");
}
function readWecodeNativeApproval(approvalId, log) {
	const approvalPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.json`);
	try {
		const record = JSON.parse(fs.readFileSync(approvalPath, "utf8"));
		const expiresAt = Number(record?.expires_at_millis ?? 0);
		if (expiresAt > 0 && expiresAt < Date.now()) return null;
		return { approvalId, approvalPath, record };
	}
	catch (err) {
		if (err?.code !== "ENOENT") {
			log(`wecode approval control read failed approvalId=${approvalId} err=${String(err)}`);
		}
		return null;
	}
}
function listWecodePendingNativeApprovalIds(log) {
	const dir = wecodeNativeApprovalsDir();
	try {
		return fs.readdirSync(dir, { withFileTypes: true })
			.filter((entry) => entry.isFile() && entry.name.endsWith(".json") && !entry.name.endsWith(".decision.json"))
			.map((entry) => entry.name.slice(0, -".json".length))
			.filter((approvalId) => Boolean(readWecodeNativeApproval(approvalId, log)))
			.sort();
	}
	catch (err) {
		if (err?.code !== "ENOENT") {
			log(`wecode approval control list failed dir=${dir} err=${String(err)}`);
		}
		return [];
	}
}
function resolveWecodeNativeApprovalId(requestedApprovalId, log) {
	const approvalId = requestedApprovalId?.trim();
	if (approvalId) {
		return readWecodeNativeApproval(approvalId, log)
			? { status: "ok", approvalId }
			: { status: "not_found", approvalId };
	}
	const ids = listWecodePendingNativeApprovalIds(log);
	if (ids.length === 1) return { status: "ok", approvalId: ids[0] };
	if (ids.length === 0) return { status: "none" };
	return { status: "multiple", ids };
}
async function sendWecodeApprovalControlMessage(params, text) {
	try {
		await sendMessageFeishu({
			cfg: params.cfg,
			to: `chat:${params.chatId}`,
			text,
			accountId: params.accountId,
		});
	}
	catch (err) {
		params.error(`wecode approval control ack failed err=${String(err)}`);
	}
}
async function handleWecodeNativeApprovalControl(params) {
	const log = typeof params.log === "function" ? params.log : console.log;
	const error = typeof params.error === "function" ? params.error : console.error;
	const approval = parseWecodeApprovalControlText(params.text);
	if (!approval) return false;
	const resolved = resolveWecodeNativeApprovalId(approval.approvalId, log);
	if (resolved.status === "none") return false;
	if (resolved.status === "not_found") {
		await sendWecodeApprovalControlMessage({ ...params, error }, `Approval ${resolved.approvalId} was not found.`);
		return true;
	}
	if (resolved.status === "multiple") {
		await sendWecodeApprovalControlMessage({ ...params, error }, `Multiple pending approvals: ${resolved.ids.join(", ")}. Reply :yes <id> or :no <id>.`);
		return true;
	}
	const approvalId = resolved.approvalId;
	const decisionPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.decision.json`);
	try {
		fs.mkdirSync(wecodeNativeApprovalsDir(), { recursive: true });
		fs.writeFileSync(decisionPath, JSON.stringify({
			approval_id: approvalId,
			decision: approval.decision,
			decided_at_millis: Date.now(),
		}, null, 2));
		log(`wecode approval control wrote decision approvalId=${approvalId} decision=${approval.decision}`);
	}
	catch (err) {
		error(`wecode approval control write failed approvalId=${approvalId} err=${String(err)}`);
		await sendWecodeApprovalControlMessage({ ...params, error }, `Failed to update Codex approval ${approvalId}: ${String(err)}`);
		return true;
	}
	return true;
}

"#;
        if patched.contains(HELPER_ANCHOR) {
            patched = patched.replace(HELPER_ANCHOR, &format!("{HELPER}{HELPER_ANCHOR}"));
        } else {
            return Err(format!(
                "unsupported Feishu monitor.account format; handleFeishuMessage anchor not found in {}",
                path.display()
            ));
        }
    }

    patched = ensure_wecode_stop_control_helpers(&patched);
    patched = ensure_wecode_stop_control_branch_feishu(&patched);
    patched = remove_legacy_wecode_native_approval_ack(&patched);

    const FEISHU_FROM_LINE: &str = r#"const feishuFrom = `feishu:${ctx.senderOpenId}`;"#;
    if !patched.contains("if (await handleWecodeNativeApprovalControl({") {
        let Some(line_start) = patched.find(FEISHU_FROM_LINE) else {
            return Err(format!(
                "unsupported Feishu monitor.account format; agent dispatch prelude not found in {}",
                path.display()
            ));
        };
        let line_indent_start = patched[..line_start]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let indent = &patched[line_indent_start..line_start];
        let branch = format!(
            r#"{indent}if (await handleWecodeNativeApprovalControl({{
{indent}	text: ctx.content,
{indent}	cfg,
{indent}	chatId: ctx.chatId,
{indent}	accountId: account.accountId,
{indent}	log,
{indent}	error,
{indent}}})) {{
{indent}	return;
{indent}}}
{indent}{FEISHU_FROM_LINE}"#
        );
        patched.replace_range(
            line_indent_start..line_start + FEISHU_FROM_LINE.len(),
            &branch,
        );
    }
    Ok(patched)
}

fn ensure_wecode_stop_control_helpers(source: &str) -> String {
    if source.contains("function parseWecodeStopControlText") {
        return source.to_string();
    }

    const HELPERS: &str = r#"function parseWecodeStopControlText(text) {
    return String(text ?? "").trim().toLowerCase() === ":stop";
}
function wecodeRunLocksDir() {
    return path.join(resolveWecodeOpenClawStateDir(), "locks");
}
function listWecodeRunLockPaths(log = () => {}) {
    const dir = wecodeRunLocksDir();
    try {
        return fs.readdirSync(dir, { withFileTypes: true })
            .filter((entry) => entry.isFile() && entry.name.startsWith("codex-run-") && entry.name.endsWith(".lock"))
            .map((entry) => path.join(dir, entry.name))
            .sort();
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop lock list failed dir=${dir} err=${String(err)}`);
        return [];
    }
}
function readWecodeRunLockPid(lockPath, log = () => {}) {
    try {
        const content = fs.readFileSync(lockPath, "utf8");
        const line = content.split(/\r?\n/).find((item) => item.startsWith("pid="));
        const pid = Number(line?.slice("pid=".length).trim());
        return Number.isInteger(pid) && pid > 0 ? pid : null;
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop lock read failed path=${lockPath} err=${String(err)}`);
        return null;
    }
}
function wecodeProcessIsAlive(pid) {
    if (!Number.isInteger(pid) || pid <= 0 || pid === process.pid) return false;
    try {
        process.kill(pid, 0);
        return true;
    }
    catch (err) {
        return err?.code === "EPERM";
    }
}
async function wecodeChildPids(pid, log = () => {}) {
    try {
        const { execFileSync } = await import("node:child_process");
        const output = execFileSync("/usr/bin/pgrep", ["-P", String(pid)], {
            encoding: "utf8",
            stdio: ["ignore", "pipe", "ignore"],
        });
        return output.split(/\r?\n/)
            .map((line) => Number(line.trim()))
            .filter((child) => Number.isInteger(child) && child > 0 && child !== process.pid);
    }
    catch (err) {
        if (err?.status !== 1) log(`wecode stop child pid lookup failed pid=${pid} err=${String(err)}`);
        return [];
    }
}
async function wecodeDescendantPids(pid, log = () => {}, seen = new Set()) {
    if (seen.has(pid)) return [];
    seen.add(pid);
    const result = [];
    for (const child of await wecodeChildPids(pid, log)) {
        result.push(...await wecodeDescendantPids(child, log, seen), child);
    }
    return result;
}
function signalWecodeProcess(pid, signal, log = () => {}) {
    if (!Number.isInteger(pid) || pid <= 0 || pid === process.pid) return;
    try {
        process.kill(pid, signal);
    }
    catch (err) {
        if (err?.code !== "ESRCH") log(`wecode stop signal failed pid=${pid} signal=${signal} err=${String(err)}`);
    }
}
async function stopWecodeProcessTree(pid, log = () => {}) {
    const descendants = await wecodeDescendantPids(pid, log);
    for (const child of descendants.slice().reverse()) signalWecodeProcess(child, "SIGTERM", log);
    signalWecodeProcess(pid, "SIGTERM", log);
    await new Promise((resolve) => setTimeout(resolve, 500));
    if (wecodeProcessIsAlive(pid)) {
        for (const child of descendants.slice().reverse()) signalWecodeProcess(child, "SIGKILL", log);
        signalWecodeProcess(pid, "SIGKILL", log);
    }
}
function removeWecodeFileIfExists(filePath, log = () => {}) {
    try {
        fs.unlinkSync(filePath);
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop file cleanup failed path=${filePath} err=${String(err)}`);
    }
}
function clearWecodeJsonFiles(dir, log = () => {}) {
    try {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            if (entry.isFile() && entry.name.endsWith(".json")) {
                removeWecodeFileIfExists(path.join(dir, entry.name), log);
            }
        }
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop approval cleanup failed dir=${dir} err=${String(err)}`);
    }
}
function clearWecodePendingApprovals(log = () => {}) {
    const approvalsDir = path.join(resolveWecodeOpenClawStateDir(), "approvals");
    clearWecodeJsonFiles(approvalsDir, log);
    clearWecodeJsonFiles(path.join(approvalsDir, "native"), log);
}
async function stopWecodeCodexRuns(log = () => {}) {
    let stopped = false;
    for (const lockPath of listWecodeRunLockPaths(log)) {
        const pid = readWecodeRunLockPid(lockPath, log);
        if (pid && wecodeProcessIsAlive(pid)) {
            await stopWecodeProcessTree(pid, log);
            stopped = true;
        }
        removeWecodeFileIfExists(lockPath, log);
    }
    return stopped;
}
"#;

    let anchors = [
        "function wecodeNativeApprovalsDir()",
        "async function sendWecodeApprovalControlMessage",
        "async function handleWecodeNativeApprovalControl",
    ];
    for anchor in anchors {
        if source.contains(anchor) {
            return source.replace(anchor, &format!("{HELPERS}{anchor}"));
        }
    }
    source.to_string()
}

fn ensure_wecode_stop_control_branch_weixin(source: &str) -> String {
    if source.contains("parseWecodeStopControlText(params.text)") {
        return source.to_string();
    }
    const HANDLE_START: &str = "async function handleWecodeNativeApprovalControl(params) {";
    const BRANCH: &str = r#"async function handleWecodeNativeApprovalControl(params) {
    if (parseWecodeStopControlText(params.text)) {
        const stopped = await stopWecodeCodexRuns();
        clearWecodePendingApprovals();
        await sendWecodeApprovalControlMessage(params, stopped ? "已停止当前任务。" : "没有正在运行的任务。");
        return true;
    }"#;
    source.replace(HANDLE_START, BRANCH)
}

fn ensure_wecode_stop_control_branch_feishu(source: &str) -> String {
    if source.contains("parseWecodeStopControlText(params.text)") {
        return source.to_string();
    }
    const LOG_BLOCK: &str = r#"	const log = typeof params.log === "function" ? params.log : console.log;
	const error = typeof params.error === "function" ? params.error : console.error;"#;
    const LOG_BLOCK_WITH_STOP: &str = r#"	const log = typeof params.log === "function" ? params.log : console.log;
	const error = typeof params.error === "function" ? params.error : console.error;
	if (parseWecodeStopControlText(params.text)) {
		const stopped = await stopWecodeCodexRuns(log);
		clearWecodePendingApprovals(log);
		await sendWecodeApprovalControlMessage({ ...params, error }, stopped ? "已停止当前任务。" : "没有正在运行的任务。");
		return true;
	}"#;
    if source.contains(LOG_BLOCK) {
        return source.replace(LOG_BLOCK, LOG_BLOCK_WITH_STOP);
    }

    const HANDLE_START: &str = "async function handleWecodeNativeApprovalControl(params) {";
    const BRANCH: &str = r#"async function handleWecodeNativeApprovalControl(params) {
	const log = typeof params.log === "function" ? params.log : console.log;
	const error = typeof params.error === "function" ? params.error : console.error;
	if (parseWecodeStopControlText(params.text)) {
		const stopped = await stopWecodeCodexRuns(log);
		clearWecodePendingApprovals(log);
		await sendWecodeApprovalControlMessage({ ...params, error }, stopped ? "已停止当前任务。" : "没有正在运行的任务。");
		return true;
	}"#;
    source.replace(HANDLE_START, BRANCH)
}

fn remove_legacy_wecode_native_approval_ack(source: &str) -> String {
    let mut patched = source.to_string();
    for legacy in [
        r#"    const action = approval.decision === "approve" ? "Approved" : "Denied";
    await sendWecodeApprovalControlMessage(params, `${action} Codex approval ${approvalId}. Codex will continue in the original turn.`);
"#,
        r#"	const action = approval.decision === "approve" ? "Approved" : "Denied";
	await sendWecodeApprovalControlMessage({ ...params, error }, `${action} Codex approval ${approvalId}. Codex will continue in the original turn.`);
"#,
    ] {
        patched = patched.replace(legacy, "");
    }
    patched
}

fn patch_feishu_approval_partial_sender(source: &str, path: &Path) -> Result<String, String> {
    const LEGACY_APPROVAL_PROMPT_DETECTION: &str = r#"					const text = String(payload?.text ?? "");
					if (!text.includes("Codex requests permission") || !text.includes("Approval: appr-")) return;
					const match = text.match(/Approval:\s*(appr-[\w-]+)/);
					const approvalId = match?.[1];"#;
    const APPROVAL_PROMPT_DETECTION: &str = r#"					const text = String(payload?.text ?? "");
					const approvalId = text.match(/审批 ID\**:\s*`?(appr-[\w-]+)`?/)?.[1]
						?? text.match(/Approval:\s*(appr-[\w-]+)/)?.[1];"#;
    if !source.contains("function createFeishuReplyDispatcher") {
        return Ok(source.to_string());
    }
    if source.contains("sendWecodeApprovalPromptFromReplyPayload") {
        if source.contains(APPROVAL_PROMPT_DETECTION) {
            return Ok(source.to_string());
        }
        if source.contains(LEGACY_APPROVAL_PROMPT_DETECTION) {
            return Ok(source.replace(LEGACY_APPROVAL_PROMPT_DETECTION, APPROVAL_PROMPT_DETECTION));
        }
        return Ok(source.to_string());
    }
    const ORIGINAL: &str = r#"			onPartialReply: streamingEnabled ? (payload) => {
				if (!payload.text) return;
				const cleaned = stripReasoningTagsFromText(payload.text, {
					mode: "strict",
					trim: "both"
				});
				if (!cleaned) return;
				queueStreamingUpdate(cleaned, {
					dedupeWithLastPartial: true,
					mode: "snapshot"
				});
			} : void 0,"#;
    const PATCHED: &str = r#"			...(() => {
				const wecodeFeishuBaseReplyOptions = {
					...replyOptions,
				};
				const wecodeFeishuSentApprovalIds = new Set();
				const sendWecodeApprovalPromptFromReplyPayload = async (payload) => {
					const text = String(payload?.text ?? "");
					const approvalId = text.match(/审批 ID\**:\s*`?(appr-[\w-]+)`?/)?.[1]
						?? text.match(/Approval:\s*(appr-[\w-]+)/)?.[1];
					if (!approvalId || wecodeFeishuSentApprovalIds.has(approvalId)) return;
					wecodeFeishuSentApprovalIds.add(approvalId);
					params.runtime.log?.(`wecode feishu approval prompt detected approvalId=${approvalId} textLen=${text.length}`);
					try {
						await sendMessageFeishu({
							cfg,
							to: chatId,
							text,
							replyToMessageId: sendReplyToMessageId,
							replyInThread: effectiveReplyInThread,
							allowTopLevelReplyFallback,
							accountId,
						});
						params.runtime.log?.(`wecode feishu approval prompt sent approvalId=${approvalId}`);
					}
					catch (err) {
						wecodeFeishuSentApprovalIds.delete(approvalId);
						params.runtime.error?.(`wecode feishu approval prompt send failed approvalId=${approvalId} err=${String(err)}`);
					}
				};
				const callWecodeFeishuPreviousReplyOption = async (name, payload, context) => {
					const previous = wecodeFeishuBaseReplyOptions?.[name];
					if (typeof previous !== "function") return;
					try {
						await previous(payload, context);
					}
					catch (err) {
						params.runtime.log?.(`wecode feishu approval prompt previous ${name} failed err=${String(err)}`);
					}
				};
				return {
					onPartialReply: async (payload) => {
						await callWecodeFeishuPreviousReplyOption("onPartialReply", payload);
						if (streamingEnabled) {
							if (payload.text) {
								const cleaned = stripReasoningTagsFromText(payload.text, {
									mode: "strict",
									trim: "both"
								});
								if (cleaned) {
									queueStreamingUpdate(cleaned, {
										dedupeWithLastPartial: true,
										mode: "snapshot"
									});
								}
							}
						}
						await sendWecodeApprovalPromptFromReplyPayload(payload);
					},
					onBlockReplyQueued: async (payload, context) => {
						await callWecodeFeishuPreviousReplyOption("onBlockReplyQueued", payload, context);
						await sendWecodeApprovalPromptFromReplyPayload(payload);
					},
				};
			})(),"#;

    if source.contains(ORIGINAL) {
        return Ok(source.replace(ORIGINAL, PATCHED));
    }
    Err(format!(
        "unsupported Feishu monitor.account format; replyOptions partial hook not found in {}",
        path.display()
    ))
}

fn ensure_wecode_approval_control_helper(source: &str, anchor: &str) -> Result<String, String> {
    if source.contains("function isWecodeApprovalControlText") {
        if source.contains(r#"normalized === ":stop""#) {
            return Ok(source.to_string());
        }
        let original = r#"	const normalized = String(text ?? "").trim().toLowerCase();
	return /^(?::?(?:yes|no|approve|deny))(?:\s+[\w-]+)?$/.test(normalized);"#;
        let patched = r#"	const normalized = String(text ?? "").trim().toLowerCase();
	if (normalized === ":stop") return true;
	return /^(?::?(?:yes|no|approve|deny))(?:\s+[\w-]+)?$/.test(normalized);"#;
        if source.contains(original) {
            return Ok(source.replace(original, patched));
        }
        return Ok(source.to_string());
    }
    let helper = r#"function isWecodeApprovalControlText(text) {
	const normalized = String(text ?? "").trim().toLowerCase();
	if (normalized === ":stop") return true;
	return /^(?::?(?:yes|no|approve|deny))(?:\s+[\w-]+)?$/.test(normalized);
}

"#;
    if source.contains(anchor) {
        Ok(source.replace(anchor, &format!("{helper}{anchor}")))
    } else {
        Ok(format!("{helper}{source}"))
    }
}

fn files_named_under(root: &Path, file_name: &str) -> Result<Vec<PathBuf>, String> {
    files_matching_under(root, &|name| name == file_name)
}

fn files_with_prefix_under(
    root: &Path,
    prefix: &str,
    suffix: &str,
) -> Result<Vec<PathBuf>, String> {
    files_matching_under(root, &|name| {
        name.starts_with(prefix) && name.ends_with(suffix)
    })
}

fn files_matching_under(
    root: &Path,
    predicate: &dyn Fn(&str) -> bool,
) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    collect_files_matching(root, predicate, &mut matches)?;
    matches.sort();
    matches.dedup();
    Ok(matches)
}

fn collect_files_matching(
    dir: &Path,
    predicate: &dyn Fn(&str) -> bool,
    matches: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|err| {
        format!(
            "failed to read OpenClaw runtime dir {}: {err}",
            dir.display()
        )
    })?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_files_matching(&path, predicate, matches)?;
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(predicate)
            .unwrap_or(false)
        {
            matches.push(path);
        }
    }
    Ok(())
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
            "jsonlDialect": "claude-stream-json",
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
            "serialize": false,
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

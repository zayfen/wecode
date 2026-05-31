use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use wecode::{
    backend::{AssistantBackend, BackendRunRequest, CodexBackend},
    bootstrap_plan_with_backend_command, codex_config_plan_with_backend_command,
    codex_model_from_openclaw_model,
    codex_remote::{
        run_codex_remote_turn_with_events, start_codex_remote_daemon, CodexRemoteRunEvent,
        CodexRemoteRunRequest,
    },
    config::CodexTransport,
    default_config, diagnose_tools, list_all_codex_sessions,
    native_approval::{self, NativeApprovalDecision},
    openclaw_bin_path, parse_node_version, patch_gateway_launch_agent_prevent_sleep,
    patch_openclaw_text_command_routing, prepare_backend_input_with_trace, read_config_str,
    render_command_input, run_lock, weixin_install_step, BackendInput, CliCommand, CommandStep,
    PreparedBackendInput, ToolReport, ToolSnapshot, WecodeConfig,
};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PendingApproval {
    command_name: String,
    prompt: String,
    resume_session_id: Option<String>,
    created_at_millis: u128,
}

pub fn run(command: CliCommand) -> Result<(), String> {
    match command {
        CliCommand::Help => {
            print_help();
            Ok(())
        }
        CliCommand::Doctor => {
            let (config, _) = load_config(None)?;
            let report = diagnose_tools(&probe_tools(&config));
            print_report(&report);
            if report.ok {
                Ok(())
            } else {
                Err("doctor found missing or incompatible tools".to_string())
            }
        }
        CliCommand::SampleConfig => {
            let json = serde_json::to_string_pretty(&default_config())
                .map_err(|err| format!("failed to serialize default config: {err}"))?;
            println!("{json}");
            Ok(())
        }
        CliCommand::ConfigValidate { path } => {
            let (config, source) = load_config(path)?;
            println!("valid config: {source}");
            println!(
                "model: {}, sandbox: {}, commands: {}",
                config.openclaw.model,
                config.codex.sandbox,
                config.commands.len()
            );
            Ok(())
        }
        CliCommand::Bootstrap {
            config_path,
            dry_run,
            channel,
        } => {
            let (config, _) = load_config(config_path)?;
            let backend_command = current_exe_string()?;
            let steps = bootstrap_plan_with_backend_command(&config, channel, &backend_command);

            if dry_run {
                print_steps(&config, &steps);
                return Ok(());
            }

            validate_bootstrap_prereqs(&config, true)?;
            ensure_private_openclaw_dirs(&config)?;
            let log_path = bootstrap_log_path(&config);
            run_steps_logged(&config, &steps, &log_path)?;
            patch_gateway_launch_agent_prevent_sleep(&config)?;
            maybe_start_codex_remote_daemon(&config, None)?;
            print_bootstrap_success();
            Ok(())
        }
        CliCommand::PatchOpenclawRuntime { runtime_dir } => {
            patch_openclaw_text_command_routing(Path::new(&expand_tilde(&runtime_dir)))?;
            eprintln!("patched OpenClaw command routing in {runtime_dir}");
            Ok(())
        }
        CliCommand::InstallWeixin => {
            let (config, source) = load_config(None)?;
            eprintln!("using config: {source}");
            ensure_private_openclaw_dirs(&config)?;
            run_steps(&config, &[weixin_install_step(&config)])
        }
        CliCommand::ConfigureCodex { config_path } => {
            let (config, source) = load_config(config_path)?;
            eprintln!("using config: {source}");
            ensure_private_openclaw_dirs(&config)?;
            let backend_command = current_exe_string()?;
            run_steps(
                &config,
                &codex_config_plan_with_backend_command(&config, &backend_command),
            )?;
            patch_gateway_launch_agent_prevent_sleep(&config)?;
            maybe_start_codex_remote_daemon(&config, None)
        }
        CliCommand::Codex {
            config_path,
            prompt,
        } => {
            let (mut config, source) = load_config(config_path)?;
            apply_project_cwd_override(&mut config)?;
            eprintln!("using config: {source}");
            run_codex_prompt(&config, &prompt, CodexRunMode::Interactive, None)
        }
        CliCommand::CodexBackend {
            config_path,
            jsonl,
            model,
            cwd,
            prompt,
            resume_session_id,
        } => {
            let (mut config, source) = load_config(config_path)?;
            if let Some(cwd) = cwd {
                config.codex.cwd = Some(cwd);
            }
            apply_project_cwd_override(&mut config)?;
            eprintln!("using config: {source}");
            let prompt_source = if prompt.is_some() { "argv" } else { "stdin" };
            let prompt = match prompt {
                Some(prompt) => prompt,
                None => read_stdin_prompt()?,
            };
            if prompt.trim().is_empty() {
                return Err("codex-backend requires a prompt on argv or stdin".to_string());
            }
            init_prompt_flow_logging(&config);
            let flow_run_id = prompt_flow_run_id();
            log_backend_input_received(
                &flow_run_id,
                prompt_source,
                &prompt,
                jsonl,
                &source,
                model.as_deref(),
                resume_session_id.as_deref(),
                config.codex.cwd.as_deref(),
            );
            let prepared = match prepare_backend_input_with_trace(&config, &prompt) {
                Ok(prepared) => prepared,
                Err(err) => {
                    tracing::info!(
                        run_id = %flow_run_id,
                        event = "backend_input_error",
                        error = ?err.as_str(),
                        "prompt_flow"
                    );
                    return Err(err);
                }
            };
            log_prepared_backend_input(&flow_run_id, &prepared);
            match prepared.input {
                BackendInput::Prompt(prompt) => run_backend_prompt(
                    &config,
                    &prompt,
                    jsonl,
                    model.clone(),
                    resume_session_id,
                    Some(&flow_run_id),
                ),
                BackendInput::Review { instructions } => run_codex_review(
                    &config,
                    instructions.as_deref(),
                    model.as_deref(),
                    jsonl,
                    Some(&flow_run_id),
                ),
                BackendInput::Help => emit_local_markdown(&weixin_help_message(&config), jsonl),
                BackendInput::Status => emit_local_message(
                    &weixin_status_message(&config, resume_session_id, model.as_deref()),
                    jsonl,
                ),
                BackendInput::Diff => emit_diff(&config, jsonl),
                BackendInput::Pwd => emit_pwd(&config, jsonl),
                BackendInput::Ls { path } => emit_ls(&config, &path, jsonl),
                BackendInput::Cat { path } => emit_cat(&config, &path, jsonl),
                BackendInput::Cd { path } => set_project_cwd(&mut config, &path, jsonl),
                BackendInput::Shell { command } => emit_shell(&config, &command, jsonl),
                BackendInput::ModelShow => emit_model_status(&config, model.as_deref(), jsonl),
                BackendInput::ModelsList => emit_models_list(&config, jsonl),
                BackendInput::ModelSet { model } => set_model_override(&config, &model, jsonl),
                BackendInput::Resume { session_id } => {
                    emit_resume_session(&config, session_id.as_deref(), jsonl)
                }
                BackendInput::Fresh { prompt } => run_fresh_thread_command(
                    &config,
                    prompt.as_deref(),
                    jsonl,
                    model.clone(),
                    Some(&flow_run_id),
                ),
                BackendInput::ApprovalRequired {
                    command_name,
                    prompt,
                } => {
                    emit_approval_request(&config, &command_name, &prompt, resume_session_id, jsonl)
                }
                BackendInput::Approve { approval_id } => approve_approval(
                    &config,
                    &approval_id,
                    resume_session_id,
                    model,
                    jsonl,
                    Some(&flow_run_id),
                ),
                BackendInput::Deny { approval_id } => deny_approval(&config, &approval_id, jsonl),
            }
        }
        CliCommand::Render { config_path, input } => {
            let (config, source) = load_config(config_path)?;
            eprintln!("using config: {source}");
            match render_command_input(&config, &input) {
                Some(rendered) => {
                    println!("{}", rendered.prompt);
                    if rendered.require_confirm {
                        eprintln!("command `{}` requires confirmation", rendered.command_name);
                    }
                    Ok(())
                }
                None => Err(format!("no configured command matched input: {input}")),
            }
        }
    }
}

fn validate_bootstrap_prereqs(
    config: &WecodeConfig,
    openclaw_will_be_installed: bool,
) -> Result<(), String> {
    let snapshot = probe_tools(config);
    let report = diagnose_tools(&snapshot);

    let required_ok = report.items.iter().all(|item| match item.name.as_str() {
        "openclaw" => item.ok || openclaw_will_be_installed,
        "codex" => true,
        _ => item.ok,
    });

    if required_ok {
        Ok(())
    } else {
        print_report(&report);
        Err("bootstrap prerequisites are not satisfied".to_string())
    }
}

fn run_steps(config: &WecodeConfig, steps: &[CommandStep]) -> Result<(), String> {
    run_steps_with_output(config, steps, StepOutput::Inherit)
}

fn run_steps_logged(
    config: &WecodeConfig,
    steps: &[CommandStep],
    log_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create bootstrap log dir: {err}"))?;
    }
    fs::write(log_path, "Wecode bootstrap log\n").map_err(|err| {
        format!(
            "failed to initialize bootstrap log {}: {err}",
            log_path.display()
        )
    })?;
    run_steps_with_output(config, steps, StepOutput::Bootstrap(log_path.to_path_buf()))
}

#[derive(Clone)]
enum StepOutput {
    Inherit,
    Bootstrap(PathBuf),
}

fn run_steps_with_output(
    config: &WecodeConfig,
    steps: &[CommandStep],
    output_mode: StepOutput,
) -> Result<(), String> {
    let runtime_node_bin_dir = resolve_supported_node_bin_dir(config);

    for step in steps {
        let path_prepend = runtime_path_prepend(&runtime_node_bin_dir, &step.path_prepend);
        let display_step = step_with_path_prepend(step, path_prepend.clone());
        if matches!(output_mode, StepOutput::Inherit) {
            eprintln!("$ {}", display_step.display_shell());
        }

        let program = expand_tilde(&step.program);
        let args = step
            .args
            .iter()
            .map(|arg| expand_tilde(arg))
            .collect::<Vec<_>>();
        let mut command = Command::new(&program);
        command
            .args(&args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        if !path_prepend.is_empty() {
            command.env("PATH", path_with_prepend(&path_prepend));
        }
        for (key, value) in &step.env {
            command.env(key, expand_tilde(value));
        }

        match output_mode {
            StepOutput::Inherit => {
                command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
                let status = command
                    .status()
                    .map_err(|err| format!("failed to start `{}`: {err}", step.program))?;

                if !status.success() {
                    return Err(format!("command failed: {}", display_step.display_shell()));
                }
            }
            StepOutput::Bootstrap(_) if is_visible_bootstrap_step(step) => {
                run_visible_step_with_rewritten_stdout(
                    &mut command,
                    &display_step,
                    visible_bootstrap_spinner_label(step),
                )?;
            }
            StepOutput::Bootstrap(ref log_path) => {
                run_logged_step(
                    &mut command,
                    &display_step,
                    log_path,
                    logged_bootstrap_spinner_label(step),
                )?;
            }
        }
    }

    Ok(())
}

fn run_visible_step_with_rewritten_stdout(
    command: &mut Command,
    display_step: &CommandStep,
    spinner_label: Option<&str>,
) -> Result<(), String> {
    command.stdout(Stdio::piped()).stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to start `{}`: {err}", display_step.program))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("failed to capture stdout for `{}`", display_step.program))?;
    let stdout_rewriter = thread::spawn(move || rewrite_openclaw_lines_to_stdout(stdout));

    let status = wait_for_child_with_spinner(&mut child, spinner_label)
        .map_err(|err| format!("failed to wait for `{}`: {err}", display_step.program))?;
    stdout_rewriter
        .join()
        .map_err(|_| "bootstrap stdout rewriter panicked".to_string())?
        .map_err(|err| format!("failed to rewrite bootstrap stdout: {err}"))?;

    if !status.success() {
        return Err(format!("command failed: {}", display_step.display_shell()));
    }

    Ok(())
}

fn rewrite_openclaw_lines_to_stdout<R: Read>(reader: R) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut stdout = io::stdout().lock();
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes = reader.read_until(b'\n', &mut line)?;
        if bytes == 0 {
            break;
        }
        let rewritten = replace_openclaw_case_insensitive(&String::from_utf8_lossy(&line));
        stdout.write_all(rewritten.as_bytes())?;
        stdout.flush()?;
    }

    Ok(())
}

fn replace_openclaw_case_insensitive(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(offset) = lower[cursor..].find("openclaw") {
        let start = cursor + offset;
        output.push_str(&input[cursor..start]);
        output.push_str("wecode");
        cursor = start + "openclaw".len();
    }

    output.push_str(&input[cursor..]);
    output
}

fn run_logged_step(
    command: &mut Command,
    display_step: &CommandStep,
    log_path: &Path,
    spinner_label: Option<&str>,
) -> Result<(), String> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|err| format!("failed to open bootstrap log {}: {err}", log_path.display()))?;
    writeln!(log, "\n$ {}", display_step.display_shell()).map_err(|err| {
        format!(
            "failed to write bootstrap log {}: {err}",
            log_path.display()
        )
    })?;
    let stdout = log.try_clone().map_err(|err| {
        format!(
            "failed to clone bootstrap log {}: {err}",
            log_path.display()
        )
    })?;
    let stderr = log.try_clone().map_err(|err| {
        format!(
            "failed to clone bootstrap log {}: {err}",
            log_path.display()
        )
    })?;
    command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let status = run_status_with_spinner(command, spinner_label)
        .map_err(|err| format!("failed to start `{}`: {err}", display_step.program))?;

    if !status.success() {
        return Err(format!(
            "command failed: {}\nBootstrap log: {}",
            display_step.display_shell(),
            log_path.display()
        ));
    }

    Ok(())
}

fn run_status_with_spinner(
    command: &mut Command,
    label: Option<&str>,
) -> std::io::Result<ExitStatus> {
    let mut child = command.spawn()?;
    wait_for_child_with_spinner(&mut child, label)
}

fn wait_for_child_with_spinner(
    child: &mut std::process::Child,
    label: Option<&str>,
) -> std::io::Result<ExitStatus> {
    let Some(label) = label else {
        return child.wait();
    };
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut idx = 0;
    eprint!("\r{} {label}", frames[0]);
    io::stderr().flush().ok();

    loop {
        if let Some(status) = child.try_wait()? {
            eprint!("\r\x1b[2K");
            io::stderr().flush().ok();
            return Ok(status);
        }
        eprint!("\r{} {label}", frames[idx % frames.len()]);
        io::stderr().flush().ok();
        idx += 1;
        thread::sleep(Duration::from_millis(80));
    }
}

fn is_openclaw_install_step(step: &CommandStep) -> bool {
    step.program == "npm"
        && step.args.iter().any(|arg| arg == "install")
        && step.args.iter().any(|arg| arg == "openclaw@latest")
}

fn is_feishu_plugin_install_step(step: &CommandStep) -> bool {
    step.args == ["plugins", "install", "@openclaw/feishu", "--force"]
}

fn is_weixin_install_step(step: &CommandStep) -> bool {
    step.program == "npx"
        && step
            .args
            .iter()
            .any(|arg| arg.contains("openclaw-weixin-cli"))
}

fn is_feishu_login_step(step: &CommandStep) -> bool {
    step.args == ["channels", "login", "--channel", "feishu"]
}

fn is_visible_bootstrap_step(step: &CommandStep) -> bool {
    is_weixin_install_step(step) || is_feishu_login_step(step)
}

fn visible_bootstrap_spinner_label(step: &CommandStep) -> Option<&'static str> {
    if is_weixin_install_step(step) {
        Some("Wecode 启动中 · 安装微信")
    } else {
        None
    }
}

fn logged_bootstrap_spinner_label(step: &CommandStep) -> Option<&'static str> {
    if is_openclaw_install_step(step) {
        Some("Wecode 启动中")
    } else if is_feishu_plugin_install_step(step) {
        Some("Wecode 启动中 · 安装飞书")
    } else {
        None
    }
}

fn print_bootstrap_success() {
    println!(
        "{}",
        [
            "╭────────────────────────╮",
            "│   🚀 Wecode 启动成功   │",
            "╰────────────────────────╯"
        ]
        .join("\n")
    );
}

fn bootstrap_log_path(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("bootstrap.log")
}

fn prompt_flow_log_dir(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("logs")
}

fn prompt_flow_run_id() -> String {
    format!("prompt-{}-{}", std::process::id(), current_millis())
}

fn init_prompt_flow_logging(config: &WecodeConfig) {
    if let Err(err) = try_init_prompt_flow_logging(config) {
        eprintln!("warning: failed to initialize prompt flow logging: {err}");
    }
}

fn try_init_prompt_flow_logging(config: &WecodeConfig) -> Result<(), String> {
    let log_dir = prompt_flow_log_dir(config);
    fs::create_dir_all(&log_dir).map_err(|err| {
        format!(
            "failed to create prompt flow log dir {}: {err}",
            log_dir.display()
        )
    })?;
    let write_check_path = log_dir.join("prompt-flow.write-check");
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&write_check_path)
        .map_err(|err| {
            format!(
                "prompt flow log dir is not writable {}: {err}",
                log_dir.display()
            )
        })?;
    let _ = fs::remove_file(&write_check_path);

    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("prompt-flow.log")
        .build(&log_dir)
        .map_err(|err| {
            format!(
                "failed to initialize rolling prompt flow log in {}: {err}",
                log_dir.display()
            )
        })?;
    let subscriber = tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_target(false)
        .with_level(true)
        .compact()
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|err| format!("failed to install tracing subscriber: {err}"))
}

fn log_backend_input_received(
    run_id: &str,
    prompt_source: &str,
    raw_prompt: &str,
    jsonl: bool,
    config_source: &str,
    selected_model: Option<&str>,
    resume_session_id: Option<&str>,
    configured_cwd: Option<&str>,
) {
    tracing::info!(
        run_id = %run_id,
        event = "backend_input_received",
        prompt_source = %prompt_source,
        raw_prompt = ?raw_prompt,
        jsonl,
        config_source = %config_source,
        selected_model = ?selected_model,
        resume_session_id = ?resume_session_id,
        configured_cwd = ?configured_cwd,
        "prompt_flow"
    );
}

fn log_prepared_backend_input(run_id: &str, prepared: &PreparedBackendInput) {
    match &prepared.input {
        BackendInput::Prompt(prompt) => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "prompt",
            final_prompt = ?prompt.as_str(),
            "prompt_flow"
        ),
        BackendInput::Review { instructions } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "review",
            instructions = ?instructions.as_deref(),
            "prompt_flow"
        ),
        BackendInput::Ls { path } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "ls",
            path = ?path.as_str(),
            "prompt_flow"
        ),
        BackendInput::Cat { path } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "cat",
            path = ?path.as_str(),
            "prompt_flow"
        ),
        BackendInput::Cd { path } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "cd",
            path = ?path.as_str(),
            "prompt_flow"
        ),
        BackendInput::Shell { command } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "shell",
            command = ?command.as_str(),
            "prompt_flow"
        ),
        BackendInput::ModelSet { model } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "model_set",
            model = ?model.as_str(),
            "prompt_flow"
        ),
        BackendInput::Fresh { prompt } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "fresh",
            prompt = ?prompt.as_deref(),
            "prompt_flow"
        ),
        BackendInput::Approve { approval_id } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "approve",
            approval_id = ?approval_id.as_str(),
            "prompt_flow"
        ),
        BackendInput::Deny { approval_id } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "deny",
            approval_id = ?approval_id.as_str(),
            "prompt_flow"
        ),
        BackendInput::ApprovalRequired {
            command_name,
            prompt,
        } => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = "approval_required",
            command_name = ?command_name.as_str(),
            final_prompt = ?prompt.as_str(),
            "prompt_flow"
        ),
        input => tracing::info!(
            run_id = %run_id,
            event = "backend_input_prepared",
            command_input = ?prepared.command_input.as_str(),
            backend_input_kind = backend_input_kind(input),
            "prompt_flow"
        ),
    }
}

fn backend_input_kind(input: &BackendInput) -> &'static str {
    match input {
        BackendInput::Prompt(_) => "prompt",
        BackendInput::Review { .. } => "review",
        BackendInput::Help => "help",
        BackendInput::Status => "status",
        BackendInput::Diff => "diff",
        BackendInput::Pwd => "pwd",
        BackendInput::Ls { .. } => "ls",
        BackendInput::Cat { .. } => "cat",
        BackendInput::Cd { .. } => "cd",
        BackendInput::Shell { .. } => "shell",
        BackendInput::ModelShow => "model_show",
        BackendInput::ModelsList => "models_list",
        BackendInput::ModelSet { .. } => "model_set",
        BackendInput::Resume { .. } => "resume",
        BackendInput::Fresh { .. } => "fresh",
        BackendInput::Approve { .. } => "approve",
        BackendInput::Deny { .. } => "deny",
        BackendInput::ApprovalRequired { .. } => "approval_required",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexRunMode {
    Interactive,
    Backend {
        jsonl: bool,
        model: Option<String>,
        resume_session_id: Option<String>,
    },
}

fn run_codex_prompt(
    config: &WecodeConfig,
    prompt: &str,
    mode: CodexRunMode,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    let resume_session_id = match &mode {
        CodexRunMode::Backend {
            resume_session_id, ..
        } => resume_session_id.as_deref(),
        CodexRunMode::Interactive => None,
    };
    let model = match &mode {
        CodexRunMode::Backend { model, .. } => model.as_deref(),
        CodexRunMode::Interactive => None,
    };

    let output_path = match &mode {
        CodexRunMode::Interactive => {
            let output_path = codex_output_path();
            if !run_codex_interactive_attempt(config, prompt, &output_path)? {
                return Err("codex exec failed".to_string());
            }
            output_path
        }
        CodexRunMode::Backend { jsonl, .. } => {
            if let Some(remote_result) = run_remote_backend_with_fallback(
                config,
                prompt,
                resume_session_id,
                model,
                *jsonl,
                flow_run_id,
            )? {
                if !*jsonl {
                    println!("{}", remote_result.final_message.trim());
                }
                return Ok(());
            }

            let mut output_path = codex_output_path();
            let mut result = run_codex_backend_attempt(
                config,
                prompt,
                &output_path,
                resume_session_id,
                model,
                *jsonl,
                flow_run_id,
            )?;

            if !result.success
                && resume_session_id.is_some()
                && codex_resume_rollout_is_missing(&result.stderr)
            {
                if let Some(run_id) = flow_run_id {
                    tracing::info!(
                        run_id = %run_id,
                        event = "codex_resume_fallback",
                        missing_resume_session_id = ?resume_session_id,
                        reason = "resume rollout unavailable locally",
                        "prompt_flow"
                    );
                }
                eprintln!(
                    "codex resume thread is unavailable locally; starting a fresh Codex thread"
                );
                let _ = fs::remove_file(&output_path);
                output_path = codex_output_path();
                result = run_codex_backend_attempt(
                    config,
                    prompt,
                    &output_path,
                    None,
                    model,
                    *jsonl,
                    flow_run_id,
                )?;
            }

            emit_codex_backend_result(&result, *jsonl)?;
            if !result.success {
                return Err("codex exec failed".to_string());
            }
            output_path
        }
    };

    if !matches!(mode, CodexRunMode::Backend { jsonl: true, .. }) {
        let final_message = fs::read_to_string(&output_path).map_err(|err| {
            format!(
                "failed to read Codex final message {}: {err}",
                output_path.display()
            )
        })?;
        println!("{}", final_message.trim());
    }
    let _ = fs::remove_file(output_path);
    Ok(())
}

fn run_remote_backend_with_fallback(
    config: &WecodeConfig,
    prompt: &str,
    resume_session_id: Option<&str>,
    selected_model: Option<&str>,
    jsonl: bool,
    flow_run_id: Option<&str>,
) -> Result<Option<wecode::codex_remote::CodexRemoteRunResult>, String> {
    if !jsonl || config.codex.transport == CodexTransport::Exec {
        return Ok(None);
    }

    let effective_model = effective_codex_model(config, selected_model)?;
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_remote_dispatch",
            prompt = ?prompt,
            resume_session_id = ?resume_session_id,
            selected_model = ?selected_model,
            effective_model = ?effective_model.as_deref(),
            "prompt_flow"
        );
    }

    let request = CodexRemoteRunRequest {
        config,
        prompt,
        selected_model: effective_model.as_deref(),
        resume_session_id,
    };
    let mut thread_id_emitted = false;
    match run_codex_remote_turn_with_events(&request, |event| match event {
        CodexRemoteRunEvent::AgentMessage {
            thread_id,
            text,
            final_answer,
        } => {
            if final_answer {
                return Ok(());
            }
            if !thread_id_emitted {
                emit_remote_thread_jsonl(&thread_id)?;
                thread_id_emitted = true;
            }
            emit_remote_assistant_message_jsonl(&text)
        }
        CodexRemoteRunEvent::NativeApprovalRequested {
            thread_id,
            approval_id: _,
            prompt,
        } => {
            if !thread_id_emitted {
                emit_remote_thread_jsonl(&thread_id)?;
                thread_id_emitted = true;
            }
            emit_remote_assistant_message_jsonl(&prompt)
        }
    }) {
        Ok(result) => {
            if let Some(run_id) = flow_run_id {
                tracing::info!(
                    run_id = %run_id,
                    event = "codex_remote_turn_completed",
                    thread_id = ?result.thread_id.as_str(),
                    final_message_bytes = result.final_message.len(),
                    "prompt_flow"
                );
            }
            emit_remote_backend_jsonl(&result, !thread_id_emitted)?;
            Ok(Some(result))
        }
        Err(err) if config.codex.transport == CodexTransport::Remote => {
            if let Some(run_id) = flow_run_id {
                tracing::info!(
                    run_id = %run_id,
                    event = "codex_remote_fallback_to_exec",
                    error = ?err.as_str(),
                    "prompt_flow"
                );
            }
            eprintln!("codex remote failed: {err}; falling back to codex exec");
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

fn emit_remote_backend_jsonl(
    result: &wecode::codex_remote::CodexRemoteRunResult,
    emit_thread: bool,
) -> Result<(), String> {
    if emit_thread {
        emit_remote_thread_jsonl(&result.thread_id)?;
    }
    emit_remote_assistant_message_jsonl(&result.final_message)
}

fn emit_remote_thread_jsonl(thread_id: &str) -> Result<(), String> {
    let thread = openclaw_thread_jsonl_value(thread_id);
    emit_jsonl_value(&thread)
}

fn emit_remote_assistant_message_jsonl(text: &str) -> Result<(), String> {
    let assistant_message = openclaw_assistant_message_jsonl_value(text);
    emit_jsonl_value(&assistant_message)
}

fn openclaw_thread_jsonl_value(thread_id: &str) -> serde_json::Value {
    serde_json::json!({ "thread_id": thread_id })
}

fn openclaw_assistant_message_jsonl_value(text: &str) -> serde_json::Value {
    serde_json::json!({
        "item": {
            "type": "message",
            "text": text
        }
    })
}

fn emit_jsonl_value(value: &serde_json::Value) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    write_jsonl_value(&mut stdout, value)
        .map_err(|err| format!("failed to write JSONL output: {err}"))?;
    stdout
        .flush()
        .map_err(|err| format!("failed to flush JSONL output: {err}"))
}

fn write_jsonl_value<W: Write>(writer: &mut W, value: &serde_json::Value) -> io::Result<()> {
    writeln!(writer, "{value}")
}

fn run_backend_prompt(
    config: &WecodeConfig,
    prompt: &str,
    jsonl: bool,
    model: Option<String>,
    resume_session_id: Option<String>,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    let force_fresh = project_fresh_next_path(config).exists();
    let effective_resume_session_id = if force_fresh {
        None
    } else {
        resume_session_id.as_deref()
    };
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_prompt_dispatch",
            prompt = ?prompt,
            jsonl,
            selected_model = ?model.as_deref(),
            requested_resume_session_id = ?resume_session_id.as_deref(),
            effective_resume_session_id = ?effective_resume_session_id,
            force_fresh_next = force_fresh,
            "prompt_flow"
        );
    }
    let lock_key = codex_run_lock_key(config, effective_resume_session_id);
    let _run_lock = run_lock::try_acquire_codex_run_lock(config, &lock_key)?;
    let result = run_codex_prompt(
        config,
        prompt,
        CodexRunMode::Backend {
            jsonl,
            model,
            resume_session_id: effective_resume_session_id.map(str::to_string),
        },
        flow_run_id,
    );
    if result.is_ok() && force_fresh {
        let _ = fs::remove_file(project_fresh_next_path(config));
    }
    result
}

fn codex_run_lock_key(config: &WecodeConfig, resume_session_id: Option<&str>) -> String {
    resume_session_id.map(str::to_string).unwrap_or_else(|| {
        codex_target_cwd(config)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| config.codex.cwd.clone().unwrap_or_else(|| ".".to_string()))
    })
}

fn run_fresh_thread_command(
    config: &WecodeConfig,
    prompt: Option<&str>,
    jsonl: bool,
    model: Option<String>,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    match prompt {
        Some(prompt) => {
            let lock_key = codex_run_lock_key(config, None);
            let _run_lock = run_lock::try_acquire_codex_run_lock(config, &lock_key)?;
            let result = run_codex_prompt(
                config,
                prompt,
                CodexRunMode::Backend {
                    jsonl,
                    model,
                    resume_session_id: None,
                },
                flow_run_id,
            );
            if result.is_ok() {
                let _ = fs::remove_file(project_fresh_next_path(config));
            }
            result
        }
        None => {
            mark_project_fresh_next(config)?;
            let cwd = codex_target_cwd(config)?;
            emit_local_message(
                &format!(
                    "Next Codex request will start a fresh thread for this project.\nProject: {}",
                    cwd.display()
                ),
                jsonl,
            )
        }
    }
}

fn run_codex_review(
    config: &WecodeConfig,
    instructions: Option<&str>,
    selected_model: Option<&str>,
    jsonl: bool,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    let lock_key = codex_run_lock_key(config, None);
    let _run_lock = run_lock::try_acquire_codex_run_lock(config, &lock_key)?;
    let output_path = codex_output_path();
    let mut command = Command::new("codex");
    command
        .arg("exec")
        .arg("review")
        .arg("--yolo")
        .arg("--json");
    command.arg("-o").arg(&output_path);
    command.arg("--uncommitted");
    if let Some(model) = effective_codex_model(config, selected_model)? {
        command.arg("-m").arg(model);
    }
    if let Some(instructions) = instructions {
        if !instructions.trim().is_empty() {
            command.arg("--").arg(instructions);
        }
    }
    command.current_dir(codex_target_cwd(config)?);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    eprintln!(
        "$ codex exec review --yolo --json -o {} --uncommitted",
        output_path.display()
    );
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_review_command",
            instructions = ?instructions,
            selected_model = ?selected_model,
            output_path = %output_path.display(),
            cwd = %codex_target_cwd(config)?.display(),
            "prompt_flow"
        );
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to start codex review: {err}"))?;
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_review_result",
            success = output.status.success(),
            status_code = ?output.status.code(),
            stdout_bytes = output.stdout.len(),
            stderr_bytes = output.stderr.len(),
            "prompt_flow"
        );
    }
    let result = CodexBackendResult {
        success: output.status.success(),
        status_code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        stdout_emitted: false,
        stderr_emitted: false,
    };
    emit_codex_backend_result(&result, jsonl)?;
    if !result.success {
        return Err("codex review failed".to_string());
    }
    if !jsonl {
        let final_message = fs::read_to_string(&output_path).map_err(|err| {
            format!(
                "failed to read Codex review final message {}: {err}",
                output_path.display()
            )
        })?;
        println!("{}", final_message.trim());
    }
    let _ = fs::remove_file(output_path);
    Ok(())
}

struct CodexBackendResult {
    success: bool,
    status_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_emitted: bool,
    stderr_emitted: bool,
}

fn run_codex_interactive_attempt(
    config: &WecodeConfig,
    prompt: &str,
    output_path: &Path,
) -> Result<bool, String> {
    let mut command = codex_exec_command(config, prompt, output_path, None, None, None)?;
    print_codex_exec_command(config, output_path, None, None);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|err| format!("failed to start codex: {err}"))?;
    Ok(status.success())
}

fn run_codex_backend_attempt(
    config: &WecodeConfig,
    prompt: &str,
    output_path: &Path,
    resume_session_id: Option<&str>,
    selected_model: Option<&str>,
    stream_jsonl: bool,
    flow_run_id: Option<&str>,
) -> Result<CodexBackendResult, String> {
    let mut command = codex_exec_command(
        config,
        prompt,
        output_path,
        resume_session_id,
        selected_model,
        flow_run_id,
    )?;
    print_codex_exec_command(config, output_path, resume_session_id, selected_model);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let result = if stream_jsonl {
        run_streaming_backend_command(command, output_path)?
    } else {
        let output = command
            .output()
            .map_err(|err| format!("failed to start codex: {err}"))?;
        CodexBackendResult {
            success: output.status.success(),
            status_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            stdout_emitted: false,
            stderr_emitted: false,
        }
    };
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_exec_result",
            success = result.success,
            status_code = ?result.status_code,
            stdout_bytes = result.stdout.len(),
            stderr_bytes = result.stderr.len(),
            stdout_emitted = result.stdout_emitted,
            stderr_emitted = result.stderr_emitted,
            "prompt_flow"
        );
    }
    Ok(result)
}

fn run_streaming_backend_command(
    mut command: Command,
    output_path: &Path,
) -> Result<CodexBackendResult, String> {
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to start codex: {err}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture Codex stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture Codex stderr".to_string())?;

    let stdout_thread = thread::spawn(move || capture_codex_stdout_session_jsonl(stdout));
    let stderr_thread = thread::spawn(move || capture_and_forward_stderr(stderr));
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for codex: {err}"))?;
    let stdout = join_output_thread(stdout_thread, "stdout")?;
    let stderr = join_output_thread(stderr_thread, "stderr")?;
    if status.success() {
        emit_codex_output_file_as_openclaw_jsonl(output_path)?;
    }

    Ok(CodexBackendResult {
        success: status.success(),
        status_code: status.code(),
        stdout,
        stderr,
        stdout_emitted: true,
        stderr_emitted: true,
    })
}

fn join_output_thread(
    handle: thread::JoinHandle<io::Result<Vec<u8>>>,
    name: &str,
) -> Result<Vec<u8>, String> {
    handle
        .join()
        .map_err(|_| format!("Codex {name} reader panicked"))?
        .map_err(|err| format!("failed to read Codex {name}: {err}"))
}

fn capture_and_forward_stderr<R: Read>(mut reader: R) -> io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    let mut buf = [0_u8; 8192];
    let mut writer = io::stderr().lock();
    loop {
        let len = reader.read(&mut buf)?;
        if len == 0 {
            break;
        }
        writer.write_all(&buf[..len])?;
        writer.flush()?;
        captured.extend_from_slice(&buf[..len]);
    }
    Ok(captured)
}

fn capture_codex_stdout_session_jsonl<R: Read>(reader: R) -> io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    let mut reader = BufReader::new(reader);
    let mut writer = io::stdout().lock();
    let mut line = String::new();

    loop {
        line.clear();
        let len = reader.read_line(&mut line)?;
        if len == 0 {
            break;
        }
        captured.extend_from_slice(line.as_bytes());
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(value) = codex_stdout_jsonl_line_to_openclaw_value(trimmed) {
            write_jsonl_value(&mut writer, &value)?;
        }
        writer.flush()?;
    }

    Ok(captured)
}

fn codex_stdout_jsonl_line_to_openclaw_value(line: &str) -> Option<serde_json::Value> {
    let parsed = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if parsed
        .get("item")
        .and_then(|item| item.get("text"))
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        return Some(parsed);
    }
    if let Some(thread_id) = parsed
        .get("thread_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(openclaw_thread_jsonl_value(thread_id));
    }
    None
}

fn emit_codex_output_file_as_openclaw_jsonl(output_path: &Path) -> Result<(), String> {
    let Ok(final_message) = fs::read_to_string(output_path) else {
        return Ok(());
    };
    let final_message = final_message.trim();
    if final_message.is_empty() {
        return Ok(());
    }
    emit_jsonl_value(&openclaw_assistant_message_jsonl_value(final_message))
}

fn codex_exec_command(
    config: &WecodeConfig,
    prompt: &str,
    output_path: &Path,
    resume_session_id: Option<&str>,
    selected_model: Option<&str>,
    flow_run_id: Option<&str>,
) -> Result<Command, String> {
    let effective_model = effective_codex_model(config, selected_model)?;
    let backend = CodexBackend::default();
    let request = BackendRunRequest {
        config,
        prompt,
        jsonl: true,
        selected_model: effective_model.as_deref(),
        resume_session_id,
    };
    let spec = backend.run_command_spec(&request, output_path)?;
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_exec_command",
            program = ?spec.program.as_str(),
            args = ?spec.args,
            cwd = %spec.cwd.display(),
            prompt = ?prompt,
            output_path = %output_path.display(),
            resume_session_id = ?resume_session_id,
            selected_model = ?selected_model,
            effective_model = ?effective_model.as_deref(),
            "prompt_flow"
        );
    }
    let mut command = Command::new(spec.program);
    command.current_dir(spec.cwd);
    command.args(spec.args);
    Ok(command)
}

fn print_codex_exec_command(
    config: &WecodeConfig,
    output_path: &Path,
    resume_session_id: Option<&str>,
    selected_model: Option<&str>,
) {
    let model = effective_codex_model(config, selected_model)
        .ok()
        .flatten()
        .map(|model| format!(" -m {model}"))
        .unwrap_or_default();
    if let Some(session_id) = resume_session_id {
        eprintln!(
            "$ codex exec resume --yolo --json -o {}{} {}",
            output_path.display(),
            model,
            session_id
        );
    } else {
        eprintln!(
            "$ codex exec --yolo --json -o {}{} -s {}",
            output_path.display(),
            model,
            config.codex.sandbox
        );
    }
}

fn emit_codex_backend_result(result: &CodexBackendResult, jsonl: bool) -> Result<(), String> {
    if jsonl && !result.stdout_emitted {
        io::stdout()
            .write_all(&result.stdout)
            .map_err(|err| format!("failed to write Codex JSONL stdout: {err}"))?;
    }
    if !result.stderr_emitted {
        io::stderr()
            .write_all(&result.stderr)
            .map_err(|err| format!("failed to write Codex stderr: {err}"))?;
    }
    Ok(())
}

fn emit_local_message(message: &str, _jsonl: bool) -> Result<(), String> {
    println!("{}", markdown_code_block(message, "text"));
    Ok(())
}

fn emit_local_bash_message(message: &str, _jsonl: bool) -> Result<(), String> {
    println!("{}", markdown_code_block(message, "bash"));
    Ok(())
}

fn emit_local_markdown(message: &str, _jsonl: bool) -> Result<(), String> {
    println!("{message}");
    Ok(())
}

fn markdown_code_block(message: &str, language: &str) -> String {
    let fence_len = longest_backtick_run(message).saturating_add(1).max(3);
    let fence = "`".repeat(fence_len);
    format!("{fence}{language}\n{message}\n{fence}")
}

fn longest_backtick_run(input: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in input.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn emit_diff(config: &WecodeConfig, jsonl: bool) -> Result<(), String> {
    let stat = run_git(config, &["diff", "--stat"])?;
    let diff = run_git(config, &["diff", "--", "."])?;
    let message = if stat.trim().is_empty() && diff.trim().is_empty() {
        "No unstaged git diff for this project.".to_string()
    } else {
        let mut output = String::new();
        if !stat.trim().is_empty() {
            output.push_str("Diff stat:\n");
            output.push_str(stat.trim());
            output.push_str("\n\n");
        }
        output.push_str("Diff:\n");
        output.push_str(&truncate_for_weixin(diff.trim(), 12000));
        output
    };
    emit_local_message(&message, jsonl)
}

fn emit_pwd(config: &WecodeConfig, jsonl: bool) -> Result<(), String> {
    emit_local_bash_message(&codex_target_cwd(config)?.display().to_string(), jsonl)
}

fn emit_ls(config: &WecodeConfig, path: &str, jsonl: bool) -> Result<(), String> {
    let dir = resolve_existing_project_path(config, path)?;
    if !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }

    let mut entries = fs::read_dir(&dir)
        .map_err(|err| format!("failed to list {}: {err}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to read directory entry in {}: {err}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path().display().to_string());

    let mut lines = vec![format!("Directory: {}", dir.display())];
    if entries.is_empty() {
        lines.push("(empty)".to_string());
    } else {
        for entry in entries {
            let path = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            let kind = if path.is_dir() { "[dir] " } else { "[file]" };
            lines.push(format!("{kind} {}", path.display()));
        }
    }

    emit_local_bash_message(&lines.join("\n"), jsonl)
}

fn emit_cat(config: &WecodeConfig, path: &str, jsonl: bool) -> Result<(), String> {
    let file_path = resolve_existing_project_path(config, path)?;
    if !file_path.is_file() {
        return Err(format!("{} is not a file", file_path.display()));
    }
    let bytes = fs::read(&file_path)
        .map_err(|err| format!("failed to read {}: {err}", file_path.display()))?;
    let content = String::from_utf8_lossy(&bytes);
    let content = truncate_for_weixin(&content, 20000);
    emit_local_message(
        &format!("File: {}\n\n{content}", file_path.display()),
        jsonl,
    )
}

fn set_project_cwd(config: &mut WecodeConfig, path: &str, jsonl: bool) -> Result<(), String> {
    let target = resolve_existing_project_path(config, path)?;
    if !target.is_dir() {
        return Err(format!("{} is not a directory", target.display()));
    }
    let target = target.display().to_string();
    ensure_project_state_paths_writable(
        config,
        "change Codex project directory",
        &[
            project_cwd_override_path(config),
            project_fresh_next_path(config),
        ],
    )?;
    let override_path = project_cwd_override_path(config);
    write_project_state_file(
        config,
        &override_path,
        &target,
        "change Codex project directory",
        "project cwd override",
    )?;
    let fresh_path = project_fresh_next_path(config);
    write_project_state_file(
        config,
        &fresh_path,
        "1",
        "change Codex project directory",
        "project fresh-thread marker",
    )?;
    config.codex.cwd = Some(target.clone());
    emit_local_message(
        &format!("Codex project directory set to:\n{target}\nNext Codex request will start a fresh thread for this project."),
        jsonl,
    )
}

fn emit_shell(config: &WecodeConfig, command: &str, jsonl: bool) -> Result<(), String> {
    let cwd = codex_target_cwd(config)?;
    if !cwd.is_dir() {
        return Err(format!(
            "Codex project directory does not exist: {}",
            cwd.display()
        ));
    }
    let (program, args) = shell_program_and_args(command);
    let output = Command::new(program)
        .args(args)
        .current_dir(&cwd)
        .output()
        .map_err(|err| format!("failed to run shell command: {err}"))?;
    emit_local_bash_message(&format_shell_output(command, &cwd, &output), jsonl)
}

fn emit_model_status(
    config: &WecodeConfig,
    selected_model: Option<&str>,
    jsonl: bool,
) -> Result<(), String> {
    let model = effective_codex_model(config, selected_model)?
        .unwrap_or_else(|| "(Codex default)".to_string());
    let cwd = codex_target_cwd(config)?;
    emit_local_message(
        &format!(
            "Current Codex model for this project: {model}\nProject: {}\nSet it with `:model <model>`",
            cwd.display()
        ),
        jsonl,
    )
}

fn emit_models_list(config: &WecodeConfig, jsonl: bool) -> Result<(), String> {
    let current =
        effective_codex_model(config, None)?.unwrap_or_else(|| "(Codex default)".to_string());
    let mut lines = vec![
        "Configured Codex models:".to_string(),
        format!("current: {current}"),
    ];
    for model in configured_codex_models(config) {
        lines.push(format!("- {model}"));
    }
    lines.push("Use `:model <model>` to set the model for the current project.".to_string());
    lines.push("Use `:model default` to clear the project model override.".to_string());
    emit_local_message(&lines.join("\n"), jsonl)
}

fn set_model_override(config: &WecodeConfig, model: &str, jsonl: bool) -> Result<(), String> {
    let normalized = codex_model_from_openclaw_model(model);
    let path = model_override_path(config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create model state dir: {err}"))?;
    }
    let cwd = codex_target_cwd(config)?;
    if let Some(model) = normalized {
        fs::write(&path, &model).map_err(|err| format!("failed to write model override: {err}"))?;
        emit_local_message(
            &format!(
                "Codex model set to `{model}` for this project.\nProject: {}",
                cwd.display()
            ),
            jsonl,
        )
    } else {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|err| format!("failed to clear model override: {err}"))?;
        }
        emit_local_message(
            &format!(
                "Codex model override cleared for this project.\nProject: {}",
                cwd.display()
            ),
            jsonl,
        )
    }
}

fn emit_resume_session(
    _config: &WecodeConfig,
    session_id: Option<&str>,
    jsonl: bool,
) -> Result<(), String> {
    let sessions_root = codex_sessions_root();
    let sessions = list_all_codex_sessions(&sessions_root, usize::MAX)?;
    let selected = if let Some(session_id) = session_id {
        sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .unwrap_or_else(|| wecode::CodexSessionSummary {
                id: session_id.to_string(),
                timestamp: String::new(),
                cwd: String::new(),
                originator: String::new(),
                title: String::new(),
                initial_prompt: String::new(),
            })
    } else {
        sessions.into_iter().next().ok_or_else(|| {
            "No Codex sessions found. Run Codex once first, or pass `:resume <session_id>`."
                .to_string()
        })?
    };

    emit_resume_binding(&selected, jsonl)
}

fn compact_home_path(value: &str) -> String {
    let Ok(home) = env::var("HOME") else {
        return value.to_string();
    };
    let home = Path::new(&home);
    let path = Path::new(value);
    if path == home {
        return "~".to_string();
    }
    if let Ok(rest) = path.strip_prefix(home) {
        return format!("~/{}", rest.display());
    }
    value.to_string()
}

fn emit_resume_binding(session: &wecode::CodexSessionSummary, jsonl: bool) -> Result<(), String> {
    let mut lines = vec![format!("Resumed Codex session `{}`.", session.id)];
    if !session.cwd.is_empty() {
        lines.push(format!("CWD: {}", compact_home_path(&session.cwd)));
    }
    if !session.title.is_empty() {
        lines.push(format!("Title: {}", session.title));
    }
    let message = lines.join("\n");

    if jsonl {
        emit_jsonl_value(&openclaw_thread_jsonl_value(&session.id))?;
        emit_jsonl_value(&openclaw_assistant_message_jsonl_value(&message))?;
        Ok(())
    } else {
        emit_local_markdown(&message, jsonl)
    }
}

fn emit_approval_request(
    config: &WecodeConfig,
    command_name: &str,
    prompt: &str,
    resume_session_id: Option<String>,
    jsonl: bool,
) -> Result<(), String> {
    let approval_id = create_approval_id();
    let approval = PendingApproval {
        command_name: command_name.to_string(),
        prompt: prompt.to_string(),
        resume_session_id,
        created_at_millis: current_millis(),
    };
    write_pending_approval(config, &approval_id, &approval)?;
    emit_local_message(
        &format!(
            "Command `{command_name}` requires approval.\nApprove: :approve {approval_id} or :yes {approval_id}\nDeny: :deny {approval_id} or :no {approval_id}"
        ),
        jsonl,
    )
}

fn approve_approval(
    config: &WecodeConfig,
    approval_id: &str,
    resume_session_id: Option<String>,
    selected_model: Option<String>,
    jsonl: bool,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    let custom_path = approval_path(config, approval_id);
    if custom_path.exists() {
        return run_approved_prompt(
            config,
            approval_id,
            resume_session_id,
            selected_model,
            jsonl,
            flow_run_id,
        );
    }

    match native_approval::write_native_approval_decision(
        config,
        approval_id,
        NativeApprovalDecision::Approve,
    ) {
        Ok(()) => emit_local_markdown(
            &format!(
                "Approved Codex approval {approval_id}. Codex will continue in the original turn."
            ),
            jsonl,
        ),
        Err(_) => emit_local_markdown(&format!("Approval {approval_id} was not found."), jsonl),
    }
}

fn run_approved_prompt(
    config: &WecodeConfig,
    approval_id: &str,
    resume_session_id: Option<String>,
    selected_model: Option<String>,
    jsonl: bool,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    let approval_path = approval_path(config, approval_id);
    let approval = read_pending_approval(&approval_path)?;
    let effective_resume_session_id = resume_session_id.or(approval.resume_session_id.clone());
    run_backend_prompt(
        config,
        &approval.prompt,
        jsonl,
        selected_model,
        effective_resume_session_id,
        flow_run_id,
    )?;
    fs::remove_file(&approval_path)
        .map_err(|err| format!("failed to consume approval {approval_id}: {err}"))?;
    Ok(())
}

fn deny_approval(config: &WecodeConfig, approval_id: &str, jsonl: bool) -> Result<(), String> {
    let path = approval_path(config, approval_id);
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|err| format!("failed to delete approval {approval_id}: {err}"))?;
        return emit_local_message(&format!("Denied approval {approval_id}."), jsonl);
    }

    match native_approval::write_native_approval_decision(
        config,
        approval_id,
        NativeApprovalDecision::Deny,
    ) {
        Ok(()) => emit_local_markdown(
            &format!(
                "Denied Codex approval {approval_id}. Codex will continue in the original turn."
            ),
            jsonl,
        ),
        Err(_) => emit_local_markdown(&format!("Approval {approval_id} was not found."), jsonl),
    }
}

fn write_pending_approval(
    config: &WecodeConfig,
    approval_id: &str,
    approval: &PendingApproval,
) -> Result<(), String> {
    let approvals_dir = approvals_dir(config);
    fs::create_dir_all(&approvals_dir)
        .map_err(|err| format!("failed to create approvals dir: {err}"))?;
    let json = serde_json::to_string_pretty(approval)
        .map_err(|err| format!("failed to serialize approval: {err}"))?;
    fs::write(approvals_dir.join(format!("{approval_id}.json")), json)
        .map_err(|err| format!("failed to write approval {approval_id}: {err}"))
}

fn read_pending_approval(path: &Path) -> Result<PendingApproval, String> {
    let input = fs::read_to_string(path)
        .map_err(|err| format!("failed to read approval {}: {err}", path.display()))?;
    serde_json::from_str(&input)
        .map_err(|err| format!("invalid approval {}: {err}", path.display()))
}

fn weixin_help_message(config: &WecodeConfig) -> String {
    let mut lines = vec![
        "# Wecode 帮助".to_string(),
        "".to_string(),
        "`:help` 和 `:commands` 由 Wecode 本地直接返回，不会请求 Codex。".to_string(),
        "".to_string(),
        "## 普通消息".to_string(),
        "".to_string(),
        "非命令消息会发送给 Codex，并在当前项目目录执行。当前聊天如果已绑定 session，Wecode 会自动续接该 Codex session。".to_string(),
        "".to_string(),
        "## 本地命令（不调用 Codex）".to_string(),
        "".to_string(),
        "- `:help` / `:commands` - 显示本帮助，不请求 Codex".to_string(),
        "- `:pwd` - 显示当前 Codex 项目目录".to_string(),
        "- `:ls [path]` - 列出目录，返回绝对路径".to_string(),
        "- `:cat <path>` - 读取项目内文本文件".to_string(),
        "- `:cd <path>` - 切换 Codex 项目目录，下一次任务会新开 session".to_string(),
        "- `:shell <command>` - 在当前 Codex 项目目录执行 shell 命令".to_string(),
        "- `:diff` - 显示当前项目 git diff".to_string(),
        "- `:status` - 显示后端、项目、session、审批、模型和 git 状态".to_string(),
        "".to_string(),
        "## Codex 命令（会调用 Codex）".to_string(),
        "".to_string(),
        "- `:init [notes]` - 转成 `/init ...` 后发送给 Codex 原生命令".to_string(),
        "- `:review [instructions]` - 对未提交改动执行 Codex review".to_string(),
        "- `:new [prompt]` - 转成 `/new ...` 后发送给 Codex 原生命令，不保证切换 Wecode 绑定的 session".to_string(),
        "- `:compact [notes]` - 转成 `/compact ...` 后发送给 Codex 原生命令".to_string(),
        "- `:plan [task]` - 转成 `/plan ...` 后发送给 Codex 原生命令".to_string(),
        "- `:goal [objective]` - 转成 `/goal ...` 后发送给 Codex 原生命令".to_string(),
        "- `:agent [task]` - 转成 `/agent ...` 后发送给 Codex 原生命令".to_string(),
        "- `:side [question]` - 转成 `/side ...` 后发送给 Codex 原生命令".to_string(),
        "- `:report [notes]` - 查询任务进展".to_string(),
        "".to_string(),
        "## Session 命令".to_string(),
        "".to_string(),
        "- `:resume [session_id]` - 绑定到最近或指定的 Codex session，不请求 Codex".to_string(),
        "- `:fresh [prompt]` - 硬新开 Codex thread；带 prompt 时立即执行，不带 prompt 时作用于下一条请求".to_string(),
        "".to_string(),
        "## 审批命令".to_string(),
        "".to_string(),
        "- `:approve <id>` / `:yes <id>` - 批准并执行待确认命令".to_string(),
        "- `:deny <id>` / `:no <id>` - 拒绝待确认命令".to_string(),
        "".to_string(),
        "## 模型命令".to_string(),
        "".to_string(),
        "- `:model` - 显示当前项目使用的 Codex 模型".to_string(),
        "- `:models` - 列出配置中的 Codex 模型候选".to_string(),
        "- `:model <model>` - 设置当前项目后续使用的 Codex 模型".to_string(),
        "- `:model default` - 清除项目模型覆盖".to_string(),
    ];
    if !config.commands.is_empty() {
        lines.push("".to_string());
        lines.push("## 自定义命令".to_string());
        lines.push("".to_string());
        for command in &config.commands {
            let approval = if command.require_confirm {
                "（需要审批）"
            } else {
                ""
            };
            lines.push(format!(
                "- `{}` - {}{}",
                custom_command_usage(&command.prefix),
                command.name,
                approval
            ));
        }
    }
    lines.join("\n")
}

fn custom_command_usage(prefix: &str) -> String {
    let trimmed = prefix.trim_end();
    if prefix.chars().last().is_some_and(char::is_whitespace) {
        format!("{trimmed} <text>")
    } else {
        trimmed.to_string()
    }
}

fn weixin_status_message(
    config: &WecodeConfig,
    resume_session_id: Option<String>,
    selected_model: Option<&str>,
) -> String {
    let target_cwd = codex_target_cwd(config)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|err| format!("unavailable ({err})"));
    let current_session = resume_session_id.unwrap_or_else(|| "(none)".to_string());
    let pending = fs::read_dir(approvals_dir(config))
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    let model = effective_codex_model(config, selected_model)
        .ok()
        .flatten()
        .unwrap_or_else(|| "(Codex default)".to_string());
    let transport = codex_transport_label(config.codex.transport);
    let git_status = run_git(config, &["status", "--short"])
        .map(|status| {
            if status.trim().is_empty() {
                "clean".to_string()
            } else {
                truncate_for_weixin(status.trim(), 4000)
            }
        })
        .unwrap_or_else(|err| format!("unavailable ({err})"));
    format!(
        "Wecode status:\nmodel: {}\nbackend model: {}\ntransport: {}\nsandbox: {}\ncwd: {}\ncurrent session: {}\npending approvals: {}\ngit: {}",
        config.openclaw.model,
        model,
        transport,
        config.codex.sandbox,
        target_cwd,
        current_session,
        pending,
        git_status
    )
}

fn codex_transport_label(transport: CodexTransport) -> &'static str {
    match transport {
        CodexTransport::Exec => "exec",
        CodexTransport::Remote => "remote",
        CodexTransport::RemoteStrict => "remote-strict",
    }
}

fn codex_sessions_root() -> PathBuf {
    if let Ok(codex_home) = env::var("CODEX_HOME") {
        return PathBuf::from(codex_home).join("sessions");
    }
    env::var("HOME")
        .map(|home| PathBuf::from(home).join(".codex").join("sessions"))
        .unwrap_or_else(|_| PathBuf::from(".codex").join("sessions"))
}

fn codex_target_cwd(config: &WecodeConfig) -> Result<PathBuf, String> {
    let path = config
        .codex
        .cwd
        .as_deref()
        .map(expand_tilde)
        .map(PathBuf::from)
        .unwrap_or(env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?);
    if path.is_absolute() {
        Ok(path.canonicalize().unwrap_or(path))
    } else {
        let cwd = env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?;
        let absolute = cwd.join(path);
        Ok(absolute.canonicalize().unwrap_or(absolute))
    }
}

fn resolve_project_path(config: &WecodeConfig, path: &str) -> Result<PathBuf, String> {
    let expanded = PathBuf::from(expand_tilde(path.trim()));
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Ok(codex_target_cwd(config)?.join(expanded))
    }
}

fn resolve_existing_project_path(config: &WecodeConfig, path: &str) -> Result<PathBuf, String> {
    let resolved = resolve_project_path(config, path)?;
    resolved
        .canonicalize()
        .map_err(|err| format!("failed to resolve {}: {err}", resolved.display()))
}

fn apply_project_cwd_override(config: &mut WecodeConfig) -> Result<(), String> {
    let path = project_cwd_override_path(config);
    if path.exists() {
        let cwd = fs::read_to_string(&path)
            .map_err(|err| {
                format!(
                    "failed to read project cwd override {}: {err}",
                    path.display()
                )
            })?
            .trim()
            .to_string();
        if !cwd.is_empty() {
            config.codex.cwd = Some(cwd);
        }
    }
    Ok(())
}

fn approvals_dir(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("approvals")
}

fn project_cwd_override_path(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("codex-cwd.txt")
}

fn project_fresh_next_path(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("codex-fresh-next.txt")
}

fn mark_project_fresh_next(config: &WecodeConfig) -> Result<(), String> {
    let path = project_fresh_next_path(config);
    ensure_project_state_paths_writable(config, "start a fresh Codex thread", &[path.clone()])?;
    write_project_state_file(
        config,
        &path,
        "1",
        "start a fresh Codex thread",
        "project fresh-thread marker",
    )
}

fn ensure_project_state_paths_writable(
    config: &WecodeConfig,
    action: &str,
    paths: &[PathBuf],
) -> Result<(), String> {
    let state_dir = PathBuf::from(expand_tilde(&config.openclaw.state_dir));
    fs::create_dir_all(&state_dir).map_err(|err| project_state_error(config, action, err))?;
    if !state_dir.is_dir() {
        return Err(project_state_error(
            config,
            action,
            format!("{} is not a directory", state_dir.display()),
        ));
    }
    for path in paths {
        if path.is_dir() {
            return Err(project_state_error(
                config,
                action,
                format!("{} is a directory, expected a file path", path.display()),
            ));
        }
    }

    let probe_path = state_dir.join(format!(
        ".wecode-write-test-{}-{}",
        std::process::id(),
        current_millis()
    ));
    fs::write(&probe_path, "test").map_err(|err| project_state_error(config, action, err))?;
    fs::remove_file(&probe_path).map_err(|err| project_state_error(config, action, err))?;
    Ok(())
}

fn write_project_state_file(
    config: &WecodeConfig,
    path: &Path,
    content: &str,
    action: &str,
    description: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| project_state_error(config, action, err))?;
    }
    fs::write(path, content).map_err(|err| {
        project_state_error(
            config,
            action,
            format!("failed to write {description} {}: {err}", path.display()),
        )
    })
}

fn project_state_error(
    config: &WecodeConfig,
    action: &str,
    cause: impl std::fmt::Display,
) -> String {
    format!(
        "Cannot {action}: Wecode cannot write state files under openclaw.stateDir `{}`. Check the configured path and permissions for this user. Cause: {cause}",
        expand_tilde(&config.openclaw.state_dir)
    )
}

fn model_override_path(config: &WecodeConfig) -> PathBuf {
    let project_key = codex_target_cwd(config)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| config.codex.cwd.clone().unwrap_or_else(|| ".".to_string()));
    PathBuf::from(expand_tilde(&config.openclaw.state_dir))
        .join("codex-models")
        .join(format!("{}.txt", stable_path_hash(&project_key)))
}

fn effective_codex_model(
    config: &WecodeConfig,
    selected_model: Option<&str>,
) -> Result<Option<String>, String> {
    let path = model_override_path(config);
    if path.exists() {
        let model = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read model override {}: {err}", path.display()))?
            .trim()
            .to_string();
        if !model.is_empty() {
            return Ok(Some(model));
        }
    }
    if let Some(model) = selected_model {
        if let Some(model) = codex_model_from_openclaw_model(model) {
            return Ok(Some(model));
        }
    }
    Ok(config.codex.model.clone())
}

fn configured_codex_models(config: &WecodeConfig) -> Vec<String> {
    let mut models = Vec::new();
    for model in &config.codex.models {
        if let Some(model) = codex_model_from_openclaw_model(model) {
            if !models.contains(&model) {
                models.push(model);
            }
        } else if !models.iter().any(|model| model == "default") {
            models.push("default".to_string());
        }
    }
    if let Some(model) = config.codex.model.as_deref() {
        if let Some(model) = codex_model_from_openclaw_model(model) {
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    models
}

fn stable_path_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn approval_path(config: &WecodeConfig, approval_id: &str) -> PathBuf {
    approvals_dir(config).join(format!("{approval_id}.json"))
}

fn create_approval_id() -> String {
    format!("appr-{}-{}", std::process::id(), current_millis())
}

fn current_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn run_git(config: &WecodeConfig, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(codex_target_cwd(config)?)
        .output()
        .map_err(|err| format!("failed to run git {}: {err}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(windows)]
fn shell_program_and_args(command: &str) -> (&'static str, Vec<&str>) {
    ("cmd", vec!["/C", command])
}

#[cfg(not(windows))]
fn shell_program_and_args(command: &str) -> (&'static str, Vec<&str>) {
    ("sh", vec!["-lc", command])
}

fn format_shell_output(_command: &str, _cwd: &Path, output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim_end();
    let stderr = stderr.trim_end();

    if stdout.is_empty() && stderr.is_empty() {
        return "(no output)".to_string();
    }
    let mut message = String::new();
    if !stdout.is_empty() {
        message.push_str(&truncate_for_weixin(stdout, 12000));
    }
    if !stderr.is_empty() {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&truncate_for_weixin(stderr, 8000));
    }
    message
}

fn truncate_for_weixin(input: &str, max_chars: usize) -> String {
    let mut output = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        output.push_str("\n... truncated ...");
    }
    output
}

fn codex_resume_rollout_is_missing(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr);
    stderr.contains("thread/resume") && stderr.contains("no rollout found for thread id")
}

fn read_stdin_prompt() -> Result<String, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| format!("failed to read prompt from stdin: {err}"))?;
    Ok(input)
}

fn probe_tools(config: &WecodeConfig) -> ToolSnapshot {
    let runtime_node_bin_dir = resolve_supported_node_bin_dir(config);
    let path_prepend = runtime_node_bin_dir.iter().cloned().collect::<Vec<_>>();

    ToolSnapshot {
        node_version: capture_version_with_path("node", &["--version"], &path_prepend),
        npm_found: capture_version_with_path("npm", &["--version"], &path_prepend).is_some(),
        npx_found: capture_version_with_path("npx", &["--version"], &path_prepend).is_some(),
        openclaw_version: capture_version_with_path(
            &openclaw_bin_path(config),
            &["--version"],
            &path_prepend,
        )
        .or_else(|| capture_version_with_path("openclaw", &["--version"], &path_prepend)),
        codex_version: capture_version_with_path("codex", &["--version"], &[]),
    }
}

fn capture_version_with_path(
    program: &str,
    args: &[&str],
    path_prepend: &[String],
) -> Option<String> {
    let program = expand_tilde(program);
    let mut command = Command::new(program);
    command.args(args);
    if !path_prepend.is_empty() {
        command.env("PATH", path_with_prepend(path_prepend));
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stdout.is_empty() {
        Some(stdout)
    } else if !stderr.is_empty() {
        Some(stderr)
    } else {
        Some("found".to_string())
    }
}

fn load_config(path: Option<String>) -> Result<(WecodeConfig, String), String> {
    if let Some(path) = path {
        let config = read_config_file(Path::new(&path))?;
        return Ok((config, path));
    }

    if let Some(path) = default_config_path() {
        if path.exists() {
            let config = read_config_file(&path)?;
            return Ok((config, path.display().to_string()));
        }
    }

    Ok((default_config(), "built-in defaults".to_string()))
}

fn read_config_file(path: &Path) -> Result<WecodeConfig, String> {
    let input = fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    read_config_str(&input).map_err(|err| format!("invalid config {}: {err}", path.display()))
}

fn default_config_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("WECODE_CONFIG") {
        return Some(PathBuf::from(path));
    }

    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(path).join("wecode").join("config.json"));
    }

    env::var("HOME").ok().map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("wecode")
            .join("config.json")
    })
}

fn codex_output_path() -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    env::temp_dir().join(format!("wecode-codex-{}-{millis}.txt", std::process::id()))
}

fn current_exe_string() -> Result<String, String> {
    env::current_exe()
        .map_err(|err| format!("failed to resolve current executable path: {err}"))
        .map(|path| path.display().to_string())
}

fn print_steps(config: &WecodeConfig, steps: &[CommandStep]) {
    let runtime_node_bin_dir = resolve_supported_node_bin_dir(config);
    for step in steps {
        let path_prepend = runtime_path_prepend(&runtime_node_bin_dir, &step.path_prepend);
        println!(
            "{}",
            step_with_path_prepend(step, path_prepend).display_shell()
        );
    }
}

fn maybe_start_codex_remote_daemon(
    config: &WecodeConfig,
    flow_run_id: Option<&str>,
) -> Result<(), String> {
    if config.codex.transport == CodexTransport::Exec || !config.codex.remote.auto_start {
        return Ok(());
    }
    if let Some(run_id) = flow_run_id {
        tracing::info!(
            run_id = %run_id,
            event = "codex_remote_daemon_start",
            command = ?config.codex.remote.start_command.as_str(),
            "prompt_flow"
        );
    }
    match start_codex_remote_daemon(config) {
        Ok(()) => Ok(()),
        Err(err)
            if config.codex.transport == CodexTransport::Remote
                || !config.codex.remote.fallback_proxy_command.trim().is_empty() =>
        {
            eprintln!("warning: {err}; will try Codex app-server fallback on demand");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn print_report(report: &ToolReport) {
    for item in &report.items {
        let status = if item.ok { "ok" } else { "fail" };
        println!("{:<9} {:<7} {}", item.name, status, item.message);
    }
}

fn ensure_private_openclaw_dirs(config: &WecodeConfig) -> Result<(), String> {
    ensure_project_state_paths_writable(config, "bootstrap Wecode", &[])?;
    fs::create_dir_all(expand_tilde(&config.openclaw.workspace_dir))
        .map_err(|err| format!("failed to create OpenClaw workspace dir: {err}"))?;
    if let Some(parent) = Path::new(&expand_tilde(&config.openclaw.config_path)).parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create OpenClaw config dir: {err}"))?;
    }
    Ok(())
}

pub fn print_help() {
    println!(
        r#"wecode - personal Codex <-> Weixin/Feishu bridge manager

Usage:
  wecode doctor
  wecode sample-config
  wecode config validate [path]
  wecode bootstrap [--config path] [--dry-run] [--weixin|--feishu]
  wecode configure-codex [--config path]
  wecode codex [--config path] <prompt...>
  wecode codex-backend [--config path] [--jsonl] [--cwd project-dir] [--model model-id] [--resume thread-id] [prompt...]
  wecode render [--config path] <message...>

Personal bridge flow:
  1. Install Node 24 or Node >=22.19.0.
  2. Run `wecode bootstrap --weixin` or `wecode bootstrap --feishu`.
  3. Complete the channel login prompt.
  4. OpenClaw receives channel messages and calls `wecode codex-backend`;
     `wecode` then calls your already logged-in local Codex CLI.
"#
    );
}

fn runtime_path_prepend(
    runtime_node_bin_dir: &Option<String>,
    step_path_prepend: &[String],
) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(node_bin_dir) = runtime_node_bin_dir {
        paths.push(node_bin_dir.clone());
    }
    for path in step_path_prepend {
        let expanded = expand_tilde(path);
        if !paths.iter().any(|existing| existing == &expanded) {
            paths.push(expanded);
        }
    }
    paths
}

fn step_with_path_prepend(step: &CommandStep, path_prepend: Vec<String>) -> CommandStep {
    let mut copy = step.clone();
    copy.path_prepend = path_prepend;
    copy
}

fn resolve_supported_node_bin_dir(config: &WecodeConfig) -> Option<String> {
    if let Some(configured) = config.openclaw.node_bin_dir.as_deref() {
        return Some(expand_tilde(configured));
    }

    if capture_version_with_path("node", &["--version"], &[])
        .as_deref()
        .and_then(parse_node_version)
        .is_some_and(|version| version >= (22, 19, 0))
    {
        return None;
    }

    supported_node_bin_dir_candidates()
        .into_iter()
        .find(|candidate| node_bin_dir_is_supported(candidate))
        .map(|path| path.display().to_string())
}

fn node_bin_dir_is_supported(path: &Path) -> bool {
    let node = path.join("node");
    let version = capture_version_with_path(&node.display().to_string(), &["--version"], &[]);
    version
        .as_deref()
        .and_then(parse_node_version)
        .is_some_and(|version| version >= (22, 19, 0))
}

fn supported_node_bin_dir_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/opt/node/bin"),
        PathBuf::from("/opt/homebrew/opt/node@24/bin"),
        PathBuf::from("/opt/homebrew/opt/node@22/bin"),
        PathBuf::from("/usr/local/opt/node/bin"),
        PathBuf::from("/usr/local/opt/node@24/bin"),
        PathBuf::from("/usr/local/opt/node@22/bin"),
    ];

    if let Ok(home) = env::var("HOME") {
        candidates.extend(versioned_node_bin_dirs(
            Path::new(&home).join(".local/share/mise/installs/node"),
        ));
        candidates.extend(versioned_node_bin_dirs(
            Path::new(&home).join(".nvm/versions/node"),
        ));
        candidates.push(Path::new(&home).join(".volta/bin"));
    }

    candidates
}

fn versioned_node_bin_dirs(root: PathBuf) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut dirs = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("bin"))
        .collect::<Vec<_>>();
    dirs.sort_by(|left, right| right.cmp(left));
    dirs
}

fn path_with_prepend(paths: &[String]) -> String {
    let mut segments = paths
        .iter()
        .map(|path| expand_tilde(path))
        .collect::<Vec<_>>();
    if let Some(existing) = env::var_os("PATH") {
        segments.push(existing.to_string_lossy().into_owned());
    }
    segments.join(if cfg!(windows) { ";" } else { ":" })
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

use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use wecode::{
    bootstrap_plan_with_backend_command, codex_config_plan_with_backend_command,
    codex_model_from_openclaw_model, default_config, diagnose_tools, list_codex_sessions,
    openclaw_bin_path, parse_cli_args, parse_node_version, prepare_backend_input, read_config_str,
    render_command_input, weixin_install_step, BackendInput, CliCommand, CommandStep, ToolReport,
    ToolSnapshot, WecodeConfig,
};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PendingApproval {
    command_name: String,
    prompt: String,
    resume_session_id: Option<String>,
    created_at_millis: u128,
}

fn main() {
    let exit_code = match parse_cli_args(env::args()) {
        Ok(command) => match run(command) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            print_help();
            2
        }
    };

    std::process::exit(exit_code);
}

fn run(command: CliCommand) -> Result<(), String> {
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
            install_openclaw,
        } => {
            let (config, source) = load_config(config_path)?;
            eprintln!("using config: {source}");
            let backend_command = current_exe_string()?;
            let steps =
                bootstrap_plan_with_backend_command(&config, install_openclaw, &backend_command);

            if dry_run {
                print_steps(&config, &steps);
                return Ok(());
            }

            validate_bootstrap_prereqs(
                &config,
                install_openclaw || config.openclaw.auto_install_openclaw,
            )?;
            ensure_private_openclaw_dirs(&config)?;
            run_steps(&config, &steps)
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
            )
        }
        CliCommand::Codex {
            config_path,
            prompt,
        } => {
            let (mut config, source) = load_config(config_path)?;
            apply_project_cwd_override(&mut config)?;
            eprintln!("using config: {source}");
            run_codex_prompt(&config, &prompt, CodexRunMode::Interactive)
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
            let prompt = match prompt {
                Some(prompt) => prompt,
                None => read_stdin_prompt()?,
            };
            if prompt.trim().is_empty() {
                return Err("codex-backend requires a prompt on argv or stdin".to_string());
            }
            match prepare_backend_input(&config, &prompt)? {
                BackendInput::Prompt(prompt) => {
                    run_backend_prompt(&config, &prompt, jsonl, model.clone(), resume_session_id)
                }
                BackendInput::FreshPrompt(prompt) => {
                    run_backend_prompt(&config, &prompt, jsonl, model.clone(), None)
                }
                BackendInput::Review { instructions } => {
                    run_codex_review(&config, instructions.as_deref(), model.as_deref(), jsonl)
                }
                BackendInput::Help => emit_local_message(&weixin_help_message(&config), jsonl),
                BackendInput::Status => emit_local_message(
                    &weixin_status_message(&config, resume_session_id, model.as_deref()),
                    jsonl,
                ),
                BackendInput::Diff => emit_diff(&config, jsonl),
                BackendInput::Pwd => emit_pwd(&config, jsonl),
                BackendInput::Ls { path } => emit_ls(&config, &path, jsonl),
                BackendInput::Cat { path } => emit_cat(&config, &path, jsonl),
                BackendInput::Cd { path } => set_project_cwd(&mut config, &path, jsonl),
                BackendInput::ModelShow => emit_model_status(&config, model.as_deref(), jsonl),
                BackendInput::ModelsList => emit_models_list(&config, jsonl),
                BackendInput::ModelSet { model } => set_model_override(&config, &model, jsonl),
                BackendInput::ResumeList => emit_resume_list(&config, jsonl),
                BackendInput::ResumeBind { session_id } => emit_resume_bind(&session_id, jsonl),
                BackendInput::ApprovalRequired {
                    command_name,
                    prompt,
                } => {
                    emit_approval_request(&config, &command_name, &prompt, resume_session_id, jsonl)
                }
                BackendInput::Approve { approval_id } => {
                    run_approved_prompt(&config, &approval_id, resume_session_id, model, jsonl)
                }
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
    print_report(&report);

    let required_ok = report.items.iter().all(|item| match item.name.as_str() {
        "openclaw" => item.ok || openclaw_will_be_installed,
        "codex" => true,
        _ => item.ok,
    });

    if required_ok {
        Ok(())
    } else {
        Err("bootstrap prerequisites are not satisfied".to_string())
    }
}

fn run_steps(config: &WecodeConfig, steps: &[CommandStep]) -> Result<(), String> {
    let runtime_node_bin_dir = resolve_supported_node_bin_dir(config);

    for step in steps {
        let path_prepend = runtime_path_prepend(&runtime_node_bin_dir, &step.path_prepend);
        let display_step = step_with_path_prepend(step, path_prepend.clone());
        eprintln!("$ {}", display_step.display_shell());

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

        let status = command
            .status()
            .map_err(|err| format!("failed to start `{}`: {err}", step.program))?;

        if !status.success() {
            return Err(format!("command failed: {}", display_step.display_shell()));
        }
    }

    Ok(())
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

fn run_codex_prompt(config: &WecodeConfig, prompt: &str, mode: CodexRunMode) -> Result<(), String> {
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
            let mut output_path = codex_output_path();
            let mut result =
                run_codex_backend_attempt(config, prompt, &output_path, resume_session_id, model)?;

            if !result.success
                && resume_session_id.is_some()
                && codex_resume_rollout_is_missing(&result.stderr)
            {
                eprintln!(
                    "codex resume thread is unavailable locally; starting a fresh Codex thread"
                );
                let _ = fs::remove_file(&output_path);
                output_path = codex_output_path();
                result = run_codex_backend_attempt(config, prompt, &output_path, None, model)?;
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

fn run_backend_prompt(
    config: &WecodeConfig,
    prompt: &str,
    jsonl: bool,
    model: Option<String>,
    resume_session_id: Option<String>,
) -> Result<(), String> {
    let force_fresh = project_fresh_next_path(config).exists();
    let result = run_codex_prompt(
        config,
        prompt,
        CodexRunMode::Backend {
            jsonl,
            model,
            resume_session_id: if force_fresh { None } else { resume_session_id },
        },
    );
    if result.is_ok() && force_fresh {
        let _ = fs::remove_file(project_fresh_next_path(config));
    }
    result
}

fn run_codex_review(
    config: &WecodeConfig,
    instructions: Option<&str>,
    selected_model: Option<&str>,
    jsonl: bool,
) -> Result<(), String> {
    let output_path = codex_output_path();
    let mut command = Command::new("codex");
    command.arg("exec").arg("review").arg("--json");
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
        "$ codex exec review --json -o {} --uncommitted",
        output_path.display()
    );
    let output = command
        .output()
        .map_err(|err| format!("failed to start codex review: {err}"))?;
    let result = CodexBackendResult {
        success: output.status.success(),
        stdout: output.stdout,
        stderr: output.stderr,
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
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_codex_interactive_attempt(
    config: &WecodeConfig,
    prompt: &str,
    output_path: &Path,
) -> Result<bool, String> {
    let mut command = codex_exec_command(config, prompt, output_path, None, None)?;
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
) -> Result<CodexBackendResult, String> {
    let mut command = codex_exec_command(
        config,
        prompt,
        output_path,
        resume_session_id,
        selected_model,
    )?;
    print_codex_exec_command(config, output_path, resume_session_id, selected_model);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command
        .output()
        .map_err(|err| format!("failed to start codex: {err}"))?;
    Ok(CodexBackendResult {
        success: output.status.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn codex_exec_command(
    config: &WecodeConfig,
    prompt: &str,
    output_path: &Path,
    resume_session_id: Option<&str>,
    selected_model: Option<&str>,
) -> Result<Command, String> {
    let target_cwd = codex_target_cwd(config)?;
    let mut command = Command::new("codex");
    command.current_dir(&target_cwd);
    command.arg("exec");
    if resume_session_id.is_some() {
        command.arg("resume");
    }
    command.arg("--json").arg("-o").arg(output_path);
    if let Ok(Some(model)) = effective_codex_model(config, selected_model) {
        command.arg("-m").arg(model);
    }
    if let Some(session_id) = resume_session_id {
        command.arg(session_id);
    } else {
        command.arg("-s").arg(&config.codex.sandbox);
        command.arg("-C").arg(&target_cwd);
    }
    command.arg("--").arg(prompt);
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
            "$ codex exec resume --json -o {}{} {}",
            output_path.display(),
            model,
            session_id
        );
    } else {
        eprintln!(
            "$ codex exec --json -o {}{} -s {}",
            output_path.display(),
            model,
            config.codex.sandbox
        );
    }
}

fn emit_codex_backend_result(result: &CodexBackendResult, jsonl: bool) -> Result<(), String> {
    if jsonl {
        io::stdout()
            .write_all(&result.stdout)
            .map_err(|err| format!("failed to write Codex JSONL stdout: {err}"))?;
    }
    io::stderr()
        .write_all(&result.stderr)
        .map_err(|err| format!("failed to write Codex stderr: {err}"))?;
    Ok(())
}

fn emit_local_message(message: &str, jsonl: bool) -> Result<(), String> {
    if jsonl {
        println!(
            "{}",
            serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": message
                    }
                ]
            })
        );
    } else {
        println!("{message}");
    }
    Ok(())
}

fn emit_resume_bind(session_id: &str, jsonl: bool) -> Result<(), String> {
    let message = format!(
        "Bound this Weixin session to Codex session {session_id}.\nNext messages will resume it."
    );
    if jsonl {
        println!("{}", serde_json::json!({ "thread_id": session_id }));
    }
    emit_local_message(&message, jsonl)
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
    emit_local_message(&codex_target_cwd(config)?.display().to_string(), jsonl)
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

    emit_local_message(&lines.join("\n"), jsonl)
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
    let override_path = project_cwd_override_path(config);
    if let Some(parent) = override_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create project cwd state dir: {err}"))?;
    }
    fs::write(&override_path, &target)
        .map_err(|err| format!("failed to write project cwd override: {err}"))?;
    fs::write(project_fresh_next_path(config), "1")
        .map_err(|err| format!("failed to write project fresh-thread marker: {err}"))?;
    config.codex.cwd = Some(target.clone());
    emit_local_message(
        &format!("Codex project directory set to:\n{target}\nNext Codex request will start a fresh thread for this project."),
        jsonl,
    )
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
            "Current Codex model for this project: {model}\nProject: {}\nSet it with `/model <model>`",
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
    lines.push("Use `/model <model>` to set the model for the current project.".to_string());
    lines.push("Use `/model default` to clear the project model override.".to_string());
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

fn emit_resume_list(config: &WecodeConfig, jsonl: bool) -> Result<(), String> {
    let sessions_root = codex_sessions_root();
    let target_cwd = codex_target_cwd(config)?;
    let sessions = list_codex_sessions(&sessions_root, &target_cwd, 10)?;
    let message = if sessions.is_empty() {
        format!(
            "No Codex sessions found for {}.\nUse `codex` in that project first, or set `codex.cwd` in wecode config.",
            target_cwd.display()
        )
    } else {
        let mut lines = vec![format!("Codex sessions for {}:", target_cwd.display())];
        for (idx, session) in sessions.iter().enumerate() {
            let title = if session.title.is_empty() {
                "(untitled)"
            } else {
                &session.title
            };
            lines.push(format!(
                "{}. {}  {}  {}",
                idx + 1,
                session.timestamp,
                session.id,
                title
            ));
        }
        lines.push("Reply `/resume <session_id>` to bind this Weixin chat.".to_string());
        lines.join("\n")
    };

    emit_local_message(&message, jsonl)
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
            "Command `{command_name}` requires approval.\nApprove: /approve {approval_id}\nDeny: /deny {approval_id}"
        ),
        jsonl,
    )
}

fn run_approved_prompt(
    config: &WecodeConfig,
    approval_id: &str,
    resume_session_id: Option<String>,
    selected_model: Option<String>,
    jsonl: bool,
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
        emit_local_message(&format!("Denied approval {approval_id}."), jsonl)
    } else {
        emit_local_message(&format!("Approval {approval_id} was not found."), jsonl)
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
        "Wecode help".to_string(),
        "".to_string(),
        "Shell commands (local, no Codex):".to_string(),
        "/pwd - show current Codex project directory".to_string(),
        "/ls [path] - list a directory with absolute paths".to_string(),
        "/cat <path> - read a local text file".to_string(),
        "/cd <path> - switch Codex project directory".to_string(),
        "/diff - show current git diff".to_string(),
        "/status - show backend, project, session, approvals, and git status".to_string(),
        "".to_string(),
        "Codex commands:".to_string(),
        "/init [notes] - create or update AGENTS.md for this project".to_string(),
        "/review [instructions] - run codex exec review on uncommitted changes".to_string(),
        "/new [prompt] - start a fresh Codex session".to_string(),
        "/compact [notes] - compact current context into a handoff summary".to_string(),
        "/plan [task] - ask Codex for a plan without editing files".to_string(),
        "/goal [objective] - ask Codex to report or update the active goal".to_string(),
        "/agent [task] - ask Codex to use subagents where useful".to_string(),
        "/side [question] - ask Codex for side analysis without file changes".to_string(),
        "".to_string(),
        "Session and approval commands:".to_string(),
        "/resume - list Codex sessions for this project".to_string(),
        "/sessions - same as /resume".to_string(),
        "/resume <session_id> - bind this Weixin chat to a Codex session".to_string(),
        "/approve <id> - approve a pending command".to_string(),
        "/deny <id> - deny a pending command".to_string(),
        "".to_string(),
        "Model commands:".to_string(),
        "/model - show current Codex model".to_string(),
        "/models - list configured Codex models".to_string(),
        "/model <model> - set Codex model for the current project".to_string(),
        "/model default - clear the project model override".to_string(),
    ];
    if !config.commands.is_empty() {
        lines.push("".to_string());
        lines.push("Custom prompt commands:".to_string());
        for command in &config.commands {
            lines.push(format!(
                "{}... - {}",
                command.prefix.trim_end(),
                command.name
            ));
        }
    }
    lines.join("\n")
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
        "Wecode status:\nmodel: {}\nbackend model: {}\nsandbox: {}\ncwd: {}\ncurrent session: {}\npending approvals: {}\ngit: {}",
        config.openclaw.model,
        model,
        config.codex.sandbox,
        target_cwd,
        current_session,
        pending,
        git_status
    )
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

fn print_report(report: &ToolReport) {
    for item in &report.items {
        let status = if item.ok { "ok" } else { "fail" };
        println!("{:<9} {:<7} {}", item.name, status, item.message);
    }
}

fn ensure_private_openclaw_dirs(config: &WecodeConfig) -> Result<(), String> {
    fs::create_dir_all(expand_tilde(&config.openclaw.state_dir))
        .map_err(|err| format!("failed to create OpenClaw state dir: {err}"))?;
    fs::create_dir_all(expand_tilde(&config.openclaw.workspace_dir))
        .map_err(|err| format!("failed to create OpenClaw workspace dir: {err}"))?;
    if let Some(parent) = Path::new(&expand_tilde(&config.openclaw.config_path)).parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create OpenClaw config dir: {err}"))?;
    }
    Ok(())
}

fn print_help() {
    println!(
        r#"wecode - personal Codex <-> Weixin bridge manager

Usage:
  wecode doctor
  wecode sample-config
  wecode config validate [path]
  wecode bootstrap [--config path] [--dry-run] [--install-openclaw]
  wecode configure-codex [--config path]
  wecode install-weixin
  wecode codex [--config path] <prompt...>
  wecode codex-backend [--config path] [--jsonl] [--cwd project-dir] [--model model-id] [--resume thread-id] [prompt...]
  wecode render [--config path] <message...>

Personal bridge flow:
  1. Install Node 24 or Node >=22.19.0.
  2. Run `wecode bootstrap --install-openclaw`.
  3. Complete the Weixin QR login prompt.
  4. OpenClaw receives Weixin messages and calls `wecode codex-backend`;
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

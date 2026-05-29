use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use wecode::{
    bootstrap_plan_with_backend_command, codex_config_plan_with_backend_command, default_config,
    diagnose_tools, openclaw_bin_path, parse_cli_args, prepare_backend_prompt, read_config_str,
    render_command_input, weixin_install_step, CliCommand, CommandStep, ToolReport, ToolSnapshot,
    WecodeConfig,
};

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
                print_steps(&steps);
                return Ok(());
            }

            validate_bootstrap_prereqs(
                &config,
                install_openclaw || config.openclaw.auto_install_openclaw,
            )?;
            ensure_private_openclaw_dirs(&config)?;
            run_steps(&steps)
        }
        CliCommand::InstallWeixin => {
            let (config, source) = load_config(None)?;
            eprintln!("using config: {source}");
            ensure_private_openclaw_dirs(&config)?;
            run_steps(&[weixin_install_step(&config)])
        }
        CliCommand::ConfigureCodex { config_path } => {
            let (config, source) = load_config(config_path)?;
            eprintln!("using config: {source}");
            ensure_private_openclaw_dirs(&config)?;
            let backend_command = current_exe_string()?;
            run_steps(&codex_config_plan_with_backend_command(
                &config,
                &backend_command,
            ))
        }
        CliCommand::Codex {
            config_path,
            prompt,
        } => {
            let (config, source) = load_config(config_path)?;
            eprintln!("using config: {source}");
            run_codex_prompt(&config, &prompt, CodexRunMode::Interactive)
        }
        CliCommand::CodexBackend {
            config_path,
            jsonl,
            prompt,
            resume_session_id,
        } => {
            let (config, source) = load_config(config_path)?;
            eprintln!("using config: {source}");
            let prompt = match prompt {
                Some(prompt) => prompt,
                None => read_stdin_prompt()?,
            };
            if prompt.trim().is_empty() {
                return Err("codex-backend requires a prompt on argv or stdin".to_string());
            }
            let prompt = prepare_backend_prompt(&config, &prompt)?;
            run_codex_prompt(
                &config,
                &prompt,
                CodexRunMode::Backend {
                    jsonl,
                    resume_session_id,
                },
            )
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

fn run_steps(steps: &[CommandStep]) -> Result<(), String> {
    for step in steps {
        eprintln!("$ {}", step.display_shell());
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

        if !step.path_prepend.is_empty() {
            command.env("PATH", path_with_prepend(&step.path_prepend));
        }
        for (key, value) in &step.env {
            command.env(key, expand_tilde(value));
        }

        let status = command
            .status()
            .map_err(|err| format!("failed to start `{}`: {err}", step.program))?;

        if !status.success() {
            return Err(format!("command failed: {}", step.display_shell()));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexRunMode {
    Interactive,
    Backend {
        jsonl: bool,
        resume_session_id: Option<String>,
    },
}

fn run_codex_prompt(config: &WecodeConfig, prompt: &str, mode: CodexRunMode) -> Result<(), String> {
    let output_path = codex_output_path();
    let mut command = Command::new("codex");
    command.arg("exec");

    let resume_session_id = match &mode {
        CodexRunMode::Backend {
            resume_session_id, ..
        } => resume_session_id.as_deref(),
        CodexRunMode::Interactive => None,
    };

    if resume_session_id.is_some() {
        command.arg("resume");
    }

    command.arg("--json").arg("-o").arg(&output_path);

    if let Some(session_id) = resume_session_id {
        command.arg(session_id);
    } else {
        command.arg("-s").arg(&config.codex.sandbox);
        if let Some(cwd) = config.codex.cwd.as_deref() {
            command.arg("-C").arg(cwd);
        }
    }
    command.arg("--").arg(prompt);

    if let Some(session_id) = resume_session_id {
        eprintln!(
            "$ codex exec resume --json -o {} {}",
            output_path.display(),
            session_id
        );
    } else {
        eprintln!(
            "$ codex exec --json -o {} -s {}",
            output_path.display(),
            config.codex.sandbox
        );
    }

    match mode {
        CodexRunMode::Interactive => {
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        }
        CodexRunMode::Backend { jsonl, .. } => {
            command.stdin(Stdio::null()).stderr(Stdio::inherit());
            if jsonl {
                command.stdout(Stdio::inherit());
            } else {
                command.stdout(Stdio::null());
            }
        }
    }

    let status = command
        .status()
        .map_err(|err| format!("failed to start codex: {err}"))?;

    if !status.success() {
        return Err("codex exec failed".to_string());
    }

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

fn read_stdin_prompt() -> Result<String, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| format!("failed to read prompt from stdin: {err}"))?;
    Ok(input)
}

fn probe_tools(config: &WecodeConfig) -> ToolSnapshot {
    ToolSnapshot {
        node_version: capture_version("node", &["--version"]),
        npm_found: capture_version("npm", &["--version"]).is_some(),
        npx_found: capture_version("npx", &["--version"]).is_some(),
        openclaw_version: capture_version(&openclaw_bin_path(config), &["--version"])
            .or_else(|| capture_version("openclaw", &["--version"])),
        codex_version: capture_version("codex", &["--version"]),
    }
}

fn capture_version(program: &str, args: &[&str]) -> Option<String> {
    let program = expand_tilde(program);
    let output = Command::new(program).args(args).output().ok()?;
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

fn print_steps(steps: &[CommandStep]) {
    for step in steps {
        println!("{}", step.display_shell());
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
  wecode codex-backend [--config path] [--jsonl] [--resume thread-id] [prompt...]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapChannel {
    Weixin,
    Feishu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Doctor,
    SampleConfig,
    ConfigValidate {
        path: Option<String>,
    },
    RuntimeStatus {
        config_path: Option<String>,
    },
    Bootstrap {
        config_path: Option<String>,
        dry_run: bool,
        channel: BootstrapChannel,
    },
    PatchOpenclawRuntime {
        runtime_dir: String,
        state_dir: String,
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
        "patch-openclaw-runtime" => parse_patch_openclaw_runtime_command(&args[1..]),
        "config" => parse_config_command(&args[1..]),
        "runtime-status" => {
            let (config_path, rest) = parse_optional_config(&args[1..])?;
            if !rest.is_empty() {
                return Err(format!("unexpected runtime-status argument: {}", rest[0]));
            }
            Ok(CliCommand::RuntimeStatus { config_path })
        }
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
    let mut channel = None;
    let mut idx = 0;

    while idx < args.len() {
        match args[idx].as_str() {
            "--dry-run" => dry_run = true,
            "--weixin" => set_bootstrap_channel(&mut channel, BootstrapChannel::Weixin)?,
            "--feishu" => set_bootstrap_channel(&mut channel, BootstrapChannel::Feishu)?,
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
        channel: channel.unwrap_or(BootstrapChannel::Weixin),
    })
}

fn parse_patch_openclaw_runtime_command(args: &[String]) -> Result<CliCommand, String> {
    let mut runtime_dir = None;
    let mut state_dir = None;
    let mut idx = 0;

    while idx < args.len() {
        match args[idx].as_str() {
            "--runtime-dir" => {
                idx += 1;
                runtime_dir = Some(
                    args.get(idx)
                        .ok_or_else(|| "--runtime-dir requires a path".to_string())?
                        .clone(),
                );
            }
            "--state-dir" => {
                idx += 1;
                state_dir = Some(
                    args.get(idx)
                        .ok_or_else(|| "--state-dir requires a path".to_string())?
                        .clone(),
                );
            }
            other => {
                return Err(format!(
                    "unexpected patch-openclaw-runtime argument: {other}"
                ))
            }
        }
        idx += 1;
    }

    Ok(CliCommand::PatchOpenclawRuntime {
        runtime_dir: runtime_dir
            .ok_or_else(|| "patch-openclaw-runtime requires --runtime-dir".to_string())?,
        state_dir: state_dir.unwrap_or_else(|| "~/.wecode/openclaw-state".to_string()),
    })
}

fn set_bootstrap_channel(
    current: &mut Option<BootstrapChannel>,
    next: BootstrapChannel,
) -> Result<(), String> {
    if matches!(current, Some(existing) if *existing != next) {
        return Err("bootstrap channel flags are mutually exclusive".to_string());
    }
    *current = Some(next);
    Ok(())
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

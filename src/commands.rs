use crate::config::WecodeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedCommand {
    pub command_name: String,
    pub prompt: String,
    pub require_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBackendInput {
    pub command_input: String,
    pub input: BackendInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendInput {
    Prompt(String),
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
    Shell {
        command: String,
    },
    ModelShow,
    ModelsList,
    ModelSet {
        model: String,
    },
    Resume {
        session_id: Option<String>,
    },
    Fresh {
        prompt: Option<String>,
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

impl BackendInput {
    pub fn is_resume(&self) -> bool {
        matches!(self, BackendInput::Resume { .. })
    }
}

pub fn render_command_input(config: &WecodeConfig, input: &str) -> Option<RenderedCommand> {
    let trimmed = command_input_candidate(input).unwrap_or_else(|| input.trim());
    render_configured_command(config, trimmed).or_else(|| {
        colon_command_body(trimmed).and_then(|body| {
            let slash_input = format!("/{body}");
            render_configured_command(config, &slash_input)
        })
    })
}

fn render_configured_command(config: &WecodeConfig, input: &str) -> Option<RenderedCommand> {
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
    let command_input = command_input_candidate(input).unwrap_or(input);
    match render_command_input(config, command_input) {
        Some(rendered) if rendered.require_confirm => Err(format!(
            "command `{}` requires confirmation, which is not available in codex-backend",
            rendered.command_name
        )),
        Some(rendered) => Ok(rendered.prompt),
        None => Ok(normalize_prompt_input(command_input)),
    }
}

pub fn prepare_backend_input(config: &WecodeConfig, input: &str) -> Result<BackendInput, String> {
    prepare_backend_input_with_trace(config, input).map(|prepared| prepared.input)
}

pub fn prepare_backend_input_with_trace(
    config: &WecodeConfig,
    input: &str,
) -> Result<PreparedBackendInput, String> {
    let command_input = command_input_candidate(input).unwrap_or(input);
    let input = match parse_control_command(command_input)? {
        Some(input) => input,
        None => match render_command_input(config, command_input) {
            Some(rendered) if rendered.require_confirm => BackendInput::ApprovalRequired {
                command_name: rendered.command_name,
                prompt: rendered.prompt,
            },
            Some(rendered) => BackendInput::Prompt(rendered.prompt),
            None => BackendInput::Prompt(normalize_prompt_input(command_input)),
        },
    };
    Ok(PreparedBackendInput {
        command_input: command_input.to_string(),
        input,
    })
}

fn parse_control_command(input: &str) -> Result<Option<BackendInput>, String> {
    let trimmed = input.trim();
    if matches!(trimmed, ":help" | ":commands") {
        return Ok(Some(BackendInput::Help));
    }
    if trimmed == ":status" {
        return Ok(Some(BackendInput::Status));
    }
    if trimmed == ":diff" {
        return Ok(Some(BackendInput::Diff));
    }
    if trimmed == ":pwd" {
        return Ok(Some(BackendInput::Pwd));
    }
    if trimmed == ":ls" {
        return Ok(Some(BackendInput::Ls {
            path: ".".to_string(),
        }));
    }
    if let Some(rest) = trimmed.strip_prefix(":ls ") {
        let path = parse_path_arg(rest, ":ls")?;
        return Ok(Some(BackendInput::Ls { path }));
    }
    if let Some(rest) = trimmed.strip_prefix(":cat ") {
        let path = parse_path_arg(rest, ":cat")?;
        return Ok(Some(BackendInput::Cat { path }));
    }
    if let Some(rest) = trimmed.strip_prefix(":cd ") {
        let path = parse_path_arg(rest, ":cd")?;
        return Ok(Some(BackendInput::Cd { path }));
    }
    if trimmed == ":shell" {
        return Err(":shell expects a command".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix(":shell ") {
        let command = parse_shell_command(rest)?;
        return Ok(Some(BackendInput::Shell { command }));
    }
    if trimmed == ":model" {
        return Ok(Some(BackendInput::ModelShow));
    }
    if trimmed == ":models" {
        return Ok(Some(BackendInput::ModelsList));
    }
    if let Some(rest) = trimmed.strip_prefix(":model ") {
        let model = parse_single_arg(rest, ":model")?;
        return Ok(Some(BackendInput::ModelSet { model }));
    }
    if trimmed == ":review" {
        return Ok(Some(BackendInput::Review { instructions: None }));
    }
    if let Some(rest) = trimmed.strip_prefix(":review ") {
        return Ok(Some(BackendInput::Review {
            instructions: Some(rest.trim().to_string()),
        }));
    }
    if trimmed == ":report" || trimmed.starts_with(":report ") {
        let details = trimmed.strip_prefix(":report").unwrap_or("").trim();
        return Ok(Some(BackendInput::Prompt(report_prompt(details))));
    }
    if trimmed == ":sessions" {
        return Err(
            "`:sessions` was removed; use `:resume` to resume the previous Codex session."
                .to_string(),
        );
    }
    if trimmed == ":resume" {
        return Ok(Some(BackendInput::Resume { session_id: None }));
    }
    if let Some(rest) = trimmed.strip_prefix(":resume ") {
        return parse_single_arg(rest, ":resume").map(|session_id| {
            Some(BackendInput::Resume {
                session_id: Some(session_id),
            })
        });
    }
    if trimmed == ":fresh" {
        return Ok(Some(BackendInput::Fresh { prompt: None }));
    }
    if let Some(rest) = trimmed.strip_prefix(":fresh ") {
        let prompt = rest.trim();
        if prompt.is_empty() {
            return Ok(Some(BackendInput::Fresh { prompt: None }));
        }
        return Ok(Some(BackendInput::Fresh {
            prompt: Some(prompt.to_string()),
        }));
    }

    if let Some(rest) = trimmed.strip_prefix(":approve ") {
        return parse_approval_id(rest, ":approve")
            .map(|approval_id| Some(BackendInput::Approve { approval_id }));
    }

    if let Some(rest) = trimmed.strip_prefix(":deny ") {
        return parse_approval_id(rest, ":deny")
            .map(|approval_id| Some(BackendInput::Deny { approval_id }));
    }

    Ok(None)
}

fn normalize_prompt_input(input: &str) -> String {
    let trimmed = input.trim();
    colon_command_body(trimmed)
        .map(|body| format!("/{body}"))
        .unwrap_or_else(|| input.to_string())
}

fn command_input_candidate(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if colon_command_body(trimmed).is_some() {
        return Some(trimmed);
    }
    decorated_openclaw_command_input(trimmed).or_else(|| metadata_wrapped_command_input(trimmed))
}

fn decorated_openclaw_command_input(input: &str) -> Option<&str> {
    if !(input.contains("[message_id:") || input.contains("Conversation info (untrusted metadata)"))
    {
        return None;
    }
    let message_marker = input.rfind("[message_id:")?;
    let after_marker = &input[message_marker..];
    let message_line_offset = after_marker.find('\n')? + 1;
    let message_section_start = message_marker + message_line_offset;
    let message_section = &input[message_section_start..];
    let sender_line = message_section
        .lines()
        .find(|line| !line.trim().is_empty())?;
    let (_sender, _message) = sender_line.split_once(": ")?;
    let sender_line_start = message_section.find(sender_line)?;
    let message_start =
        message_section_start + sender_line_start + sender_line.find(": ")? + ": ".len();
    let message = input[message_start..].trim();
    colon_command_body(message).map(|_| message)
}

fn metadata_wrapped_command_input(input: &str) -> Option<&str> {
    let rest = input.strip_prefix("Conversation info (untrusted metadata):")?;
    let rest = rest.trim_start();
    if !rest.starts_with("```") {
        return None;
    }
    let body_start = rest.find('\n')? + 1;
    let body = &rest[body_start..];
    let closing_fence = body.find("\n```")?;
    let message = body[closing_fence + "\n```".len()..].trim();
    colon_command_body(message).map(|_| message)
}

fn colon_command_body(input: &str) -> Option<&str> {
    let body = input.strip_prefix(':')?;
    let command_name = body.split_whitespace().next()?;
    if command_name.is_empty()
        || !command_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }
    Some(body)
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

fn parse_shell_command(input: &str) -> Result<String, String> {
    let command = input.trim();
    if command.is_empty() {
        return Err(":shell expects a command".to_string());
    }
    Ok(command.to_string())
}

fn side_prompt(details: &str) -> String {
    append_details(
        "Handle this as a side analysis: answer without changing files unless explicitly requested, and keep it separate from the main implementation path.",
        details,
    )
}

fn report_prompt(details: &str) -> String {
    let request = if details.is_empty() {
        "任务状态".to_string()
    } else {
        format!("任务状态\n\n补充说明: {details}")
    };
    side_prompt(&request)
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

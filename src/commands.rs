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
    let extracted = backend_message_input(input);
    let input = extracted.as_deref().unwrap_or(input);
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
    let extracted = backend_message_input(input);
    let input = extracted.as_deref().unwrap_or(input);
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
    let message = backend_message_input(input);
    let input = message.as_deref().unwrap_or(input);
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

fn backend_message_input(input: &str) -> Option<String> {
    channel_message_input(input)
}

fn channel_message_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if let Some(message) = decorated_openclaw_message_input(trimmed)
        .or_else(|| metadata_wrapped_message_input(trimmed))
        .and_then(non_empty_string)
    {
        return Some(message.to_string());
    }

    if !trimmed.starts_with('{') {
        return None;
    }

    let value = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    channel_message_value(&value)
}

fn channel_message_value(value: &serde_json::Value) -> Option<String> {
    openclaw_channel_context_message(value)
        .or_else(|| feishu_event_message(value))
        .or_else(|| weixin_protocol_message(value))
}

fn openclaw_channel_context_message(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    let channel = [
        "OriginatingChannel",
        "Provider",
        "Surface",
        "channel",
        "provider",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))?;

    if !matches!(
        channel,
        "feishu" | "Feishu" | "openclaw-weixin" | "weixin" | "Weixin" | "wechat" | "WeChat"
    ) {
        return None;
    }

    [
        "BodyForCommands",
        "CommandBody",
        "RawBody",
        "BodyForAgent",
        "Body",
    ]
    .iter()
    .find_map(|key| non_empty_json_string(object.get(*key)?))
    .map(str::to_string)
}

fn feishu_event_message(value: &serde_json::Value) -> Option<String> {
    let event = if value.get("event").is_some() {
        value.get("event")?
    } else {
        value
    };
    let event = event.as_object()?;
    let message = event.get("message")?.as_object()?;
    if !event.get("sender")?.is_object() {
        return None;
    }

    let message_type = non_empty_json_string(message.get("message_type")?)?;
    let content = non_empty_json_string(message.get("content")?)?;
    feishu_message_content(content, message_type)
}

fn feishu_message_content(content: &str, message_type: &str) -> Option<String> {
    match message_type {
        "text" => serde_json::from_str::<serde_json::Value>(content)
            .ok()
            .and_then(|value| non_empty_json_string(value.get("text")?).map(str::to_string))
            .or_else(|| non_empty_string(content).map(str::to_string)),
        "post" => feishu_post_content(content),
        "audio" => feishu_media_content(content, "<media:audio>"),
        "image" => feishu_media_content(content, "<media:image>"),
        "file" => feishu_media_content(content, "<media:document>"),
        "video" | "media" => feishu_media_content(content, "<media:video>"),
        "sticker" => feishu_media_content(content, "<media:sticker>"),
        "share_chat" => feishu_share_chat_content(content),
        "merge_forward" => Some("[Merged and Forwarded Message - loading...]".to_string()),
        _ => non_empty_string(content).map(str::to_string),
    }
}

fn feishu_media_content(content: &str, placeholder: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    if placeholder == "<media:audio>" {
        if let Some(transcript) = value.get("speech_to_text").and_then(non_empty_json_string) {
            return Some(transcript.to_string());
        }
    }

    let file_name = value.get("file_name").and_then(non_empty_json_string);
    Some(match file_name {
        Some(file_name) => format!("{placeholder} ({file_name})"),
        None => placeholder.to_string(),
    })
}

fn feishu_share_chat_content(content: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    ["body", "summary", "share_chat_id"]
        .iter()
        .find_map(|key| value.get(*key).and_then(non_empty_json_string))
        .map(|message| {
            if value.get("share_chat_id").and_then(non_empty_json_string) == Some(message) {
                format!("[Forwarded message: {message}]")
            } else {
                message.to_string()
            }
        })
        .or_else(|| Some("[Forwarded message]".to_string()))
}

fn feishu_post_content(content: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let payload = feishu_post_payload(&value)?;
    let title = payload
        .get("title")
        .and_then(non_empty_json_string)
        .unwrap_or("");
    let paragraphs = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|paragraph| {
            let rendered = paragraph
                .as_array()?
                .iter()
                .map(feishu_post_element_text)
                .collect::<String>()
                .trim()
                .to_string();
            non_empty_string(&rendered).map(str::to_string)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rendered = [title, paragraphs.as_str()]
        .into_iter()
        .filter_map(non_empty_string)
        .collect::<Vec<_>>()
        .join("\n\n");
    non_empty_string(&rendered).map(str::to_string)
}

fn feishu_post_payload(value: &serde_json::Value) -> Option<&serde_json::Value> {
    if value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        return Some(value);
    }
    if let Some(post) = value.get("post").and_then(feishu_post_locale_payload) {
        return Some(post);
    }
    feishu_post_locale_payload(value)
}

fn feishu_post_locale_payload(value: &serde_json::Value) -> Option<&serde_json::Value> {
    if value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        return Some(value);
    }
    value.as_object()?.values().find(|candidate| {
        candidate
            .get("content")
            .and_then(serde_json::Value::as_array)
            .is_some()
    })
}

fn feishu_post_element_text(element: &serde_json::Value) -> String {
    if let Some(text) = element.as_str() {
        return text.to_string();
    }
    let Some(object) = element.as_object() else {
        return String::new();
    };
    match object.get("tag").and_then(serde_json::Value::as_str) {
        Some("text" | "a" | "md" | "lark_md" | "code") => ["text", "content"]
            .iter()
            .find_map(|key| object.get(*key).and_then(non_empty_json_string))
            .unwrap_or("")
            .to_string(),
        Some("at") => ["user_name", "name", "open_id", "user_id"]
            .iter()
            .find_map(|key| object.get(*key).and_then(non_empty_json_string))
            .map(|name| format!("@{name}"))
            .unwrap_or_default(),
        Some("img") => "![image]".to_string(),
        Some("media") => "[media]".to_string(),
        Some("br") => "\n".to_string(),
        _ => object
            .get("text")
            .and_then(non_empty_json_string)
            .unwrap_or("")
            .to_string(),
    }
}

fn weixin_protocol_message(value: &serde_json::Value) -> Option<String> {
    if let Some(message) = weixin_message_body(value) {
        return Some(message);
    }

    if let Some(messages) = value.get("msgs").and_then(serde_json::Value::as_array) {
        return messages.iter().find_map(weixin_message_body);
    }

    value.get("msg").and_then(weixin_message_body)
}

fn weixin_message_body(value: &serde_json::Value) -> Option<String> {
    let item_list = value.get("item_list")?.as_array()?;
    weixin_body_from_item_list(item_list)
}

fn weixin_body_from_item_list(item_list: &[serde_json::Value]) -> Option<String> {
    for item in item_list {
        if let Some(text) = item
            .get("text_item")
            .and_then(|text_item| text_item.get("text"))
            .and_then(non_empty_json_string)
        {
            let Some(ref_msg) = item.get("ref_msg") else {
                return Some(text.to_string());
            };
            if ref_msg
                .get("message_item")
                .is_some_and(is_weixin_media_item)
            {
                return Some(text.to_string());
            }

            let mut parts = Vec::new();
            if let Some(title) = ref_msg.get("title").and_then(non_empty_json_string) {
                parts.push(title.to_string());
            }
            if let Some(ref_body) = ref_msg.get("message_item").and_then(|message_item| {
                weixin_body_from_item_list(std::slice::from_ref(message_item))
            }) {
                parts.push(ref_body);
            }
            if parts.is_empty() {
                return Some(text.to_string());
            }
            return Some(format!("[引用: {}]\n{text}", parts.join(" | ")));
        }

        if let Some(text) = item
            .get("voice_item")
            .and_then(|voice_item| voice_item.get("text"))
            .and_then(non_empty_json_string)
        {
            return Some(text.to_string());
        }
    }

    None
}

fn is_weixin_media_item(item: &serde_json::Value) -> bool {
    matches!(
        item.get("type").and_then(serde_json::Value::as_i64),
        Some(2 | 3 | 4 | 5)
    )
}

fn non_empty_json_string(value: &serde_json::Value) -> Option<&str> {
    non_empty_string(value.as_str()?)
}

fn non_empty_string(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
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
    if let Some(rest) = trimmed.strip_prefix(":yes ") {
        return parse_approval_id(rest, ":yes")
            .map(|approval_id| Some(BackendInput::Approve { approval_id }));
    }

    if let Some(rest) = trimmed.strip_prefix(":deny ") {
        return parse_approval_id(rest, ":deny")
            .map(|approval_id| Some(BackendInput::Deny { approval_id }));
    }
    if let Some(rest) = trimmed.strip_prefix(":no ") {
        return parse_approval_id(rest, ":no")
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
    decorated_openclaw_message_input(trimmed)
        .or_else(|| metadata_wrapped_message_input(trimmed))
        .filter(|message| colon_command_body(message).is_some())
}

fn decorated_openclaw_message_input(input: &str) -> Option<&str> {
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
    Some(input[message_start..].trim())
}

fn metadata_wrapped_message_input(input: &str) -> Option<&str> {
    let rest = input.strip_prefix("Conversation info (untrusted metadata):")?;
    let rest = rest.trim_start();
    if !rest.starts_with("```") {
        return None;
    }
    let body_start = rest.find('\n')? + 1;
    let body = &rest[body_start..];
    let closing_fence = body.find("\n```")?;
    Some(body[closing_fence + "\n```".len()..].trim())
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

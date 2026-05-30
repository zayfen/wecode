use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Command,
};

use crate::paths::comparable_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSessionSummary {
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    pub originator: String,
    pub title: String,
    pub initial_prompt: String,
}

pub fn list_codex_sessions(
    sessions_root: &Path,
    target_cwd: &Path,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    list_codex_sessions_for_scope(sessions_root, Some(target_cwd), limit)
}

pub fn list_all_codex_sessions(
    sessions_root: &Path,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    list_codex_sessions_for_scope(sessions_root, None, limit)
}

fn list_codex_sessions_for_scope(
    sessions_root: &Path,
    target_cwd: Option<&Path>,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }

    let target_cwd = target_cwd.map(comparable_path);
    let mut files = Vec::new();
    collect_session_files(sessions_root, &mut files)
        .map_err(|err| format!("failed to scan Codex sessions: {err}"))?;

    let mut sessions = files
        .iter()
        .filter_map(|path| read_codex_session(path).ok().flatten())
        .filter(|session| {
            target_cwd
                .as_ref()
                .is_none_or(|target_cwd| comparable_path(Path::new(&session.cwd)) == *target_cwd)
        })
        .collect::<Vec<_>>();
    apply_codex_state_titles(sessions_root, &mut sessions);
    sessions.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| right.id.cmp(&left.id))
    });
    sessions.truncate(limit);
    Ok(sessions)
}

fn apply_codex_state_titles(sessions_root: &Path, sessions: &mut [CodexSessionSummary]) {
    let Some(titles) = read_codex_state_titles(sessions_root) else {
        return;
    };
    for session in sessions {
        if let Some(title) = titles.get(&session.id) {
            let title = visible_session_text(title).trim();
            if !title.is_empty() {
                session.title = sanitize_title(title);
            }
        }
    }
}

fn read_codex_state_titles(sessions_root: &Path) -> Option<HashMap<String, String>> {
    let codex_home = sessions_root.parent()?;
    let db_path = codex_home.join("state_5.sqlite");
    if !db_path.exists() {
        return None;
    }

    let output = Command::new("sqlite3")
        .args([
            "-readonly",
            "-json",
            db_path.to_str()?,
            "select id, title from threads where title != ''",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let rows = serde_json::from_slice::<Value>(&output.stdout).ok()?;
    let rows = rows.as_array()?;
    let mut titles = HashMap::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(title) = row.get("title").and_then(Value::as_str) else {
            continue;
        };
        titles.insert(id.to_string(), title.to_string());
    }
    Some(titles)
}

fn collect_session_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn read_codex_session(path: &Path) -> Result<Option<CodexSessionSummary>, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open Codex session {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut meta = None;
    let mut title = String::new();
    let mut initial_prompt = String::new();

    for (line_idx, line) in reader.lines().enumerate() {
        if line_idx >= 200 {
            break;
        }
        let line = line.map_err(|err| {
            format!(
                "failed to read Codex session line from {}: {err}",
                path.display()
            )
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if meta.is_none() {
            meta = extract_session_meta(&value);
        }
        if title.is_empty() {
            title = extract_session_title(&value);
        }
        if initial_prompt.is_empty() {
            initial_prompt = extract_session_prompt(&value);
        }
        if meta.is_some() && !title.is_empty() && !initial_prompt.is_empty() {
            break;
        }
    }

    let Some((id, timestamp, cwd, originator)) = meta else {
        return Ok(None);
    };

    Ok(Some(CodexSessionSummary {
        id,
        timestamp,
        cwd,
        originator,
        title,
        initial_prompt,
    }))
}

fn extract_session_meta(value: &Value) -> Option<(String, String, String, String)> {
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = value.get("payload")?;
    Some((
        payload.get("id")?.as_str()?.to_string(),
        payload.get("timestamp")?.as_str()?.to_string(),
        payload.get("cwd")?.as_str()?.to_string(),
        payload
            .get("originator")
            .and_then(Value::as_str)
            .unwrap_or("codex")
            .to_string(),
    ))
}

fn extract_session_title(value: &Value) -> String {
    if value.get("type").and_then(Value::as_str) == Some("event_msg") {
        if let Some(payload) = value.get("payload") {
            if payload.get("type").and_then(Value::as_str) == Some("user_message") {
                if let Some(message) = payload.get("message").and_then(Value::as_str) {
                    let message = visible_session_text(message);
                    if !is_injected_user_context(message) {
                        return sanitize_title(message);
                    }
                }
            }
        }
    }

    if value.get("type").and_then(Value::as_str) == Some("response_item") {
        let Some(payload) = value.get("payload") else {
            return String::new();
        };
        if payload.get("role").and_then(Value::as_str) != Some("user") {
            return String::new();
        }
        if let Some(content) = payload.get("content").and_then(Value::as_array) {
            for item in content {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let text = visible_session_text(text);
                    if !is_injected_user_context(text) {
                        return sanitize_title(text);
                    }
                }
            }
        }
    }

    String::new()
}

fn extract_session_prompt(value: &Value) -> String {
    if value.get("type").and_then(Value::as_str) == Some("event_msg") {
        if let Some(payload) = value.get("payload") {
            if payload.get("type").and_then(Value::as_str) == Some("user_message") {
                if let Some(message) = payload.get("message").and_then(Value::as_str) {
                    let message = visible_session_text(message);
                    if !is_injected_user_context(message) {
                        return sanitize_prompt(message);
                    }
                }
            }
        }
    }

    if value.get("type").and_then(Value::as_str) == Some("response_item") {
        let Some(payload) = value.get("payload") else {
            return String::new();
        };
        if payload.get("role").and_then(Value::as_str) != Some("user") {
            return String::new();
        }
        if let Some(content) = payload.get("content").and_then(Value::as_array) {
            for item in content {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let text = visible_session_text(text);
                    if !is_injected_user_context(text) {
                        return sanitize_prompt(text);
                    }
                }
            }
        }
    }

    String::new()
}

fn visible_session_text(input: &str) -> &str {
    strip_openclaw_reply_prefix(decorated_openclaw_message(input).unwrap_or(input))
}

fn decorated_openclaw_message(input: &str) -> Option<&str> {
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
    let sender_line_start = message_section.find(sender_line)?;
    let message_start =
        message_section_start + sender_line_start + sender_line.find(": ")? + ": ".len();
    let message = input[message_start..].trim();
    if message.is_empty() {
        None
    } else {
        Some(message)
    }
}

fn strip_openclaw_reply_prefix(input: &str) -> &str {
    let trimmed = input.trim_start();
    if !trimmed.starts_with("[Replying to: ") {
        return input;
    }
    let Some(reply_end) = trimmed.find(']') else {
        return input;
    };
    let reply_body = trimmed[reply_end + "]".len()..].trim_start();
    if reply_body.is_empty() {
        input
    } else {
        reply_body
    }
}

fn is_injected_user_context(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with("# AGENTS.md instructions for ")
        || trimmed.starts_with("<environment_context>")
}

fn sanitize_title(input: &str) -> String {
    truncate_single_line(input, 80)
}

fn sanitize_prompt(input: &str) -> String {
    truncate_single_line(input, 160)
}

fn truncate_single_line(input: &str, max_chars: usize) -> String {
    let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

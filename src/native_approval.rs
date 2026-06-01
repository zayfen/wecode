use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{config::WecodeConfig, paths::expand_tilde};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeApprovalDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeApprovalRecord {
    pub kind: String,
    pub approval_id: String,
    pub request_method: String,
    pub jsonrpc_id: Value,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub summary: Vec<String>,
    pub prompt: String,
    pub request_params: Value,
    pub created_at_millis: u128,
    pub expires_at_millis: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeApprovalDecisionRecord {
    pub approval_id: String,
    pub decision: NativeApprovalDecision,
    pub decided_at_millis: u128,
}

pub fn native_approvals_dir(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir))
        .join("approvals")
        .join("native")
}

pub fn native_approval_path(config: &WecodeConfig, approval_id: &str) -> PathBuf {
    native_approvals_dir(config).join(format!("{approval_id}.json"))
}

pub fn native_decision_path(config: &WecodeConfig, approval_id: &str) -> PathBuf {
    native_approvals_dir(config).join(format!("{approval_id}.decision.json"))
}

pub fn pending_native_approval_ids(config: &WecodeConfig) -> Vec<String> {
    let mut ids = Vec::new();
    let Ok(entries) = fs::read_dir(native_approvals_dir(config)) else {
        return ids;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if !path_is_native_approval_record(&path) || native_approval_is_expired(&path) {
            continue;
        }
        if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
            ids.push(id.to_string());
        }
    }
    ids.sort();
    ids
}

pub fn is_supported_approval_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
    )
}

fn path_is_native_approval_record(path: &std::path::Path) -> bool {
    path.is_file()
        && path.extension().and_then(|ext| ext.to_str()) == Some("json")
        && !path.to_string_lossy().ends_with(".decision.json")
}

fn native_approval_is_expired(path: &std::path::Path) -> bool {
    let Ok(input) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&input) else {
        return false;
    };
    value
        .get("expires_at_millis")
        .and_then(Value::as_u64)
        .is_some_and(|expires| u128::from(expires) <= current_millis())
}

pub fn create_native_approval_record(
    config: &WecodeConfig,
    message: &Value,
) -> Result<NativeApprovalRecord, String> {
    let request_method = message
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "Codex approval request did not include method".to_string())?
        .to_string();
    let jsonrpc_id = message
        .get("id")
        .cloned()
        .ok_or_else(|| "Codex approval request did not include id".to_string())?;
    let request_params = message.get("params").cloned().unwrap_or(Value::Null);
    let approval_id = create_approval_id();
    let now = current_millis();
    let timeout_ms = effective_approval_timeout(config).as_millis();
    let command = approval_command(&request_params);
    let cwd = read_string(&request_params, "cwd");
    let thread_id = read_string(&request_params, "threadId")
        .or_else(|| read_string(&request_params, "conversationId"));
    let turn_id = read_string(&request_params, "turnId");
    let summary = approval_summary(&request_method, &request_params);
    let prompt = format_native_approval_prompt(
        &approval_id,
        &request_method,
        command.as_deref(),
        cwd.as_deref(),
        &summary,
    );
    let record = NativeApprovalRecord {
        kind: "codex-native".to_string(),
        approval_id,
        request_method,
        jsonrpc_id,
        thread_id,
        turn_id,
        command,
        cwd,
        summary,
        prompt,
        request_params,
        created_at_millis: now,
        expires_at_millis: now.saturating_add(timeout_ms),
    };
    write_native_approval_record(config, &record)?;
    Ok(record)
}

pub fn write_native_approval_decision(
    config: &WecodeConfig,
    approval_id: &str,
    decision: NativeApprovalDecision,
) -> Result<(), String> {
    let pending = native_approval_path(config, approval_id);
    if !pending.exists() {
        return Err(format!("Approval {approval_id} was not found."));
    }
    fs::create_dir_all(native_approvals_dir(config))
        .map_err(|err| format!("failed to create native approvals dir: {err}"))?;
    let record = NativeApprovalDecisionRecord {
        approval_id: approval_id.to_string(),
        decision,
        decided_at_millis: current_millis(),
    };
    let json = serde_json::to_string_pretty(&record)
        .map_err(|err| format!("failed to serialize native approval decision: {err}"))?;
    fs::write(native_decision_path(config, approval_id), json)
        .map_err(|err| format!("failed to write native approval decision {approval_id}: {err}"))
}

pub fn wait_for_native_approval_decision(
    config: &WecodeConfig,
    record: &NativeApprovalRecord,
) -> Result<NativeApprovalDecision, String> {
    wait_for_native_approval_decision_with_progress(config, record, || Ok(()))
}

pub fn wait_for_native_approval_decision_with_progress<F>(
    config: &WecodeConfig,
    record: &NativeApprovalRecord,
    mut on_waiting: F,
) -> Result<NativeApprovalDecision, String>
where
    F: FnMut() -> Result<(), String>,
{
    let deadline = Instant::now() + effective_approval_timeout(config);
    let progress_interval = approval_wait_progress_interval(config);
    let mut next_progress = Instant::now() + progress_interval;
    let decision_path = native_decision_path(config, &record.approval_id);
    loop {
        if let Ok(input) = fs::read_to_string(&decision_path) {
            let decision: NativeApprovalDecisionRecord =
                serde_json::from_str(&input).map_err(|err| {
                    format!(
                        "invalid native approval decision {}: {err}",
                        decision_path.display()
                    )
                })?;
            cleanup_native_approval(config, &record.approval_id);
            return Ok(decision.decision);
        }
        if Instant::now() >= deadline {
            cleanup_native_approval(config, &record.approval_id);
            return Ok(NativeApprovalDecision::Deny);
        }
        if Instant::now() >= next_progress {
            if let Err(err) = on_waiting() {
                cleanup_native_approval(config, &record.approval_id);
                return Err(err);
            }
            next_progress = Instant::now() + progress_interval;
        }
        thread::sleep(Duration::from_millis(500));
    }
}

pub fn cleanup_native_approval(config: &WecodeConfig, approval_id: &str) {
    let _ = fs::remove_file(native_approval_path(config, approval_id));
    let _ = fs::remove_file(native_decision_path(config, approval_id));
}

pub fn approval_response_for_decision(
    method: &str,
    request_params: &Value,
    decision: NativeApprovalDecision,
) -> Value {
    match method {
        "item/commandExecution/requestApproval" => {
            command_approval_response(request_params, decision)
        }
        "item/fileChange/requestApproval" => file_change_approval_response(decision),
        "item/permissions/requestApproval" => {
            permissions_approval_response(request_params, decision)
        }
        _ => json!({ "decision": "decline", "reason": "unsupported Codex approval method" }),
    }
}

pub fn requested_permissions(request_params: &Value) -> Value {
    let permissions = request_params.get("permissions").and_then(Value::as_object);
    let mut granted = serde_json::Map::new();
    if let Some(network) = permissions
        .and_then(|items| items.get("network"))
        .filter(|value| value.is_object())
    {
        granted.insert("network".to_string(), network.clone());
    }
    if let Some(file_system) = permissions
        .and_then(|items| items.get("fileSystem"))
        .filter(|value| value.is_object())
    {
        granted.insert("fileSystem".to_string(), file_system.clone());
    }
    Value::Object(granted)
}

fn command_approval_response(request_params: &Value, decision: NativeApprovalDecision) -> Value {
    match decision {
        NativeApprovalDecision::Approve => {
            json!({ "decision": command_accept_decision(request_params) })
        }
        NativeApprovalDecision::Deny => {
            json!({ "decision": command_rejection_decision(request_params, "decline") })
        }
    }
}

fn file_change_approval_response(decision: NativeApprovalDecision) -> Value {
    match decision {
        NativeApprovalDecision::Approve => json!({ "decision": "accept" }),
        NativeApprovalDecision::Deny => json!({ "decision": "decline" }),
    }
}

fn permissions_approval_response(
    request_params: &Value,
    decision: NativeApprovalDecision,
) -> Value {
    match decision {
        NativeApprovalDecision::Approve => json!({
            "permissions": requested_permissions(request_params),
            "scope": "turn"
        }),
        NativeApprovalDecision::Deny => json!({
            "permissions": {},
            "scope": "turn"
        }),
    }
}

fn command_accept_decision(params: &Value) -> &'static str {
    let available = params.get("availableDecisions").and_then(Value::as_array);
    match available {
        Some(items) if items.iter().any(|item| item.as_str() == Some("accept")) => "accept",
        Some(items)
            if items
                .iter()
                .any(|item| item.as_str() == Some("acceptForSession")) =>
        {
            "acceptForSession"
        }
        _ => "accept",
    }
}

fn command_rejection_decision(params: &Value, preferred: &'static str) -> &'static str {
    let available = params.get("availableDecisions").and_then(Value::as_array);
    match available {
        Some(items) if items.iter().any(|item| item.as_str() == Some(preferred)) => preferred,
        Some(items)
            if preferred == "decline"
                && items.iter().any(|item| item.as_str() == Some("cancel")) =>
        {
            "cancel"
        }
        Some(items)
            if preferred == "cancel"
                && items.iter().any(|item| item.as_str() == Some("decline")) =>
        {
            "decline"
        }
        _ => preferred,
    }
}

fn write_native_approval_record(
    config: &WecodeConfig,
    record: &NativeApprovalRecord,
) -> Result<(), String> {
    let dir = native_approvals_dir(config);
    fs::create_dir_all(&dir)
        .map_err(|err| format!("failed to create native approvals dir: {err}"))?;
    let json = serde_json::to_string_pretty(record)
        .map_err(|err| format!("failed to serialize native approval: {err}"))?;
    fs::write(dir.join(format!("{}.json", record.approval_id)), json).map_err(|err| {
        format!(
            "failed to write native approval {}: {err}",
            record.approval_id
        )
    })
}

fn approval_summary(method: &str, request_params: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(command) = approval_command(request_params) {
        lines.push(format!("Command: {}", compact_text(&command, 500)));
    }
    if let Some(cwd) = read_string(request_params, "cwd") {
        lines.push(format!("CWD: {}", compact_text(&cwd, 240)));
    }
    if method == "item/permissions/requestApproval" {
        let permissions = requested_permissions(request_params);
        if permissions != json!({}) {
            lines.push(format!(
                "Permissions: {}",
                compact_text(&permissions.to_string(), 800)
            ));
        }
    }
    if method == "item/fileChange/requestApproval" {
        if let Some(path) =
            read_string(request_params, "path").or_else(|| read_string(request_params, "filePath"))
        {
            lines.push(format!("File: {}", compact_text(&path, 240)));
        } else {
            lines.push("File change requested.".to_string());
        }
    }
    if lines.is_empty() {
        lines.push("Codex requested native approval.".to_string());
    }
    lines
}

fn format_native_approval_prompt(
    approval_id: &str,
    method: &str,
    command: Option<&str>,
    cwd: Option<&str>,
    summary: &[String],
) -> String {
    let title = match method {
        "item/commandExecution/requestApproval" => "Codex requests permission to run a command.",
        "item/fileChange/requestApproval" => "Codex requests permission to change files.",
        "item/permissions/requestApproval" => "Codex requests additional permissions.",
        _ => "Codex requests permission.",
    };
    let mut lines = vec![title.to_string(), format!("Approval: {approval_id}")];
    if let Some(command) = command {
        lines.push(format!("Command: {}", compact_text(command, 500)));
    }
    if let Some(cwd) = cwd {
        lines.push(format!("CWD: {}", compact_text(cwd, 240)));
    }
    for item in summary {
        if !lines.iter().any(|line| line == item) {
            lines.push(item.clone());
        }
    }
    lines.push(format!("Approve: yes, :yes, or :yes {approval_id}"));
    lines.push(format!("Deny: no, :no, or :no {approval_id}"));
    lines.join("\n")
}

fn approval_command(params: &Value) -> Option<String> {
    if let Some(command) = read_string(params, "command") {
        return Some(command);
    }
    if let Some(parts) = params.get("command").and_then(Value::as_array) {
        if parts.iter().all(|part| part.as_str().is_some()) {
            return Some(
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }
    let actions = params.get("commandActions").and_then(Value::as_array)?;
    let commands = actions
        .iter()
        .filter_map(|action| read_string(action, "command"))
        .collect::<Vec<_>>();
    if commands.is_empty() {
        None
    } else {
        Some(commands.join(" && "))
    }
}

fn read_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn effective_approval_timeout(config: &WecodeConfig) -> Duration {
    let requested = Duration::from_secs(config.codex.remote.approval_timeout_seconds.max(1));
    let watchdog_ms = config.openclaw.cli_no_output_timeout_ms;
    if watchdog_ms > 1_000 {
        requested.min(Duration::from_millis(watchdog_ms - 1_000))
    } else {
        requested
    }
}

fn approval_wait_progress_interval(config: &WecodeConfig) -> Duration {
    let timeout = effective_approval_timeout(config);
    let millis = (timeout.as_millis() / 10).clamp(1_000, 60_000) as u64;
    Duration::from_millis(millis)
}

fn compact_text(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
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

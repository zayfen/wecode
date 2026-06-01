use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{json, Value};

use crate::{
    config::{codex_model_from_openclaw_model, WecodeConfig},
    native_approval::{
        self, approval_response_for_decision, create_native_approval_record,
        wait_for_native_approval_decision_with_progress,
    },
    paths::expand_tilde,
};

pub struct CodexRemoteRunRequest<'a> {
    pub config: &'a WecodeConfig,
    pub prompt: &'a str,
    pub selected_model: Option<&'a str>,
    pub resume_session_id: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexRemoteRunEvent {
    AgentMessage {
        thread_id: String,
        text: String,
        final_answer: bool,
    },
    NativeApprovalRequested {
        thread_id: String,
        approval_id: String,
        prompt: String,
    },
    NativeApprovalStillPending {
        thread_id: String,
        approval_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexRemoteRunResult {
    pub thread_id: String,
    pub final_message: String,
}

struct RemoteAgentMessage {
    text: String,
    final_answer: bool,
}

struct JsonRpcClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

#[derive(Default)]
struct RemoteTurnState {
    message_deltas: HashMap<String, String>,
    last_agent_message: Option<String>,
    last_final_message: Option<String>,
}

pub fn start_codex_remote_daemon(config: &WecodeConfig) -> Result<(), String> {
    if !config.codex.remote.auto_start {
        return Ok(());
    }
    let command = config.codex.remote.start_command.trim();
    if command.is_empty() {
        return Ok(());
    }
    let output = shell_command(command)
        .output()
        .map_err(|err| format!("failed to start Codex remote-control daemon: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Codex remote-control start failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn run_codex_remote_turn(
    request: &CodexRemoteRunRequest<'_>,
) -> Result<CodexRemoteRunResult, String> {
    run_codex_remote_turn_with_events(request, |_| Ok(()))
}

pub fn run_codex_remote_turn_with_events<F>(
    request: &CodexRemoteRunRequest<'_>,
    mut event_handler: F,
) -> Result<CodexRemoteRunResult, String>
where
    F: FnMut(CodexRemoteRunEvent) -> Result<(), String>,
{
    let mut errors = Vec::new();
    if let Err(err) = start_codex_remote_daemon(request.config) {
        errors.push(format!("daemon start: {err}"));
    }

    let commands = remote_proxy_commands(request.config);
    if commands.is_empty() {
        errors.push("no Codex remote proxy command configured".to_string());
    }
    for command in commands {
        match run_codex_remote_turn_with_proxy_command(request, command, &mut event_handler) {
            Ok(result) => return Ok(result),
            Err(err) => errors.push(format!("proxy `{command}`: {err}")),
        }
    }

    Err(format!(
        "Codex remote app-server is unavailable: {}",
        errors.join("; ")
    ))
}

fn run_codex_remote_turn_with_proxy_command(
    request: &CodexRemoteRunRequest<'_>,
    proxy_command: &str,
    event_handler: &mut impl FnMut(CodexRemoteRunEvent) -> Result<(), String>,
) -> Result<CodexRemoteRunResult, String> {
    let mut client = JsonRpcClient::spawn(proxy_command)?;
    client.request(
        "initialize",
        json!({
            "clientInfo": {
                "name": "wecode",
                "title": "Wecode",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "experimentalApi": true
            }
        }),
    )?;

    let thread_id = if let Some(session_id) = request.resume_session_id {
        let response = client.request(
            "thread/resume",
            thread_params(request.config, request.selected_model, Some(session_id))?,
        )?;
        extract_thread_id(&response).unwrap_or_else(|| session_id.to_string())
    } else {
        let response = client.request(
            "thread/start",
            thread_params(request.config, request.selected_model, None)?,
        )?;
        extract_thread_id(&response).ok_or_else(|| {
            "Codex remote thread/start response did not include thread.id".to_string()
        })?
    };

    let turn_params = json!({
        "threadId": thread_id,
        "input": [
            {
                "type": "text",
                "text": request.prompt,
                "text_elements": []
            }
        ],
        "cwd": codex_target_cwd(request.config)?.display().to_string(),
        "model": effective_codex_model(request.config, request.selected_model)?
    });
    let turn_response = client.request_with_unhandled_server_requests(
        "turn/start",
        turn_params,
        |client, message| {
            handle_native_approval_request(
                request.config,
                client,
                message,
                &thread_id,
                event_handler,
            )
        },
    )?;
    if let Some(message) = extract_final_message(turn_response.get("turn")) {
        return Ok(CodexRemoteRunResult {
            thread_id,
            final_message: message,
        });
    }

    let mut turn_state = RemoteTurnState::default();
    loop {
        let message = client.read_message()?;
        if message.get("method").and_then(Value::as_str) == Some("turn/completed") {
            let params = message.get("params").unwrap_or(&Value::Null);
            let completed_thread = params.get("threadId").and_then(Value::as_str);
            if completed_thread.map_or(true, |id| id == thread_id) {
                return Ok(CodexRemoteRunResult {
                    thread_id,
                    final_message: extract_final_message(params.get("turn"))
                        .or_else(|| turn_state.final_message())
                        .unwrap_or_default(),
                });
            }
        } else if message.get("id").is_some() && message.get("method").is_some() {
            handle_native_approval_request(
                request.config,
                &mut client,
                &message,
                &thread_id,
                event_handler,
            )?;
        } else {
            if let Some(agent_message) = turn_state.observe_notification(&message) {
                event_handler(CodexRemoteRunEvent::AgentMessage {
                    thread_id: thread_id.clone(),
                    text: agent_message.text,
                    final_answer: agent_message.final_answer,
                })?;
            }
        }
    }
}

fn handle_native_approval_request(
    config: &WecodeConfig,
    client: &mut JsonRpcClient,
    message: &Value,
    fallback_thread_id: &str,
    event_handler: &mut impl FnMut(CodexRemoteRunEvent) -> Result<(), String>,
) -> Result<(), String> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if !native_approval::is_supported_approval_method(method) {
        return client.decline_server_request(message);
    }
    let record = create_native_approval_record(config, message)?;
    let thread_id = record
        .thread_id
        .clone()
        .unwrap_or_else(|| fallback_thread_id.to_string());
    let approval_id = record.approval_id.clone();
    event_handler(CodexRemoteRunEvent::NativeApprovalRequested {
        thread_id: thread_id.clone(),
        approval_id: approval_id.clone(),
        prompt: record.prompt.clone(),
    })?;
    let decision = wait_for_native_approval_decision_with_progress(config, &record, || {
        event_handler(CodexRemoteRunEvent::NativeApprovalStillPending {
            thread_id: thread_id.clone(),
            approval_id: approval_id.clone(),
        })
    })?;
    let result =
        approval_response_for_decision(&record.request_method, &record.request_params, decision);
    client.respond_to_server_request(message, result)
}

fn remote_proxy_commands(config: &WecodeConfig) -> Vec<&str> {
    let mut commands = Vec::new();
    let primary = config.codex.remote.proxy_command.trim();
    if !primary.is_empty() {
        commands.push(primary);
    }
    let fallback = config.codex.remote.fallback_proxy_command.trim();
    if !fallback.is_empty() && !commands.iter().any(|command| *command == fallback) {
        commands.push(fallback);
    }
    commands
}

impl RemoteTurnState {
    fn observe_notification(&mut self, message: &Value) -> Option<RemoteAgentMessage> {
        match message.get("method").and_then(Value::as_str) {
            Some("item/agentMessage/delta") => {
                let Some(params) = message.get("params") else {
                    return None;
                };
                let Some(item_id) = params.get("itemId").and_then(Value::as_str) else {
                    return None;
                };
                let Some(delta) = params.get("delta").and_then(Value::as_str) else {
                    return None;
                };
                self.message_deltas
                    .entry(item_id.to_string())
                    .or_default()
                    .push_str(delta);
                None
            }
            Some("item/completed") | Some("item/started") => {
                let completed =
                    message.get("method").and_then(Value::as_str) == Some("item/completed");
                let item = message
                    .get("params")
                    .and_then(|params| params.get("item"))
                    .unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
                    return None;
                }
                let item_id = item.get("id").and_then(Value::as_str);
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        item_id.and_then(|id| {
                            self.message_deltas
                                .get(id)
                                .filter(|text| !text.is_empty())
                                .cloned()
                        })
                    });
                if let Some(text) = text {
                    self.last_agent_message = Some(text.clone());
                    let final_answer =
                        item.get("phase").and_then(Value::as_str) == Some("final_answer");
                    if final_answer {
                        self.last_final_message = Some(text.clone());
                    }
                    if completed {
                        return Some(RemoteAgentMessage { text, final_answer });
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn final_message(&self) -> Option<String> {
        self.last_final_message
            .clone()
            .or_else(|| self.last_agent_message.clone())
            .or_else(|| {
                self.message_deltas
                    .values()
                    .filter(|text| !text.is_empty())
                    .last()
                    .cloned()
            })
    }
}

impl JsonRpcClient {
    fn spawn(proxy_command: &str) -> Result<Self, String> {
        let mut command = shell_command(proxy_command);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|err| format!("failed to start Codex app-server proxy: {err}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to capture Codex app-server proxy stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture Codex app-server proxy stdout".to_string())?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_unhandled_server_requests(method, params, |client, message| {
            client.decline_server_request(message)
        })
    }

    fn request_with_unhandled_server_requests<F>(
        &mut self,
        method: &str,
        params: Value,
        mut handler: F,
    ) -> Result<Value, String>
    where
        F: FnMut(&mut JsonRpcClient, &Value) -> Result<(), String>,
    {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.write_message(&request)?;

        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("Codex app-server `{method}` failed: {error}"));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            if message.get("id").is_some() && message.get("method").is_some() {
                handler(self, &message)?;
            }
        }
    }

    fn write_message(&mut self, message: &Value) -> Result<(), String> {
        let line = serde_json::to_string(message)
            .map_err(|err| format!("failed to serialize Codex JSON-RPC message: {err}"))?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|err| format!("failed to write Codex JSON-RPC message: {err}"))
    }

    fn read_message(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let len = self
                .stdout
                .read_line(&mut line)
                .map_err(|err| format!("failed to read Codex JSON-RPC message: {err}"))?;
            if len == 0 {
                return Err("Codex app-server proxy closed stdout".to_string());
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            return serde_json::from_str(trimmed)
                .map_err(|err| format!("invalid Codex JSON-RPC message `{trimmed}`: {err}"));
        }
    }

    fn decline_server_request(&mut self, message: &Value) -> Result<(), String> {
        if message.get("id").is_none() {
            return Ok(());
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "item/commandExecution/requestApproval" => json!({ "decision": "decline" }),
            "item/fileChange/requestApproval" => json!({ "decision": "decline" }),
            "item/permissions/requestApproval" => {
                json!({ "permissions": {}, "scope": "turn", "strictAutoReview": true })
            }
            _ => Value::Null,
        };
        self.respond_to_server_request(message, result)
    }

    fn respond_to_server_request(&mut self, message: &Value, result: Value) -> Result<(), String> {
        let Some(id) = message.get("id").cloned() else {
            return Ok(());
        };
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }))
    }
}

impl Drop for JsonRpcClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn thread_params(
    config: &WecodeConfig,
    selected_model: Option<&str>,
    resume_session_id: Option<&str>,
) -> Result<Value, String> {
    let mut params = json!({
        "cwd": codex_target_cwd(config)?.display().to_string(),
        "model": effective_codex_model(config, selected_model)?,
        "sandbox": config.codex.sandbox
    });
    if let Some(session_id) = resume_session_id {
        params["threadId"] = Value::String(session_id.to_string());
    }
    Ok(params)
}

fn extract_thread_id(response: &Value) -> Option<String> {
    response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_final_message(turn: Option<&Value>) -> Option<String> {
    let items = turn?.get("items")?.as_array()?;
    let mut last_agent_message = None;
    let mut last_final_message = None;
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("agentMessage") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                last_agent_message = Some(text.to_string());
                if item.get("phase").and_then(Value::as_str) == Some("final_answer") {
                    last_final_message = Some(text.to_string());
                }
            }
        }
    }
    last_final_message.or(last_agent_message)
}

fn effective_codex_model(
    config: &WecodeConfig,
    selected_model: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(model) = selected_model {
        if let Some(model) = codex_model_from_openclaw_model(model) {
            return Ok(Some(model));
        }
    }
    Ok(config.codex.model.clone())
}

fn codex_target_cwd(config: &WecodeConfig) -> Result<PathBuf, String> {
    let path = config
        .codex
        .cwd
        .as_deref()
        .map(expand_tilde)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?);
    if path.is_absolute() {
        Ok(path.canonicalize().unwrap_or(path))
    } else {
        let cwd = std::env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?;
        let absolute = cwd.join(path);
        Ok(absolute.canonicalize().unwrap_or(absolute))
    }
}

fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut shell = Command::new("cmd");
        shell.arg("/C").arg(command);
        shell
    } else {
        let mut shell = Command::new("sh");
        shell.arg("-c").arg(command);
        shell
    }
}

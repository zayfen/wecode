# Codex Native Approval Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bridge Codex app-server native approval requests to WeChat/Feishu so `:approve appr-...` or `:deny appr-...` unblocks the running Codex turn.

**Architecture:** Keep the bridge inside the existing `wecode codex-backend` CLI flow. The running remote Codex process persists a native approval record, emits a channel-visible approval prompt, polls for a decision file, then sends the matching JSON-RPC approval response back to Codex. Because OpenClaw `serialize: true` can prevent the second `:approve` backend invocation from running while the first turn is blocked, generated backend config must switch to `serialize: false` and Wecode must add its own Codex-run lock that excludes approval commands.

**Tech Stack:** Rust, serde/serde_json, JSONL stdout for OpenClaw, Codex app-server JSON-RPC, file-backed state under `openclaw.stateDir`.

---

## Scope And Decisions

- Supported in this implementation: Codex remote transports only, covering `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, and `item/permissions/requestApproval`.
- Not supported in this implementation: bridging `codex exec --json` fallback approvals. The exec transport does not expose the interactive app-server server-request JSON-RPC path currently used by Wecode.
- Exec fallback must invoke `codex exec --yolo --json ...` and `codex exec resume --yolo --json ...` so fallback turns run with Codex's YOLO approval mode instead of hanging on an interactive approval request.
- Existing custom command approvals (`requireConfirm: true`) must keep working. Native approvals must not start a second Codex run when approved.
- Native approval files live under `openclaw.stateDir/approvals/native/` to avoid changing the old custom approval record format.
- Native approval timeout defaults to 10 minutes and is capped below OpenClaw's CLI no-output watchdog so the backend auto-denies before OpenClaw kills the process.
- Approval messages are plain Markdown text. Remote-turn approval prompts are emitted as OpenClaw JSONL assistant-message items so WeChat/Feishu can display them while the process keeps waiting.

## Protocol Contract

This is based on the local OpenClaw Codex relay implementation in:

- `/Users/riven/.wecode/openclaw-runtime/node_modules/openclaw/dist/native-hook-relay-Ch2pKgop.js`

Codex approval response shapes:

```json
// item/commandExecution/requestApproval approve once
{ "decision": "accept" }

// item/commandExecution/requestApproval deny
{ "decision": "decline" }

// item/fileChange/requestApproval approve once
{ "decision": "accept" }

// item/fileChange/requestApproval deny
{ "decision": "decline" }

// item/permissions/requestApproval approve once
{ "permissions": { "network": {}, "fileSystem": {} }, "scope": "turn" }

// item/permissions/requestApproval deny
{ "permissions": {}, "scope": "turn" }
```

For command approvals, if `params.availableDecisions` exists and lacks `accept`, Wecode must fall back to a supported decision:

```rust
fn command_accept_decision(params: &serde_json::Value) -> &'static str {
    let available = params
        .get("availableDecisions")
        .and_then(serde_json::Value::as_array);
    match available {
        Some(items) if items.iter().any(|item| item.as_str() == Some("accept")) => "accept",
        Some(items) if items.iter().any(|item| item.as_str() == Some("acceptForSession")) => {
            "acceptForSession"
        }
        _ => "accept",
    }
}
```

## File Structure

- Create `src/native_approval.rs`
  - Owns native approval record schema, paths, prompt formatting, decision file writing/reading, timeout waiting, and JSON-RPC response construction.
- Create `src/run_lock.rs`
  - Owns file-backed Wecode Codex-run locking after OpenClaw backend serialization is disabled.
- Modify `src/lib.rs`
  - Export `native_approval` and `run_lock`.
- Modify `src/config.rs`
  - Add `codex.remote.approvalTimeoutSeconds`.
- Modify `src/codex_remote.rs`
  - Replace protocol-level auto-decline with native approval handling and event emission.
- Modify `src/backend.rs`
  - Add `--yolo` to generated `codex exec` and `codex exec resume` fallback commands.
- Modify `src/app.rs`
  - Emit approval prompts to channel, route `:approve` and `:deny` to either old custom approvals or new native approval decisions, apply run lock to Codex-running inputs.
- Modify `src/openclaw.rs`
  - Generate `"serialize": false` for the Wecode CLI backend.
- Modify `tests/config.rs`, `tests/bootstrap.rs`, `tests/codex_backend.rs`
  - Cover config defaults, generated OpenClaw config, native approval approve/deny/timeout flows, and legacy custom approvals.
- Modify `README.md`
  - Replace the old "native approvals are declined" limitation with the new remote-only approval bridge behavior and explain that users should rerun `wecode configure-codex`.

---

### Task 1: Native Approval State Module

**Files:**
- Create: `src/native_approval.rs`
- Modify: `src/lib.rs`
- Modify: `src/config.rs`
- Modify: `src/backend.rs`
- Test: `tests/config.rs`
- Test: `tests/codex_backend.rs`
- Test: `tests/native_approval.rs`

- [ ] **Step 1: Write failing config test**

Add to `tests/config.rs`:

```rust
#[test]
fn defaults_codex_remote_approval_timeout() {
    let cfg = wecode::default_config();
    assert_eq!(cfg.codex.remote.approval_timeout_seconds, 600);
}

#[test]
fn parses_codex_remote_approval_timeout() {
    let cfg = wecode::read_config_str(
        r#"{
          "codex": {
            "remote": {
              "approvalTimeoutSeconds": 42
            }
          }
        }"#,
    )
    .expect("config");

    assert_eq!(cfg.codex.remote.approval_timeout_seconds, 42);
}
```

- [ ] **Step 2: Write failing exec fallback yolo test**

Add to `tests/codex_backend.rs`:

```rust
#[test]
fn codex_backend_exec_fallback_uses_yolo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    fs::write(&config_path, r#"{"codex":{"transport":"exec"}}"#).expect("write config");
    write_fake_codex(&codex_path, &calls_path, "yolo-thread", "yolo ok");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "hello yolo",
        ])
        .env("PATH", &path)
        .output()
        .expect("run backend");

    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("exec --yolo --json"), "{calls}");
}
```

- [ ] **Step 3: Write failing native approval tests**

Create `tests/native_approval.rs`:

```rust
use std::fs;

use serde_json::json;
use tempfile::tempdir;
use wecode::{
    native_approval::{
        approval_response_for_decision, create_native_approval_record, native_approvals_dir,
        requested_permissions, NativeApprovalDecision,
    },
    read_config_str,
};

#[test]
fn command_approval_accept_respects_available_decisions() {
    let params = json!({
        "threadId": "thread-1",
        "turnId": "turn-1",
        "command": ["npm", "install"],
        "cwd": "/tmp/project",
        "availableDecisions": ["decline", "accept"]
    });
    let response = approval_response_for_decision(
        "item/commandExecution/requestApproval",
        &params,
        NativeApprovalDecision::Approve,
    );

    assert_eq!(response, json!({ "decision": "accept" }));
}

#[test]
fn command_approval_denies_with_decline() {
    let params = json!({
        "availableDecisions": ["cancel", "decline"]
    });
    let response = approval_response_for_decision(
        "item/commandExecution/requestApproval",
        &params,
        NativeApprovalDecision::Deny,
    );

    assert_eq!(response, json!({ "decision": "decline" }));
}

#[test]
fn permissions_approval_grants_requested_network_and_filesystem_for_turn() {
    let params = json!({
        "permissions": {
            "network": { "allow": ["registry.npmjs.org"] },
            "fileSystem": { "write": ["/tmp/project"] },
            "other": { "ignored": true }
        }
    });

    assert_eq!(
        requested_permissions(&params),
        json!({
            "network": { "allow": ["registry.npmjs.org"] },
            "fileSystem": { "write": ["/tmp/project"] }
        })
    );
    assert_eq!(
        approval_response_for_decision(
            "item/permissions/requestApproval",
            &params,
            NativeApprovalDecision::Approve,
        ),
        json!({
            "permissions": {
                "network": { "allow": ["registry.npmjs.org"] },
                "fileSystem": { "write": ["/tmp/project"] }
            },
            "scope": "turn"
        })
    );
}

#[test]
fn native_record_is_written_under_native_approval_dir() {
    let temp = tempdir().expect("tempdir");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    fs::write(
        &config_path,
        format!(
            r#"{{"openclaw":{{"stateDir":{}}}}}"#,
            serde_json::to_string(&state_dir.display().to_string()).expect("state json")
        ),
    )
    .expect("write config");
    let config = read_config_str(&fs::read_to_string(config_path).expect("config")).expect("parse");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "item/commandExecution/requestApproval",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "command": "cargo test",
            "cwd": "/tmp/project"
        }
    });

    let record = create_native_approval_record(&config, &request).expect("record");

    assert!(record.approval_id.starts_with("appr-"));
    assert_eq!(record.request_method, "item/commandExecution/requestApproval");
    assert!(native_approvals_dir(&config).join(format!("{}.json", record.approval_id)).exists());
    assert!(record.prompt.contains(":approve "));
    assert!(record.prompt.contains(":deny "));
    assert!(record.prompt.contains("cargo test"));
}
```

- [ ] **Step 4: Run tests to verify failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test defaults_codex_remote_approval_timeout parses_codex_remote_approval_timeout codex_backend_exec_fallback_uses_yolo command_approval_accept_respects_available_decisions command_approval_denies_with_decline permissions_approval_grants_requested_network_and_filesystem_for_turn native_record_is_written_under_native_approval_dir
```

Expected: compile failure because `approval_timeout_seconds` and `native_approval` do not exist, and the yolo test would fail against current exec args.

- [ ] **Step 5: Add config field**

Modify `src/config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexRemoteConfig {
    #[serde(default = "default_codex_remote_auto_start", rename = "autoStart")]
    pub auto_start: bool,
    #[serde(
        default = "default_codex_remote_proxy_command",
        rename = "proxyCommand"
    )]
    pub proxy_command: String,
    #[serde(
        default = "default_codex_remote_start_command",
        rename = "startCommand"
    )]
    pub start_command: String,
    #[serde(
        default = "default_codex_remote_fallback_proxy_command",
        rename = "fallbackProxyCommand"
    )]
    pub fallback_proxy_command: String,
    #[serde(
        default = "default_codex_remote_approval_timeout_seconds",
        rename = "approvalTimeoutSeconds"
    )]
    pub approval_timeout_seconds: u64,
}
```

Update `Default for CodexRemoteConfig`:

```rust
approval_timeout_seconds: default_codex_remote_approval_timeout_seconds(),
```

Add the default function:

```rust
fn default_codex_remote_approval_timeout_seconds() -> u64 {
    600
}
```

- [ ] **Step 6: Add `--yolo` to exec fallback command specs**

Modify `src/backend.rs` inside `CodexBackend::run_command_spec` after optional `resume`:

```rust
let mut args = vec!["exec".to_string()];
if request.resume_session_id.is_some() {
    args.push("resume".to_string());
}
args.push("--yolo".to_string());
args.push("--json".to_string());
```

Update `print_codex_exec_command` in `src/app.rs` so stderr diagnostics also show `--yolo`:

```rust
"$ codex exec resume --yolo --json -o {}{} {}"
"$ codex exec --yolo --json -o {}{} -s {}"
```

- [ ] **Step 7: Create native approval module**

Create `src/native_approval.rs` with these public types and functions:

```rust
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

pub fn is_supported_approval_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
    )
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
    let prompt = format_native_approval_prompt(&approval_id, &request_method, command.as_deref(), cwd.as_deref(), &summary);
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
    let deadline = Instant::now() + effective_approval_timeout(config);
    let decision_path = native_decision_path(config, &record.approval_id);
    loop {
        if let Ok(input) = fs::read_to_string(&decision_path) {
            let decision: NativeApprovalDecisionRecord = serde_json::from_str(&input).map_err(|err| {
                format!("invalid native approval decision {}: {err}", decision_path.display())
            })?;
            cleanup_native_approval(config, &record.approval_id);
            return Ok(decision.decision);
        }
        if Instant::now() >= deadline {
            cleanup_native_approval(config, &record.approval_id);
            return Ok(NativeApprovalDecision::Deny);
        }
        thread::sleep(Duration::from_millis(500));
    }
}

pub fn approval_response_for_decision(
    method: &str,
    request_params: &Value,
    decision: NativeApprovalDecision,
) -> Value {
    match method {
        "item/commandExecution/requestApproval" => command_approval_response(request_params, decision),
        "item/fileChange/requestApproval" => file_change_approval_response(decision),
        "item/permissions/requestApproval" => permissions_approval_response(request_params, decision),
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
```

The private helpers in the same file must:

```rust
fn native_approval_path(config: &WecodeConfig, approval_id: &str) -> PathBuf {
    native_approvals_dir(config).join(format!("{approval_id}.json"))
}

fn native_decision_path(config: &WecodeConfig, approval_id: &str) -> PathBuf {
    native_approvals_dir(config).join(format!("{approval_id}.decision.json"))
}

fn command_approval_response(request_params: &Value, decision: NativeApprovalDecision) -> Value {
    match decision {
        NativeApprovalDecision::Approve => json!({ "decision": command_accept_decision(request_params) }),
        NativeApprovalDecision::Deny => json!({ "decision": command_rejection_decision(request_params, "decline") }),
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
```

Use `read_string`, `approval_command`, `approval_summary`, `format_native_approval_prompt`, `effective_approval_timeout`, `write_native_approval_record`, `cleanup_native_approval`, `current_millis`, and `create_approval_id` as private helpers. Keep summaries compact: command, cwd, requested permissions, and file-change preview from top-level JSON fields only.

- [ ] **Step 8: Export module**

Modify `src/lib.rs`:

```rust
pub mod native_approval;
```

- [ ] **Step 9: Run tests**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test defaults_codex_remote_approval_timeout parses_codex_remote_approval_timeout codex_backend_exec_fallback_uses_yolo command_approval_accept_respects_available_decisions command_approval_denies_with_decline permissions_approval_grants_requested_network_and_filesystem_for_turn native_record_is_written_under_native_approval_dir
```

Expected: all listed tests pass.

- [ ] **Step 10: Commit**

```bash
git add src/config.rs src/lib.rs src/native_approval.rs src/backend.rs src/app.rs tests/config.rs tests/codex_backend.rs tests/native_approval.rs
git commit -m "feat: add codex native approval state"
```

---

### Task 2: Approve/Deny Commands For Native Decisions

**Files:**
- Modify: `src/app.rs`
- Test: `tests/codex_backend.rs`

- [ ] **Step 1: Write failing integration tests**

Add to `tests/codex_backend.rs`:

```rust
#[test]
fn codex_backend_approve_writes_native_approval_decision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    fs::write(
        &config_path,
        format!(
            r#"{{"openclaw":{{"stateDir":{}}},"codex":{{"transport":"remote-strict"}}}}"#,
            serde_json::to_string(&state_dir.display().to_string()).expect("state json")
        ),
    )
    .expect("write config");
    let native_dir = state_dir.join("approvals").join("native");
    fs::create_dir_all(&native_dir).expect("native dir");
    fs::write(
        native_dir.join("appr-native.json"),
        r#"{
          "kind": "codex-native",
          "approval_id": "appr-native",
          "request_method": "item/commandExecution/requestApproval",
          "jsonrpc_id": 99,
          "thread_id": "thread-1",
          "turn_id": "turn-1",
          "command": "cargo test",
          "cwd": "/tmp/project",
          "summary": ["Command: cargo test"],
          "prompt": "Codex requests permission.",
          "request_params": {"command":"cargo test"},
          "created_at_millis": 1,
          "expires_at_millis": 9999999999999
        }"#,
    )
    .expect("pending native");

    let approved = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":approve",
            "appr-native",
        ])
        .output()
        .expect("approve");

    assert!(
        approved.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    let decision = fs::read_to_string(native_dir.join("appr-native.decision.json"))
        .expect("decision");
    assert!(decision.contains(r#""decision": "approve""#), "{decision}");
    assert!(String::from_utf8_lossy(&approved.stdout).contains("Approved Codex approval appr-native"));
}

#[test]
fn codex_backend_deny_writes_native_approval_decision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    fs::write(
        &config_path,
        format!(
            r#"{{"openclaw":{{"stateDir":{}}},"codex":{{"transport":"remote-strict"}}}}"#,
            serde_json::to_string(&state_dir.display().to_string()).expect("state json")
        ),
    )
    .expect("write config");
    let native_dir = state_dir.join("approvals").join("native");
    fs::create_dir_all(&native_dir).expect("native dir");
    fs::write(
        native_dir.join("appr-native.json"),
        r#"{
          "kind": "codex-native",
          "approval_id": "appr-native",
          "request_method": "item/fileChange/requestApproval",
          "jsonrpc_id": 99,
          "thread_id": "thread-1",
          "turn_id": "turn-1",
          "command": null,
          "cwd": "/tmp/project",
          "summary": ["File change requested"],
          "prompt": "Codex requests permission.",
          "request_params": {},
          "created_at_millis": 1,
          "expires_at_millis": 9999999999999
        }"#,
    )
    .expect("pending native");

    let denied = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":deny",
            "appr-native",
        ])
        .output()
        .expect("deny");

    assert!(
        denied.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    let decision = fs::read_to_string(native_dir.join("appr-native.decision.json"))
        .expect("decision");
    assert!(decision.contains(r#""decision": "deny""#), "{decision}");
    assert!(String::from_utf8_lossy(&denied.stdout).contains("Denied Codex approval appr-native"));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_backend_approve_writes_native_approval_decision codex_backend_deny_writes_native_approval_decision
```

Expected: failure because `:approve` and `:deny` only know old custom approval files.

- [ ] **Step 3: Route approve and deny commands**

Modify imports in `src/app.rs`:

```rust
use wecode::native_approval::{self, NativeApprovalDecision};
```

Change `BackendInput::Approve` handling to a new wrapper:

```rust
BackendInput::Approve { approval_id } => approve_approval(
    &config,
    &approval_id,
    resume_session_id,
    model,
    jsonl,
    Some(&flow_run_id),
),
```

Add this wrapper near `run_approved_prompt`:

```rust
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
            &format!("Approved Codex approval {approval_id}. Codex will continue in the original turn."),
            jsonl,
        ),
        Err(_) => emit_local_markdown(&format!("Approval {approval_id} was not found."), jsonl),
    }
}
```

Modify `deny_approval`:

```rust
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
            &format!("Denied Codex approval {approval_id}. Codex will continue in the original turn."),
            jsonl,
        ),
        Err(_) => emit_local_markdown(&format!("Approval {approval_id} was not found."), jsonl),
    }
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_backend_approve_writes_native_approval_decision codex_backend_deny_writes_native_approval_decision codex_backend_defers_confirmed_command_until_approved
```

Expected: native approve/deny tests pass and the old custom approval test still passes.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs tests/codex_backend.rs
git commit -m "feat: route channel approvals to codex native decisions"
```

---

### Task 3: Remote JSON-RPC Approval Bridge

**Files:**
- Modify: `src/codex_remote.rs`
- Modify: `src/app.rs`
- Test: `tests/codex_backend.rs`

- [ ] **Step 1: Write failing remote approve test**

Add a fake remote proxy helper to `tests/codex_backend.rs`:

```rust
fn write_fake_remote_proxy_with_command_approval(
    proxy_path: &std::path::Path,
    calls_path: &std::path::Path,
    thread_id: &str,
) {
    fs::write(
        proxy_path,
        format!(
            r#"#!/bin/sh
thread_id={thread_id}
while IFS= read -r line; do
  printf '%s\n' "$line" >> {calls}
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"codexHome":"/tmp/codex","platformFamily":"unix","platformOs":"macos","userAgent":"fake"}}}}\n' "$id"
      ;;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"%s"}},"cwd":"/tmp","model":"fake","modelProvider":"fake","approvalPolicy":"never","approvalsReviewer":"user","sandbox":{{"mode":"workspace-write"}}}}}}\n' "$id" "$thread_id"
      ;;
    *'"method":"turn/start"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"turn":{{"id":"turn-1","status":"inProgress","items":[]}}}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","id":99,"method":"item/commandExecution/requestApproval","params":{{"threadId":"%s","turnId":"turn-1","command":"cargo test","cwd":"/tmp/project","availableDecisions":["decline","accept"]}}}}\n' "$thread_id"
      ;;
    *'"id":99*'"result"'*)
      printf '{{"jsonrpc":"2.0","method":"item/completed","params":{{"threadId":"%s","turnId":"turn-1","item":{{"type":"agentMessage","id":"msg-final","text":"approval ok","phase":"final_answer","memoryCitation":null}}}}}}\n' "$thread_id"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"%s","turn":{{"id":"turn-1","status":"completed","items":[],"itemsView":"notLoaded"}}}}}}\n' "$thread_id"
      ;;
  esac
done
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 calls path")),
            thread_id = shell_quote(thread_id),
        ),
    )
    .expect("write fake remote proxy");
    let mut permissions = fs::metadata(proxy_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(proxy_path, permissions).expect("chmod fake remote proxy");
}
```

Add the test:

```rust
#[test]
fn codex_backend_remote_approval_waits_for_wechat_approve() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proxy_path = temp.path().join("codex-remote-proxy");
    let calls_path = temp.path().join("remote-calls.txt");
    let state_dir = temp.path().join("state");
    write_fake_remote_proxy_with_command_approval(&proxy_path, &calls_path, "approval-thread");
    let config_path = temp.path().join("wecode.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw":{{"stateDir":{},"cliNoOutputTimeoutMs":900000}},
              "codex":{{
                "transport":"remote-strict",
                "remote":{{
                  "autoStart":false,
                  "proxyCommand":{},
                  "fallbackProxyCommand":"",
                  "approvalTimeoutSeconds":30
                }}
              }}
            }}"#,
            serde_json::to_string(&state_dir.display().to_string()).expect("state json"),
            serde_json::to_string(&proxy_path.display().to_string()).expect("proxy json")
        ),
    )
    .expect("write config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "please run tests",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn backend");

    let approval_file = wait_for_first_native_approval_file(&state_dir);
    let approval_id = approval_file
        .file_stem()
        .expect("stem")
        .to_str()
        .expect("utf-8")
        .to_string();

    let approved = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":approve",
            &approval_id,
        ])
        .output()
        .expect("approve");
    assert!(approved.status.success());

    let output = child.wait_with_output().expect("wait backend");
    assert!(output.status.success(), "stderr unavailable for spawned process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Codex requests permission"), "{stdout}");
    assert!(stdout.contains(":approve"), "{stdout}");
    assert!(stdout.contains("approval ok"), "{stdout}");
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains(r#""id":99"#), "{calls}");
    assert!(calls.contains(r#""decision":"accept""#), "{calls}");
}
```

Add helper:

```rust
fn wait_for_first_native_approval_file(state_dir: &std::path::Path) -> std::path::PathBuf {
    let native_dir = state_dir.join("approvals").join("native");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(entries) = fs::read_dir(&native_dir) {
            if let Some(path) = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && !path.to_string_lossy().ends_with(".decision.json"))
            {
                return path;
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("native approval file was not written under {}", native_dir.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
```

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_backend_remote_approval_waits_for_wechat_approve -- --nocapture
```

Expected: failure because the current remote adapter auto-declines requestApproval.

- [ ] **Step 3: Add approval event variant**

Modify `src/codex_remote.rs`:

```rust
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
}
```

- [ ] **Step 4: Replace auto-decline with approval handler**

Modify imports in `src/codex_remote.rs`:

```rust
use crate::{
    config::{codex_model_from_openclaw_model, WecodeConfig},
    native_approval::{
        self, approval_response_for_decision, create_native_approval_record,
        wait_for_native_approval_decision,
    },
    paths::expand_tilde,
};
```

Add a method on `JsonRpcClient`:

```rust
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
```

Add a helper near `run_codex_remote_turn_with_proxy_command`:

```rust
fn handle_native_approval_request(
    config: &WecodeConfig,
    client: &mut JsonRpcClient,
    message: &Value,
    fallback_thread_id: &str,
    event_handler: &mut impl FnMut(CodexRemoteRunEvent) -> Result<(), String>,
) -> Result<(), String> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if !native_approval::is_supported_approval_method(method) {
        return client.respond_to_server_request(
            message,
            json!({ "decision": "decline", "reason": "unsupported Codex approval method" }),
        );
    }
    let record = create_native_approval_record(config, message)?;
    event_handler(CodexRemoteRunEvent::NativeApprovalRequested {
        thread_id: record
            .thread_id
            .clone()
            .unwrap_or_else(|| fallback_thread_id.to_string()),
        approval_id: record.approval_id.clone(),
        prompt: record.prompt.clone(),
    })?;
    let decision = wait_for_native_approval_decision(config, &record)?;
    let result = approval_response_for_decision(
        &record.request_method,
        &record.request_params,
        decision,
    );
    client.respond_to_server_request(message, result)
}
```

Refactor `JsonRpcClient::request` into two methods:

```rust
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
    self.write_message(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    }))?;

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
```

Use `request_with_unhandled_server_requests` for `turn/start`, passing `handle_native_approval_request`. In the main loop, replace `client.decline_server_request(&message)?` with `handle_native_approval_request(...)`.

- [ ] **Step 5: Emit native approval prompts from app**

Modify `run_remote_backend_with_fallback` event handler in `src/app.rs`:

```rust
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
```

- [ ] **Step 6: Run remote approval test**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_backend_remote_approval_waits_for_wechat_approve -- --nocapture
```

Expected: test passes and fake proxy call log contains `"decision":"accept"`.

- [ ] **Step 7: Commit**

```bash
git add src/codex_remote.rs src/app.rs tests/codex_backend.rs
git commit -m "feat: bridge codex remote approvals to channel"
```

---

### Task 4: Avoid OpenClaw Serialization Deadlock

**Files:**
- Create: `src/run_lock.rs`
- Modify: `src/lib.rs`
- Modify: `src/app.rs`
- Modify: `src/openclaw.rs`
- Test: `tests/bootstrap.rs`
- Test: `tests/run_lock.rs`

- [ ] **Step 1: Write failing bootstrap test change**

Modify `tests/bootstrap.rs` expected CLI backend JSON from:

```json
"serialize":true
```

to:

```json
"serialize":false
```

- [ ] **Step 2: Write run-lock tests**

Create `tests/run_lock.rs`:

```rust
use std::fs;

use tempfile::tempdir;
use wecode::{read_config_str, run_lock::try_acquire_codex_run_lock};

#[test]
fn codex_run_lock_blocks_second_owner() {
    let temp = tempdir().expect("tempdir");
    let state_dir = temp.path().join("state");
    let config = read_config_str(&format!(
        r#"{{"openclaw":{{"stateDir":{}}}}}"#,
        serde_json::to_string(&state_dir.display().to_string()).expect("state json")
    ))
    .expect("config");

    let first = try_acquire_codex_run_lock(&config, "thread-1").expect("first lock");
    let second = try_acquire_codex_run_lock(&config, "thread-1");
    assert!(second.is_err());

    drop(first);
    assert!(try_acquire_codex_run_lock(&config, "thread-1").is_ok());
    assert!(fs::read_dir(state_dir.join("locks")).is_ok());
}
```

- [ ] **Step 3: Run tests to verify failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_run_lock_blocks_second_owner bootstrap_plan_configures_cli_backend
```

Expected: compile failure because `run_lock` does not exist, plus bootstrap expectation mismatch.

- [ ] **Step 4: Implement file-backed run lock**

Create `src/run_lock.rs`:

```rust
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
};

use crate::{config::WecodeConfig, paths::expand_tilde};

pub struct CodexRunLock {
    path: PathBuf,
}

impl Drop for CodexRunLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn try_acquire_codex_run_lock(
    config: &WecodeConfig,
    key: &str,
) -> Result<CodexRunLock, String> {
    let lock_dir = PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("locks");
    fs::create_dir_all(&lock_dir)
        .map_err(|err| format!("failed to create Wecode lock dir: {err}"))?;
    let path = lock_dir.join(format!("codex-run-{}.lock", stable_key_hash(key)));
    let content = format!("pid={}\nkey={key}\n", std::process::id());
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(content.as_bytes())
                .map_err(|err| format!("failed to write Wecode run lock {}: {err}", path.display()))?;
            Ok(CodexRunLock { path })
        }
        Err(err) => Err(format!(
            "Codex is already running for this session or project. Try again after the current turn finishes, or approve/deny any pending Codex approval. Lock: {}. Cause: {err}",
            path.display()
        )),
    }
}

fn stable_key_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
```

Export it from `src/lib.rs`:

```rust
pub mod run_lock;
```

- [ ] **Step 5: Apply run lock only to Codex-running paths**

Modify `run_backend_prompt` in `src/app.rs`:

```rust
let lock_key = effective_resume_session_id
    .map(str::to_string)
    .unwrap_or_else(|| {
        codex_target_cwd(config)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "unknown-cwd".to_string())
    });
let _run_lock = wecode::run_lock::try_acquire_codex_run_lock(config, &lock_key)?;
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
```

Do not acquire this lock in `BackendInput::Approve`, `BackendInput::Deny`, `BackendInput::Status`, `BackendInput::Help`, `BackendInput::Pwd`, `BackendInput::Ls`, `BackendInput::Cat`, `BackendInput::Shell`, `BackendInput::ModelShow`, `BackendInput::ModelsList`, or `BackendInput::ModelSet`.

- [ ] **Step 6: Disable OpenClaw backend serialization**

Modify `src/openclaw.rs` in `cli_backend_config_json`:

```rust
"serialize": false,
```

- [ ] **Step 7: Run tests**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_run_lock_blocks_second_owner bootstrap_plan_configures_cli_backend
```

Expected: tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/run_lock.rs src/lib.rs src/app.rs src/openclaw.rs tests/bootstrap.rs tests/run_lock.rs
git commit -m "fix: avoid approval deadlock in openclaw backend"
```

---

### Task 5: Timeout, Status, And Documentation

**Files:**
- Modify: `src/native_approval.rs`
- Modify: `src/app.rs`
- Modify: `README.md`
- Test: `tests/codex_backend.rs`

- [ ] **Step 1: Write timeout test**

Add to `tests/codex_backend.rs` a fake proxy variant that sends a file-change request and completes after receiving the denial. Configure `"approvalTimeoutSeconds":1`. The assertion must verify fake proxy received `"decision":"decline"` and the process exits successfully.

Use this test body:

```rust
#[test]
fn codex_backend_remote_approval_times_out_with_decline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let proxy_path = temp.path().join("codex-remote-proxy");
    let calls_path = temp.path().join("remote-calls.txt");
    let state_dir = temp.path().join("state");
    write_fake_remote_proxy_with_file_approval(&proxy_path, &calls_path, "timeout-thread");
    let config_path = temp.path().join("wecode.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw":{{"stateDir":{},"cliNoOutputTimeoutMs":900000}},
              "codex":{{
                "transport":"remote-strict",
                "remote":{{
                  "autoStart":false,
                  "proxyCommand":{},
                  "fallbackProxyCommand":"",
                  "approvalTimeoutSeconds":1
                }}
              }}
            }}"#,
            serde_json::to_string(&state_dir.display().to_string()).expect("state json"),
            serde_json::to_string(&proxy_path.display().to_string()).expect("proxy json")
        ),
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "edit a file",
        ])
        .output()
        .expect("backend");

    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains(r#""decision":"decline""#), "{calls}");
    let native_dir = state_dir.join("approvals").join("native");
    let pending = fs::read_dir(native_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                .filter(|path| !path.to_string_lossy().ends_with(".decision.json"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(pending, 0);
}
```

- [ ] **Step 2: Add status pending count for native approvals**

Modify `weixin_status_message` in `src/app.rs` so `pending approvals` counts:

```rust
let custom_pending = fs::read_dir(approvals_dir(config))
    .map(|entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count()
    })
    .unwrap_or(0);
let native_pending = fs::read_dir(wecode::native_approval::native_approvals_dir(config))
    .map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .filter(|path| !path.to_string_lossy().ends_with(".decision.json"))
            .count()
    })
    .unwrap_or(0);
let pending = custom_pending + native_pending;
```

- [ ] **Step 3: Update README**

Replace the limitation:

```markdown
- Codex 原生工具审批请求在 remote v1 中会被协议级拒绝，避免后端进程悬挂；现在的微信审批仍是 `wecode` 自己的确认队列，适合保护自定义高风险命令。
```

with:

```markdown
- remote 模式会把 Codex app-server 原生审批请求转成微信/飞书可见的 `appr-...` 审批提示。发送 `:approve appr-...` 会批准当前 Codex turn 的这一次请求，发送 `:deny appr-...` 会拒绝。这个能力只覆盖 remote/app-server transport；`codex exec` fallback 仍不能做交互式审批桥接。
- 如果你已经运行过旧版 `wecode configure-codex`，升级后重新运行一次 `wecode configure-codex`，让 OpenClaw backend 配置从 `serialize: true` 更新为 `serialize: false`。Wecode 会用自己的运行锁串行化 Codex turn，并允许 `:approve` / `:deny` 在等待审批时进入。
```

- [ ] **Step 4: Run focused tests**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test codex_backend_remote_approval_times_out_with_decline codex_backend_remote_approval_waits_for_wechat_approve codex_backend_defers_confirmed_command_until_approved
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/native_approval.rs src/app.rs README.md tests/codex_backend.rs
git commit -m "docs: document codex native approval bridge"
```

---

### Task 6: Full Verification

**Files:**
- No source edits unless verification exposes a bug.

- [ ] **Step 1: Format check**

Run:

```bash
cargo fmt --check
```

Expected: success.

- [ ] **Step 2: Full test suite**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo test
```

Expected: success.

- [ ] **Step 3: Validate example config**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo run -- config validate examples/wecode.config.json
```

Expected: `valid config: examples/wecode.config.json`.

- [ ] **Step 4: Dry-run bootstrap config**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/wecode-target cargo run -- bootstrap --dry-run --install-openclaw
```

Expected: generated backend config includes `"serialize":false`, `sessionIdFields:["thread_id"]`, `output:"jsonl"`, and `resumeOutput:"jsonl"`.

- [ ] **Step 5: Manual WeChat test after user approval**

Only after the user confirms their private OpenClaw Gateway is running, run:

```bash
node scripts/openclaw-agent-smoke.mjs
```

Expected: existing smoke test still succeeds. Then ask the user to send a WeChat prompt that forces Codex to request approval, such as installing a package or writing outside the sandbox. Watch the backend log for:

```text
Codex requests permission
:approve appr-
```

Expected user flow:

```text
User sends normal task
Wecode emits approval prompt with appr-...
User sends :approve appr-...
Wecode writes native decision file
Original Codex turn continues and returns final assistant output
```

- [ ] **Step 6: Final status**

Run:

```bash
git status --short --branch
```

Expected: only intentional committed changes, or a clean tracked worktree plus unrelated user files that were present before.

---

## Self-Review

- Spec coverage: the plan captures Codex native approval requests, sends WeChat/Feishu visible approval prompts, unblocks the original JSON-RPC request on `:approve` / `:deny`, keeps old custom approvals, handles timeout cleanup, and documents remote-only scope.
- Deadlock coverage: the plan explicitly changes generated OpenClaw backend serialization to `false` and replaces it with a Wecode lock that approval commands bypass.
- Protocol coverage: approve responses use `accept`, not `approve`; permissions approval echoes requested `network` and `fileSystem` permissions with `scope: "turn"`.
- Test coverage: protocol unit tests, channel command integration tests, remote fake-proxy approval tests, timeout test, bootstrap config test, legacy custom approval test, full Rust verification.

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
    assert_eq!(
        record.request_method,
        "item/commandExecution/requestApproval"
    );
    assert!(native_approvals_dir(&config)
        .join(format!("{}.json", record.approval_id))
        .exists());
    assert!(record.prompt.contains(":approve "));
    assert!(record.prompt.contains(":deny "));
    assert!(record.prompt.contains("cargo test"));
}

use std::fs;

use wecode::{
    default_config, prepare_backend_input, prepare_backend_prompt, read_config_str,
    render_command_input, BackendInput,
};

#[test]
fn renders_custom_command_prompt_from_prefix_match() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "review",
              "prefix": ":review ",
              "prompt": "Review this path: {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let rendered = render_command_input(&cfg, ":review src/main.rs").expect("command should match");

    assert_eq!(rendered.command_name, "review");
    assert_eq!(rendered.prompt, "Review this path: src/main.rs");
    assert!(rendered.require_confirm);
    assert!(render_command_input(&cfg, "/unknown").is_none());
}

#[test]
fn backend_prompt_applies_custom_command_template() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "review",
              "prefix": ":review ",
              "prompt": "Review this path: {{message}}",
              "requireConfirm": false
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let prompt = prepare_backend_prompt(&cfg, ":review src/main.rs").expect("prompt");

    assert_eq!(prompt, "Review this path: src/main.rs");
}

#[test]
fn backend_prompt_rejects_custom_command_that_requires_confirmation() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "deploy",
              "prefix": ":deploy ",
              "prompt": "Deploy {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let error = prepare_backend_prompt(&cfg, ":deploy production").expect_err("blocked");

    assert!(error.contains("requires confirmation"));
}

#[test]
fn backend_input_rejects_removed_sessions_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    let error = prepare_backend_input(&cfg, ":sessions").expect_err("sessions removed");

    assert!(error.contains("`:sessions` was removed"));
}

#[test]
fn backend_input_recognizes_resume_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, ":resume").expect("resume latest"),
        BackendInput::Resume { session_id: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":resume 019e746e-4c43-74c2-b47a-424fd4f025c7")
            .expect("resume explicit"),
        BackendInput::Resume {
            session_id: Some(session_id)
        } if session_id == "019e746e-4c43-74c2-b47a-424fd4f025c7"
    ));
}

#[test]
fn backend_input_recognizes_fresh_thread_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, ":fresh").expect("fresh"),
        BackendInput::Fresh { prompt: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":fresh start clean").expect("fresh prompt"),
        BackendInput::Fresh {
            prompt: Some(prompt)
        } if prompt == "start clean"
    ));
}

#[test]
fn backend_input_converts_colon_codex_commands_to_slash_prompts() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    for (command, expected) in [
        (":init", "/init"),
        (":init notes", "/init notes"),
        (":new", "/new"),
        (":new investigate bug", "/new investigate bug"),
        (":compact", "/compact"),
        (":compact keep decisions", "/compact keep decisions"),
        (":plan", "/plan"),
        (":plan refactor this", "/plan refactor this"),
        (":goal", "/goal"),
        (":goal ship this", "/goal ship this"),
        (":agent", "/agent"),
        (":agent split tasks", "/agent split tasks"),
        (":side", "/side"),
        (":side explain risk", "/side explain risk"),
    ] {
        assert!(
            matches!(
                prepare_backend_input(&cfg, command).expect(command),
                BackendInput::Prompt(prompt) if prompt == expected
            ),
            "{command} should be sent to Codex as {expected}"
        );
    }
}

#[test]
fn backend_input_recognizes_control_commands() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, ":help").expect("help"),
        BackendInput::Help
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":status").expect("status"),
        BackendInput::Status
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":approve abc123").expect("approve"),
        BackendInput::Approve {
            approval_id: Some(approval_id),
        } if approval_id == "abc123"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":yes abc123").expect("yes"),
        BackendInput::Approve {
            approval_id: Some(approval_id),
        } if approval_id == "abc123"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":yes").expect("bare yes"),
        BackendInput::Approve { approval_id: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":deny abc123").expect("deny"),
        BackendInput::Deny {
            approval_id: Some(approval_id),
        } if approval_id == "abc123"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":no abc123").expect("no"),
        BackendInput::Deny {
            approval_id: Some(approval_id),
        } if approval_id == "abc123"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":no").expect("bare no"),
        BackendInput::Deny { approval_id: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":pwd").expect("pwd"),
        BackendInput::Pwd
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":ls src").expect("ls"),
        BackendInput::Ls { path } if path == "src"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":cat README.md").expect("cat"),
        BackendInput::Cat { path } if path == "README.md"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":cd /tmp").expect("cd"),
        BackendInput::Cd { path } if path == "/tmp"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":shell ls -al").expect("shell"),
        BackendInput::Shell { command } if command == "ls -al"
    ));
    assert_eq!(
        prepare_backend_input(&cfg, ":shell"),
        Err(":shell expects a command".to_string())
    );
}

#[test]
fn backend_input_treats_plain_yes_no_as_approval_only_when_approval_is_pending() {
    let temp = tempfile::tempdir().expect("tempdir");
    let empty_state_dir = temp.path().join("empty-state");
    let cfg = read_config_str(&format!(
        r#"{{"openclaw":{{"stateDir":{}}},"commands":[]}}"#,
        serde_json::to_string(&empty_state_dir.display().to_string()).expect("state json")
    ))
    .expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, "yes").expect("plain yes without pending approval"),
        BackendInput::Prompt(prompt) if prompt == "yes"
    ));
    let expired_native_dir = empty_state_dir.join("approvals").join("native");
    fs::create_dir_all(&expired_native_dir).expect("expired native dir");
    fs::write(
        expired_native_dir.join("appr-expired.json"),
        r#"{"approval_id":"appr-expired","expires_at_millis":1}"#,
    )
    .expect("expired approval");
    assert!(matches!(
        prepare_backend_input(&cfg, "yes").expect("plain yes with expired approval"),
        BackendInput::Prompt(prompt) if prompt == "yes"
    ));

    let state_dir = temp.path().join("state");
    let cfg = read_config_str(&format!(
        r#"{{"openclaw":{{"stateDir":{}}},"commands":[]}}"#,
        serde_json::to_string(&state_dir.display().to_string()).expect("state json")
    ))
    .expect("config should parse");
    let native_dir = state_dir.join("approvals").join("native");
    fs::create_dir_all(&native_dir).expect("native dir");
    fs::write(native_dir.join("appr-one.json"), "{}").expect("pending native approval");

    assert!(matches!(
        prepare_backend_input(&cfg, "yes").expect("plain yes with pending approval"),
        BackendInput::Approve { approval_id: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "NO").expect("plain no with pending approval"),
        BackendInput::Deny { approval_id: None }
    ));
}

#[test]
fn backend_input_recognizes_metadata_wrapped_control_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"Conversation info (untrusted metadata):
```json
{
  "chat_id": "o9cq805CIQyEJ1pliCh0GGdeTy98@im.wechat",
  "message_id": "openclaw-weixin:1780220368799-642cfab6",
  "timestamp": "Sun 2026-05-31 17:39:28 GMT+8"
}
```

:help"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("metadata wrapped help"),
        BackendInput::Help
    ));
}

#[test]
fn backend_input_does_not_treat_generic_json_string_as_channel_message() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"":help""#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("generic json string"),
        BackendInput::Prompt(prompt) if prompt == r#"":help""#
    ));
}

#[test]
fn backend_input_does_not_guess_generic_json_object_message_fields() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"{"message":":cd ~/Github/zcode"}"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("generic json object"),
        BackendInput::Prompt(prompt) if prompt == r#"{"message":":cd ~/Github/zcode"}"#
    ));
}

#[test]
fn backend_input_extracts_feishu_text_event_message() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"{
      "schema": "2.0",
      "header": { "event_type": "im.message.receive_v1" },
      "event": {
        "sender": { "sender_id": { "open_id": "ou_08e494561f9ff0e2bd8015472c28e6e5" } },
        "message": {
          "message_id": "om_x100b6e977ba6b4b0b34edb547991066",
          "chat_id": "oc_xxx",
          "chat_type": "p2p",
          "message_type": "text",
          "content": "{\"text\":\"请总结这个 repo\"}"
        }
      }
    }"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("feishu event prompt"),
        BackendInput::Prompt(prompt) if prompt == "请总结这个 repo"
    ));
}

#[test]
fn backend_input_extracts_weixin_getupdates_text_message() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"{
      "ret": 0,
      "msgs": [
        {
          "message_id": 1780220368799,
          "from_user_id": "o9cq805CIQyEJ1pliCh0GGdeTy98@im.wechat",
          "to_user_id": "bot@im.wechat",
          "message_type": 1,
          "item_list": [
            { "type": 1, "text_item": { "text": "帮我看 README" } }
          ]
        }
      ]
    }"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("weixin getupdates prompt"),
        BackendInput::Prompt(prompt) if prompt == "帮我看 README"
    ));
}

#[test]
fn backend_input_extracts_decorated_openclaw_plain_message_for_codex() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"Conversation info (untrusted metadata):
```json
{"message_id":"om_x100b6e977ba6b4b0b34edb547991066"}
```

[message_id: om_x100b6e977ba6b4b0b34edb547991066]
ou_08e494561f9ff0e2bd8015472c28e6e5: 解释 src/commands.rs 的输入处理"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("decorated plain prompt"),
        BackendInput::Prompt(prompt) if prompt == "解释 src/commands.rs 的输入处理"
    ));
}

#[test]
fn backend_input_recognizes_decorated_openclaw_control_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"Conversation info (untrusted metadata):
```json
{"message_id":"om_x100b6e977ba6b4b0b34edb547991066"}
```

[message_id: om_x100b6e977ba6b4b0b34edb547991066]
ou_08e494561f9ff0e2bd8015472c28e6e5: :cd ~/Github/zcode"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("decorated cd"),
        BackendInput::Cd { path } if path == "~/Github/zcode"
    ));
}

#[test]
fn backend_input_converts_decorated_openclaw_colon_command_to_slash_prompt() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"[message_id: om_x100b6e977ba6b4b0b34edb547991066]
ou_08e494561f9ff0e2bd8015472c28e6e5: :compact keep decisions"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("decorated compact"),
        BackendInput::Prompt(prompt) if prompt == "/compact keep decisions"
    ));
}

#[test]
fn backend_input_preserves_decorated_openclaw_multiline_feishu_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"Conversation info (untrusted metadata):
```json
{"message_id":"om_x100b6e977ba6b4b0b34edb547991066"}
```

[message_id: om_x100b6e977ba6b4b0b34edb547991066]
ou_08e494561f9ff0e2bd8015472c28e6e5: :compact keep decisions
保留项目边界
保留当前计划"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("decorated multiline compact"),
        BackendInput::Prompt(prompt)
            if prompt == "/compact keep decisions\n保留项目边界\n保留当前计划"
    ));
}

#[test]
fn backend_input_preserves_decorated_openclaw_multiline_weixin_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");
    let input = r#"[message_id: wx-msg-1]
o9cq805CIQyEJ1pliCh0GGdeTy98@im.wechat: :plan 兼容微信多行消息
- 提取 sender 后面的正文
- 保留后续换行"#;

    assert!(matches!(
        prepare_backend_input(&cfg, input).expect("decorated multiline weixin plan"),
        BackendInput::Prompt(prompt)
            if prompt == "/plan 兼容微信多行消息\n- 提取 sender 后面的正文\n- 保留后续换行"
    ));
}

#[test]
fn backend_input_recognizes_codex_builtin_commands() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, ":diff").expect("diff"),
        BackendInput::Diff
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":model").expect("model show"),
        BackendInput::ModelShow
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":models").expect("models list"),
        BackendInput::ModelsList
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":model gpt-5.5").expect("model set"),
        BackendInput::ModelSet { model } if model == "gpt-5.5"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":review").expect("review"),
        BackendInput::Review { instructions: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":review focus security").expect("review with instructions"),
        BackendInput::Review { instructions: Some(instructions) } if instructions == "focus security"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":report").expect("report"),
        BackendInput::Prompt(prompt)
            if prompt.contains("side analysis") && prompt.contains("User request: 任务状态")
    ));

    assert!(matches!(
        prepare_backend_input(&cfg, ":report note").expect("report"),
        BackendInput::Prompt(prompt)
            if prompt.contains("side analysis") && prompt.contains("补充说明: note")
    ));
}

#[test]
fn custom_commands_can_override_codex_builtin_command_prompts() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "plan-override",
              "prefix": ":plan ",
              "prompt": "Custom plan prompt: {{message}}",
              "requireConfirm": false
            },
            {
              "name": "init-guard",
              "prefix": ":init ",
              "prompt": "Guarded init: {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, ":plan module split").expect("plan"),
        BackendInput::Prompt(prompt) if prompt == "Custom plan prompt: module split"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, ":init repo notes").expect("init"),
        BackendInput::ApprovalRequired { command_name, prompt }
            if command_name == "init-guard" && prompt == "Guarded init: repo notes"
    ));
}

#[test]
fn backend_input_returns_approval_request_for_confirmed_command() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "deploy",
              "prefix": ":deploy ",
              "prompt": "Deploy {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let input = prepare_backend_input(&cfg, ":deploy production").expect("backend input");

    assert!(matches!(
        input,
        BackendInput::ApprovalRequired { command_name, prompt }
            if command_name == "deploy" && prompt == "Deploy production"
    ));
}

#[test]
fn default_commands_include_common_codex_workflows() {
    let cfg = default_config();
    let prefixes = cfg
        .commands
        .iter()
        .map(|command| command.prefix.as_str())
        .collect::<Vec<_>>();

    for prefix in [
        ":codex ",
        ":explain ",
        ":fix ",
        ":test ",
        ":debug ",
        ":refactor ",
        ":docs ",
    ] {
        assert!(prefixes.contains(&prefix), "missing prefix {prefix}");
    }
}

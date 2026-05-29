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
              "prefix": "/review ",
              "prompt": "Review this path: {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let rendered = render_command_input(&cfg, "/review src/main.rs").expect("command should match");

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
              "prefix": "/review ",
              "prompt": "Review this path: {{message}}",
              "requireConfirm": false
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let prompt = prepare_backend_prompt(&cfg, "/review src/main.rs").expect("prompt");

    assert_eq!(prompt, "Review this path: src/main.rs");
}

#[test]
fn backend_prompt_rejects_custom_command_that_requires_confirmation() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "deploy",
              "prefix": "/deploy ",
              "prompt": "Deploy {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let error = prepare_backend_prompt(&cfg, "/deploy production").expect_err("blocked");

    assert!(error.contains("requires confirmation"));
}

#[test]
fn backend_input_recognizes_resume_list_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    let input = prepare_backend_input(&cfg, "/resume").expect("backend input");

    assert!(input.is_resume_list());
}

#[test]
fn backend_input_recognizes_resume_bind_command() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    let input = prepare_backend_input(&cfg, "/resume 019e746e-4c43-74c2-b47a-424fd4f025c7")
        .expect("backend input");

    assert_eq!(
        input.resume_session_id(),
        Some("019e746e-4c43-74c2-b47a-424fd4f025c7")
    );
}

#[test]
fn backend_input_recognizes_control_commands() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, "/help").expect("help"),
        BackendInput::Help
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/status").expect("status"),
        BackendInput::Status
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/sessions").expect("sessions"),
        BackendInput::ResumeList
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/approve abc123").expect("approve"),
        BackendInput::Approve { approval_id } if approval_id == "abc123"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/deny abc123").expect("deny"),
        BackendInput::Deny { approval_id } if approval_id == "abc123"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/pwd").expect("pwd"),
        BackendInput::Pwd
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/ls src").expect("ls"),
        BackendInput::Ls { path } if path == "src"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/cat README.md").expect("cat"),
        BackendInput::Cat { path } if path == "README.md"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/cd /tmp").expect("cd"),
        BackendInput::Cd { path } if path == "/tmp"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/shell ls -al").expect("shell"),
        BackendInput::Shell { command } if command == "ls -al"
    ));
    assert_eq!(
        prepare_backend_input(&cfg, "/shell"),
        Err("/shell expects a command".to_string())
    );
}

#[test]
fn backend_input_recognizes_codex_builtin_commands() {
    let cfg = read_config_str(r#"{"commands":[]}"#).expect("config should parse");

    assert!(matches!(
        prepare_backend_input(&cfg, "/diff").expect("diff"),
        BackendInput::Diff
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/model").expect("model show"),
        BackendInput::ModelShow
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/models").expect("models list"),
        BackendInput::ModelsList
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/model gpt-5.5").expect("model set"),
        BackendInput::ModelSet { model } if model == "gpt-5.5"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/review").expect("review"),
        BackendInput::Review { instructions: None }
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/review focus security").expect("review with instructions"),
        BackendInput::Review { instructions: Some(instructions) } if instructions == "focus security"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/new investigate bug").expect("new"),
        BackendInput::FreshPrompt(prompt) if prompt == "investigate bug"
    ));
    assert!(matches!(
        prepare_backend_input(&cfg, "/report").expect("report"),
        BackendInput::Prompt(prompt)
            if prompt.contains("side analysis") && prompt.contains("User request: 任务状态")
    ));

    for command in [
        "/init", "/compact", "/plan", "/goal", "/agent", "/side", "/report",
    ] {
        assert!(
            matches!(
                prepare_backend_input(&cfg, command).expect(command),
                BackendInput::Prompt(_)
            ),
            "{command} should map to a Codex prompt"
        );
    }
}

#[test]
fn backend_input_returns_approval_request_for_confirmed_command() {
    let cfg = read_config_str(
        r#"{
          "commands": [
            {
              "name": "deploy",
              "prefix": "/deploy ",
              "prompt": "Deploy {{message}}",
              "requireConfirm": true
            }
          ]
        }"#,
    )
    .expect("config should parse");

    let input = prepare_backend_input(&cfg, "/deploy production").expect("backend input");

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
        "/codex ",
        "/explain ",
        "/fix ",
        "/test ",
        "/debug ",
        "/refactor ",
        "/docs ",
    ] {
        assert!(prefixes.contains(&prefix), "missing prefix {prefix}");
    }
}

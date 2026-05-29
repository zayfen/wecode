use wecode::{prepare_backend_prompt, read_config_str, render_command_input};

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

use wecode::{default_config, read_config_str};

#[test]
fn parses_config_with_custom_command() {
    let cfg = read_config_str(
        r#"{
          "openclaw": {
            "model": "wecode-codex/default",
            "autoInstallOpenclaw": true
          },
          "codex": {
            "sandbox": "workspace-write",
            "cwd": "/Users/riven/Github/wecode"
          },
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

    assert_eq!(cfg.openclaw.model, "wecode-codex/default");
    assert!(cfg.openclaw.auto_install_openclaw);
    assert_eq!(cfg.openclaw.runtime_dir, "~/.wecode/openclaw-runtime");
    assert_eq!(cfg.openclaw.profile, "wecode");
    assert_eq!(cfg.openclaw.state_dir, "~/.wecode/openclaw-state");
    assert_eq!(
        cfg.openclaw.config_path,
        "~/.wecode/openclaw-state/openclaw.json"
    );
    assert_eq!(cfg.openclaw.workspace_dir, "~/.wecode/workspace");
    assert_eq!(cfg.openclaw.gateway_port, 19789);
    assert_eq!(cfg.openclaw.node_bin_dir, None);
    assert_eq!(cfg.codex.sandbox, "workspace-write");
    assert_eq!(cfg.commands[0].name, "review");
    assert_eq!(cfg.commands[0].prefix, "/review ");
    assert!(cfg.commands[0].require_confirm);
}

#[test]
fn default_config_is_personal_codex_weixin_bridge() {
    let cfg = default_config();

    assert_eq!(cfg.openclaw.model, "wecode-codex/default");
    assert_eq!(cfg.openclaw.runtime_dir, "~/.wecode/openclaw-runtime");
    assert_eq!(cfg.openclaw.profile, "wecode");
    assert_eq!(cfg.openclaw.state_dir, "~/.wecode/openclaw-state");
    assert_eq!(
        cfg.openclaw.config_path,
        "~/.wecode/openclaw-state/openclaw.json"
    );
    assert_eq!(cfg.openclaw.workspace_dir, "~/.wecode/workspace");
    assert_eq!(cfg.openclaw.gateway_port, 19789);
    assert_eq!(cfg.openclaw.node_bin_dir, None);
    assert_eq!(cfg.codex.sandbox, "workspace-write");
    assert_eq!(cfg.commands[0].name, "ask");
    assert_eq!(cfg.commands[0].prefix, "/codex ");
}

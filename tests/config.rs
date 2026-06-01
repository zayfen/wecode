use wecode::{default_config, read_config_str, CodexTransport, PreventSleepMode};

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
              "prefix": ":review ",
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
    assert_eq!(cfg.openclaw.timeout_seconds, 1200);
    assert_eq!(cfg.openclaw.cli_no_output_timeout_ms, 900_000);
    assert_eq!(cfg.openclaw.prevent_sleep, PreventSleepMode::Ac);
    assert_eq!(cfg.openclaw.node_bin_dir, None);
    assert_eq!(cfg.codex.sandbox, "workspace-write");
    assert_eq!(cfg.codex.transport, CodexTransport::Remote);
    assert!(cfg.codex.remote.auto_start);
    assert_eq!(cfg.codex.remote.proxy_command, "codex app-server proxy");
    assert_eq!(
        cfg.codex.remote.start_command,
        "codex remote-control start --json"
    );
    assert_eq!(
        cfg.codex.remote.fallback_proxy_command,
        "codex app-server --listen stdio://"
    );
    assert_eq!(cfg.commands[0].name, "review");
    assert_eq!(cfg.commands[0].prefix, ":review ");
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
    assert_eq!(cfg.openclaw.timeout_seconds, 1200);
    assert_eq!(cfg.openclaw.cli_no_output_timeout_ms, 900_000);
    assert_eq!(cfg.openclaw.prevent_sleep, PreventSleepMode::Ac);
    assert_eq!(cfg.openclaw.node_bin_dir, None);
    assert_eq!(cfg.codex.sandbox, "workspace-write");
    assert_eq!(cfg.codex.transport, CodexTransport::Remote);
    assert!(cfg.codex.remote.auto_start);
    assert_eq!(cfg.codex.remote.proxy_command, "codex app-server proxy");
    assert_eq!(
        cfg.codex.remote.start_command,
        "codex remote-control start --json"
    );
    assert_eq!(
        cfg.codex.remote.fallback_proxy_command,
        "codex app-server --listen stdio://"
    );
    assert_eq!(cfg.codex.remote.approval_timeout_seconds, 600);
    assert_eq!(cfg.codex.models, vec!["default", "gpt-5.4"]);
    assert_eq!(cfg.commands[0].name, "ask");
    assert_eq!(cfg.commands[0].prefix, ":codex ");
}

#[test]
fn defaults_codex_remote_approval_timeout() {
    let cfg = wecode::default_config();

    assert_eq!(cfg.codex.remote.approval_timeout_seconds, 600);
}

#[test]
fn parses_codex_transport_modes_and_remote_commands() {
    let cfg = read_config_str(
        r#"{
          "codex": {
            "transport": "remote-strict",
              "remote": {
                "autoStart": false,
                "proxyCommand": "codex app-server proxy --sock /tmp/codex.sock",
                "startCommand": "codex app-server daemon start",
                "fallbackProxyCommand": "codex app-server --listen stdio://",
                "approvalTimeoutSeconds": 42
              }
          }
        }"#,
    )
    .expect("config should parse");

    assert_eq!(cfg.codex.transport, CodexTransport::RemoteStrict);
    assert!(!cfg.codex.remote.auto_start);
    assert_eq!(
        cfg.codex.remote.proxy_command,
        "codex app-server proxy --sock /tmp/codex.sock"
    );
    assert_eq!(
        cfg.codex.remote.start_command,
        "codex app-server daemon start"
    );
    assert_eq!(
        cfg.codex.remote.fallback_proxy_command,
        "codex app-server --listen stdio://"
    );
    assert_eq!(cfg.codex.remote.approval_timeout_seconds, 42);
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

#[test]
fn parses_openclaw_prevent_sleep_mode() {
    let cfg = read_config_str(
        r#"{
          "openclaw": {
            "preventSleep": "off"
          }
        }"#,
    )
    .expect("config should parse");

    assert_eq!(cfg.openclaw.prevent_sleep, PreventSleepMode::Off);

    let cfg = read_config_str(
        r#"{
          "openclaw": {
            "preventSleep": "always"
          }
        }"#,
    )
    .expect("config should parse");

    assert_eq!(cfg.openclaw.prevent_sleep, PreventSleepMode::Always);
}

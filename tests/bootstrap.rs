mod common;

use wecode::{bootstrap_plan_with_backend_command, default_config, CommandStep};

#[test]
fn bootstrap_plan_connects_weixin_to_wecode_codex_backend() {
    let mut cfg = default_config();
    cfg.openclaw.auto_install_openclaw = true;

    let steps = bootstrap_plan_with_backend_command(&cfg, true, "/usr/local/bin/wecode");

    assert_eq!(
        steps[0],
        CommandStep::new(
            "npm",
            [
                "install",
                "--prefix",
                "~/.wecode/openclaw-runtime",
                "openclaw@latest"
            ]
        )
    );
    assert!(common::has_step(
        &steps,
        &CommandStep::new(
            "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw",
            ["config", "set", "gateway.port", "19789"]
        )
    ));
    assert!(common::has_step(
        &steps,
        &CommandStep::new(
            "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw",
            [
                "config",
                "set",
                "agents.defaults.workspace",
                "~/.wecode/workspace"
            ]
        )
    ));
    assert!(!steps
        .iter()
        .any(|step| step.args == ["models", "auth", "login", "--provider", "openai-codex"]));
    assert!(!steps.iter().any(|step| step
        .args
        .iter()
        .any(|arg| arg == "plugins.entries.codex.enabled")));
    assert!(common::has_openclaw_args(
        &steps,
        &[
            "config",
            "set",
            "agents.defaults.cliBackends",
            "{\"wecode-codex\":{\"args\":[\"codex-backend\",\"--jsonl\"],\"command\":\"/usr/local/bin/wecode\",\"input\":\"stdin\",\"output\":\"jsonl\",\"resumeArgs\":[\"codex-backend\",\"--jsonl\",\"--resume\",\"{sessionId}\"],\"resumeOutput\":\"jsonl\",\"serialize\":true,\"sessionIdFields\":[\"thread_id\"]}}",
            "--strict-json",
            "--merge"
        ]
    ));
    assert!(common::has_openclaw_args(
        &steps,
        &[
            "config",
            "set",
            "agents.defaults.models",
            "{\"wecode-codex/default\":{\"alias\":\"Wecode Codex\"}}",
            "--strict-json",
            "--merge"
        ]
    ));
    assert!(common::has_step(
        &steps,
        &CommandStep::new(
            "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw",
            [
                "config",
                "set",
                "agents.defaults.model",
                "\"wecode-codex/default\"",
                "--strict-json"
            ]
        )
    ));
    let weixin_install = steps
        .iter()
        .find(|step| step.program == "npx")
        .expect("npx installer step");
    assert_eq!(
        weixin_install.args,
        vec![
            "-y",
            "@tencent-weixin/openclaw-weixin-cli@latest",
            "install"
        ]
    );
    assert_eq!(
        weixin_install.path_prepend,
        vec!["~/.wecode/openclaw-runtime/node_modules/.bin"]
    );
    assert_eq!(
        weixin_install.env,
        vec![
            ("OPENCLAW_PROFILE".to_string(), "wecode".to_string()),
            (
                "OPENCLAW_STATE_DIR".to_string(),
                "~/.wecode/openclaw-state".to_string()
            ),
            (
                "OPENCLAW_CONFIG_PATH".to_string(),
                "~/.wecode/openclaw-state/openclaw.json".to_string()
            )
        ]
    );
    let openclaw_step = steps
        .iter()
        .find(|step| step.program.ends_with("/openclaw"))
        .expect("openclaw step");
    assert_eq!(
        openclaw_step.env,
        vec![
            ("OPENCLAW_PROFILE".to_string(), "wecode".to_string()),
            (
                "OPENCLAW_STATE_DIR".to_string(),
                "~/.wecode/openclaw-state".to_string()
            ),
            (
                "OPENCLAW_CONFIG_PATH".to_string(),
                "~/.wecode/openclaw-state/openclaw.json".to_string()
            )
        ]
    );
    assert!(!steps[0].args.iter().any(|arg| arg == "-g"));
    assert!(!steps.iter().any(|step| step.program == "openclaw"));
    let last = steps.last().expect("last step");
    assert_eq!(
        last.program,
        "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw"
    );
    assert_eq!(
        last.args,
        vec!["gateway", "install", "--force", "--port", "19789"]
    );
}

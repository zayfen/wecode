mod common;

use std::env;
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
    let project_dir = env::current_dir()
        .expect("current dir")
        .display()
        .to_string();
    let openclaw_workspace = std::path::PathBuf::from(env::var("HOME").expect("HOME"))
        .join(".wecode")
        .join("workspace")
        .display()
        .to_string();
    assert!(steps.iter().any(|step| {
        step.program == "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw"
            && step.args
                == vec![
                    "config".to_string(),
                    "set".to_string(),
                    "agents.defaults.workspace".to_string(),
                    openclaw_workspace.clone(),
                ]
    }));
    assert!(common::has_step(
        &steps,
        &CommandStep::new(
            "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw",
            ["config", "set", "commands.text", "false", "--strict-json"]
        )
    ));
    assert!(!steps
        .iter()
        .any(|step| step.args == ["models", "auth", "login", "--provider", "openai-codex"]));
    assert!(!steps.iter().any(|step| step
        .args
        .iter()
        .any(|arg| arg == "plugins.entries.codex.enabled")));
    let cli_backend_json = format!(
        "{{\"wecode-codex\":{{\"args\":[\"codex-backend\",\"--jsonl\",\"--cwd\",{}],\"command\":\"/usr/local/bin/wecode\",\"input\":\"stdin\",\"modelArg\":\"--model\",\"output\":\"jsonl\",\"resumeArgs\":[\"codex-backend\",\"--jsonl\",\"--cwd\",{},\"--resume\",\"{{sessionId}}\"],\"resumeOutput\":\"jsonl\",\"serialize\":true,\"sessionIdFields\":[\"thread_id\"]}}}}",
        serde_json::to_string(&project_dir).expect("project dir json"),
        serde_json::to_string(&project_dir).expect("project dir json")
    );
    assert!(steps.iter().any(|step| {
        step.program == "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw"
            && step.args
                == vec![
                    "config".to_string(),
                    "set".to_string(),
                    "agents.defaults.cliBackends".to_string(),
                    cli_backend_json.clone(),
                    "--strict-json".to_string(),
                    "--merge".to_string(),
                ]
    }));
    assert!(common::has_openclaw_args(
        &steps,
        &[
            "config",
            "set",
            "agents.defaults.models",
            "{\"wecode-codex/default\":{\"alias\":\"Wecode Codex\"},\"wecode-codex/gpt-5.4\":{\"alias\":\"Wecode Codex gpt-5.4\"}}",
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

#[test]
fn bootstrap_plan_prepends_configured_node_bin_dir_to_node_based_steps() {
    let mut cfg = default_config();
    cfg.openclaw.node_bin_dir = Some("~/node24/bin".to_string());

    let steps = bootstrap_plan_with_backend_command(&cfg, true, "/usr/local/bin/wecode");

    assert_eq!(steps[0].program, "npm");
    assert_eq!(steps[0].path_prepend, vec!["~/node24/bin"]);

    let openclaw_steps = steps
        .iter()
        .filter(|step| step.program.ends_with("/openclaw"))
        .collect::<Vec<_>>();
    assert!(!openclaw_steps.is_empty());
    assert!(openclaw_steps
        .iter()
        .all(|step| step.path_prepend == vec!["~/node24/bin"]));

    let weixin_install = steps
        .iter()
        .find(|step| step.program == "npx")
        .expect("npx installer step");
    assert_eq!(
        weixin_install.path_prepend,
        vec![
            "~/node24/bin",
            "~/.wecode/openclaw-runtime/node_modules/.bin"
        ]
    );
}

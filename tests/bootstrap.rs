mod common;

use std::{env, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use wecode::{
    bootstrap_plan_with_backend_command, default_config, patch_openclaw_runtime,
    patch_openclaw_text_command_routing, prevent_sleep_program_arguments, BootstrapChannel,
    CommandStep, PreventSleepMode,
};

#[test]
fn bootstrap_plan_connects_weixin_to_wecode_codex_backend() {
    let cfg = default_config();

    let steps = bootstrap_plan_with_backend_command(
        &cfg,
        BootstrapChannel::Weixin,
        "/usr/local/bin/wecode",
    );

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
    assert!(steps
        .iter()
        .any(|step| step.args.iter().any(|arg| arg == "patch-openclaw-runtime")));
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
    assert!(!common::has_step(
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
        "{{\"wecode-codex\":{{\"args\":[\"codex-backend\",\"--jsonl\",\"--cwd\",{}],\"command\":\"/usr/local/bin/wecode\",\"input\":\"stdin\",\"jsonlDialect\":\"claude-stream-json\",\"modelArg\":\"--model\",\"output\":\"jsonl\",\"reliability\":{{\"watchdog\":{{\"fresh\":{{\"noOutputTimeoutMs\":900000}},\"resume\":{{\"noOutputTimeoutMs\":900000}}}}}},\"resumeArgs\":[\"codex-backend\",\"--jsonl\",\"--cwd\",{},\"--resume\",\"{{sessionId}}\"],\"resumeOutput\":\"jsonl\",\"serialize\":false,\"sessionIdFields\":[\"thread_id\"]}}}}",
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
            ["config", "set", "agents.defaults.timeoutSeconds", "1200"]
        )
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
fn bootstrap_suppresses_openclaw_output_and_prints_wecode_success_banner() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    let runtime_dir = temp.path().join("openclaw-runtime");
    let state_dir = temp.path().join("openclaw-state");
    let workspace_dir = temp.path().join("workspace");
    let config_path = temp.path().join("wecode.json");
    fs::create_dir(&project_dir).expect("project dir");

    write_executable(
        &temp.path().join("node"),
        r#"#!/bin/sh
echo v24.0.0
"#,
    );
    write_executable(
        &temp.path().join("codex"),
        r#"#!/bin/sh
echo codex 1.0.0
"#,
    );
    write_executable(
        &temp.path().join("npx"),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 10.0.0
  exit 0
fi
echo "Weixin QR code stdout"
echo "openclaw-weixin internal stdout"
echo "OpenClaw mixed-case stdout"
echo "Local login saved auth for openclaw-weixin/default"
echo "Config: $OPENCLAW_CONFIG_PATH"
echo "Weixin login stderr" >&2
"#,
    );
    write_executable(
        &temp.path().join("npm"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 10.0.0
  exit 0
fi
prefix=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--prefix" ]; then
    shift
    prefix="$1"
  fi
  shift
done
mkdir -p "$prefix/node_modules/openclaw/dist" "$prefix/node_modules/.bin"
cat > "$prefix/node_modules/openclaw/dist/commands-text-routing-test.js" <<'EOF'
{routing_source}
EOF
cat > "$prefix/node_modules/.bin/openclaw" <<'EOF'
#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "OpenClaw 2026.5.27"
  exit 0
fi
if [ "$1" = "gateway" ] && [ "$2" = "install" ]; then
  mkdir -p "$HOME/Library/LaunchAgents"
  cat > "$HOME/Library/LaunchAgents/ai.openclaw.wecode.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>ai.openclaw.wecode</string>
  <key>ProgramArguments</key>
  <array>
    <string>/tmp/wecode-env-wrapper.sh</string>
    <string>/tmp/wecode.env</string>
    <string>/tmp/node</string>
    <string>/tmp/openclaw/dist/index.js</string>
    <string>gateway</string>
    <string>--port</string>
    <string>19789</string>
  </array>
</dict>
</plist>
PLIST
fi
echo "OpenClaw noisy gateway stdout"
echo "OpenClaw noisy gateway stderr" >&2
EOF
chmod 755 "$prefix/node_modules/.bin/openclaw"
echo "OpenClaw noisy npm stdout"
echo "OpenClaw noisy npm stderr" >&2
"#,
            routing_source = openclaw_text_routing_source()
        ),
    );

    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{
                "runtimeDir": "{}",
                "stateDir": "{}",
                "configPath": "{}",
                "workspaceDir": "{}",
                "nodeBinDir": "{}"
              }},
              "codex": {{"cwd": "{}"}},
              "commands": []
            }}"#,
            runtime_dir.display(),
            state_dir.display(),
            state_dir.join("openclaw.json").display(),
            workspace_dir.display(),
            temp.path().display(),
            project_dir.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "bootstrap",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--weixin",
        ])
        .env("HOME", temp.path())
        .env("PATH", path)
        .output()
        .expect("run bootstrap");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(stdout.contains("Wecode 启动成功"), "stdout:\n{stdout}");
    assert!(stdout.contains("🚀"), "stdout:\n{stdout}");
    assert!(
        combined.contains("Wecode 启动中"),
        "combined output:\n{combined}"
    );
    assert!(
        combined.contains("安装微信"),
        "combined output:\n{combined}"
    );
    assert!(
        stdout.contains("Weixin QR code stdout"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("openclaw-weixin internal stdout"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Wecode mixed-case stdout"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Local login saved auth for openclaw-weixin/default"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "Config: {}",
            state_dir.join("openclaw.json").display()
        )),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("wecode-weixin/default"),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("wecode-state/wecode.json"),
        "stdout:\n{stdout}"
    );
    assert!(stderr.contains("Weixin login stderr"), "stderr:\n{stderr}");
    assert!(
        !combined.contains("OpenClaw noisy"),
        "combined output:\n{combined}"
    );
    assert!(
        !combined.contains("$ "),
        "bootstrap should not print raw child commands:\n{combined}"
    );

    let log = fs::read_to_string(state_dir.join("bootstrap.log")).expect("bootstrap log");
    assert!(log.contains("OpenClaw noisy npm stdout"), "log:\n{log}");
    assert!(log.contains("OpenClaw noisy npm stderr"), "log:\n{log}");
    assert!(log.contains("OpenClaw noisy gateway stdout"), "log:\n{log}");
    assert!(log.contains("OpenClaw noisy gateway stderr"), "log:\n{log}");
    assert!(!log.contains("Weixin QR code stdout"), "log:\n{log}");
    assert!(!log.contains("Weixin login stderr"), "log:\n{log}");
    assert_launch_agent_prevents_sleep(temp.path());
}

#[test]
fn feishu_bootstrap_shows_login_prompts_and_forces_plugin_install() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    let runtime_dir = temp.path().join("openclaw-runtime");
    let state_dir = temp.path().join("openclaw-state");
    let workspace_dir = temp.path().join("workspace");
    let config_path = temp.path().join("wecode.json");
    fs::create_dir(&project_dir).expect("project dir");

    write_executable(
        &temp.path().join("node"),
        r#"#!/bin/sh
echo v24.0.0
"#,
    );
    write_executable(
        &temp.path().join("codex"),
        r#"#!/bin/sh
echo codex 1.0.0
"#,
    );
    write_executable(
        &temp.path().join("npm"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 10.0.0
  exit 0
fi
prefix=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--prefix" ]; then
    shift
    prefix="$1"
  fi
  shift
done
mkdir -p "$prefix/node_modules/openclaw/dist" "$prefix/node_modules/.bin"
cat > "$prefix/node_modules/openclaw/dist/commands-text-routing-test.js" <<'EOF'
{routing_source}
EOF
cat > "$prefix/node_modules/.bin/openclaw" <<'EOF'
#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "OpenClaw 2026.5.27"
  exit 0
fi
if [ "$1" = "gateway" ] && [ "$2" = "install" ]; then
  mkdir -p "$HOME/Library/LaunchAgents"
  cat > "$HOME/Library/LaunchAgents/ai.openclaw.wecode.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>ai.openclaw.wecode</string>
  <key>ProgramArguments</key>
  <array>
    <string>/tmp/wecode-env-wrapper.sh</string>
    <string>/tmp/wecode.env</string>
    <string>/tmp/node</string>
    <string>/tmp/openclaw/dist/index.js</string>
    <string>gateway</string>
    <string>--port</string>
    <string>19789</string>
  </array>
</dict>
</plist>
PLIST
  echo "OpenClaw gateway installed"
  exit 0
fi
case "$*" in
  *"plugins install @openclaw/feishu --force"*)
    echo "OpenClaw feishu plugin replaced"
    exit 0
    ;;
  *"plugins install @openclaw/feishu"*)
    echo "plugin already exists" >&2
    exit 1
    ;;
  *"channels login --channel feishu"*)
    printf "OpenClaw Feishu App ID: "
    echo "Feishu App Secret prompt" >&2
    exit 0
    ;;
  *)
    echo "OpenClaw setup stdout"
    exit 0
    ;;
esac
EOF
chmod 755 "$prefix/node_modules/.bin/openclaw"
"#,
            routing_source = openclaw_text_routing_source()
        ),
    );

    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{
                "runtimeDir": "{}",
                "stateDir": "{}",
                "configPath": "{}",
                "workspaceDir": "{}",
                "nodeBinDir": "{}"
              }},
              "codex": {{"cwd": "{}"}},
              "commands": []
            }}"#,
            runtime_dir.display(),
            state_dir.display(),
            state_dir.join("openclaw.json").display(),
            workspace_dir.display(),
            temp.path().display(),
            project_dir.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "bootstrap",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--feishu",
        ])
        .env("HOME", temp.path())
        .env("PATH", path)
        .output()
        .expect("run feishu bootstrap");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("Wecode 启动中"),
        "combined output:\n{combined}"
    );
    assert!(
        combined.contains("安装飞书"),
        "combined output:\n{combined}"
    );
    assert!(
        stdout.contains("Wecode Feishu App ID: "),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.to_ascii_lowercase().contains("openclaw"),
        "stdout:\n{stdout}"
    );
    assert!(
        stderr.contains("Feishu App Secret prompt"),
        "stderr:\n{stderr}"
    );

    let log = fs::read_to_string(state_dir.join("bootstrap.log")).expect("bootstrap log");
    assert!(log.contains("@openclaw/feishu --force"), "log:\n{log}");
    assert!(log.contains("channels.feishu.streaming"), "log:\n{log}");
    assert!(log.contains("channels.feishu.renderMode"), "log:\n{log}");
    assert!(
        log.contains("channels.feishu.blockStreaming"),
        "log:\n{log}"
    );
    assert!(!log.contains("Feishu App Secret prompt"), "log:\n{log}");
    assert_launch_agent_prevents_sleep(temp.path());
}

#[test]
fn bootstrap_checks_openclaw_state_dir_is_writable_before_running_steps() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    let runtime_dir = temp.path().join("openclaw-runtime");
    let state_dir = temp.path().join("openclaw-state");
    let workspace_dir = temp.path().join("workspace");
    let config_path = temp.path().join("wecode.json");
    let marker_path = temp.path().join("npm-ran.txt");
    fs::create_dir(&project_dir).expect("project dir");
    fs::create_dir(&state_dir).expect("state dir");
    let mut permissions = fs::metadata(&state_dir)
        .expect("state metadata")
        .permissions();
    permissions.set_mode(0o555);
    fs::set_permissions(&state_dir, permissions).expect("chmod state dir");

    write_executable(
        &temp.path().join("node"),
        r#"#!/bin/sh
echo v24.0.0
"#,
    );
    write_executable(
        &temp.path().join("codex"),
        r#"#!/bin/sh
echo codex 1.0.0
"#,
    );
    write_executable(
        &temp.path().join("npm"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 10.0.0
  exit 0
fi
touch {}
"#,
            shell_quote(marker_path.to_str().expect("utf-8 marker path"))
        ),
    );

    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{
                "runtimeDir": "{}",
                "stateDir": "{}",
                "configPath": "{}",
                "workspaceDir": "{}",
                "nodeBinDir": "{}"
              }},
              "codex": {{"cwd": "{}"}},
              "commands": []
            }}"#,
            runtime_dir.display(),
            state_dir.display(),
            state_dir.join("openclaw.json").display(),
            workspace_dir.display(),
            temp.path().display(),
            project_dir.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "bootstrap",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--weixin",
        ])
        .env("PATH", path)
        .output()
        .expect("run bootstrap");

    let mut restore_permissions = fs::metadata(&state_dir)
        .expect("state metadata after bootstrap")
        .permissions();
    restore_permissions.set_mode(0o755);
    fs::set_permissions(&state_dir, restore_permissions).expect("restore state dir");

    assert!(!output.status.success(), "bootstrap should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("openclaw.stateDir"),
        "stderr should mention stateDir permissions:\n{stderr}"
    );
    assert!(
        stderr.contains("bootstrap Wecode"),
        "stderr should explain bootstrap failed before setup:\n{stderr}"
    );
    assert!(
        !marker_path.exists(),
        "bootstrap must not run setup commands when stateDir is not writable"
    );
}

#[test]
fn patch_openclaw_text_routing_disables_all_text_commands() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dist = temp
        .path()
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    fs::create_dir_all(&dist).expect("dist dir");
    let routing_file = dist.join("commands-text-routing-test.js");
    fs::write(
        &routing_file,
        r#"function shouldHandleTextCommands(params) {
	if (params.commandSource === "native") return true;
	if (params.cfg.commands?.text !== false) return true;
	return !isNativeCommandSurface(params.surface);
}
"#,
    )
    .expect("routing file");

    patch_openclaw_text_command_routing(temp.path()).expect("patch routing");
    patch_openclaw_text_command_routing(temp.path()).expect("patch routing again");

    let patched = fs::read_to_string(&routing_file).expect("patched routing");
    assert!(patched.contains(r#"return params.cfg.commands?.text !== false;"#));
    assert!(!patched.contains(r#"params.commandSource === "native""#));
    assert!(!patched.contains("return !isNativeCommandSurface(params.surface);"));
}

#[test]
fn patch_openclaw_text_routing_upgrades_previous_wecode_patch() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dist = temp
        .path()
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    fs::create_dir_all(&dist).expect("dist dir");
    let routing_file = dist.join("commands-text-routing-test.js");
    fs::write(
        &routing_file,
        r#"function shouldHandleTextCommands(params) {
	if (params.commandSource === "native") return true;
	return params.cfg.commands?.text !== false;
}
"#,
    )
    .expect("routing file");

    patch_openclaw_text_command_routing(temp.path()).expect("patch routing");

    let patched = fs::read_to_string(&routing_file).expect("patched routing");
    assert!(patched.contains(r#"return params.cfg.commands?.text !== false;"#));
    assert!(!patched.contains(r#"params.commandSource === "native""#));
}

#[test]
fn patch_openclaw_runtime_disables_builtin_compact_when_text_commands_disabled() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dist = temp
        .path()
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    fs::create_dir_all(&dist).expect("dist dir");
    let routing_file = dist.join("commands-text-routing-test.js");
    fs::write(&routing_file, openclaw_text_routing_source()).expect("routing file");
    let handlers_file = dist.join("commands-handlers.runtime-test.js");
    fs::write(
        &handlers_file,
        r#"const handleCompactCommand = async (params) => {
	if (!(params.command.commandBodyNormalized === "/compact" || params.command.commandBodyNormalized.startsWith("/compact "))) return null;
	if (!params.command.isAuthorizedSender) {
		logVerbose(`Ignoring /compact from unauthorized sender: ${params.command.senderId || "<unknown>"}`);
		return { shouldContinue: false };
	}
	return { shouldContinue: false, reply: { text: "compact" } };
};
"#,
    )
    .expect("handlers file");

    patch_openclaw_text_command_routing(temp.path()).expect("patch runtime");
    patch_openclaw_text_command_routing(temp.path()).expect("patch runtime again");

    let patched = fs::read_to_string(&handlers_file).expect("patched handlers");
    let guard = "if (params.cfg.commands?.text === false) return null;";
    assert!(
        patched.contains(guard),
        "compact handler should respect commands.text=false:\n{patched}"
    );
    assert_eq!(
        patched.matches(guard).count(),
        1,
        "compact handler patch should be idempotent:\n{patched}"
    );
}

#[test]
fn patch_openclaw_runtime_enables_weixin_block_streaming_for_approval_prompts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime_dist = temp
        .path()
        .join("runtime")
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    fs::create_dir_all(&runtime_dist).expect("runtime dist");
    fs::write(
        runtime_dist.join("commands-text-routing-test.js"),
        openclaw_text_routing_source(),
    )
    .expect("routing file");
    let state_dir = temp.path().join("state");
    let weixin_messaging = state_dir
        .join("npm")
        .join("projects")
        .join("weixin-project")
        .join("node_modules")
        .join("@tencent-weixin")
        .join("openclaw-weixin")
        .join("dist")
        .join("src")
        .join("messaging");
    fs::create_dir_all(&weixin_messaging).expect("weixin messaging dir");
    let process_file = weixin_messaging.join("process-message.js");
    fs::write(
        &process_file,
        r#"await deps.channelRuntime.reply.dispatchReplyFromConfig({
                ctx: finalized,
                cfg: deps.config,
                dispatcher,
                replyOptions: {
                    ...replyOptions,
                    ...(replyProgressSender?.replyOptions ?? {}),
                    disableBlockStreaming: true,
                },
            });
"#,
    )
    .expect("process message file");

    patch_openclaw_runtime(&temp.path().join("runtime"), &state_dir).expect("patch runtime");
    patch_openclaw_runtime(&temp.path().join("runtime"), &state_dir).expect("patch runtime again");

    let patched = fs::read_to_string(&process_file).expect("patched process message");
    assert!(
        patched.contains("disableBlockStreaming: false"),
        "weixin block streaming must be enabled so approval prompts are delivered before the Codex turn finishes:\n{patched}"
    );
    assert!(!patched.contains("disableBlockStreaming: true"));
}

#[test]
fn patch_openclaw_runtime_routes_weixin_approval_replies_on_control_lane() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime_dist = temp
        .path()
        .join("runtime")
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    fs::create_dir_all(&runtime_dist).expect("runtime dist");
    fs::write(
        runtime_dist.join("commands-text-routing-test.js"),
        openclaw_text_routing_source(),
    )
    .expect("routing file");
    let state_dir = temp.path().join("state");
    let weixin_messaging = state_dir
        .join("npm")
        .join("projects")
        .join("weixin-project")
        .join("node_modules")
        .join("@tencent-weixin")
        .join("openclaw-weixin")
        .join("dist")
        .join("src")
        .join("messaging");
    fs::create_dir_all(&weixin_messaging).expect("weixin messaging dir");
    let lane_file = weixin_messaging.join("lane-key.js");
    fs::write(
        &lane_file,
        r#"export function getWeixinLaneKey(ctx) {
    const userId = ctx.msg.from_user_id?.trim() || "unknown";
    const baseKey = `wx:${ctx.accountId}:${userId}`;
    if (isWeixinAbortMessage(ctx.msg)) {
        return `${baseKey}:control`;
    }
    return baseKey;
}
"#,
    )
    .expect("lane key file");

    patch_openclaw_runtime(&temp.path().join("runtime"), &state_dir).expect("patch runtime");

    let patched = fs::read_to_string(&lane_file).expect("patched lane key");
    assert!(patched.contains("isWecodeApprovalControlText"));
    assert!(
        patched.contains("isWeixinAbortMessage(ctx.msg) || isWecodeApprovalControlText(textBody)"),
        "weixin approval replies should bypass the busy main lane:\n{patched}"
    );
}

#[test]
fn patch_openclaw_runtime_routes_feishu_approval_replies_on_control_lane() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime_dist = temp
        .path()
        .join("runtime")
        .join("node_modules")
        .join("openclaw")
        .join("dist");
    fs::create_dir_all(&runtime_dist).expect("runtime dist");
    fs::write(
        runtime_dist.join("commands-text-routing-test.js"),
        openclaw_text_routing_source(),
    )
    .expect("routing file");
    let state_dir = temp.path().join("state");
    let feishu_dist = state_dir
        .join("npm")
        .join("projects")
        .join("feishu-project")
        .join("node_modules")
        .join("@openclaw")
        .join("feishu")
        .join("dist");
    fs::create_dir_all(&feishu_dist).expect("feishu dist dir");
    let monitor_file = feishu_dist.join("monitor.account-test.js");
    fs::write(
        &monitor_file,
        r#"function getFeishuSequentialKey(params) {
	const { accountId, event, botOpenId, botName } = params;
	const baseKey = `feishu:${accountId}:${event.message.chat_id?.trim() || "unknown"}`;
	const text = parseFeishuMessageEvent(event, botOpenId, botName).content.trim();
	if (isAbortRequestText(text)) return `${baseKey}:control`;
	if (isBtwRequestText(text)) return `${baseKey}:btw`;
	return baseKey;
}
"#,
    )
    .expect("feishu monitor file");

    patch_openclaw_runtime(&temp.path().join("runtime"), &state_dir).expect("patch runtime");

    let patched = fs::read_to_string(&monitor_file).expect("patched feishu monitor");
    assert!(patched.contains("isWecodeApprovalControlText"));
    assert!(
        patched.contains("if (isAbortRequestText(text) || isWecodeApprovalControlText(text)) return `${baseKey}:control`;"),
        "feishu approval replies should bypass the busy per-chat FIFO lane:\n{patched}"
    );
}

#[test]
fn prevent_sleep_wraps_gateway_launch_agent_arguments_on_ac_power() {
    let original = vec![
        "/Users/riven/.wecode/openclaw-state/service-env/ai.openclaw.wecode-env-wrapper.sh"
            .to_string(),
        "/Users/riven/.wecode/openclaw-state/service-env/ai.openclaw.wecode.env".to_string(),
        "/opt/homebrew/opt/node@22/bin/node".to_string(),
        "/Users/riven/.wecode/openclaw-runtime/node_modules/openclaw/dist/index.js".to_string(),
        "gateway".to_string(),
        "--port".to_string(),
        "19789".to_string(),
    ];

    let wrapped = prevent_sleep_program_arguments(&original, PreventSleepMode::Ac);

    assert_eq!(wrapped[0], "/usr/bin/caffeinate");
    assert_eq!(wrapped[1], "-s");
    assert_eq!(&wrapped[2..], original.as_slice());
    assert_eq!(
        prevent_sleep_program_arguments(&wrapped, PreventSleepMode::Ac),
        wrapped,
        "sleep prevention wrapper should be idempotent"
    );
    assert_eq!(
        prevent_sleep_program_arguments(&wrapped, PreventSleepMode::Off),
        original,
        "turning the mode off should remove the wecode caffeinate wrapper"
    );
}

#[test]
fn bootstrap_plan_connects_feishu_to_wecode_codex_backend() {
    let cfg = default_config();

    let steps = bootstrap_plan_with_backend_command(
        &cfg,
        BootstrapChannel::Feishu,
        "/usr/local/bin/wecode",
    );

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
    assert!(common::has_openclaw_args(
        &steps,
        &["plugins", "install", "@openclaw/feishu", "--force"]
    ));
    assert!(common::has_openclaw_args(
        &steps,
        &["channels", "login", "--channel", "feishu"]
    ));
    assert!(common::has_openclaw_args(
        &steps,
        &[
            "config",
            "set",
            "channels.feishu.streaming",
            "true",
            "--strict-json"
        ]
    ));
    assert!(common::has_openclaw_args(
        &steps,
        &[
            "config",
            "set",
            "channels.feishu.renderMode",
            "\"card\"",
            "--strict-json"
        ]
    ));
    assert!(common::has_openclaw_args(
        &steps,
        &[
            "config",
            "set",
            "channels.feishu.blockStreaming",
            "true",
            "--strict-json"
        ]
    ));
    let login_index = steps
        .iter()
        .position(|step| step.args == ["channels", "login", "--channel", "feishu"])
        .expect("feishu login step");
    let render_mode_index = steps
        .iter()
        .position(|step| {
            step.args
                == [
                    "config",
                    "set",
                    "channels.feishu.renderMode",
                    "\"card\"",
                    "--strict-json",
                ]
        })
        .expect("feishu render mode step");
    assert!(
        login_index < render_mode_index,
        "Feishu streaming config must be written after login so login cannot overwrite it"
    );
    assert!(!steps.iter().any(|step| step.program == "npx"));
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

    let steps = bootstrap_plan_with_backend_command(
        &cfg,
        BootstrapChannel::Weixin,
        "/usr/local/bin/wecode",
    );

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

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable");
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn openclaw_text_routing_source() -> &'static str {
    r#"function shouldHandleTextCommands(params) {
	if (params.commandSource === "native") return true;
	if (params.cfg.commands?.text !== false) return true;
	return !isNativeCommandSurface(params.surface);
}
"#
}

fn assert_launch_agent_prevents_sleep(home: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let plist = home
        .join("Library")
        .join("LaunchAgents")
        .join("ai.openclaw.wecode.plist");
    let source = fs::read_to_string(&plist).expect("LaunchAgent plist");
    assert!(
        source.contains("<string>/usr/bin/caffeinate</string>"),
        "LaunchAgent should run under caffeinate:\n{source}"
    );
    assert!(
        source.contains("<string>-s</string>"),
        "LaunchAgent should prevent system sleep on AC power:\n{source}"
    );
}

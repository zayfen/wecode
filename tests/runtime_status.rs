use std::{fs, process::Command};

#[test]
fn runtime_status_reports_launch_agent_sleep_guard() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let launch_agents = home.join("Library").join("LaunchAgents");
    fs::create_dir_all(&launch_agents).expect("launch agents dir");
    fs::write(
        launch_agents.join("ai.openclaw.wecode.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/caffeinate</string>
    <string>-i</string>
    <string>/tmp/openclaw/dist/index.js</string>
    <string>gateway</string>
  </array>
</dict>
</plist>
"#,
    )
    .expect("launch agent plist");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{
                "profile": "wecode",
                "stateDir": {},
                "preventSleep": "always"
              }}
            }}"#,
            serde_json::to_string(&state_dir.display().to_string()).expect("state json")
        ),
    )
    .expect("config");

    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "runtime-status",
            "--config",
            config_path.to_str().expect("utf-8 config"),
        ])
        .env("HOME", &home)
        .output()
        .expect("runtime status");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Wecode Runtime Status"), "{stdout}");
    assert!(
        stdout.contains("**Configured preventSleep**: `always`"),
        "{stdout}"
    );
    assert!(stdout.contains("**Sleep guard**: `always`"), "{stdout}");
    assert!(stdout.contains("**Gateway process**:"), "{stdout}");
}

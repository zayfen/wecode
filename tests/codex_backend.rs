use std::{
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

#[test]
fn codex_backend_falls_back_to_fresh_thread_when_resume_rollout_is_missing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    fs::write(
        &codex_path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {calls}
case " $* " in
  *" resume "*)
    echo "Error: thread/resume: thread/resume failed: no rollout found for thread id stale-thread (code -32600)" >&2
    exit 1
    ;;
  *)
    echo '{{"thread_id":"fresh-thread"}}'
    echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"ok"}}]}}'
    exit 0
    ;;
esac
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(&codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex_path, permissions).expect("chmod fake codex");
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--resume",
            "stale-thread",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run wecode");

    assert!(
        output.status.success(),
        "status: {:?}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(r#""thread_id":"fresh-thread""#),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("exec resume --json"));
    assert!(calls.contains("exec --json"));
}

#[test]
fn codex_backend_streams_jsonl_before_codex_exits() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");

    fs::write(
        &codex_path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {calls}
echo '{{"thread_id":"stream-thread"}}'
sleep 4
echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"stream ok"}}]}}'
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    fs::write(&config_path, r#"{"codex":{"transport":"exec"}}"#).expect("write config");
    let mut permissions = fs::metadata(&codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex_path, permissions).expect("chmod fake codex");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "hello",
        ])
        .env("PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wecode");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let result = reader.read_line(&mut line);
        let _ = tx.send((result, line));
        let mut rest = String::new();
        let _ = reader.read_to_string(&mut rest);
    });

    let (read_result, first_line) = match rx.recv_timeout(Duration::from_millis(2500)) {
        Ok(value) => value,
        Err(err) => {
            let _ = child.wait();
            panic!("expected streamed JSONL before codex exited: {err}");
        }
    };
    assert_eq!(read_result.expect("read stdout line"), first_line.len());
    assert!(
        first_line.contains(r#""thread_id":"stream-thread""#),
        "first stdout line:\n{first_line}"
    );

    let status = child.wait().expect("wait wecode");
    assert!(status.success(), "status: {:?}", status.code());
}

#[test]
fn codex_backend_remote_transport_runs_turn_over_app_server_proxy() {
    let temp = tempfile::tempdir().expect("temp dir");
    let proxy_path = temp.path().join("codex-remote-proxy");
    let calls_path = temp.path().join("remote-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).expect("project dir");
    write_fake_remote_proxy(&proxy_path, &calls_path, "remote-thread", "remote ok");

    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{
                "cwd": "{}",
                "transport": "remote-strict",
                "remote": {{
                  "autoStart": false,
                  "proxyCommand": "{}"
                }}
              }}
            }}"#,
            state_dir.display(),
            project_dir.display(),
            proxy_path.display()
        ),
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "hello remote",
        ])
        .output()
        .expect("run remote backend");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(r#""thread_id":"remote-thread""#),
        "{stdout}"
    );
    assert!(stdout.contains("remote ok"), "{stdout}");
    let calls = fs::read_to_string(calls_path).expect("remote calls");
    assert!(calls.contains(r#""method":"initialize""#), "{calls}");
    assert!(calls.contains(r#""method":"thread/start""#), "{calls}");
    assert!(calls.contains(r#""method":"turn/start""#), "{calls}");
    assert!(calls.contains("hello remote"), "{calls}");
    assert!(
        calls.contains(&project_dir.display().to_string()),
        "{calls}"
    );
}

#[test]
fn codex_backend_remote_transport_resumes_existing_thread() {
    let temp = tempfile::tempdir().expect("temp dir");
    let proxy_path = temp.path().join("codex-remote-proxy");
    let calls_path = temp.path().join("remote-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    write_fake_remote_proxy(&proxy_path, &calls_path, "existing-thread", "resumed ok");

    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{
                "transport": "remote-strict",
                "remote": {{
                  "autoStart": false,
                  "proxyCommand": "{}"
                }}
              }}
            }}"#,
            state_dir.display(),
            proxy_path.display()
        ),
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--resume",
            "existing-thread",
            "continue remote",
        ])
        .output()
        .expect("run remote resume");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(r#""thread_id":"existing-thread""#),
        "{stdout}"
    );
    assert!(stdout.contains("resumed ok"), "{stdout}");
    let calls = fs::read_to_string(calls_path).expect("remote calls");
    assert!(calls.contains(r#""method":"thread/resume""#), "{calls}");
    assert!(!calls.contains(r#""method":"thread/start""#), "{calls}");
    assert!(calls.contains("continue remote"), "{calls}");
}

#[test]
fn codex_backend_remote_transport_falls_back_to_exec_when_proxy_fails() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{
                "transport": "remote",
                "remote": {{
                  "autoStart": false,
                  "proxyCommand": "{}"
                }}
              }}
            }}"#,
            state_dir.display(),
            temp.path().join("missing-proxy").display()
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
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "fallback please",
        ])
        .env("PATH", path)
        .output()
        .expect("run fallback backend");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(r#""thread_id":"fake-thread""#), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("falling back to codex exec"), "{stderr}");
    let calls = fs::read_to_string(calls_path).expect("codex calls");
    assert!(calls.contains("exec --json"), "{calls}");
}

#[test]
fn codex_backend_remote_transport_uses_stdio_app_server_when_managed_proxy_unavailable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let codex_calls_path = temp.path().join("codex-calls.txt");
    let start_path = temp.path().join("remote-start");
    let start_calls_path = temp.path().join("remote-start-calls.txt");
    let stdio_app_server_path = temp.path().join("stdio-app-server");
    let remote_calls_path = temp.path().join("remote-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex_with_stdio_app_server(&codex_path, &codex_calls_path, &stdio_app_server_path);
    write_failing_remote_start(&start_path, &start_calls_path);
    write_fake_remote_proxy(
        &stdio_app_server_path,
        &remote_calls_path,
        "stdio-thread",
        "stdio ok",
    );
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{
                "transport": "remote-strict",
                "remote": {{
                  "autoStart": true,
                  "startCommand": "{}"
                }}
              }}
            }}"#,
            state_dir.display(),
            start_path.display()
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
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "hello stdio",
        ])
        .env("PATH", path)
        .output()
        .expect("run remote fallback backend");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(r#""thread_id":"stdio-thread""#), "{stdout}");
    assert!(stdout.contains("stdio ok"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("falling back to codex exec"),
        "stderr:\n{stderr}"
    );
    assert!(
        !fs::read_to_string(&codex_calls_path)
            .unwrap_or_default()
            .contains("exec --json"),
        "remote-strict compatibility fallback must not invoke codex exec"
    );
    let start_calls = fs::read_to_string(start_calls_path).expect("start calls");
    assert!(start_calls.contains("start attempted"), "{start_calls}");
    let codex_calls = fs::read_to_string(codex_calls_path).expect("codex calls");
    assert!(codex_calls.contains("app-server proxy"), "{codex_calls}");
    assert!(
        codex_calls.contains("app-server --listen stdio://"),
        "{codex_calls}"
    );
    let remote_calls = fs::read_to_string(remote_calls_path).expect("remote calls");
    assert!(
        remote_calls.contains(r#""method":"initialize""#),
        "{remote_calls}"
    );
    assert!(remote_calls.contains("hello stdio"), "{remote_calls}");
}

#[test]
fn codex_backend_defers_confirmed_command_until_approved() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    fs::write(
        &codex_path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {calls}
echo '{{"thread_id":"approved-thread"}}'
echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"approved ok"}}]}}'
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(&codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex_path, permissions).expect("chmod fake codex");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{
                "stateDir": "{}"
              }},
              "commands": [
                {{
                  "name": "deploy",
                  "prefix": ":deploy ",
                  "prompt": "Deploy {{{{message}}}}",
                  "requireConfirm": true
                }}
              ]
            }}"#,
            state_dir.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let request = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":deploy",
            "production",
        ])
        .env("PATH", &path)
        .output()
        .expect("request approval");

    assert!(
        request.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&request.stderr)
    );
    let request_stdout = String::from_utf8_lossy(&request.stdout);
    assert!(request_stdout.contains("requires approval"));
    assert!(request_stdout.contains(":approve "));
    assert!(!calls_path.exists(), "codex should not run before approval");

    let approvals_dir = state_dir.join("approvals");
    let approval_file = fs::read_dir(&approvals_dir)
        .expect("approvals dir")
        .next()
        .expect("approval file")
        .expect("approval dir entry")
        .path();
    let approval_id = approval_file
        .file_stem()
        .expect("approval file stem")
        .to_str()
        .expect("utf-8 approval id")
        .to_string();

    let approved = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--resume",
            "existing-thread",
            ":approve",
            &approval_id,
        ])
        .env("PATH", path)
        .output()
        .expect("approve command");

    assert!(
        approved.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    assert!(String::from_utf8_lossy(&approved.stdout).contains("approved-thread"));
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("exec resume --json"));
    assert!(calls.contains("Deploy production"));
    assert!(!approval_file.exists(), "approval should be consumed");
}

#[test]
fn model_command_sets_model_for_next_codex_run() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let set_model = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":model",
            "gpt-5.5",
        ])
        .env("PATH", &path)
        .output()
        .expect("set model");

    assert!(set_model.status.success());

    let run = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":codex",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run codex");

    assert!(
        run.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("-m gpt-5.5"));
}

#[test]
fn model_command_keeps_separate_model_state_per_project() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    fs::create_dir_all(&project_a).expect("project a");
    fs::create_dir_all(&project_b).expect("project b");
    let project_a = project_a.canonicalize().expect("canonical project a");
    let project_b = project_b.canonicalize().expect("canonical project b");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    for (project, model) in [(&project_a, "gpt-project-a"), (&project_b, "gpt-project-b")] {
        let set = Command::new(env!("CARGO_BIN_EXE_wecode"))
            .args([
                "codex-backend",
                "--config",
                config_path.to_str().expect("utf-8 config"),
                "--jsonl",
                "--cwd",
                project.to_str().expect("utf-8 project"),
                ":model",
                model,
            ])
            .env("PATH", &path)
            .output()
            .expect("set model");
        assert!(
            set.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&set.stderr)
        );
    }

    for (project, model) in [(&project_a, "gpt-project-a"), (&project_b, "gpt-project-b")] {
        let run = Command::new(env!("CARGO_BIN_EXE_wecode"))
            .args([
                "codex-backend",
                "--config",
                config_path.to_str().expect("utf-8 config"),
                "--jsonl",
                "--cwd",
                project.to_str().expect("utf-8 project"),
                ":codex",
                "hello",
            ])
            .env("PATH", &path)
            .output()
            .expect("run codex");
        assert!(
            run.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let calls = fs::read_to_string(&calls_path).expect("calls");
        assert!(
            calls.contains(&format!("PWD={}", project.display()))
                || calls.contains(&format!("-C {}", project.display())),
            "calls:\n{calls}"
        );
        assert!(calls.contains(&format!("-m {model}")), "calls:\n{calls}");
    }
}

#[test]
fn models_command_lists_configured_models_without_invoking_codex() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        r#"{"codex":{"models":["default","gpt-5.4","gpt-next"]}}"#,
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":models",
        ])
        .env("PATH", path)
        .output()
        .expect("run models");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configured Codex models"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("gpt-5.4"), "stdout:\n{stdout}");
    assert!(stdout.contains("gpt-next"), "stdout:\n{stdout}");
    assert!(!calls_path.exists(), "models must not invoke codex");
}

#[test]
fn openclaw_default_model_arg_still_allows_local_model_override() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let set_model = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":model",
            "gpt-5.5",
        ])
        .env("PATH", &path)
        .output()
        .expect("set model");
    assert!(
        set_model.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&set_model.stderr)
    );

    let run = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--model",
            "wecode-codex/default",
            ":codex",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run codex");

    assert!(
        run.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("-m gpt-5.5"));
}

#[test]
fn openclaw_model_arg_is_passed_to_codex_without_backend_prefix() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");

    write_fake_codex(&codex_path, &calls_path);
    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--jsonl",
            "--model",
            "wecode-codex/gpt-5.4",
            ":codex",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run codex");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("-m gpt-5.4"));
    assert!(!calls.contains("-m wecode-codex/gpt-5.4"));
}

#[test]
fn project_model_state_takes_precedence_over_openclaw_model_arg() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let set = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":model",
            "gpt-project",
        ])
        .env("PATH", &path)
        .output()
        .expect("set model");
    assert!(set.status.success());

    let run = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--model",
            "wecode-codex/gpt-openclaw",
            ":codex",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run codex");

    assert!(
        run.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(calls.contains("-m gpt-project"), "calls:\n{calls}");
    assert!(!calls.contains("-m gpt-openclaw"), "calls:\n{calls}");
}

#[test]
fn backend_cwd_arg_makes_codex_run_inside_project_directory() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).expect("project dir");
    let project_dir = project_dir.canonicalize().expect("canonical project dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    fs::write(
        &codex_path,
        format!(
            r#"#!/bin/sh
printf 'PWD=%s\n' "$(pwd)" >> {calls}
printf '%s\n' "$*" >> {calls}
echo '{{"thread_id":"fake-thread"}}'
echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"ok"}}]}}'
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(&codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex_path, permissions).expect("chmod fake codex");
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--cwd",
            project_dir.to_str().expect("utf-8 project dir"),
            ":codex",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run codex");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(
        calls.contains(&format!("PWD={}", project_dir.display())),
        "calls:\n{calls}"
    );
    assert!(
        calls.contains(&format!("-C {}", project_dir.display())),
        "calls:\n{calls}"
    );
}

#[test]
fn local_filesystem_commands_do_not_invoke_codex() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    let nested_dir = project_dir.join("nested");
    fs::create_dir_all(&nested_dir).expect("project dirs");
    let readme_path = project_dir.join("README.md");
    fs::write(&readme_path, "hello from project\n").expect("write file");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
            project_dir.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let pwd = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":pwd",
        ])
        .env("PATH", &path)
        .output()
        .expect("run pwd");
    assert!(pwd.status.success());
    assert!(String::from_utf8_lossy(&pwd.stdout).contains(&project_dir.display().to_string()));

    let ls = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":ls",
            ".",
        ])
        .env("PATH", &path)
        .output()
        .expect("run ls");
    assert!(ls.status.success());
    let ls_stdout = String::from_utf8_lossy(&ls.stdout);
    assert!(
        !ls_stdout.trim_start().starts_with('{'),
        "local ls should be plain text, stdout:\n{ls_stdout}"
    );
    assert!(
        !ls_stdout.contains("\"type\":\"message\""),
        "local ls should not expose JSON message wrappers, stdout:\n{ls_stdout}"
    );
    assert!(
        ls_stdout.starts_with("```text\n"),
        "local ls should be wrapped in a markdown code block, stdout:\n{ls_stdout}"
    );
    assert!(
        ls_stdout.trim_end().ends_with("\n```"),
        "local ls should close its markdown code block, stdout:\n{ls_stdout}"
    );
    assert!(ls_stdout.contains(&readme_path.display().to_string()));
    assert!(ls_stdout.contains(&nested_dir.display().to_string()));

    let cat = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":cat",
            "README.md",
        ])
        .env("PATH", &path)
        .output()
        .expect("run cat");
    assert!(cat.status.success());
    assert!(String::from_utf8_lossy(&cat.stdout).contains("hello from project"));

    let shell = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":shell",
            "pwd",
        ])
        .env("PATH", &path)
        .output()
        .expect("run shell");
    assert!(
        shell.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&shell.stderr)
    );
    let shell_stdout = String::from_utf8_lossy(&shell.stdout);
    assert!(
        !shell_stdout.trim_start().starts_with('{'),
        "local shell should be plain text, stdout:\n{shell_stdout}"
    );
    let canonical_project_dir = project_dir.canonicalize().expect("canonical project dir");
    assert_eq!(
        shell_stdout,
        format!("```text\n{}\n```\n", canonical_project_dir.display())
    );
    assert!(!calls_path.exists(), "local commands must not invoke codex");
}

#[test]
fn local_command_output_uses_longer_markdown_fence_when_needed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
            project_dir.display()
        ),
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":shell",
            "printf 'before\n```\nafter\n'",
        ])
        .output()
        .expect("run shell with markdown fence");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "````text\nbefore\n```\nafter\n````\n"
    );
}

#[test]
fn resume_command_binds_latest_codex_session_without_invoking_codex() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).expect("project dir");
    let project_dir = project_dir.canonicalize().expect("canonical project dir");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    let codex_home = temp.path().join("codex-home");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let sqlite_path = temp.path().join("sqlite3");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &sqlite_path,
        r#"#!/bin/sh
cat <<'JSON'
[{"id":"019e746e-aaaa-7bbb-8ccc-424fd4f025c7","title":"Conversation info (untrusted metadata):\n```json\n{\"message_id\":\"om_session\"}\n```\n\n[message_id: om_session]\nou_user: Codex title: review api | cli boundaries"},
{"id":"019e746e-4c43-74c2-b47a-424fd4f025c7","title":"Codex title: export accounts"},
{"id":"019e746e-bbbb-7ccc-8ddd-424fd4f025c7","title":"Codex title: other project"}]
JSON
"#,
    )
    .expect("write fake sqlite3");
    let mut sqlite_permissions = fs::metadata(&sqlite_path)
        .expect("sqlite metadata")
        .permissions();
    sqlite_permissions.set_mode(0o755);
    fs::set_permissions(&sqlite_path, sqlite_permissions).expect("chmod fake sqlite3");
    fs::create_dir_all(&codex_home).expect("codex home");
    fs::write(codex_home.join("state_5.sqlite"), "").expect("fake codex state db");
    fs::create_dir_all(codex_home.join("sessions/2026/05/29")).expect("session dirs");
    fs::write(
        codex_home
            .join("sessions/2026/05/29/rollout-019e746e-4c43-74c2-b47a-424fd4f025c7.jsonl"),
        format!(
            r#"{{"timestamp":"2026-05-29T15:50:46.110Z","type":"session_meta","payload":{{"id":"019e746e-4c43-74c2-b47a-424fd4f025c7","timestamp":"2026-05-29T15:50:46.110Z","cwd":"{}","originator":"codex_exec"}}}}
{{"timestamp":"2026-05-29T15:50:47.110Z","type":"event_msg","payload":{{"type":"user_message","message":"Implement account export command"}}}}
"#,
            project_dir.display()
        ),
    )
    .expect("write session");
    fs::write(
        codex_home
            .join("sessions/2026/05/29/rollout-019e746e-aaaa-7bbb-8ccc-424fd4f025c7.jsonl"),
        format!(
            r#"{{"timestamp":"2026-05-29T16:50:46.110Z","type":"session_meta","payload":{{"id":"019e746e-aaaa-7bbb-8ccc-424fd4f025c7","timestamp":"2026-05-29T16:50:46.110Z","cwd":"{}","originator":"codex-tui"}}}}
{{"timestamp":"2026-05-29T16:50:47.110Z","type":"event_msg","payload":{{"type":"user_message","message":"Review api | cli boundaries"}}}}
"#,
            project_dir.display()
        ),
    )
    .expect("write newer session");
    let other_project_dir = temp.path().join("other-project");
    fs::create_dir(&other_project_dir).expect("other project dir");
    let other_project_dir = other_project_dir
        .canonicalize()
        .expect("canonical other project dir");
    fs::write(
        codex_home
            .join("sessions/2026/05/29/rollout-019e746e-bbbb-7ccc-8ddd-424fd4f025c7.jsonl"),
        format!(
            r#"{{"timestamp":"2026-05-29T14:50:46.110Z","type":"session_meta","payload":{{"id":"019e746e-bbbb-7ccc-8ddd-424fd4f025c7","timestamp":"2026-05-29T14:50:46.110Z","cwd":"{}","originator":"codex-tui"}}}}
{{"timestamp":"2026-05-29T14:50:47.110Z","type":"event_msg","payload":{{"type":"user_message","message":"Other project JSONL prompt"}}}}
"#,
            other_project_dir.display()
        ),
    )
    .expect("write other project session");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
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
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":resume",
        ])
        .env("CODEX_HOME", &codex_home)
        .env("PATH", path)
        .output()
        .expect("run resume");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(r#""thread_id":"019e746e-aaaa-7bbb-8ccc-424fd4f025c7""#),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Resumed Codex session"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Codex title: review api | cli boundaries"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains(&project_dir.display().to_string()),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("019e746e-4c43-74c2-b47a-424fd4f025c7"),
        "resume should bind only the latest session, stdout:\n{stdout}"
    );
    assert!(!calls_path.exists(), "resume must not invoke codex");
}

#[test]
fn help_lists_shell_and_codex_commands_without_invoking_codex() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");

    write_fake_codex(&codex_path, &calls_path);
    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args(["codex-backend", "--jsonl", ":help"])
        .env("PATH", path)
        .output()
        .expect("run help");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "# Wecode 帮助",
        "`:help` 和 `:commands` 由 Wecode 本地直接返回，不会请求 Codex。",
        "## 普通消息",
        "## 本地命令（不调用 Codex）",
        "- `:help` / `:commands` - 显示本帮助，不请求 Codex",
        "- `:pwd` - 显示当前 Codex 项目目录",
        "- `:ls [path]` - 列出目录，返回绝对路径",
        "- `:cat <path>` - 读取项目内文本文件",
        "- `:cd <path>` - 切换 Codex 项目目录，下一次任务会新开 session",
        "- `:shell <command>` - 在当前 Codex 项目目录执行 shell 命令",
        "- `:diff` - 显示当前项目 git diff",
        "- `:status` - 显示后端、项目、session、审批、模型和 git 状态",
        "## Codex 命令（会调用 Codex）",
        "- `:init [notes]` - 转成 `/init ...` 后发送给 Codex 原生命令",
        "- `:review [instructions]` - 对未提交改动执行 Codex review",
        "- `:new [prompt]` - 转成 `/new ...` 后发送给 Codex 原生命令，不保证切换 Wecode 绑定的 session",
        "- `:compact [notes]` - 转成 `/compact ...` 后发送给 Codex 原生命令",
        "- `:plan [task]` - 转成 `/plan ...` 后发送给 Codex 原生命令",
        "- `:goal [objective]` - 转成 `/goal ...` 后发送给 Codex 原生命令",
        "- `:agent [task]` - 转成 `/agent ...` 后发送给 Codex 原生命令",
        "- `:side [question]` - 转成 `/side ...` 后发送给 Codex 原生命令",
        "- `:resume [session_id]` - 绑定到最近或指定的 Codex session，不请求 Codex",
        "- `:report [notes]` - 查询任务进展",
        "## Session 命令",
        "- `:fresh [prompt]` - 硬新开 Codex thread；带 prompt 时立即执行，不带 prompt 时作用于下一条请求",
        "## 审批命令",
        "- `:approve <id>` - 批准并执行待确认命令",
        "- `:deny <id>` - 拒绝待确认命令",
        "## 模型命令",
        "- `:model default` - 清除项目模型覆盖",
        "## 自定义命令",
        "- `:codex <text>` - ask",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains(":sessions"),
        "help should not list removed :sessions command:\n{stdout}"
    );
    assert!(!calls_path.exists(), "help must not invoke codex");
}

#[test]
fn cd_persists_project_directory_and_next_codex_run_starts_fresh_there() {
    let temp = tempfile::tempdir().expect("temp dir");
    let old_project = temp.path().join("old-project");
    let new_project = temp.path().join("new-project");
    fs::create_dir_all(&old_project).expect("old project");
    fs::create_dir_all(&new_project).expect("new project");
    let old_project = old_project.canonicalize().expect("canonical old project");
    let new_project = new_project.canonicalize().expect("canonical new project");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    fs::write(
        &codex_path,
        format!(
            r#"#!/bin/sh
printf 'PWD=%s\n' "$(pwd)" >> {calls}
printf '%s\n' "$*" >> {calls}
echo '{{"thread_id":"new-project-thread"}}'
echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"ok"}}]}}'
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(&codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex_path, permissions).expect("chmod fake codex");
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
            old_project.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let cd = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":cd",
            new_project.to_str().expect("utf-8 new project"),
        ])
        .env("PATH", &path)
        .output()
        .expect("run cd");
    assert!(cd.status.success());
    assert!(String::from_utf8_lossy(&cd.stdout).contains(&new_project.display().to_string()));

    let run = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--cwd",
            old_project.to_str().expect("utf-8 old project"),
            "--resume",
            "old-thread",
            ":codex",
            "hello",
        ])
        .env("PATH", path)
        .output()
        .expect("run codex");
    assert!(
        run.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(
        !calls.contains("exec resume"),
        "cd should force a fresh project thread"
    );
    assert!(calls.contains(&format!("PWD={}", new_project.display())));
    assert!(calls.contains(&format!("-C {}", new_project.display())));
}

#[test]
fn cd_from_decorated_openclaw_message_persists_project_directory() {
    let temp = tempfile::tempdir().expect("temp dir");
    let old_project = temp.path().join("old-project");
    let new_project = temp.path().join("new-project");
    fs::create_dir_all(&old_project).expect("old project");
    fs::create_dir_all(&new_project).expect("new project");
    let old_project = old_project.canonicalize().expect("canonical old project");
    let new_project = new_project.canonicalize().expect("canonical new project");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
            old_project.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let decorated = format!(
        r#"Conversation info (untrusted metadata):
```json
{{"message_id":"om_x100b6e977ba6b4b0b34edb547991066"}}
```

[message_id: om_x100b6e977ba6b4b0b34edb547991066]
ou_08e494561f9ff0e2bd8015472c28e6e5: :cd {}"#,
        new_project.display()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
        ])
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn decorated cd");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(decorated.as_bytes())
        .expect("write decorated cd");
    let output = child.wait_with_output().expect("run decorated cd");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(state_dir.join("codex-cwd.txt")).expect("cwd override")
            == new_project.display().to_string()
    );
    assert!(
        state_dir.join("codex-fresh-next.txt").exists(),
        "cd should force the next turn to start fresh"
    );
    assert!(!calls_path.exists(), "cd must not invoke codex");
}

#[test]
fn codex_backend_writes_prompt_flow_log_for_decorated_multiline_prompt() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let project = project.canonicalize().expect("canonical project");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
            project.display()
        ),
    )
    .expect("write config");

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let decorated = r#"Conversation info (untrusted metadata):
```json
{"message_id":"om_prompt_flow"}
```

[message_id: om_prompt_flow]
o9cq805CIQyEJ1pliCh0GGdeTy98@im.wechat: :compact keep decisions
保留项目边界
保留当前计划"#;
    let expected_command_input = ":compact keep decisions\n保留项目边界\n保留当前计划";
    let expected_final_prompt = "/compact keep decisions\n保留项目边界\n保留当前计划";

    let mut child = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--resume",
            "existing-thread",
        ])
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn prompt flow run");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(decorated.as_bytes())
        .expect("write decorated prompt");
    let output = child.wait_with_output().expect("run prompt flow");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let log_dir = state_dir.join("logs");
    let mut prompt_flow_logs = fs::read_dir(&log_dir)
        .expect("prompt flow log dir")
        .map(|entry| entry.expect("log entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("prompt-flow.log"))
        })
        .collect::<Vec<_>>();
    prompt_flow_logs.sort();
    assert!(
        !prompt_flow_logs.is_empty(),
        "missing rotated prompt flow log in {}",
        log_dir.display()
    );
    let log = prompt_flow_logs
        .iter()
        .map(|path| fs::read_to_string(path).expect("prompt flow log"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !log.trim_start().starts_with('{'),
        "log must not be JSONL:\n{log}"
    );
    assert!(
        log.contains("event=\"backend_input_received\""),
        "log:\n{log}"
    );
    assert!(log.contains("prompt_source=stdin"), "log:\n{log}");
    assert!(log.contains("raw_prompt="), "log:\n{log}");
    assert!(log.contains("om_prompt_flow"), "log:\n{log}");
    assert!(
        log.contains("event=\"backend_input_prepared\""),
        "log:\n{log}"
    );
    assert!(log.contains("backend_input_kind=\"prompt\""), "log:\n{log}");
    assert!(
        log.contains(&format!("{expected_command_input:?}")),
        "log:\n{log}"
    );
    assert!(
        log.contains(&format!("{expected_final_prompt:?}")),
        "log:\n{log}"
    );
    assert!(
        log.contains("event=\"codex_prompt_dispatch\""),
        "log:\n{log}"
    );
    assert!(
        log.contains("requested_resume_session_id=Some(\"existing-thread\")"),
        "log:\n{log}"
    );
    assert!(log.contains("event=\"codex_exec_command\""), "log:\n{log}");
    assert!(log.contains(&project.display().to_string()), "log:\n{log}");
    assert!(log.contains("\"existing-thread\""), "log:\n{log}");
}

#[test]
fn cd_does_not_persist_partial_state_when_state_dir_cannot_store_fresh_marker() {
    let temp = tempfile::tempdir().expect("temp dir");
    let old_project = temp.path().join("old-project");
    let new_project = temp.path().join("new-project");
    fs::create_dir_all(&old_project).expect("old project");
    fs::create_dir_all(&new_project).expect("new project");
    let old_project = old_project.canonicalize().expect("canonical old project");
    let new_project = new_project.canonicalize().expect("canonical new project");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&state_dir).expect("state dir");
    fs::create_dir(state_dir.join("codex-fresh-next.txt")).expect("fresh marker directory");

    fs::write(
        &config_path,
        format!(
            r#"{{
              "openclaw": {{"stateDir": "{}"}},
              "codex": {{"cwd": "{}"}}
            }}"#,
            state_dir.display(),
            old_project.display()
        ),
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":cd",
            new_project.to_str().expect("utf-8 new project"),
        ])
        .output()
        .expect("run cd");

    assert!(!output.status.success(), "cd should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot change Codex project directory"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("openclaw.stateDir"), "stderr:\n{stderr}");
    assert!(
        !state_dir.join("codex-cwd.txt").exists(),
        "failed cd must not persist a new cwd override"
    );
}

#[test]
fn new_command_is_forwarded_to_codex_even_when_weixin_session_is_bound() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(r#"{{"openclaw":{{"stateDir":"{}"}}}}"#, state_dir.display()),
    )
    .expect("write config");
    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--resume",
            "existing-thread",
            ":new",
            "start fresh",
        ])
        .env("PATH", path)
        .output()
        .expect("run new command");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(
        calls.contains("exec resume"),
        "wecode should forward :new as /new to the current Codex invocation"
    );
    assert!(calls.contains("/new start fresh"));
}

#[test]
fn fresh_command_runs_prompt_in_new_codex_thread() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");

    write_fake_codex(&codex_path, &calls_path);
    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--jsonl",
            "--resume",
            "existing-thread",
            ":fresh",
            "start clean",
        ])
        .env("PATH", path)
        .output()
        .expect("run fresh command");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(calls_path).expect("calls");
    assert!(
        !calls.contains("exec resume"),
        ":fresh should force a new Codex thread"
    );
    assert!(calls.contains("start clean"));
}

#[test]
fn fresh_prompt_consumes_pending_fresh_marker() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");
    let config_path = temp.path().join("wecode.json");
    let state_dir = temp.path().join("state");

    write_fake_codex(&codex_path, &calls_path);
    fs::write(
        &config_path,
        format!(
            r#"{{"openclaw": {{"stateDir": "{}"}}}}"#,
            state_dir.display()
        ),
    )
    .expect("write config");
    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );

    let mark = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            ":fresh",
        ])
        .env("PATH", &path)
        .output()
        .expect("mark fresh");
    assert!(mark.status.success());

    let run = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "--resume",
            "existing-thread",
            ":fresh",
            "start clean",
        ])
        .env("PATH", path)
        .output()
        .expect("run fresh prompt");
    assert!(
        run.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let marker = state_dir.join("codex-fresh-next.txt");
    assert!(
        !marker.exists(),
        ":fresh prompt should consume the pending fresh marker"
    );
}

fn write_fake_codex(codex_path: &std::path::Path, calls_path: &std::path::Path) {
    fs::write(
        codex_path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {calls}
echo '{{"thread_id":"fake-thread"}}'
echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"ok"}}]}}'
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(codex_path, permissions).expect("chmod fake codex");
}

fn write_fake_codex_with_stdio_app_server(
    codex_path: &std::path::Path,
    calls_path: &std::path::Path,
    stdio_app_server_path: &std::path::Path,
) {
    fs::write(
        codex_path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {calls}
case "$*" in
  "app-server --listen stdio://"*)
    exec {stdio_app_server}
    ;;
  "app-server proxy"*)
    echo 'managed proxy unavailable' >&2
    exit 1
    ;;
  "exec "*)
    echo '{{"thread_id":"unexpected-exec-thread"}}'
    echo '{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"unexpected exec"}}]}}'
    exit 0
    ;;
  *)
    echo "unexpected codex command: $*" >&2
    exit 1
    ;;
esac
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 path")),
            stdio_app_server = shell_quote(stdio_app_server_path.to_str().expect("utf-8 path"))
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(codex_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(codex_path, permissions).expect("chmod fake codex");
}

fn write_fake_remote_proxy(
    proxy_path: &std::path::Path,
    calls_path: &std::path::Path,
    thread_id: &str,
    final_text: &str,
) {
    fs::write(
        proxy_path,
        format!(
            r#"#!/bin/sh
thread_id={thread_id}
final_text={final_text}
while IFS= read -r line; do
  printf '%s\n' "$line" >> {calls}
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"codexHome":"/tmp/codex","platformFamily":"unix","platformOs":"macos","userAgent":"fake"}}}}\n' "$id"
      ;;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"%s"}},"cwd":"/tmp","model":"fake","modelProvider":"fake","approvalPolicy":"never","approvalsReviewer":"user","sandbox":{{"mode":"workspace-write"}}}}}}\n' "$id" "$thread_id"
      ;;
    *'"method":"turn/start"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"turn":{{"id":"turn-1","status":"inProgress","items":[]}}}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{{"threadId":"%s","turnId":"turn-1","itemId":"msg-1","delta":"%s"}}}}\n' "$thread_id" "$final_text"
      printf '{{"jsonrpc":"2.0","method":"item/completed","params":{{"threadId":"%s","turnId":"turn-1","item":{{"type":"agentMessage","id":"msg-1","text":"%s","phase":"final_answer","memoryCitation":null}}}}}}\n' "$thread_id" "$final_text"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"%s","turn":{{"id":"turn-1","status":"completed","items":[],"itemsView":"notLoaded"}}}}}}\n' "$thread_id"
      ;;
  esac
done
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 calls path")),
            thread_id = shell_quote(thread_id),
            final_text = shell_quote(final_text)
        ),
    )
    .expect("write fake remote proxy");
    let mut permissions = fs::metadata(proxy_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(proxy_path, permissions).expect("chmod fake remote proxy");
}

fn write_failing_remote_start(start_path: &std::path::Path, calls_path: &std::path::Path) {
    fs::write(
        start_path,
        format!(
            r#"#!/bin/sh
echo 'start attempted' >> {calls}
echo 'Error: managed standalone Codex install not found at ~/.codex/packages/standalone/current/codex' >&2
exit 1
"#,
            calls = shell_quote(calls_path.to_str().expect("utf-8 calls path")),
        ),
    )
    .expect("write fake remote start");
    let mut permissions = fs::metadata(start_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(start_path, permissions).expect("chmod fake remote start");
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

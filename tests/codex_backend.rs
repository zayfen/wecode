use std::{env, fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn codex_backend_falls_back_to_fresh_thread_when_resume_rollout_is_missing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let codex_path = temp.path().join("codex");
    let calls_path = temp.path().join("codex-calls.txt");

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
                  "prefix": "/deploy ",
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
            "/deploy",
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
    assert!(request_stdout.contains("/approve "));
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
            "/approve",
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
            "/model",
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
            "/codex",
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
                "/model",
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
                "/codex",
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
            "/models",
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
            "/model",
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
            "/codex",
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
            "/codex",
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
            "/model",
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
            "/codex",
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

    let path = format!(
        "{}:{}",
        temp.path().display(),
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--jsonl",
            "--cwd",
            project_dir.to_str().expect("utf-8 project dir"),
            "/codex",
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
            "/pwd",
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
            "/ls",
            ".",
        ])
        .env("PATH", &path)
        .output()
        .expect("run ls");
    assert!(ls.status.success());
    let ls_stdout = String::from_utf8_lossy(&ls.stdout);
    assert!(ls_stdout.contains(&readme_path.display().to_string()));
    assert!(ls_stdout.contains(&nested_dir.display().to_string()));

    let cat = Command::new(env!("CARGO_BIN_EXE_wecode"))
        .args([
            "codex-backend",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--jsonl",
            "/cat",
            "README.md",
        ])
        .env("PATH", path)
        .output()
        .expect("run cat");
    assert!(cat.status.success());
    assert!(String::from_utf8_lossy(&cat.stdout).contains("hello from project"));
    assert!(!calls_path.exists(), "local commands must not invoke codex");
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
        .args(["codex-backend", "--jsonl", "/help"])
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
        "Wecode help",
        "Shell commands",
        "/pwd",
        "/ls [path]",
        "/cat <path>",
        "/cd <path>",
        "Codex commands",
        "/init [notes]",
        "/review [instructions]",
        "/new [prompt]",
        "/compact [notes]",
        "/plan [task]",
        "/goal [objective]",
        "/agent [task]",
        "/side [question]",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
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
            "/cd",
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
            "/codex",
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
fn new_command_starts_fresh_thread_even_when_weixin_session_is_bound() {
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
            "/new",
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
        !calls.contains("exec resume"),
        "new must not resume old thread"
    );
    assert!(calls.contains("start fresh"));
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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

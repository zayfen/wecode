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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

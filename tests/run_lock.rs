use std::fs;

use tempfile::tempdir;
use wecode::{read_config_str, run_lock::try_acquire_codex_run_lock};

#[test]
fn codex_run_lock_blocks_second_owner() {
    let temp = tempdir().expect("tempdir");
    let state_dir = temp.path().join("state");
    let config = read_config_str(&format!(
        r#"{{"openclaw":{{"stateDir":{}}}}}"#,
        serde_json::to_string(&state_dir.display().to_string()).expect("state json")
    ))
    .expect("config");

    let first = try_acquire_codex_run_lock(&config, "thread-1").expect("first lock");
    let second = try_acquire_codex_run_lock(&config, "thread-1");
    assert!(second.is_err());

    drop(first);
    assert!(try_acquire_codex_run_lock(&config, "thread-1").is_ok());
    assert!(fs::read_dir(state_dir.join("locks")).is_ok());
}

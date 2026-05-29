use std::{fs, path::Path};

use wecode::list_codex_sessions;

#[test]
fn lists_codex_sessions_for_current_project_only() {
    let temp = tempfile::tempdir().expect("temp dir");
    let sessions_root = temp.path().join("sessions");
    write_session(
        &sessions_root.join("2026/05/29/rollout-new.jsonl"),
        "019e746e-4c43-74c2-b47a-424fd4f025c7",
        "2026-05-29T15:50:46.110Z",
        "/repo/wecode",
        "codex-tui",
        "new project session",
    );
    write_session(
        &sessions_root.join("2026/05/28/rollout-old.jsonl"),
        "019e6f79-2feb-7f42-9880-629b573262f5",
        "2026-05-28T16:44:33.000Z",
        "/repo/wecode",
        "codex_exec",
        "old project session",
    );
    write_session(
        &sessions_root.join("2026/05/29/rollout-other.jsonl"),
        "019e73f3-a1d1-7f11-ac5f-8029b86f53a7",
        "2026-05-29T13:36:47.081Z",
        "/repo/other",
        "codex-tui",
        "other project session",
    );

    let sessions =
        list_codex_sessions(&sessions_root, Path::new("/repo/wecode"), 10).expect("session list");

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "019e746e-4c43-74c2-b47a-424fd4f025c7");
    assert_eq!(sessions[0].title, "new project session");
    assert_eq!(sessions[0].initial_prompt, "new project session");
    assert_eq!(sessions[1].id, "019e6f79-2feb-7f42-9880-629b573262f5");
}

fn write_session(path: &Path, id: &str, timestamp: &str, cwd: &str, originator: &str, title: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(
        path,
        format!(
            r#"{{"timestamp":"{timestamp}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{timestamp}","cwd":"{cwd}","originator":"{originator}"}}}}
{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"user_message","message":"{title}"}}}}
"#
        ),
    )
    .expect("write session");
}

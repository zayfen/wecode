use std::{fs, path::Path};

use wecode::{list_all_codex_sessions, list_codex_sessions};

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

#[test]
fn lists_all_codex_sessions_without_project_filter() {
    let temp = tempfile::tempdir().expect("temp dir");
    let sessions_root = temp.path().join("sessions");
    write_session(
        &sessions_root.join("2026/05/29/rollout-project.jsonl"),
        "019e746e-4c43-74c2-b47a-424fd4f025c7",
        "2026-05-29T15:50:46.110Z",
        "/repo/wecode",
        "codex-tui",
        "project session",
    );
    write_session(
        &sessions_root.join("2026/05/29/rollout-other.jsonl"),
        "019e73f3-a1d1-7f11-ac5f-8029b86f53a7",
        "2026-05-29T13:36:47.081Z",
        "/repo/other",
        "codex-tui",
        "other project session",
    );

    let sessions = list_all_codex_sessions(&sessions_root, 10).expect("session list");

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "019e746e-4c43-74c2-b47a-424fd4f025c7");
    assert_eq!(sessions[1].id, "019e73f3-a1d1-7f11-ac5f-8029b86f53a7");
}

#[test]
fn session_title_skips_injected_context_when_state_db_is_unavailable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let sessions_root = temp.path().join("sessions");
    let path = sessions_root.join("2026/05/30/rollout-init.jsonl");
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(
        &path,
        r##"{"timestamp":"2026-05-30T00:19:26.118Z","type":"session_meta","payload":{"id":"019e763f-ff0c-7683-a4e9-a17e2bf964fb","timestamp":"2026-05-30T00:19:26.118Z","cwd":"/repo/wecode","originator":"codex_exec"}}
{"timestamp":"2026-05-30T00:19:27.044Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /repo/wecode\n\n<INSTRUCTIONS>local instructions</INSTRUCTIONS>"},{"type":"input_text","text":"<environment_context>\n  <cwd>/repo/wecode</cwd>\n</environment_context>"}]}}
{"timestamp":"2026-05-30T00:19:27.045Z","type":"event_msg","payload":{"type":"user_message","message":"/init"}}
"##,
    )
    .expect("write session");

    let sessions =
        list_codex_sessions(&sessions_root, Path::new("/repo/wecode"), 10).expect("session list");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, "/init");
    assert_eq!(sessions[0].initial_prompt, "/init");
}

#[test]
fn session_title_strips_openclaw_message_metadata_when_state_db_is_unavailable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let sessions_root = temp.path().join("sessions");
    let path = sessions_root.join("2026/05/30/rollout-openclaw.jsonl");
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(
        &path,
        r##"{"timestamp":"2026-05-30T00:19:26.118Z","type":"session_meta","payload":{"id":"019e763f-ff0c-7683-a4e9-a17e2bf964fb","timestamp":"2026-05-30T00:19:26.118Z","cwd":"/repo/wecode","originator":"codex_exec"}}
{"timestamp":"2026-05-30T00:19:27.044Z","type":"event_msg","payload":{"type":"user_message","message":"Conversation info (untrusted metadata):\n```json\n{\"message_id\":\"om_prompt\"}\n```\n\n[message_id: om_prompt]\nou_08e494561f9ff0e2bd8015472c28e6e5: 获取当前项目的sessions\n第二行"}}
"##,
    )
    .expect("write session");

    let sessions =
        list_codex_sessions(&sessions_root, Path::new("/repo/wecode"), 10).expect("session list");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, "获取当前项目的sessions 第二行");
    assert_eq!(sessions[0].initial_prompt, "获取当前项目的sessions 第二行");
}

#[test]
fn session_title_strips_openclaw_reply_prefix_when_state_db_is_unavailable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let sessions_root = temp.path().join("sessions");
    let path = sessions_root.join("2026/05/30/rollout-reply.jsonl");
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(
        &path,
        r##"{"timestamp":"2026-05-30T00:19:26.118Z","type":"session_meta","payload":{"id":"019e763f-ff0c-7683-a4e9-a17e2bf964fb","timestamp":"2026-05-30T00:19:26.118Z","cwd":"/repo/wecode","originator":"codex_exec"}}
{"timestamp":"2026-05-30T00:19:27.044Z","type":"event_msg","payload":{"type":"user_message","message":"[Replying to: \":resume \"]\n\n:resume"}}
"##,
    )
    .expect("write session");

    let sessions =
        list_codex_sessions(&sessions_root, Path::new("/repo/wecode"), 10).expect("session list");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, ":resume");
    assert_eq!(sessions[0].initial_prompt, ":resume");
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

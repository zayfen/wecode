use wecode::{
    backend::{AssistantBackend, BackendRunRequest, CodexBackend},
    default_config,
    openclaw::{channel_spec, ChannelKind},
    BootstrapChannel,
};

#[test]
fn channel_specs_expose_channel_specific_bootstrap_metadata() {
    let weixin = channel_spec(BootstrapChannel::Weixin);
    assert_eq!(weixin.kind, ChannelKind::Weixin);
    assert_eq!(weixin.name, "weixin");
    assert_eq!(
        weixin.visible_install_spinner,
        Some("Wecode 启动中 · 安装微信")
    );
    assert!(!weixin.requires_runtime_patch);

    let feishu = channel_spec(BootstrapChannel::Feishu);
    assert_eq!(feishu.kind, ChannelKind::Feishu);
    assert_eq!(feishu.name, "feishu");
    assert_eq!(feishu.visible_install_spinner, None);
    assert!(!feishu.requires_runtime_patch);
}

#[test]
fn codex_backend_exposes_backend_identifier() {
    let cfg = default_config();
    let backend = CodexBackend::default();
    let request = BackendRunRequest {
        config: &cfg,
        prompt: "hello",
        jsonl: true,
        selected_model: None,
        resume_session_id: None,
    };

    assert_eq!(backend.id(), "codex");
    assert_eq!(request.prompt, "hello");
    assert!(request.jsonl);
}

#[test]
fn codex_backend_builds_codex_exec_command_spec() {
    let mut cfg = default_config();
    cfg.codex.cwd = Some(".".to_string());
    let backend = CodexBackend::default();
    let request = BackendRunRequest {
        config: &cfg,
        prompt: "inspect project",
        jsonl: true,
        selected_model: Some("wecode-codex/gpt-5.5"),
        resume_session_id: None,
    };

    let spec = backend
        .run_command_spec(&request, std::path::Path::new("/tmp/wecode-output.txt"))
        .expect("command spec");

    assert_eq!(spec.program, "codex");
    assert_eq!(
        spec.args,
        vec![
            "exec",
            "--json",
            "-o",
            "/tmp/wecode-output.txt",
            "-m",
            "gpt-5.5",
            "-s",
            "workspace-write",
            "-C",
            spec.cwd.to_str().expect("cwd utf-8"),
            "--",
            "inspect project"
        ]
    );
}

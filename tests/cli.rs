use wecode::{parse_cli_args, CliCommand};

#[test]
fn parses_cli_commands() {
    assert_eq!(
        parse_cli_args(["wecode", "bootstrap", "--dry-run", "--install-openclaw"]),
        Ok(CliCommand::Bootstrap {
            config_path: None,
            dry_run: true,
            install_openclaw: true,
        })
    );

    assert_eq!(
        parse_cli_args(["wecode", "config", "validate", "wecode.json"]),
        Ok(CliCommand::ConfigValidate {
            path: Some("wecode.json".to_string())
        })
    );

    assert_eq!(
        parse_cli_args(["wecode", "render", "/codex", "hello"]),
        Ok(CliCommand::Render {
            config_path: None,
            input: "/codex hello".to_string(),
        })
    );

    assert_eq!(
        parse_cli_args([
            "wecode",
            "codex-backend",
            "--config",
            "wecode.json",
            "hello"
        ]),
        Ok(CliCommand::CodexBackend {
            config_path: Some("wecode.json".to_string()),
            jsonl: false,
            model: None,
            cwd: None,
            prompt: Some("hello".to_string()),
            resume_session_id: None,
        })
    );

    assert_eq!(
        parse_cli_args(["wecode", "codex-backend"]),
        Ok(CliCommand::CodexBackend {
            config_path: None,
            jsonl: false,
            model: None,
            cwd: None,
            prompt: None,
            resume_session_id: None,
        })
    );

    assert_eq!(
        parse_cli_args([
            "wecode",
            "codex-backend",
            "--jsonl",
            "--resume",
            "019e715e-a44c-70d3-a732-9e9e55e1a1c1"
        ]),
        Ok(CliCommand::CodexBackend {
            config_path: None,
            jsonl: true,
            model: None,
            cwd: None,
            prompt: None,
            resume_session_id: Some("019e715e-a44c-70d3-a732-9e9e55e1a1c1".to_string()),
        })
    );

    assert_eq!(
        parse_cli_args([
            "wecode",
            "codex-backend",
            "--jsonl",
            "--model",
            "wecode-codex/gpt-5.4",
            "hello"
        ]),
        Ok(CliCommand::CodexBackend {
            config_path: None,
            jsonl: true,
            model: Some("wecode-codex/gpt-5.4".to_string()),
            cwd: None,
            prompt: Some("hello".to_string()),
            resume_session_id: None,
        })
    );

    assert_eq!(
        parse_cli_args([
            "wecode",
            "codex-backend",
            "--jsonl",
            "--cwd",
            "/Users/riven/Github/wecode",
            "hello"
        ]),
        Ok(CliCommand::CodexBackend {
            config_path: None,
            jsonl: true,
            model: None,
            cwd: Some("/Users/riven/Github/wecode".to_string()),
            prompt: Some("hello".to_string()),
            resume_session_id: None,
        })
    );
}

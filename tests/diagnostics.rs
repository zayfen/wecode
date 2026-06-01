use wecode::{diagnose_tools, parse_node_version, ToolSnapshot};

#[test]
fn parses_node_versions_from_common_outputs() {
    assert_eq!(parse_node_version("v24.1.0"), Some((24, 1, 0)));
    assert_eq!(parse_node_version("node v22.19.1\n"), Some((22, 19, 1)));
    assert_eq!(parse_node_version("not installed"), None);
}

#[test]
fn doctor_requires_node_22_19_openclaw_codex_and_npm() {
    let report = diagnose_tools(&ToolSnapshot {
        node_version: Some("v20.18.1".to_string()),
        npm_found: true,
        npx_found: true,
        openclaw_version: None,
        codex_version: Some("codex-cli 0.134.0".to_string()),
    });

    assert!(!report.ok);
    assert!(report
        .items
        .iter()
        .any(|item| item.name == "node" && !item.ok && item.message.contains(">=22.19.0")));
    assert!(report
        .items
        .iter()
        .any(|item| item.name == "openclaw" && !item.ok));
}

#[test]
fn doctor_does_not_require_npx_for_bootstrap() {
    let report = diagnose_tools(&ToolSnapshot {
        node_version: Some("v24.14.1".to_string()),
        npm_found: true,
        npx_found: false,
        openclaw_version: Some("OpenClaw 2026.5.28".to_string()),
        codex_version: Some("codex-cli 0.134.0".to_string()),
    });

    assert!(
        report.ok,
        "npx is not needed after Weixin install stopped shelling out to the installer CLI: {report:?}"
    );
    assert!(
        !report.items.iter().any(|item| item.name == "npx"),
        "doctor should not report npx as a required tool: {report:?}"
    );
}

const MIN_NODE_VERSION: (u64, u64, u64) = (22, 19, 0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSnapshot {
    pub node_version: Option<String>,
    pub npm_found: bool,
    pub npx_found: bool,
    pub openclaw_version: Option<String>,
    pub codex_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReport {
    pub ok: bool,
    pub items: Vec<ToolCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCheck {
    pub name: String,
    pub ok: bool,
    pub message: String,
}

impl ToolCheck {
    fn ok(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            message: message.into(),
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: false,
            message: message.into(),
        }
    }
}

pub fn parse_node_version(output: &str) -> Option<(u64, u64, u64)> {
    let mut numbers = Vec::new();
    let mut current = String::new();

    for ch in output.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' {
            if current.is_empty() {
                return None;
            }
            numbers.push(current.parse().ok()?);
            current.clear();
        } else if !current.is_empty() {
            numbers.push(current.parse().ok()?);
            break;
        }
    }

    if !current.is_empty() && numbers.len() < 3 {
        numbers.push(current.parse().ok()?);
    }

    match numbers.as_slice() {
        [major, minor, patch, ..] => Some((*major, *minor, *patch)),
        _ => None,
    }
}

pub fn diagnose_tools(snapshot: &ToolSnapshot) -> ToolReport {
    let mut items = Vec::new();

    match snapshot
        .node_version
        .as_deref()
        .and_then(parse_node_version)
    {
        Some(version) if version >= MIN_NODE_VERSION => items.push(ToolCheck::ok(
            "node",
            format!(
                "found {}",
                snapshot.node_version.as_deref().unwrap_or("node")
            ),
        )),
        Some(version) => items.push(ToolCheck::fail(
            "node",
            format!(
                "found {}. wecode requires Node >=22.19.0 for OpenClaw/latest",
                format_version(version)
            ),
        )),
        None => items.push(ToolCheck::fail(
            "node",
            "not found. Install Node 24 or Node >=22.19.0",
        )),
    }

    items.push(if snapshot.npm_found {
        ToolCheck::ok("npm", "found npm")
    } else {
        ToolCheck::fail("npm", "not found. Install Node with npm")
    });

    items.push(if snapshot.npx_found {
        ToolCheck::ok("npx", "found npx")
    } else {
        ToolCheck::fail("npx", "not found. Install Node with npx")
    });

    items.push(match snapshot.openclaw_version.as_deref() {
        Some(version) => ToolCheck::ok("openclaw", format!("found {version}")),
        None => ToolCheck::fail(
            "openclaw",
            "not found in wecode private runtime. Run `wecode bootstrap`",
        ),
    });

    items.push(match snapshot.codex_version.as_deref() {
        Some(version) => ToolCheck::ok("codex", format!("found {version}")),
        None => ToolCheck::fail(
            "codex",
            "not found. Install and log in to Codex CLI before using local smoke tests",
        ),
    });

    ToolReport {
        ok: items.iter().all(|item| item.ok),
        items,
    }
}

fn format_version(version: (u64, u64, u64)) -> String {
    format!("v{}.{}.{}", version.0, version.1, version.2)
}

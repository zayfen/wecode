use std::{fs, path::PathBuf};

use crate::{config::WecodeConfig, paths::expand_tilde};

pub fn project_yolo_enabled(config: &WecodeConfig) -> Result<bool, String> {
    let path = project_yolo_state_path(config)?;
    if !path.exists() {
        return Ok(false);
    }
    let value = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read yolo state {}: {err}", path.display()))?;
    Ok(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "on" | "1"
    ))
}

pub fn set_project_yolo_enabled(config: &WecodeConfig, enabled: bool) -> Result<(), String> {
    let path = project_yolo_state_path(config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create yolo state dir {}: {err}",
                parent.display()
            )
        })?;
    }
    fs::write(&path, if enabled { "true\n" } else { "false\n" })
        .map_err(|err| format!("failed to write yolo state {}: {err}", path.display()))
}

pub fn toggle_project_yolo_enabled(config: &WecodeConfig) -> Result<bool, String> {
    let enabled = !project_yolo_enabled(config)?;
    set_project_yolo_enabled(config, enabled)?;
    Ok(enabled)
}

fn project_yolo_state_path(config: &WecodeConfig) -> Result<PathBuf, String> {
    let project_key = codex_project_key(config)?;
    Ok(PathBuf::from(expand_tilde(&config.openclaw.state_dir))
        .join("codex-yolo")
        .join(format!("{}.txt", stable_path_hash(&project_key))))
}

fn codex_project_key(config: &WecodeConfig) -> Result<String, String> {
    Ok(codex_target_cwd(config)?.display().to_string())
}

fn codex_target_cwd(config: &WecodeConfig) -> Result<PathBuf, String> {
    let path = config
        .codex
        .cwd
        .as_deref()
        .map(expand_tilde)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?);
    if path.is_absolute() {
        Ok(path.canonicalize().unwrap_or(path))
    } else {
        let cwd = std::env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?;
        let absolute = cwd.join(path);
        Ok(absolute.canonicalize().unwrap_or(absolute))
    }
}

fn stable_path_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

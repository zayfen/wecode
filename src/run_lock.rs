use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{config::WecodeConfig, paths::expand_tilde};

pub struct CodexRunLock {
    path: Option<PathBuf>,
}

impl Drop for CodexRunLock {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn try_acquire_codex_run_lock(
    config: &WecodeConfig,
    key: &str,
) -> Result<CodexRunLock, String> {
    let lock_dir = PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("locks");
    if fs::create_dir_all(&lock_dir).is_err() {
        return Ok(CodexRunLock { path: None });
    }
    let path = lock_dir.join(format!("codex-run-{}.lock", stable_key_hash(key)));
    let content = format!("pid={}\nkey={key}\n", std::process::id());
    match create_lock_file(&path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes()).map_err(|err| {
                format!("failed to write Wecode run lock {}: {err}", path.display())
            })?;
            Ok(CodexRunLock { path: Some(path) })
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if remove_stale_lock_for_dead_pid(&path)? {
                let mut file = create_lock_file(&path).map_err(|err| {
                    format!("failed to create Wecode run lock {}: {err}", path.display())
                })?;
                file.write_all(content.as_bytes()).map_err(|err| {
                    format!("failed to write Wecode run lock {}: {err}", path.display())
                })?;
                return Ok(CodexRunLock { path: Some(path) });
            }
            Err(format!(
                "Codex is already running for this session or project. Try again after the current turn finishes, or approve/deny any pending Codex approval. Lock: {}. Cause: {err}",
                path.display()
            ))
        }
        Err(_) => Ok(CodexRunLock { path: None }),
    }
}

fn create_lock_file(path: &Path) -> io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn remove_stale_lock_for_dead_pid(path: &Path) -> Result<bool, String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Ok(false);
    };
    let Some(pid) = lock_pid(&content) else {
        return Ok(false);
    };
    if process_is_alive(pid) {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(err) => Err(format!(
            "failed to remove stale Wecode run lock {}: {err}",
            path.display()
        )),
    }
}

fn lock_pid(content: &str) -> Option<u32> {
    content
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|pid| pid.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0)
}

fn process_is_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

fn stable_key_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

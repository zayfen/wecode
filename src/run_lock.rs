use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::{config::WecodeConfig, paths::expand_tilde, platform};

pub struct CodexRunLock {
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopCodexRunResult {
    pub stopped: bool,
    pub pid: Option<u32>,
    pub lock_path: PathBuf,
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
    let lock_dir = codex_run_lock_dir(config);
    if fs::create_dir_all(&lock_dir).is_err() {
        return Ok(CodexRunLock { path: None });
    }
    let path = codex_run_lock_path(config, key);
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

pub fn stop_codex_run(config: &WecodeConfig, key: &str) -> Result<StopCodexRunResult, String> {
    let path = codex_run_lock_path(config, key);
    let Ok(content) = fs::read_to_string(&path) else {
        return Ok(StopCodexRunResult {
            stopped: false,
            pid: None,
            lock_path: path,
        });
    };
    let pid = lock_pid(&content);
    let stopped = if let Some(pid) = pid {
        if process_is_alive(pid) {
            terminate_process_tree(pid);
            true
        } else {
            false
        }
    } else {
        false
    };
    let _ = fs::remove_file(&path);
    Ok(StopCodexRunResult {
        stopped,
        pid,
        lock_path: path,
    })
}

fn codex_run_lock_dir(config: &WecodeConfig) -> PathBuf {
    PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("locks")
}

fn codex_run_lock_path(config: &WecodeConfig, key: &str) -> PathBuf {
    codex_run_lock_dir(config).join(format!("codex-run-{}.lock", stable_key_hash(key)))
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
    platform::process_is_alive(pid)
}

fn terminate_process_tree(pid: u32) {
    platform::kill_process_tree(pid);
}

fn stable_key_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

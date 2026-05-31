use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
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
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes()).map_err(|err| {
                format!("failed to write Wecode run lock {}: {err}", path.display())
            })?;
            Ok(CodexRunLock { path: Some(path) })
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Err(format!(
            "Codex is already running for this session or project. Try again after the current turn finishes, or approve/deny any pending Codex approval. Lock: {}. Cause: {err}",
            path.display()
        )),
        Err(_) => Ok(CodexRunLock { path: None }),
    }
}

fn stable_key_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

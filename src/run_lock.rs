use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use crate::{config::WecodeConfig, paths::expand_tilde};

pub struct CodexRunLock {
    path: PathBuf,
}

impl Drop for CodexRunLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn try_acquire_codex_run_lock(
    config: &WecodeConfig,
    key: &str,
) -> Result<CodexRunLock, String> {
    let lock_dir = PathBuf::from(expand_tilde(&config.openclaw.state_dir)).join("locks");
    fs::create_dir_all(&lock_dir)
        .map_err(|err| format!("failed to create Wecode lock dir: {err}"))?;
    let path = lock_dir.join(format!("codex-run-{}.lock", stable_key_hash(key)));
    let content = format!("pid={}\nkey={key}\n", std::process::id());
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes()).map_err(|err| {
                format!("failed to write Wecode run lock {}: {err}", path.display())
            })?;
            Ok(CodexRunLock { path })
        }
        Err(err) => Err(format!(
            "Codex is already running for this session or project. Try again after the current turn finishes, or approve/deny any pending Codex approval. Lock: {}. Cause: {err}",
            path.display()
        )),
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

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir()
}

pub fn process_is_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    platform_impl::process_is_alive(pid)
}

pub fn kill_process_tree(pid: u32) {
    if pid == std::process::id() {
        return;
    }
    platform_impl::kill_process_tree(pid);
}

#[cfg(unix)]
mod platform_impl {
    use super::*;

    pub fn process_is_alive(pid: u32) -> bool {
        Command::new("/bin/kill")
            .arg("-0")
            .arg(pid.to_string())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(true)
    }

    pub fn kill_process_tree(pid: u32) {
        let descendants = descendant_pids(pid);
        for child in descendants.iter().rev() {
            signal_process(*child, "TERM");
        }
        signal_process(pid, "TERM");

        let deadline = Instant::now() + Duration::from_millis(500);
        while super::process_is_alive(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if super::process_is_alive(pid) {
            for child in descendants.iter().rev() {
                signal_process(*child, "KILL");
            }
            signal_process(pid, "KILL");
        }
    }

    fn descendant_pids(pid: u32) -> Vec<u32> {
        let mut result = Vec::new();
        for child in child_pids(pid) {
            result.extend(descendant_pids(child));
            result.push(child);
        }
        result
    }

    fn child_pids(pid: u32) -> Vec<u32> {
        let output = Command::new("/usr/bin/pgrep")
            .arg("-P")
            .arg(pid.to_string())
            .output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .filter(|child| *child > 0 && *child != std::process::id())
            .collect()
    }

    fn signal_process(pid: u32, signal: &str) {
        if pid == std::process::id() {
            return;
        }
        let _ = Command::new("/bin/kill")
            .arg(format!("-{signal}"))
            .arg(pid.to_string())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(windows)]
mod platform_impl {
    use super::*;

    pub fn process_is_alive(pid: u32) -> bool {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.contains(&pid.to_string())
            }
            Err(_) => true,
        }
    }

    pub fn kill_process_tree(pid: u32) {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status();
    }
}

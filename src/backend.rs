use std::path::{Path, PathBuf};

use crate::{
    config::{codex_model_from_openclaw_model, WecodeConfig},
    paths::expand_tilde,
};

pub struct BackendRunRequest<'a> {
    pub config: &'a WecodeConfig,
    pub prompt: &'a str,
    pub jsonl: bool,
    pub selected_model: Option<&'a str>,
    pub resume_session_id: Option<&'a str>,
}

pub struct BackendReviewRequest<'a> {
    pub config: &'a WecodeConfig,
    pub instructions: Option<&'a str>,
    pub jsonl: bool,
    pub selected_model: Option<&'a str>,
}

pub trait AssistantBackend {
    fn id(&self) -> &'static str;

    fn run_command_spec(
        &self,
        request: &BackendRunRequest<'_>,
        output_path: &Path,
    ) -> Result<BackendCommandSpec, String>;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CodexBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl AssistantBackend for CodexBackend {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn run_command_spec(
        &self,
        request: &BackendRunRequest<'_>,
        output_path: &Path,
    ) -> Result<BackendCommandSpec, String> {
        let target_cwd = backend_target_cwd(request.config)?;
        let mut args = vec!["exec".to_string()];
        if request.resume_session_id.is_some() {
            args.push("resume".to_string());
        }
        args.push("--yolo".to_string());
        args.push("--json".to_string());
        args.push("-o".to_string());
        args.push(output_path.display().to_string());
        if let Some(model) = effective_request_model(request) {
            args.push("-m".to_string());
            args.push(model);
        }
        if let Some(session_id) = request.resume_session_id {
            args.push(session_id.to_string());
        } else {
            args.push("-s".to_string());
            args.push(request.config.codex.sandbox.clone());
            args.push("-C".to_string());
            args.push(target_cwd.display().to_string());
        }
        args.push("--".to_string());
        args.push(request.prompt.to_string());

        Ok(BackendCommandSpec {
            program: "codex".to_string(),
            args,
            cwd: target_cwd,
        })
    }
}

fn effective_request_model(request: &BackendRunRequest<'_>) -> Option<String> {
    if let Some(model) = request.selected_model {
        if let Some(model) = codex_model_from_openclaw_model(model) {
            return Some(model);
        }
    }
    request.config.codex.model.clone()
}

fn backend_target_cwd(config: &WecodeConfig) -> Result<PathBuf, String> {
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

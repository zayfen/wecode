use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStep {
    pub program: String,
    pub args: Vec<String>,
    pub path_prepend: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl CommandStep {
    pub fn new<const N: usize>(program: &str, args: [&str; N]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            path_prepend: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn from_vec(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            path_prepend: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn with_path_prepend(mut self, path: impl Into<String>) -> Self {
        self.path_prepend.push(path.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env.extend(
            envs.into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    pub fn display_shell(&self) -> String {
        let mut parts = Vec::new();
        parts.extend(
            self.env
                .iter()
                .map(|(key, value)| format!("{key}={}", shell_quote(value))),
        );
        if !self.path_prepend.is_empty() {
            parts.push(format!(
                "PATH={}:$PATH",
                self.path_prepend
                    .iter()
                    .map(|path| shell_quote(path))
                    .collect::<Vec<_>>()
                    .join(":")
            ));
        }
        parts.push(shell_quote(&self.program));
        parts.extend(self.args.iter().map(|arg| shell_quote(arg)));
        parts.join(" ")
    }
}

impl fmt::Display for CommandStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_shell())
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=@~".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

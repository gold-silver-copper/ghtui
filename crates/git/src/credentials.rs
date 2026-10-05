//! Credentials for git operations on the cache clone.
//!
//! The token reaches git through a credential helper or `GIT_ASKPASS`, with
//! configuration passed as `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/
//! `GIT_CONFIG_VALUE_n` environment variables. It never appears on a command
//! line (visible in `ps`), in a remote URL, or in a config file on disk.
//! The user's own clones keep their own credentials; this isn't applied there.

use std::path::PathBuf;

use tokio::process::Command;

/// Environment variable telling the ghtui binary to act as `GIT_ASKPASS`.
pub const ASKPASS_FLAG: &str = "GHTUI_ASKPASS";
/// Environment variable carrying the token to the askpass helper (only ever
/// set on git child processes).
pub const ASKPASS_TOKEN: &str = "GHTUI_ASKPASS_TOKEN";

#[derive(Clone, Default)]
pub enum Credentials {
    /// Use whatever git is configured with.
    #[default]
    Ambient,
    /// `gh auth git-credential` as the only credential helper.
    GhHelper,
    /// Run `program` (the ghtui binary) as `GIT_ASKPASS`, answering with
    /// `token`.
    AskPass { program: PathBuf, token: String },
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credentials::Ambient => f.write_str("Ambient"),
            Credentials::GhHelper => f.write_str("GhHelper"),
            Credentials::AskPass { program, .. } => f
                .debug_struct("AskPass")
                .field("program", program)
                .field("token", &"<redacted>")
                .finish(),
        }
    }
}

impl Credentials {
    /// Environment variables to set on a git command.
    pub fn env(&self) -> Vec<(String, String)> {
        // An empty helper value resets the list, so the user's configured
        // helpers can't answer (or prompt) for these requests.
        let helpers: &[&str] = match self {
            Credentials::Ambient => return Vec::new(),
            Credentials::GhHelper => &["", "!gh auth git-credential"],
            Credentials::AskPass { .. } => &[""],
        };
        let mut env = vec![("GIT_CONFIG_COUNT".to_owned(), helpers.len().to_string())];
        for (i, helper) in helpers.iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{i}"), "credential.helper".into()));
            env.push((format!("GIT_CONFIG_VALUE_{i}"), (*helper).into()));
        }
        if let Credentials::AskPass { program, token } = self {
            env.push(("GIT_ASKPASS".into(), program.to_string_lossy().into()));
            env.push((ASKPASS_FLAG.into(), "1".into()));
            env.push((ASKPASS_TOKEN.into(), token.clone()));
        }
        env
    }

    pub(crate) fn apply(&self, cmd: &mut Command) {
        for (k, v) in self.env() {
            cmd.env(k, v);
        }
    }
}

/// The askpass helper's answer to git's prompt: a fixed username (GitHub
/// ignores it for token auth) or the token.
pub fn askpass_answer(prompt: &str, token: &str) -> String {
    if prompt.to_ascii_lowercase().starts_with("username") {
        "x-access-token".to_owned()
    } else {
        token.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn gh_helper_replaces_configured_helpers() {
        let env = Credentials::GhHelper.env();
        assert_eq!(get(&env, "GIT_CONFIG_COUNT"), Some("2"));
        assert_eq!(get(&env, "GIT_CONFIG_VALUE_0"), Some(""));
        assert_eq!(
            get(&env, "GIT_CONFIG_VALUE_1"),
            Some("!gh auth git-credential")
        );
        assert_eq!(get(&env, "GIT_ASKPASS"), None);
    }

    #[test]
    fn askpass_passes_token_only_in_env() {
        let creds = Credentials::AskPass {
            program: "/bin/ghtui".into(),
            token: "secret".into(),
        };
        let env = creds.env();
        assert_eq!(get(&env, "GIT_ASKPASS"), Some("/bin/ghtui"));
        assert_eq!(get(&env, ASKPASS_TOKEN), Some("secret"));
        assert!(!format!("{creds:?}").contains("secret"));
        assert!(Credentials::Ambient.env().is_empty());
    }

    #[test]
    fn askpass_answers_prompts() {
        assert_eq!(
            askpass_answer("Username for 'https://github.com': ", "t"),
            "x-access-token"
        );
        assert_eq!(
            askpass_answer("Password for 'https://x-access-token@github.com': ", "t"),
            "t"
        );
    }
}

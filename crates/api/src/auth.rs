//! Token resolution: `GH_TOKEN`, then `GITHUB_TOKEN`, then `gh auth token`.
//!
//! The token is never logged or written anywhere; [`Token`]'s `Debug` output
//! is redacted.

use std::process::Command;

#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    GhTokenEnv,
    GithubTokenEnv,
    GhCli,
}

impl std::fmt::Display for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TokenSource::GhTokenEnv => "GH_TOKEN",
            TokenSource::GithubTokenEnv => "GITHUB_TOKEN",
            TokenSource::GhCli => "gh auth token",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no GitHub token found.\n\nRun `gh auth login` to sign in, or set GH_TOKEN.\n({0})")]
    NoToken(String),
}

/// Resolves a token from the real environment and `gh`.
pub fn resolve_token() -> Result<(Token, TokenSource), AuthError> {
    resolve_with(|k| std::env::var(k).ok(), run_gh_auth_token)
}

/// Testable core of [`resolve_token`].
pub fn resolve_with(
    env: impl Fn(&str) -> Option<String>,
    gh: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<(Token, TokenSource), AuthError> {
    for (var, source) in [
        ("GH_TOKEN", TokenSource::GhTokenEnv),
        ("GITHUB_TOKEN", TokenSource::GithubTokenEnv),
    ] {
        if let Some(value) = env(var).map(|v| v.trim().to_owned())
            && !value.is_empty()
        {
            return Ok((Token(value), source));
        }
    }
    match gh() {
        Ok(Some(token)) => Ok((Token(token), TokenSource::GhCli)),
        Ok(None) => Err(AuthError::NoToken(
            "`gh auth token` returned nothing".into(),
        )),
        Err(why) => Err(AuthError::NoToken(why)),
    }
}

fn run_gh_auth_token() -> Result<Option<String>, String> {
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => "the `gh` CLI is not installed".to_owned(),
            _ => format!("could not run `gh auth token`: {err}"),
        })?;
    if !output.status.success() {
        // gh's stderr says "not logged in" etc. It never contains the token.
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`gh auth token` failed: {}", stderr.trim()));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!token.is_empty()).then_some(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn prefers_gh_token_env() {
        let (token, source) =
            resolve_with(env(&[("GH_TOKEN", "a"), ("GITHUB_TOKEN", "b")]), || {
                panic!("gh must not run")
            })
            .unwrap();
        assert_eq!(token.expose(), "a");
        assert_eq!(source, TokenSource::GhTokenEnv);
    }

    #[test]
    fn falls_back_to_github_token_then_gh() {
        let (token, source) =
            resolve_with(env(&[("GH_TOKEN", "  "), ("GITHUB_TOKEN", "b")]), || {
                panic!("gh must not run")
            })
            .unwrap();
        assert_eq!((token.expose(), source), ("b", TokenSource::GithubTokenEnv));

        let (token, source) = resolve_with(env(&[]), || Ok(Some("c".into()))).unwrap();
        assert_eq!((token.expose(), source), ("c", TokenSource::GhCli));
    }

    #[test]
    fn missing_token_mentions_gh_auth_login() {
        let err = resolve_with(env(&[]), || Err("not logged in".into())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("gh auth login"), "{msg}");
        assert!(msg.contains("not logged in"), "{msg}");
    }

    #[test]
    fn debug_is_redacted() {
        assert_eq!(format!("{:?}", Token::new("secret")), "Token(<redacted>)");
    }
}

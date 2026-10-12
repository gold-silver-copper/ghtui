//! Token resolution, as gh does it. For github.com: `GH_TOKEN`, then
//! `GITHUB_TOKEN`, then `gh auth token`; for an Enterprise Server,
//! `GH_ENTERPRISE_TOKEN`, then `GITHUB_ENTERPRISE_TOKEN`, then
//! `gh auth token --hostname HOST`.
//!
//! The token is never logged or written anywhere; [`Token`]'s `Debug` output
//! is redacted.

use std::process::Command;

use sha2::{Digest, Sha256};

use crate::host::Host;

#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Names the token without giving it away: the start of its SHA-256,
    /// to tell accounts apart before GitHub says whose it is.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.0.as_bytes());
        digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// An environment variable.
    Env(&'static str),
    GhCli,
}

impl std::fmt::Display for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TokenSource::Env(var) => var,
            TokenSource::GhCli => "gh auth token",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no token found for {host}.\n\nTo sign in, {}.\n({why})", sign_in_hint(*host))]
    NoToken { host: Host, why: String },
}

/// How to sign in to `host`: "run `gh auth login` or set GH_TOKEN".
pub fn sign_in_hint(host: Host) -> String {
    let [var, _] = token_vars(host);
    if host.is_enterprise() {
        format!("run `gh auth login --hostname {host}` or set {var}")
    } else {
        format!("run `gh auth login` or set {var}")
    }
}

/// The variables a token for `host` is read from, in order.
fn token_vars(host: Host) -> [&'static str; 2] {
    if host.is_enterprise() {
        ["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
    } else {
        ["GH_TOKEN", "GITHUB_TOKEN"]
    }
}

/// Resolves a token for `host` from the real environment and `gh`.
pub fn resolve_token(host: Host) -> Result<(Token, TokenSource), AuthError> {
    resolve_with(host, |k| std::env::var(k).ok(), || run_gh_auth_token(host))
}

/// Testable core of [`resolve_token`].
pub fn resolve_with(
    host: Host,
    env: impl Fn(&str) -> Option<String>,
    gh: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<(Token, TokenSource), AuthError> {
    let vars = token_vars(host);
    for var in vars {
        if let Some(value) = env(var).map(|v| v.trim().to_owned())
            && !value.is_empty()
        {
            return Ok((Token(value), TokenSource::Env(var)));
        }
    }
    let no_token = |why: String| AuthError::NoToken { host, why };

    match gh() {
        Ok(Some(token)) => Ok((Token(token), TokenSource::GhCli)),
        Ok(None) => Err(no_token("`gh auth token` returned nothing".into())),
        Err(why) => Err(no_token(why)),
    }
}

fn run_gh_auth_token(host: Host) -> Result<Option<String>, String> {
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", host.name()])
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

    const DOTCOM: Host = Host::GITHUB_COM;

    #[test]
    fn prefers_gh_token_env() {
        let (token, source) = resolve_with(
            DOTCOM,
            env(&[("GH_TOKEN", "a"), ("GITHUB_TOKEN", "b")]),
            || panic!("gh must not run"),
        )
        .unwrap();
        assert_eq!(token.expose(), "a");
        assert_eq!(source, TokenSource::Env("GH_TOKEN"));
    }

    #[test]
    fn falls_back_to_github_token_then_gh() {
        let (token, source) = resolve_with(
            DOTCOM,
            env(&[("GH_TOKEN", "  "), ("GITHUB_TOKEN", "b")]),
            || panic!("gh must not run"),
        )
        .unwrap();
        assert_eq!(
            (token.expose(), source),
            ("b", TokenSource::Env("GITHUB_TOKEN"))
        );

        let (token, source) = resolve_with(DOTCOM, env(&[]), || Ok(Some("c".into()))).unwrap();
        assert_eq!((token.expose(), source), ("c", TokenSource::GhCli));
    }

    #[test]
    fn enterprise_reads_its_own_variables() {
        let ghe = Host::parse("ghe.example.com").unwrap();
        let both = env(&[("GH_TOKEN", "dotcom"), ("GITHUB_ENTERPRISE_TOKEN", "e")]);
        let (token, source) = resolve_with(ghe, both, || panic!("gh must not run")).unwrap();
        assert_eq!(
            (token.expose(), source),
            ("e", TokenSource::Env("GITHUB_ENTERPRISE_TOKEN"))
        );
        let err = resolve_with(ghe, env(&[("GH_TOKEN", "dotcom")]), || Ok(None)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("gh auth login --hostname ghe.example.com"),
            "{msg}"
        );
        assert!(msg.contains("GH_ENTERPRISE_TOKEN"), "{msg}");
    }

    #[test]
    fn missing_token_mentions_gh_auth_login() {
        let err = resolve_with(DOTCOM, env(&[]), || Err("not logged in".into())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("gh auth login"), "{msg}");
        assert!(msg.contains("not logged in"), "{msg}");
    }

    #[test]
    fn debug_is_redacted() {
        assert_eq!(format!("{:?}", Token::new("secret")), "Token(<redacted>)");
        assert!(!Token::new("secret").fingerprint().contains("secret"));
    }
}

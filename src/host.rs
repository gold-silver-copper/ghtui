//! The GitHub ghtui works with: github.com, or a GitHub Enterprise Server
//! named by `GHTUI_HOST`, `[github] host` or `GH_HOST`. Every URL ghtui
//! builds or reads (the website's, the API's, git's) starts here, so a link
//! to another GitHub opens in the browser.

use std::sync::OnceLock;

const DOTCOM: &str = "github.com";

/// A GitHub's hostname, lowercase (with a port if it has one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Host(&'static str);

impl Default for Host {
    fn default() -> Self {
        Self::GITHUB_COM
    }
}

impl std::fmt::Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl Host {
    pub const GITHUB_COM: Host = Host(DOTCOM);

    /// A hostname, or a URL whose host is taken (`https://ghe.example.com/`).
    pub fn parse(s: &str) -> Result<Host, String> {
        let s = s.trim();
        let rest = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
            .unwrap_or(s);
        let name = rest.trim_end_matches('/').to_ascii_lowercase();
        let valid = !name.is_empty()
            && !name.starts_with(['.', '-', ':'])
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
        if !valid {
            return Err(format!("`{s}` is not a GitHub hostname"));
        }
        Ok(match name.as_str() {
            DOTCOM | "www.github.com" | "api.github.com" => Self::GITHUB_COM,
            // Once per process: hosts live as long as ghtui does.
            _ => Host(Box::leak(name.into_boxed_str())),
        })
    }

    /// The host from the environment (`GHTUI_HOST`, then `GH_HOST`, as gh
    /// reads it) and the config file, which comes between them.
    pub fn resolve(
        env: impl Fn(&str) -> Option<String>,
        config: Option<&str>,
    ) -> Result<Host, String> {
        let set = |v: Option<String>| v.filter(|v| !v.trim().is_empty());
        let named = set(env("GHTUI_HOST"))
            .or_else(|| set(config.map(str::to_owned)))
            .or_else(|| set(env("GH_HOST")));
        named.map_or(Ok(Self::GITHUB_COM), |name| Self::parse(&name))
    }

    pub fn name(self) -> &'static str {
        self.0
    }

    /// A GitHub Enterprise Server rather than github.com.
    pub fn is_enterprise(self) -> bool {
        self.0 != DOTCOM
    }

    /// The website: `https://github.com`.
    pub fn web(self) -> String {
        format!("https://{}", self.0)
    }

    /// The REST API's base.
    pub fn rest_api(self) -> String {
        if self.is_enterprise() {
            format!("https://{}/api/v3", self.0)
        } else {
            format!("https://api.{DOTCOM}")
        }
    }

    /// The GraphQL endpoint.
    pub fn graphql_api(self) -> String {
        if self.is_enterprise() {
            format!("https://{}/api/graphql", self.0)
        } else {
            format!("https://api.{DOTCOM}/graphql")
        }
    }

    /// Gists' website. An Enterprise Server serves them under `/gist`
    /// (and at `gist.HOST` with subdomain isolation, which is read too).
    pub fn gists(self) -> String {
        if self.is_enterprise() {
            format!("https://{}/gist", self.0)
        } else {
            format!("https://gist.{DOTCOM}")
        }
    }

    /// Raw files: `<raw>/<owner>/<repo>/<rev>/<path>`.
    pub fn raw(self) -> String {
        if self.is_enterprise() {
            format!("https://{}/raw", self.0)
        } else {
            "https://raw.githubusercontent.com".to_owned()
        }
    }

    /// A repository's git URL (`name` may be `<repo>.wiki`).
    pub fn git_url(self, owner: &str, name: &str) -> String {
        format!("{}/{owner}/{name}.git", self.web())
    }

    /// Whether `host` (a URL's or a git remote's) is this GitHub's website.
    pub fn serves(self, host: &str) -> bool {
        host.eq_ignore_ascii_case(self.0)
            || (!self.is_enterprise() && host.eq_ignore_ascii_case("www.github.com"))
    }

    /// Whether `url` is on this GitHub's website: its host, and its port
    /// when the host names one.
    pub fn serves_url(self, url: &url::Url) -> bool {
        url_host(url).is_some_and(|h| self.serves(&h))
    }

    /// Whether `name`, a hostname without a port (an SSH remote's, whose
    /// port is SSH's), is this GitHub's.
    pub fn serves_hostname(self, name: &str) -> bool {
        let ours = self.0.split(':').next().unwrap_or(self.0);
        name.eq_ignore_ascii_case(ours) || self.serves(name)
    }

    /// Whether `host` is this GitHub's gist subdomain.
    pub fn serves_gists(self, host: &str) -> bool {
        let (gist, rest) = host.split_at_checked(5).unwrap_or_default();
        gist.eq_ignore_ascii_case("gist.") && rest.eq_ignore_ascii_case(self.0)
    }
}

/// A URL's host as hosts are named here: with its port, when it has one.
pub fn url_host(url: &url::Url) -> Option<String> {
    let name = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{name}:{port}"),
        None => name.to_owned(),
    })
}

static CURRENT: OnceLock<Host> = OnceLock::new();

/// Sets the host for the rest of the run; only the first call counts.
pub fn set(host: Host) {
    if CURRENT.set(host).is_err() {
        tracing::warn!(%host, "the GitHub host is already set");
    }
}

/// The GitHub this run works with.
pub fn current() -> Host {
    #[cfg(test)]
    if let Some(host) = TEST_HOST.with(std::cell::Cell::get) {
        return host;
    }
    CURRENT.get().copied().unwrap_or_default()
}

#[cfg(test)]
thread_local! {
    static TEST_HOST: std::cell::Cell<Option<Host>> = const { std::cell::Cell::new(None) };
}

/// For tests: `host` is this thread's GitHub until the guard drops.
#[cfg(test)]
pub fn set_for_test(host: &str) -> impl Drop {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_HOST.with(|h| h.set(None));
        }
    }
    TEST_HOST.with(|h| h.set(Some(Host::parse(host).unwrap())));
    Guard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enterprise_bases() {
        let ghe = Host::parse("https://GHE.example.com/").unwrap();
        assert_eq!(ghe.name(), "ghe.example.com");
        assert!(ghe.is_enterprise());
        assert_eq!(ghe.web(), "https://ghe.example.com");
        assert_eq!(ghe.rest_api(), "https://ghe.example.com/api/v3");
        assert_eq!(ghe.graphql_api(), "https://ghe.example.com/api/graphql");
        assert_eq!(ghe.gists(), "https://ghe.example.com/gist");
        assert!(ghe.serves("ghe.example.com") && !ghe.serves("github.com"));
        assert!(ghe.serves_gists("gist.ghe.example.com"));

        let dotcom = Host::default();
        assert_eq!(dotcom.rest_api(), "https://api.github.com");
        assert_eq!(dotcom.graphql_api(), "https://api.github.com/graphql");
        assert!(dotcom.serves("www.github.com") && !dotcom.serves("ghe.example.com"));
        assert_eq!(Host::parse("api.github.com"), Ok(dotcom));
        Host::parse("ghe.example.com/path").unwrap_err();
        Host::parse("").unwrap_err();
    }

    #[test]
    fn resolves_env_then_config_then_gh_host() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let gh_host = env(&[("GH_HOST", "gh.example.com")]);
        assert_eq!(
            Host::resolve(gh_host, None).unwrap().name(),
            "gh.example.com"
        );
        assert_eq!(
            Host::resolve(gh_host, Some("conf.example.com"))
                .unwrap()
                .name(),
            "conf.example.com"
        );
        let both = env(&[
            ("GHTUI_HOST", "mine.example.com"),
            ("GH_HOST", "gh.example.com"),
        ]);
        assert_eq!(
            Host::resolve(both, Some("conf.example.com"))
                .unwrap()
                .name(),
            "mine.example.com"
        );
        assert_eq!(Host::resolve(env(&[]), None), Ok(Host::GITHUB_COM));
    }
}

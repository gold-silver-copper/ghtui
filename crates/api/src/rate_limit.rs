//! Rate-limit tracking from GitHub's `x-ratelimit-*` response headers.
//! GraphQL responses carry the same headers with resource `graphql`, so one
//! parser covers both budgets.

use http::HeaderMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub remaining: u64,
    pub limit: u64,
    /// Unix seconds.
    pub reset_at: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimits {
    pub rest: Option<Bucket>,
    pub graphql: Option<Bucket>,
}

impl RateLimits {
    pub fn update(&mut self, headers: &HeaderMap) {
        let Some(bucket) = parse_bucket(headers) else {
            return;
        };
        match header(headers, "x-ratelimit-resource") {
            Some("graphql") => self.graphql = Some(bucket),
            Some("core") | None => self.rest = Some(bucket),
            // search, code_search, ...: separate budgets we don't display.
            Some(_) => {}
        }
    }

    /// The bucket closest to running out, as a fraction remaining.
    pub fn tightest(&self) -> Option<(&'static str, Bucket)> {
        [("graphql", self.graphql), ("rest", self.rest)]
            .into_iter()
            .filter_map(|(name, b)| b.map(|b| (name, b)))
            .min_by(|(_, a), (_, b)| {
                let fa = a.remaining as f64 / a.limit.max(1) as f64;
                let fb = b.remaining as f64 / b.limit.max(1) as f64;
                fa.total_cmp(&fb)
            })
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn parse_bucket(headers: &HeaderMap) -> Option<Bucket> {
    let num = |name| header(headers, name)?.trim().parse::<u64>().ok();
    Some(Bucket {
        remaining: num("x-ratelimit-remaining")?,
        limit: num("x-ratelimit-limit")?,
        reset_at: num("x-ratelimit-reset")?,
    })
}

/// How long the server asked us to wait, in seconds, if this response is a
/// (primary or secondary) rate-limit rejection.
pub fn retry_after(status: http::StatusCode, headers: &HeaderMap, now: u64) -> Option<u64> {
    if status != http::StatusCode::TOO_MANY_REQUESTS && status != http::StatusCode::FORBIDDEN {
        return None;
    }
    if let Some(secs) = header(headers, "retry-after").and_then(|v| v.trim().parse().ok()) {
        return Some(secs);
    }
    if header(headers, "x-ratelimit-remaining") == Some("0") {
        let reset = parse_bucket(headers)?.reset_at;
        return Some(reset.saturating_sub(now));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(*k, v.parse().unwrap());
        }
        map
    }

    #[test]
    fn tracks_buckets_by_resource() {
        let mut limits = RateLimits::default();
        for [resource, remaining, limit, reset] in [
            ["graphql", "4990", "5000", "100"],
            ["core", "10", "5000", "200"],
            ["search", "1", "30", "300"],
        ] {
            limits.update(&headers(&[
                ("x-ratelimit-resource", resource),
                ("x-ratelimit-remaining", remaining),
                ("x-ratelimit-limit", limit),
                ("x-ratelimit-reset", reset),
            ]));
        }
        assert_eq!(limits.graphql.unwrap().remaining, 4990);
        assert_eq!(limits.rest.unwrap().remaining, 10);
        assert_eq!(limits.tightest().unwrap().0, "rest");
    }

    #[test]
    fn ignores_responses_without_headers() {
        let mut limits = RateLimits::default();
        limits.update(&HeaderMap::new());
        assert_eq!(limits, RateLimits::default());
    }

    #[test]
    fn retry_after_rules() {
        let forbidden = http::StatusCode::FORBIDDEN;
        assert_eq!(
            retry_after(forbidden, &headers(&[("retry-after", "30")]), 0),
            Some(30)
        );
        assert_eq!(
            retry_after(
                forbidden,
                &headers(&[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-limit", "5000"),
                    ("x-ratelimit-reset", "1060"),
                ]),
                1000
            ),
            Some(60)
        );
        // A plain permissions 403 isn't a rate limit.
        assert_eq!(retry_after(forbidden, &HeaderMap::new(), 0), None);
        assert_eq!(
            retry_after(http::StatusCode::OK, &headers(&[("retry-after", "5")]), 0),
            None
        );
    }
}

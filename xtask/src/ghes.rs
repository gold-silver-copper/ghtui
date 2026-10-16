//! A stand-in GitHub Enterprise Server: an HTTPS proxy on
//! `https://localhost:PORT` that serves an Enterprise Server's paths from
//! github.com's services, so ghtui can be run as against a real one.
//!
//! - `/api/v3/…` → `https://api.github.com/…`
//! - `/api/graphql` → `https://api.github.com/graphql`
//! - `/raw/…` → `https://raw.githubusercontent.com/…`
//! - `/gist/…` → `https://gist.github.com/…`
//! - anything else (the website, git over HTTP) → `https://github.com/…`
//!
//! API answers name the stand-in, not github.com: URLs in their bodies and
//! in `Link` and `Location` headers are rewritten to its host, as a real
//! server's answers would read. To stand in for an older server,
//! `--version 3.N` is the version its `/api/v3/meta` says, and `--strip
//! FIELD` (or `Type.field`) a field it lacks: a query naming it is refused
//! as an older server refuses it ("Field 'x' doesn't exist on type 'Y'").
//!
//! Its certificate authority is made here with `openssl`, and trusted only
//! by the programs given [`StandIn::env`] (`SSL_CERT_FILE`,
//! `GIT_SSL_CAINFO`): nothing else on the machine trusts it.

use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::sync::oneshot;

/// `cargo xtask ghes`.
#[derive(clap::Args)]
pub struct Args {
    /// The port on localhost.
    #[arg(long, default_value_t = 8443)]
    port: u16,
    /// Refuse GraphQL queries naming this field (`field` or
    /// `Type.field`), as an older server lacking it does (again for more).
    #[arg(long = "strip", value_name = "FIELD")]
    strip: Vec<String>,
    /// The version `/api/v3/meta` says it is (`3.19.0`).
    #[arg(long, value_name = "VERSION")]
    version: Option<String>,
    /// Where its certificate authority is kept.
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool"
)]
pub fn main(args: &Args) -> ExitCode {
    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| crate::live::default_work_dir().join("ghes"));
    let standin = match StandIn::start(&dir, args.port, &args.strip, args.version.as_deref()) {
        Ok(standin) => standin,
        Err(err) => {
            eprintln!("ghes: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "Serving https://{} (ctrl-c stops it). Run ghtui with:\n",
        standin.host
    );
    let vars: Vec<String> = standin
        .env()
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    println!(
        "  GH_ENTERPRISE_TOKEN=$(gh auth token) {} ghtui\n",
        vars.join(" ")
    );
    let stopped = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|rt| rt.block_on(tokio::signal::ctrl_c()));
    drop(standin);
    match stopped {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(err)) => {
            eprintln!("ghes: {err}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("ghes: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The running stand-in; dropping it stops it.
pub struct StandIn {
    /// `localhost:PORT`, as ghtui's `GHTUI_HOST`.
    pub host: String,
    ca: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StandIn {
    /// Makes a certificate authority in `dir` (once) and starts serving,
    /// as an older server lacking the fields in `strip`, of `version`.
    pub fn start(
        dir: &Path,
        port: u16,
        strip: &[String],
        version: Option<&str>,
    ) -> Result<StandIn> {
        let (ca, cert, key) = certificates(dir)?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = tls_config(&cert, &key)?;
        let host = format!("localhost:{port}");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ghtui-live-ghes")
            .build()?;
        let shared = Arc::new(Shared {
            host: host.clone(),
            client,
            strip: strip.iter().cloned().collect(),
            version: version.map(str::to_owned),
        });
        let (stop, stopped) = oneshot::channel();
        let (ready_tx, ready) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = ready_tx.send(Err(anyhow::Error::from(err)));
                    return;
                }
            };
            runtime.block_on(serve(port, tls, shared, stopped, ready_tx));
        });
        match ready.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => bail!("the stand-in server stopped before it started"),
        }
        Ok(StandIn {
            host,
            ca,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// What a program needs to use it: the host, and trust in its
    /// authority alone. `GH_TOKEN` is emptied so only
    /// `GH_ENTERPRISE_TOKEN` is read.
    pub fn env(&self) -> Vec<(String, String)> {
        let ca = self.ca.display().to_string();
        vec![
            ("GHTUI_HOST".into(), self.host.clone()),
            ("SSL_CERT_FILE".into(), ca.clone()),
            ("GIT_SSL_CAINFO".into(), ca),
            ("GH_TOKEN".into(), String::new()),
        ]
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Shared {
    host: String,
    client: reqwest::Client,
    strip: HashSet<String>,
    version: Option<String>,
}

/// The authority (`ca.pem`) and the server's certificate for localhost,
/// made with openssl the first time.
fn certificates(dir: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let ca = dir.join("ca.pem");
    let cert = dir.join("server.pem");
    let key = dir.join("server.key");
    if ca.exists() && cert.exists() && key.exists() {
        return Ok((ca, cert, key));
    }
    std::fs::create_dir_all(dir)?;
    let ext = dir.join("server.ext");
    std::fs::write(
        &ext,
        "subjectAltName=DNS:localhost,IP:127.0.0.1,IP:::1\n\
         basicConstraints=critical,CA:FALSE\n\
         keyUsage=critical,digitalSignature,keyEncipherment\n\
         extendedKeyUsage=serverAuth\n",
    )?;
    let p = |path: &Path| path.display().to_string();
    let steps: [Vec<String>; 3] = [
        vec![
            "req".into(),
            "-x509".into(),
            "-newkey".into(),
            "rsa:2048".into(),
            "-nodes".into(),
            "-days".into(),
            "30".into(),
            "-subj".into(),
            "/CN=ghtui live check CA".into(),
            "-addext".into(),
            "basicConstraints=critical,CA:TRUE".into(),
            "-addext".into(),
            "keyUsage=critical,keyCertSign,cRLSign".into(),
            "-keyout".into(),
            p(&dir.join("ca.key")),
            "-out".into(),
            p(&ca),
        ],
        vec![
            "req".into(),
            "-newkey".into(),
            "rsa:2048".into(),
            "-nodes".into(),
            "-subj".into(),
            "/CN=localhost".into(),
            "-keyout".into(),
            p(&key),
            "-out".into(),
            p(&dir.join("server.csr")),
        ],
        vec![
            "x509".into(),
            "-req".into(),
            "-days".into(),
            "30".into(),
            "-in".into(),
            p(&dir.join("server.csr")),
            "-CA".into(),
            p(&ca),
            "-CAkey".into(),
            p(&dir.join("ca.key")),
            "-CAcreateserial".into(),
            "-extfile".into(),
            p(&ext),
            "-out".into(),
            p(&cert),
        ],
    ];
    for step in steps {
        let out = Command::new("openssl")
            .args(&step)
            .output()
            .context("cannot run openssl, which makes the stand-in's certificates")?;
        if !out.status.success() {
            bail!(
                "openssl {} failed: {}",
                step.first().map_or("", String::as_str),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    Ok((ca, cert, key))
}

fn tls_config(cert: &Path, key: &Path) -> Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs = CertificateDer::pem_file_iter(cert)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key)?;
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

async fn serve(
    port: u16,
    tls: Arc<rustls::ServerConfig>,
    shared: Arc<Shared>,
    mut stopped: oneshot::Receiver<()>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) {
    // `localhost` may be either address; git and reqwest try both.
    let v4 = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let v4 = match v4 {
        Ok(listener) => listener,
        Err(err) => {
            let _ = ready.send(Err(anyhow::anyhow!(
                "cannot listen on localhost:{port} for the stand-in server: {err}"
            )));
            return;
        }
    };
    let v6 = tokio::net::TcpListener::bind(SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)))
        .await
        .ok();
    let _ = ready.send(Ok(()));
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    loop {
        let accepted = tokio::select! {
            _ = &mut stopped => return,
            accepted = v4.accept() => accepted,
            accepted = async {
                match &v6 {
                    Some(v6) => v6.accept().await,
                    None => std::future::pending().await,
                }
            } => accepted,
        };
        let Ok((stream, _)) = accepted else { continue };
        let acceptor = acceptor.clone();
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let Ok(stream) = acceptor.accept(stream).await else {
                return;
            };
            let service = hyper::service::service_fn(move |req| {
                let shared = Arc::clone(&shared);
                async move { Ok::<_, Infallible>(handle(req, &shared).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

async fn handle(req: Request<Incoming>, shared: &Shared) -> Response<Full<Bytes>> {
    match forward(req, shared).await {
        Ok(response) => response,
        Err(err) => {
            let mut response = Response::new(Full::new(Bytes::from(format!("{err:#}"))));
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            response
        }
    }
}

/// Where a path is served from, and whether it's the API (whose answers
/// are rewritten).
fn upstream(path_and_query: &str) -> (String, Kind) {
    if path_and_query == "/api/graphql" || path_and_query.starts_with("/api/graphql?") {
        let rest = path_and_query.trim_start_matches("/api/graphql");
        return (
            format!("https://api.github.com/graphql{rest}"),
            Kind::Graphql,
        );
    }
    if let Some(rest) = path_and_query.strip_prefix("/api/v3") {
        let rest = if rest.is_empty() { "/" } else { rest };
        return (format!("https://api.github.com{rest}"), Kind::Rest);
    }
    if let Some(rest) = path_and_query.strip_prefix("/raw/") {
        return (
            format!("https://raw.githubusercontent.com/{rest}"),
            Kind::Other,
        );
    }
    if let Some(rest) = path_and_query.strip_prefix("/gist/") {
        return (format!("https://gist.github.com/{rest}"), Kind::Other);
    }
    (format!("https://github.com{path_and_query}"), Kind::Other)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rest,
    Graphql,
    Other,
}

/// github.com's addresses as the stand-in names them, longest first.
fn rewrite(text: &str, host: &str) -> String {
    text.replace("https://api.github.com", &format!("https://{host}/api/v3"))
        .replace(
            "https://raw.githubusercontent.com",
            &format!("https://{host}/raw"),
        )
        .replace("https://gist.github.com", &format!("https://{host}/gist"))
        .replace("https://github.com", &format!("https://{host}"))
}

/// An older server's refusal of a GraphQL query naming a field it lacks
/// (`strip`'s `field` or `Type.field`), as GitHub's servers word it.
fn refusal(query: &str, strip: &HashSet<String>) -> Option<String> {
    let named = |word: &str| {
        query
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|w| w == word)
    };
    let (ty, field) = strip
        .iter()
        .map(|s| s.rsplit_once('.').unwrap_or(("Object", s)))
        .find(|(_, field)| named(field))?;
    let message = format!("Field '{field}' doesn't exist on type '{ty}'");
    Some(serde_json::json!({ "errors": [{ "message": message }] }).to_string())
}

/// `/meta`'s answer with the version an Enterprise Server says there.
fn with_version(text: &str, version: &str) -> String {
    let Ok(mut meta) = serde_json::from_str::<Value>(text) else {
        return text.to_owned();
    };
    if let Some(map) = meta.as_object_mut() {
        map.insert("installed_version".into(), version.into());
    }
    meta.to_string()
}

async fn forward(req: Request<Incoming>, shared: &Shared) -> Result<Response<Full<Bytes>>> {
    let path = req
        .uri()
        .path_and_query()
        .map_or_else(|| "/".to_owned(), |p| p.as_str().to_owned());
    let (url, kind) = upstream(&path);
    let method = req.method().clone();
    let headers = req.headers().clone();
    let body = req.into_body().collect().await?.to_bytes();
    if kind == Kind::Graphql
        && let Ok(json) = serde_json::from_slice::<Value>(&body)
        && let Some(refused) = refusal(json["query"].as_str().unwrap_or_default(), &shared.strip)
    {
        let mut response = Response::new(Full::new(Bytes::from(refused)));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        return Ok(response);
    }
    let mut up = shared.client.request(method, &url);
    for (name, value) in &headers {
        // Answers come back plain, so they can be rewritten.
        if name == header::HOST
            || name == header::CONNECTION
            || name == header::CONTENT_LENGTH
            || name == header::ACCEPT_ENCODING
            || name == header::TRANSFER_ENCODING
        {
            continue;
        }
        up = up.header(name, value);
    }
    let answer = up.body(body).send().await?;
    let status = answer.status();
    let answer_headers = answer.headers().clone();
    let mut bytes = answer.bytes().await?;
    if kind != Kind::Other {
        let mut text = rewrite(&String::from_utf8_lossy(&bytes), &shared.host);
        if path.trim_end_matches('/') == "/api/v3/meta"
            && let Some(version) = &shared.version
        {
            text = with_version(&text, version);
        }
        bytes = Bytes::from(text);
    }
    let mut response = Response::new(Full::new(bytes));
    *response.status_mut() = status;
    for (name, value) in &answer_headers {
        if name == header::CONTENT_LENGTH
            || name == header::TRANSFER_ENCODING
            || name == header::CONNECTION
        {
            continue;
        }
        let value = if name == header::LINK || name == header::LOCATION {
            let text = rewrite(value.to_str().unwrap_or_default(), &shared.host);
            HeaderValue::from_str(&text)?
        } else {
            value.clone()
        };
        response.headers_mut().append(name, value);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_enterprise_paths_to_github_com() {
        let url = |p: &str| upstream(p).0;
        assert_eq!(url("/api/v3/user"), "https://api.github.com/user");
        assert_eq!(url("/api/v3"), "https://api.github.com/");
        assert_eq!(url("/api/graphql"), "https://api.github.com/graphql");
        assert_eq!(
            url("/raw/o/r/HEAD/a.md"),
            "https://raw.githubusercontent.com/o/r/HEAD/a.md"
        );
        assert_eq!(
            url("/o/r.git/info/refs?service=git-upload-pack"),
            "https://github.com/o/r.git/info/refs?service=git-upload-pack"
        );
    }

    #[test]
    fn answers_name_the_stand_in() {
        let text =
            r#"{"url":"https://api.github.com/repos/o/r","html_url":"https://github.com/o/r"}"#;
        assert_eq!(
            rewrite(text, "localhost:8443"),
            r#"{"url":"https://localhost:8443/api/v3/repos/o/r","html_url":"https://localhost:8443/o/r"}"#
        );
    }

    #[test]
    fn refuses_a_query_naming_a_field_it_lacks() {
        let strip = HashSet::from(["PullRequest.mergeQueueEntry".to_owned()]);
        let refused = refusal(
            "query { pullRequest { mergeQueueEntry { position } } }",
            &strip,
        );
        assert_eq!(
            refused.as_deref(),
            Some(
                r#"{"errors":[{"message":"Field 'mergeQueueEntry' doesn't exist on type 'PullRequest'"}]}"#
            )
        );
        assert_eq!(
            refusal("query { pullRequest { mergeQueue } }", &strip),
            None
        );
        let meta = with_version(r#"{"verifiable_password_authentication":true}"#, "3.19.0");
        assert!(meta.contains(r#""installed_version":"3.19.0""#));
    }
}

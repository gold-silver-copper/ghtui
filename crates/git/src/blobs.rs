//! Blob contents through one long-lived `git cat-file --batch` process.
//! Going through git (not a library) keeps partial-clone lazy fetching
//! working for anything the prefetch missed.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::GitError;

pub struct BlobReader {
    inner: Mutex<Batch>,
}

struct Batch {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl BlobReader {
    pub(crate) fn spawn(mut cmd: Command) -> Result<Self, GitError> {
        cmd.args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        Ok(Self {
            inner: Mutex::new(Batch {
                _child: child,
                stdin,
                stdout,
            }),
        })
    }

    /// The contents of blob `oid`, or `None` if it doesn't exist.
    pub async fn read(&self, oid: &str) -> Result<Option<Vec<u8>>, GitError> {
        if oid.contains(['\n', ' ']) {
            return Err(GitError::Parse(format!("bad object id {oid:?}")));
        }
        let mut batch = self.inner.lock().await;
        batch.stdin.write_all(format!("{oid}\n").as_bytes()).await?;
        batch.stdin.flush().await?;
        let mut header = String::new();
        if batch.stdout.read_line(&mut header).await? == 0 {
            return Err(GitError::Parse("cat-file exited".into()));
        }
        let header = header.trim_end();
        if header.ends_with(" missing") || header.ends_with(" ambiguous") {
            return Ok(None);
        }
        // "<oid> <type> <size>"
        let size: usize = header
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| GitError::Parse(format!("cat-file header {header:?}")))?;
        let mut data = vec![0u8; size + 1];
        batch.stdout.read_exact(&mut data).await?;
        data.pop(); // trailing newline
        Ok(Some(data))
    }
}

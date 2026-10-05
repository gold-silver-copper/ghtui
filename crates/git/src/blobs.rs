//! Blob contents through one long-lived `git cat-file --batch` process.
//! Going through git (not a library) keeps partial-clone lazy fetching
//! working for anything the prefetch missed.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::{GitError, piped};

type MakeCommand = Box<dyn Fn() -> Command + Send + Sync>;

pub struct BlobReader {
    make: MakeCommand,
    /// Taken out for the length of a read and put back only once the reply
    /// has been read in full. A read that's cancelled or fails halfway
    /// leaves `None`, so the next read starts a fresh process instead of
    /// parsing the rest of an old reply.
    batch: Mutex<Option<Batch>>,
}

struct Batch {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Batch {
    fn spawn(make: &MakeCommand) -> Result<Self, GitError> {
        let mut cmd = make();
        cmd.args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let stdin = piped(child.stdin.take(), "stdin")?;
        let stdout = BufReader::new(piped(child.stdout.take(), "stdout")?);
        Ok(Self {
            _child: child,
            stdin,
            stdout,
        })
    }

    async fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, GitError> {
        self.stdin.write_all(format!("{name}\n").as_bytes()).await?;
        self.stdin.flush().await?;
        let mut header = String::new();
        if self.stdout.read_line(&mut header).await? == 0 {
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
        let mut data = vec![0u8; size.saturating_add(1)];
        self.stdout.read_exact(&mut data).await?;
        data.pop(); // trailing newline
        Ok(Some(data))
    }
}

impl BlobReader {
    /// Reads through `git cat-file --batch`, run from commands `make`
    /// builds (again, if a read is interrupted).
    pub(crate) fn spawn(
        make: impl Fn() -> Command + Send + Sync + 'static,
    ) -> Result<Self, GitError> {
        let make: MakeCommand = Box::new(make);
        let batch = Batch::spawn(&make)?;
        Ok(Self {
            make,
            batch: Mutex::new(Some(batch)),
        })
    }

    /// The contents of `name` (an object ID or `<rev>:<path>`), or `None`
    /// if it doesn't exist.
    pub async fn read(&self, name: &str) -> Result<Option<Vec<u8>>, GitError> {
        if name.contains(['\n', '\r']) {
            return Err(GitError::Parse(format!("bad object name {name:?}")));
        }
        let mut slot = self.batch.lock().await;
        let mut batch = match slot.take() {
            Some(batch) => batch,
            None => Batch::spawn(&self.make)?,
        };
        let result = batch.read(name).await;
        if result.is_ok() {
            *slot = Some(batch);
        }
        result
    }
}

//! Blob contents through one long-lived `git cat-file --batch` process.
//! Going through git (not a library) keeps partial-clone lazy fetching
//! working for anything the prefetch missed.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::git::{GitError, piped};

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

    /// Asks for every one of `names` at once, then reads the replies.
    async fn read(&mut self, names: &[&str]) -> Result<Vec<Option<Vec<u8>>>, GitError> {
        let asked: String = names.iter().flat_map(|name| [*name, "\n"]).collect();
        self.stdin.write_all(asked.as_bytes()).await?;
        self.stdin.flush().await?;
        let mut replies = Vec::with_capacity(names.len());
        for _ in names {
            replies.push(self.reply().await?);
        }
        Ok(replies)
    }

    async fn reply(&mut self) -> Result<Option<Vec<u8>>, GitError> {
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
        Ok(self.read_each(&[name]).await?.into_iter().next().flatten())
    }

    /// [`Self::read`] for each of a few `names`, asked together: one wait
    /// on git, not one each.
    pub async fn read_each(&self, names: &[&str]) -> Result<Vec<Option<Vec<u8>>>, GitError> {
        if let Some(name) = names.iter().find(|name| name.contains(['\n', '\r'])) {
            return Err(GitError::Parse(format!("bad object name {name:?}")));
        }
        let mut slot = self.batch.lock().await;
        let mut batch = match slot.take() {
            Some(batch) => batch,
            None => Batch::spawn(&self.make)?,
        };
        let result = batch.read(names).await;
        if result.is_ok() {
            *slot = Some(batch);
        }
        result
    }
}

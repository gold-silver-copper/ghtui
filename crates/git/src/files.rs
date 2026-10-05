//! Changed files from `git diff --raw -z`.

use crate::{GitError, Oid};

pub const ZERO_OID: &str = "0000000000000000000000000000000000000000";

pub const MODE_SYMLINK: u32 = 0o120000;
pub const MODE_SUBMODULE: u32 = 0o160000;
pub const MODE_EXECUTABLE: u32 = 0o100755;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub status: FileStatus,
    /// Path before the change; `None` for added files.
    pub old_path: Option<String>,
    /// Path after the change; `None` for deleted files.
    pub new_path: Option<String>,
    pub old_mode: u32,
    pub new_mode: u32,
    /// Full object IDs; [`ZERO_OID`] for a missing side.
    pub old_oid: Oid,
    pub new_oid: Oid,
    /// Rename/copy similarity, 0–100.
    pub similarity: Option<u8>,
}

impl ChangedFile {
    /// The path to show: the new path, or the old one for deletions.
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }

    pub fn is_submodule(&self) -> bool {
        self.old_mode == MODE_SUBMODULE || self.new_mode == MODE_SUBMODULE
    }

    pub fn is_symlink(&self) -> bool {
        self.old_mode == MODE_SYMLINK || self.new_mode == MODE_SYMLINK
    }

    pub fn mode_changed(&self) -> bool {
        self.old_mode != 0 && self.new_mode != 0 && self.old_mode != self.new_mode
    }

    /// Blob IDs whose contents the diff needs (none for submodules).
    pub fn blob_oids(&self) -> impl Iterator<Item = &str> {
        let submodule = self.is_submodule();
        [&*self.old_oid, &*self.new_oid]
            .into_iter()
            .filter(move |oid| !submodule && *oid != ZERO_OID)
    }
}

/// Parses `git diff --raw -z --no-abbrev` output.
pub fn parse_raw(out: &str) -> Result<Vec<ChangedFile>, GitError> {
    let bad = |what: &str| GitError::Parse(format!("diff --raw: {what}"));
    let mut fields = out.split('\0');
    let mut files = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let header = header.strip_prefix(':').ok_or_else(|| bad("missing ':'"))?;
        let parts: Vec<&str> = header.split(' ').collect();
        let [old_mode, new_mode, old_oid, new_oid, status] = parts[..] else {
            return Err(bad(header));
        };
        let mode = |m: &str| u32::from_str_radix(m, 8).map_err(|_| bad(m));
        let (letter, score) = status.split_at(1);
        let similarity = score.parse::<u8>().ok();
        let first = fields.next().ok_or_else(|| bad("missing path"))?.to_owned();
        let (status, old_path, new_path) = match letter {
            "A" => (FileStatus::Added, None, Some(first)),
            "D" => (FileStatus::Deleted, Some(first), None),
            "M" => (FileStatus::Modified, Some(first.clone()), Some(first)),
            "T" => (FileStatus::TypeChanged, Some(first.clone()), Some(first)),
            "R" | "C" => {
                let second = fields.next().ok_or_else(|| bad("missing rename target"))?;
                let status = if letter == "R" {
                    FileStatus::Renamed
                } else {
                    FileStatus::Copied
                };
                (status, Some(first), Some(second.to_owned()))
            }
            other => return Err(bad(&format!("unknown status {other}"))),
        };
        files.push(ChangedFile {
            status,
            old_path,
            new_path,
            old_mode: mode(old_mode)?,
            new_mode: mode(new_mode)?,
            old_oid: Oid::new(old_oid),
            new_oid: Oid::new(new_oid),
            similarity,
        });
    }
    Ok(files)
}

/// Lockfiles are collapsed in the file tree by default, like generated code.
pub fn is_lockfile(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "Cargo.lock"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "bun.lock"
            | "Gemfile.lock"
            | "composer.lock"
            | "poetry.lock"
            | "uv.lock"
            | "Pipfile.lock"
            | "go.sum"
            | "flake.lock"
            | "mix.lock"
            | "pubspec.lock"
            | "Podfile.lock"
            | "Package.resolved"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn parses_all_statuses() {
        let out = format!(
            ":000000 100644 {ZERO_OID} {A} A\0new.txt\0\
             :100644 000000 {A} {ZERO_OID} D\0gone.txt\0\
             :100644 100755 {A} {B} M\0script.sh\0\
             :100644 100644 {A} {B} R087\0old name.rs\0new name.rs\0\
             :100644 120000 {A} {B} T\0link\0"
        );
        let files = parse_raw(&out).unwrap();
        assert_eq!(files.len(), 5);
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[0].path(), "new.txt");
        assert_eq!(files[0].blob_oids().collect::<Vec<_>>(), [A]);
        assert_eq!(files[1].path(), "gone.txt");
        assert!(files[2].mode_changed());
        assert_eq!(files[3].status, FileStatus::Renamed);
        assert_eq!(files[3].similarity, Some(87));
        assert_eq!(files[3].old_path.as_deref(), Some("old name.rs"));
        assert_eq!(files[3].path(), "new name.rs");
        assert!(files[4].is_symlink());
    }

    #[test]
    fn submodules_have_no_blobs() {
        let out = format!(":160000 160000 {A} {B} M\0vendor/lib\0");
        let files = parse_raw(&out).unwrap();
        assert!(files[0].is_submodule());
        assert_eq!(files[0].blob_oids().count(), 0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_raw("nonsense\0").is_err());
        assert!(parse_raw(&format!(":100644 100644 {A} {B} Z\0x\0")).is_err());
        assert_eq!(parse_raw("").unwrap(), Vec::new());
    }

    #[test]
    fn lockfiles() {
        assert!(is_lockfile("Cargo.lock"));
        assert!(is_lockfile("web/package-lock.json"));
        assert!(!is_lockfile("src/lock.rs"));
    }
}

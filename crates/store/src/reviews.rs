//! Review state (draft comments, reviewed marks) kept apart from the cache,
//! as one JSON file per pull request under the data directory. Clearing or
//! discarding the cache never touches it. Files are replaced atomically, so
//! a crash mid-write leaves the previous version.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{ReviewState, StoreError};

/// Cheap to clone.
#[derive(Debug, Clone)]
pub struct Reviews {
    dir: PathBuf,
}

impl Reviews {
    /// Uses (and creates) `dir`.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_owned(),
        })
    }

    fn path(&self, owner: &str, repo: &str, number: u64) -> Result<PathBuf, StoreError> {
        for part in [owner, repo] {
            if part.is_empty() || part == "." || part == ".." || part.contains(['/', '\\', '\0']) {
                return Err(StoreError::Io(std::io::Error::other(format!(
                    "bad repository name {part:?}"
                ))));
            }
        }
        Ok(self
            .dir
            .join(owner)
            .join(repo)
            .join(format!("{number}.json")))
    }

    /// The PR's review state; empty if there's none. A file that no longer
    /// parses is moved aside (never overwritten) and reads as empty.
    pub fn get(&self, owner: &str, repo: &str, number: u64) -> Result<ReviewState, StoreError> {
        let path = self.path(owner, repo, number)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ReviewState::default());
            }
            Err(err) => return Err(err.into()),
        };
        match serde_json::from_slice(&bytes) {
            Ok(state) => Ok(state),
            Err(err) => {
                let aside = path.with_extension(format!("corrupt-{}.json", crate::now()));
                tracing::warn!(%err, path = %path.display(), aside = %aside.display(), "review file unreadable; moved aside");
                std::fs::rename(&path, &aside)?;
                Ok(ReviewState::default())
            }
        }
    }

    pub fn exists(&self, owner: &str, repo: &str, number: u64) -> bool {
        self.path(owner, repo, number).is_ok_and(|p| p.exists())
    }

    /// Replaces the PR's review state; an empty state removes its file.
    pub fn put(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        state: &ReviewState,
    ) -> Result<(), StoreError> {
        let path = self.path(owner, repo, number)?;
        if *state == ReviewState::default() {
            return match std::fs::remove_file(&path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
                _ => Ok(()),
            };
        }
        let dir = path.parent().unwrap_or(&self.dir);
        std::fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec_pretty(state)?)?;
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|e| StoreError::Io(e.error))?;
        Ok(())
    }
}

/// `owner/repo#number`, the cache's old review key.
pub(crate) fn parse_key(key: &str) -> Option<(&str, &str, u64)> {
    let (repo, number) = key.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    Some((owner, name, number.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DraftComment, DraftSide};

    fn draft(body: &str) -> ReviewState {
        ReviewState {
            pending: vec![DraftComment {
                id: 1,
                path: "a.rs".into(),
                body: body.into(),
                side: DraftSide::Right,
                line: Some(3),
                start_line: None,
                start_side: None,
                commit: "abc".into(),
                error: None,
            }],
            ..ReviewState::default()
        }
    }

    #[test]
    fn round_trips_and_empty_removes() {
        let dir = tempfile::tempdir().unwrap();
        let reviews = Reviews::open(&dir.path().join("reviews")).unwrap();
        assert_eq!(reviews.get("o", "r", 1).unwrap(), ReviewState::default());
        reviews.put("o", "r", 1, &draft("nit")).unwrap();
        assert!(reviews.exists("o", "r", 1));
        assert_eq!(reviews.get("o", "r", 1).unwrap(), draft("nit"));
        reviews.put("o", "r", 1, &draft("better")).unwrap();
        assert_eq!(reviews.get("o", "r", 1).unwrap(), draft("better"));
        reviews.put("o", "r", 1, &ReviewState::default()).unwrap();
        assert!(!reviews.exists("o", "r", 1));
    }

    #[test]
    fn corrupt_files_are_moved_aside_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let reviews = Reviews::open(dir.path()).unwrap();
        let path = dir.path().join("o/r/1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(reviews.get("o", "r", 1).unwrap(), ReviewState::default());
        let kept: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(kept.iter().any(|n| n.contains("corrupt")), "{kept:?}");
    }

    #[test]
    fn rejects_path_tricks() {
        let dir = tempfile::tempdir().unwrap();
        let reviews = Reviews::open(dir.path()).unwrap();
        assert!(reviews.put("..", "r", 1, &draft("x")).is_err());
        assert!(reviews.put("o", "a/b", 1, &draft("x")).is_err());
    }

    #[test]
    fn parses_old_keys() {
        assert_eq!(parse_key("o/r#12"), Some(("o", "r", 12)));
        assert_eq!(parse_key("o/r"), None);
    }
}

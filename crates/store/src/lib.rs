//! On-disk cache and review-state persistence, backed by redb.
//!
//! The database carries a schema version. When it doesn't match
//! [`SCHEMA_VERSION`] the file is discarded and recreated; there are no
//! migrations. If the database can't be opened (another ghtui process holds
//! the lock, the disk is read-only, ...) the store runs disabled: reads miss
//! and writes are dropped, so the app keeps working without a cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use redb::{Database, ReadableDatabase, TableDefinition};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Bump whenever the meaning or encoding of any table changes.
pub const SCHEMA_VERSION: u64 = 2;

const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
/// REST responses keyed by request path, stored with their ETag.
const HTTP: TableDefinition<&str, &[u8]> = TableDefinition::new("http");
/// Decoded GraphQL results keyed by a caller-chosen key.
const QUERIES: TableDefinition<&str, &[u8]> = TableDefinition::new("queries");
/// Per-PR review state keyed by `owner/repo#number`.
const REVIEW: TableDefinition<&str, &[u8]> = TableDefinition::new("review");

const VERSION_KEY: &str = "schema_version";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("cache database error: {0}")]
    Db(#[from] redb::Error),
    #[error("cache encoding error: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("cache file error: {0}")]
    Io(#[from] std::io::Error),
}

macro_rules! from_redb {
    ($($ty:ty),*) => {$(
        impl From<$ty> for StoreError {
            fn from(err: $ty) -> Self {
                StoreError::Db(err.into())
            }
        }
    )*};
}
from_redb!(
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError
);

/// A cached REST response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEntry {
    pub etag: String,
    pub body: String,
    /// Unix seconds.
    pub fetched_at: u64,
}

/// A cached value with the time it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached<T> {
    pub value: T,
    /// Unix seconds.
    pub fetched_at: u64,
}

#[derive(Serialize, Deserialize)]
struct QueryEntry {
    fetched_at: u64,
    value: serde_json::Value,
}

/// What we remember about a PR between sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewState {
    /// Head SHA at the time of my last submitted review.
    pub last_reviewed_head: Option<String>,
    /// Content hashes of hunks I've marked reviewed.
    pub reviewed_hunks: Vec<String>,
}

/// Cheap to clone; all clones share one database.
#[derive(Clone)]
pub struct Store {
    db: Option<Arc<Database>>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("enabled", &self.db.is_some())
            .finish()
    }
}

impl Store {
    /// Opens (or creates) the cache at `path`. Never fails: problems are
    /// logged and yield a disabled store.
    pub fn open(path: &Path) -> Self {
        match open_versioned(path) {
            Ok(db) => Self {
                db: Some(Arc::new(db)),
                path: Some(path.to_owned()),
            },
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cache disabled");
                Self::disabled()
            }
        }
    }

    /// A store that never hits and drops all writes.
    pub fn disabled() -> Self {
        Self {
            db: None,
            path: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.db.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn http_get(&self, key: &str) -> Option<HttpEntry> {
        self.read_json(HTTP, key)
    }

    pub fn http_put(&self, key: &str, entry: &HttpEntry) {
        self.write_json(HTTP, key, entry);
    }

    pub fn query_get<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        let entry: QueryEntry = self.read_json(QUERIES, key)?;
        match serde_json::from_value(entry.value) {
            Ok(value) => Some(Cached {
                value,
                fetched_at: entry.fetched_at,
            }),
            Err(err) => {
                tracing::debug!(key, %err, "cached query no longer decodes; ignoring");
                None
            }
        }
    }

    pub fn query_put<T: Serialize>(&self, key: &str, value: &T) {
        let value = match serde_json::to_value(value) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(key, %err, "cannot encode query result for cache");
                return;
            }
        };
        self.write_json(
            QUERIES,
            key,
            &QueryEntry {
                fetched_at: now(),
                value,
            },
        );
    }

    pub fn review_get(&self, pr_key: &str) -> ReviewState {
        self.read_json(REVIEW, pr_key).unwrap_or_default()
    }

    pub fn review_put(&self, pr_key: &str, state: &ReviewState) {
        self.write_json(REVIEW, pr_key, state);
    }

    fn read_json<T: DeserializeOwned>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Option<T> {
        let db = self.db.as_ref()?;
        let result = (|| -> Result<Option<T>, StoreError> {
            let txn = db.begin_read()?;
            let table = match txn.open_table(table) {
                Ok(table) => table,
                Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
                Err(err) => return Err(err.into()),
            };
            let Some(bytes) = table.get(key)? else {
                return Ok(None);
            };
            Ok(Some(serde_json::from_slice(bytes.value())?))
        })();
        result.unwrap_or_else(|err| {
            tracing::warn!(key, %err, "cache read failed");
            None
        })
    }

    fn write_json<T: Serialize + ?Sized>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        key: &str,
        value: &T,
    ) {
        let Some(db) = self.db.as_ref() else { return };
        let result = (|| -> Result<(), StoreError> {
            let bytes = serde_json::to_vec(value)?;
            let txn = db.begin_write()?;
            {
                let mut table = txn.open_table(table)?;
                table.insert(key, bytes.as_slice())?;
            }
            txn.commit()?;
            Ok(())
        })();
        if let Err(err) = result {
            tracing::warn!(key, %err, "cache write failed");
        }
    }
}

fn open_versioned(path: &Path) -> Result<Database, StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        match Database::create(path) {
            Ok(db) if stored_version(&db)? == Some(SCHEMA_VERSION) => return Ok(db),
            Ok(db) => {
                tracing::info!("cache schema changed; discarding {}", path.display());
                drop(db);
            }
            // Someone else has it open: don't delete their file.
            Err(err @ redb::DatabaseError::DatabaseAlreadyOpen) => return Err(err.into()),
            Err(err) => tracing::warn!(%err, "cache unreadable; discarding {}", path.display()),
        }
        std::fs::remove_file(path)?;
    }
    let db = Database::create(path)?;
    let txn = db.begin_write()?;
    {
        let mut meta = txn.open_table(META)?;
        meta.insert(VERSION_KEY, SCHEMA_VERSION)?;
    }
    txn.commit()?;
    Ok(db)
}

fn stored_version(db: &Database) -> Result<Option<u64>, StoreError> {
    let txn = db.begin_read()?;
    let meta = match txn.open_table(META) {
        Ok(meta) => meta,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(meta.get(VERSION_KEY)?.map(|v| v.value()))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_queries_and_http() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cache.redb"));
        assert!(store.is_enabled());

        store.query_put("prs", &vec![1, 2, 3]);
        let cached: Cached<Vec<u32>> = store.query_get("prs").unwrap();
        assert_eq!(cached.value, vec![1, 2, 3]);

        let entry = HttpEntry {
            etag: "W/\"abc\"".into(),
            body: "{}".into(),
            fetched_at: 7,
        };
        store.http_put("/user", &entry);
        assert_eq!(store.http_get("/user"), Some(entry));
        assert_eq!(store.http_get("/missing"), None);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let store = Store::open(&path);
            let state = ReviewState {
                last_reviewed_head: Some("abc".into()),
                reviewed_hunks: vec!["h1".into()],
            };
            store.review_put("o/r#1", &state);
        }
        let store = Store::open(&path);
        assert_eq!(
            store.review_get("o/r#1").last_reviewed_head.as_deref(),
            Some("abc")
        );
        assert_eq!(store.review_get("o/r#2"), ReviewState::default());
    }

    #[test]
    fn discards_cache_on_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let store = Store::open(&path);
            store.query_put("k", &1);
            let db = store.db.as_ref().unwrap();
            let txn = db.begin_write().unwrap();
            txn.open_table(META)
                .unwrap()
                .insert(VERSION_KEY, SCHEMA_VERSION + 1)
                .unwrap();
            txn.commit().unwrap();
        }
        let store = Store::open(&path);
        assert!(store.is_enabled());
        assert!(store.query_get::<i32>("k").is_none());
    }

    #[test]
    fn discards_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        std::fs::write(&path, b"definitely not a redb file").unwrap();
        let store = Store::open(&path);
        assert!(store.is_enabled());
        store.query_put("k", &1);
        assert_eq!(store.query_get::<i32>("k").unwrap().value, 1);
    }

    #[test]
    fn second_open_degrades_to_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        let first = Store::open(&path);
        first.query_put("k", &1);
        let second = Store::open(&path);
        assert!(!second.is_enabled());
        assert!(second.query_get::<i32>("k").is_none());
        // The first holder's data survives.
        assert_eq!(first.query_get::<i32>("k").unwrap().value, 1);
    }

    #[test]
    fn disabled_store_is_inert() {
        let store = Store::disabled();
        store.query_put("k", &1);
        assert!(store.query_get::<i32>("k").is_none());
    }
}

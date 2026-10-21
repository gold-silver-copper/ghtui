//! On-disk cache, backed by redb, and review-state persistence
//! ([`reviews`], kept outside the cache). Both keep each account's apart
//! ([`account`]).
//!
//! The database carries a schema version. When it doesn't match
//! [`SCHEMA_VERSION`] the file is discarded and recreated; there are no
//! migrations. If the database can't be opened (another ghtui process holds
//! the lock, the disk is read-only, ...) the store runs disabled: reads miss
//! and writes are dropped, so the app keeps working without a cache.
//!
//! Cached answers (REST responses and queries) can be fetched again, so
//! they're written behind the reply: kept in memory, where reads see them,
//! and committed together in the background without waiting on the disk
//! ([`Db::flush`]). The rest is committed, durably, before returning.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redb::{Database, Durability, ReadableDatabase, TableDefinition, TableHandle};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::lock;

/// Bump whenever the meaning or encoding of any table changes.
pub const SCHEMA_VERSION: u64 = 17;

/// Where the pages visited lately are kept.
pub const VISITS: &str = "visits";
/// Where the tabs open when ghtui last quit are kept.
pub const TABS: &str = "tabs";

const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
/// REST responses keyed by request path, stored with their ETag.
const HTTP: TableDefinition<&str, &[u8]> = TableDefinition::new(HTTP_NAME);
const HTTP_NAME: &str = "http";
/// Decoded GraphQL results keyed by a caller-chosen key.
const QUERIES: TableDefinition<&str, &[u8]> = TableDefinition::new(QUERIES_NAME);
const QUERIES_NAME: &str = "queries";
/// Logins keyed by `<host>/<token fingerprint>`.
const ACCOUNTS: TableDefinition<&str, &[u8]> = TableDefinition::new("accounts");
pub mod account;
pub mod queue;
pub mod reviews;

pub use account::Account;
pub use reviews::Reviews;

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEntry {
    pub etag: String,
    /// The `Last-Modified` header, sent back as `If-Modified-Since`.
    #[serde(default)]
    pub last_modified: Option<String>,
    /// The `Link` header: where a list's next page is.
    #[serde(default)]
    pub link: Option<String>,
    pub body: String,
    /// How many seconds GitHub asks to wait before asking again
    /// (`X-Poll-Interval`), from the last answer: not kept.
    #[serde(skip)]
    pub poll_interval: Option<u64>,
}

/// A cached value with the time it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached<T> {
    pub value: T,
    /// Unix seconds.
    pub fetched_at: u64,
}

/// A query's row: written from a borrow and read straight into `T`, with
/// no JSON tree between.
#[derive(Serialize, Deserialize)]
struct QueryEntry<T> {
    fetched_at: u64,
    value: T,
}

/// What we remember about a PR between sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewState {
    /// Head SHA at the time of my last submitted review.
    #[serde(default)]
    pub last_reviewed_head: Option<String>,
    /// Content hashes of hunks I've marked reviewed.
    #[serde(default)]
    pub reviewed_hunks: Vec<String>,
    /// Comments written but not yet submitted to GitHub.
    #[serde(default)]
    pub pending: Vec<DraftComment>,
}

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DraftSide {
    Left,
    #[default]
    Right,
}

/// An unsubmitted review comment, anchored by file line numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftComment {
    /// Local identifier, unique within the PR.
    pub id: u64,
    pub path: String,
    pub body: String,
    pub side: DraftSide,
    /// `None` for a file-level comment.
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    pub start_side: Option<DraftSide>,
    /// Head commit the comment was written against.
    pub commit: String,
    /// GitHub's reason for rejecting it on the last submit.
    #[serde(default)]
    pub error: Option<String>,
}

impl ReviewState {
    /// An ID not used by any pending comment.
    pub fn next_draft_id(&self) -> u64 {
        self.pending.iter().map(|d| d.id).max().map_or(1, |m| m + 1)
    }
}

/// Cheap to clone; all clones share one database.
#[derive(Clone)]
pub struct Store {
    db: Option<Arc<Db>>,
    /// Where the database is, to say how big it is.
    path: Option<Arc<std::path::PathBuf>>,
    /// Whose entries this store reads and writes.
    account: Account,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("enabled", &self.db.is_some())
            .finish()
    }
}

impl Store {
    /// Opens (or creates) the cache at `path`. Never fails: problems are
    /// logged and yield a disabled store.
    pub fn open(path: &Path) -> Self {
        match open_versioned(path) {
            Ok(database) => Self {
                db: Some(Arc::new(Db {
                    database,
                    later: Mutex::default(),
                })),
                path: Some(Arc::new(path.to_owned())),
                account: Account::default(),
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
            account: Account::default(),
        }
    }

    /// This store, keeping to the account on `host` whose token has
    /// `fingerprint`: the login last seen with it, if any.
    #[must_use]
    pub fn for_account(mut self, host: &str, fingerprint: &str) -> Self {
        self.account = Account::new(host, fingerprint, None);
        let login = self
            .account
            .token_key()
            .and_then(|key| self.read_json::<String>(ACCOUNTS, &key));
        if let Some(login) = login {
            self.account.sign_in(&login);
        }
        self
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    /// GitHub says the token is `login`'s: its entries from now on, and
    /// from the start next time.
    pub fn signed_in(&self, login: &str) {
        if self.account.sign_in(login)
            && let Some(key) = self.account.token_key()
        {
            tracing::info!(login, "caching for this account");
            self.write_json(ACCOUNTS, &key, login);
        }
    }

    fn scoped(&self, key: &str) -> String {
        format!("{}{key}", self.account.scope())
    }

    pub fn is_enabled(&self) -> bool {
        self.db.is_some()
    }

    /// How big the database is on disk, in bytes.
    pub fn size(&self) -> Option<u64> {
        let path = self.path.as_ref()?;
        std::fs::metadata(path.as_path()).ok().map(|m| m.len())
    }

    pub fn http_get(&self, key: &str) -> Option<HttpEntry> {
        self.read_json(HTTP, &self.scoped(key))
    }

    pub fn http_put(&self, key: &str, entry: &HttpEntry) {
        self.write_later(HTTP_NAME, &self.scoped(key), entry);
    }

    pub fn query_get<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        let entry: QueryEntry<T> = self.read_json(QUERIES, &self.scoped(key))?;
        Some(Cached {
            value: entry.value,
            fetched_at: entry.fetched_at,
        })
    }

    pub fn query_put<T: Serialize>(&self, key: &str, value: &T) {
        let entry = QueryEntry {
            fetched_at: now(),
            value,
        };
        self.write_later(QUERIES_NAME, &self.scoped(key), &entry);
    }

    /// Commits what's written behind, durably, and waits for it: before
    /// exiting.
    pub fn sync(&self) {
        if let Some(db) = &self.db {
            db.sync();
        }
    }

    fn read_json<T: DeserializeOwned>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Option<T> {
        let db = self.db.as_ref()?;
        let result = (|| -> Result<Option<T>, StoreError> {
            if let Some(bytes) = lock(&db.later).rows.get(&(table.name(), key.to_owned())) {
                return Ok(Some(serde_json::from_slice(bytes)?));
            }
            let txn = db.database.begin_read()?;
            let Some(table) = open(&txn, table)? else {
                return Ok(None);
            };
            let Some(bytes) = table.get(key)? else {
                return Ok(None);
            };
            Ok(Some(serde_json::from_slice(bytes.value())?))
        })();
        result.unwrap_or_else(|err| {
            // What an older build wrote is fetched again.
            if matches!(err, StoreError::Encoding(_)) {
                tracing::debug!(key, %err, "cached row no longer decodes");
            } else {
                tracing::warn!(key, %err, "cache read failed");
            }
            None
        })
    }

    /// Runs `f` in one transaction, committed durably before returning;
    /// false when it couldn't (the store is disabled, or the disk refused).
    fn write(
        &self,
        key: &str,
        f: impl FnOnce(&redb::WriteTransaction) -> Result<(), StoreError>,
    ) -> bool {
        let Some(db) = self.db.as_ref() else {
            return false;
        };
        let result = (|| -> Result<(), StoreError> {
            let txn = db.database.begin_write()?;
            f(&txn)?;
            txn.commit()?;
            Ok(())
        })();
        result
            .inspect_err(|err| tracing::warn!(key, %err, "cache write failed"))
            .is_ok()
    }

    fn write_json<T: Serialize + ?Sized>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        key: &str,
        value: &T,
    ) -> bool {
        self.write(key, |txn| {
            let bytes = serde_json::to_vec(value)?;
            txn.open_table(table)?.insert(key, bytes.as_slice())?;
            Ok(())
        })
    }

    /// Writes `value` behind ([`Db::flush`]): reads see it at once.
    fn write_later<T: Serialize + ?Sized>(&self, table: &'static str, key: &str, value: &T) {
        let Some(db) = &self.db else { return };
        let bytes = match serde_json::to_vec(value) {
            Ok(bytes) => bytes,
            Err(err) => return tracing::warn!(key, %err, "cannot encode for the cache"),
        };
        let start = {
            let mut later = lock(&db.later);
            later.rows.insert((table, key.to_owned()), bytes.into());
            !std::mem::replace(&mut later.flushing, true)
        };
        if start {
            let db = db.clone();
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => drop(runtime.spawn_blocking(move || db.flush())),
                Err(_) => db.flush(),
            }
        }
    }
}

/// A row: its table's name and its key.
type Row = (&'static str, String);

/// The database, and the rows written behind.
struct Db {
    database: Database,
    later: Mutex<Later>,
}

#[derive(Default)]
struct Later {
    /// Rows not yet committed, by table name and key: the newest value.
    rows: HashMap<Row, Arc<[u8]>>,
    /// Whether a [`Db::flush`] is running.
    flushing: bool,
    /// When a commit last reached the disk.
    durable: Option<Instant>,
}

/// How long rows written behind may go without reaching the disk: commits
/// in between aren't synced, and a crash loses them.
const SYNC_EVERY: Duration = Duration::from_secs(30);

impl Db {
    /// Commits the rows written behind, together, until none are left:
    /// those written meanwhile go in the next commit. Synced only every
    /// [`SYNC_EVERY`].
    fn flush(&self) {
        loop {
            let (rows, due) = {
                let mut later = lock(&self.later);
                if later.rows.is_empty() {
                    later.flushing = false;
                    return;
                }
                let rows: Vec<_> = later
                    .rows
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let due = later.durable.is_none_or(|at| at.elapsed() >= SYNC_EVERY);
                (rows, due)
            };
            self.commit(
                &rows,
                if due {
                    Durability::Immediate
                } else {
                    Durability::None
                },
            );
            let mut later = lock(&self.later);
            if due {
                later.durable = Some(Instant::now());
            }
            // Committed (or failed, and said so): unless written again since.
            for (row, bytes) in rows {
                if later
                    .rows
                    .get(&row)
                    .is_some_and(|now| Arc::ptr_eq(now, &bytes))
                {
                    later.rows.remove(&row);
                }
            }
        }
    }

    /// Commits whatever is written behind, durably.
    fn sync(&self) {
        let rows: Vec<_> = lock(&self.later).rows.drain().collect();
        self.commit(&rows, Durability::Immediate);
    }

    fn commit(&self, rows: &[(Row, Arc<[u8]>)], durability: Durability) {
        let result = (|| -> Result<(), StoreError> {
            let mut txn = self.database.begin_write()?;
            txn.set_durability(durability)
                .map_err(|err| StoreError::Db(err.into()))?;
            for ((table, key), bytes) in rows {
                let mut table = txn.open_table(TableDefinition::<&str, &[u8]>::new(table))?;
                table.insert(key.as_str(), &**bytes)?;
            }
            txn.commit()?;
            Ok(())
        })();
        if let Err(err) = result {
            tracing::warn!(rows = rows.len(), %err, "cache write failed");
        }
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        self.sync();
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
    let Some(meta) = open(&txn, META)? else {
        return Ok(None);
    };
    Ok(meta.get(VERSION_KEY)?.map(|v| v.value()))
}

/// Table `def` to read; `None` when nothing was ever written to it.
fn open<K: redb::Key + 'static, V: redb::Value + 'static>(
    txn: &redb::ReadTransaction,
    def: TableDefinition<K, V>,
) -> Result<Option<redb::ReadOnlyTable<K, V>>, StoreError> {
    match txn.open_table(def) {
        Ok(table) => Ok(Some(table)),
        Err(redb::TableError::TableDoesNotExist(_)) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
            last_modified: None,
            link: None,
            body: "{}".into(),
            poll_interval: None,
        };
        store.http_put("/user", &entry);
        assert_eq!(store.http_get("/user"), Some(entry));
        assert_eq!(store.http_get("/missing"), None);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        Store::open(&path).query_put("k", &7);
        let store = Store::open(&path);
        assert_eq!(store.query_get::<i32>("k").unwrap().value, 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn written_behind_is_read_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cache.redb"));
        for i in 0..100 {
            store.query_put("k", &i);
            assert_eq!(store.query_get::<i32>("k").unwrap().value, i);
        }
        store.sync();
        assert_eq!(store.query_get::<i32>("k").unwrap().value, 99);
    }

    #[test]
    fn discards_cache_on_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let store = Store::open(&path);
            store.query_put("k", &1);
            let db = &store.db.as_ref().unwrap().database;
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
    fn keeps_accounts_apart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let alice = Store::open(&path).for_account("github.com", "aaaa");
            alice.query_put("tabs", &"alice's");
            alice.signed_in("alice");
            // What came before GitHub said whose the token is stays apart.
            assert!(alice.query_get::<String>("tabs").is_none());
            alice.query_put("tabs", &"alice's");
            let bob = alice.clone().for_account("github.com", "bbbb");
            assert!(bob.query_get::<String>("tabs").is_none());
            bob.signed_in("bob");
            bob.query_put("tabs", &"bob's");
            let ghe = alice.for_account("ghe.example.com", "aaaa");
            ghe.signed_in("alice");
            assert!(ghe.query_get::<String>("tabs").is_none());
        }
        // Next time, each token finds its account's at once.
        let alice = Store::open(&path).for_account("github.com", "aaaa");
        assert_eq!(alice.query_get::<String>("tabs").unwrap().value, "alice's");
        assert_eq!(alice.account().scope(), "github.com/alice/");
        let bob = alice.for_account("github.com", "bbbb");
        assert_eq!(bob.query_get::<String>("tabs").unwrap().value, "bob's");
    }

    #[test]
    fn disabled_store_is_inert() {
        let store = Store::disabled();
        store.query_put("k", &1);
        assert!(store.query_get::<i32>("k").is_none());
    }
}

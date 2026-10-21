//! Writes waiting for GitHub ([`crate::offline::queue`]), kept in the
//! cache's database, apart for each account: one row per write, by its
//! number, so each change to one is its own transaction, written before
//! anything is sent.

use redb::{ReadableDatabase, TableDefinition};
use serde::{Serialize, de::DeserializeOwned};

use super::{Store, StoreError};

/// Rows keyed by `<account scope>queue/<number, 20 digits>`, so an
/// account's are together and in order.
const QUEUE: TableDefinition<&str, &[u8]> = TableDefinition::new("queue");

impl Store {
    fn queue_key(&self, id: u64) -> String {
        self.scoped(&format!("queue/{id:020}"))
    }

    /// Writes (or rewrites) row `id`, durably, before returning; false
    /// when it couldn't (the store is disabled, or the disk refused).
    pub fn queue_put<T: Serialize>(&self, id: u64, value: &T) -> bool {
        self.write_json(QUEUE, &self.queue_key(id), value)
    }

    /// Takes row `id` out.
    pub fn queue_remove(&self, id: u64) -> bool {
        let key = self.queue_key(id);
        self.write(&key, |txn| {
            txn.open_table(QUEUE)?.remove(key.as_str())?;
            Ok(())
        })
    }

    /// This account's rows, in order; a row that no longer decodes is
    /// left out (and said in the log).
    pub fn queue_all<T: DeserializeOwned>(&self) -> Vec<(u64, T)> {
        let Some(db) = self.db.as_ref() else { return Vec::new() };
        let prefix = self.scoped("queue/");
        let result = (|| -> Result<Vec<(u64, T)>, StoreError> {
            let txn = db.database.begin_read()?;
            let Some(table) = super::open(&txn, QUEUE)? else { return Ok(Vec::new()) };
            let mut rows = Vec::new();
            for row in table.range(prefix.as_str()..)? {
                let (key, value) = row?;
                let Some(id) = key.value().strip_prefix(prefix.as_str()) else { break };
                let Ok(id) = id.parse::<u64>() else { continue };
                match serde_json::from_slice(value.value()) {
                    Ok(value) => rows.push((id, value)),
                    Err(err) => tracing::warn!(%err, id, "a queued write no longer reads"),
                }
            }
            Ok(rows)
        })();
        result.unwrap_or_else(|err| {
            tracing::warn!(%err, "couldn't read the queued writes");
            Vec::new()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_their_order_apart_for_each_account() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let alice = Store::open(&path).for_account("github.com", "aaaa");
            alice.signed_in("alice");
            let bob = alice.clone().for_account("github.com", "bbbb");
            bob.signed_in("bob");
            // Ten after nine, as numbers.
            assert!(alice.queue_put(10, &"ten"));
            assert!(alice.queue_put(9, &"nine"));
            assert!(bob.queue_put(1, &"bob's"));
            assert!(alice.queue_remove(9));
            assert!(alice.queue_put(11, &"eleven"));
        }
        // After a restart, as they were.
        let alice = Store::open(&path).for_account("github.com", "aaaa");
        let rows: Vec<(u64, String)> = alice.queue_all();
        assert_eq!(rows, [(10, "ten".into()), (11, "eleven".into())]);
        let other = alice.for_account("ghe.example.com", "aaaa");
        assert!(other.queue_all::<String>().is_empty());
    }
}

use std::{collections::HashMap, sync::Mutex};

use uuid::Uuid;

use crate::{db::Database, Result};

/// In-memory full-image hit accumulator.
///
/// Requests only update this map. A background task periodically drains it and
/// persists all distinct screenshot counts in one database transaction.
#[derive(Default)]
pub struct HitCounter {
    pending: Mutex<HashMap<Uuid, u64>>,
}

impl HitCounter {
    pub fn record(&self, screenshot_id: Uuid) {
        let mut pending = self.pending.lock().unwrap();
        let count = pending.entry(screenshot_id).or_default();
        *count = count.saturating_add(1);
    }

    /// Persist all currently pending hits, returning the number of hits flushed.
    ///
    /// Hits recorded while the database transaction runs accumulate in a fresh
    /// map. If the transaction fails, the drained batch is merged back in.
    pub fn flush(&self, db: &Database) -> Result<u64> {
        let batch = {
            let mut pending = self.pending.lock().unwrap();
            pending.drain().collect::<Vec<_>>()
        };

        if batch.is_empty() {
            return Ok(0);
        }

        let hit_total = batch
            .iter()
            .fold(0_u64, |total, (_, count)| total.saturating_add(*count));

        if let Err(err) = db.increment_screenshot_hit_counts(&batch) {
            let mut pending = self.pending.lock().unwrap();
            for (screenshot_id, count) in batch {
                let pending_count = pending.entry(screenshot_id).or_default();
                *pending_count = pending_count.saturating_add(count);
            }
            return Err(err);
        }

        Ok(hit_total)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn concurrent_recording_keeps_every_hit() {
        let counter = Arc::new(HitCounter::default());
        let screenshot_id = Uuid::new_v4();
        let mut threads = Vec::new();

        for _ in 0..8 {
            let counter = counter.clone();
            threads.push(std::thread::spawn(move || {
                for _ in 0..1_000 {
                    counter.record(screenshot_id);
                }
            }));
        }

        for thread in threads {
            thread.join().unwrap();
        }

        let pending = counter.pending.lock().unwrap();
        assert_eq!(pending.get(&screenshot_id), Some(&8_000));
    }

    #[test]
    fn failed_flush_restores_the_drained_batch() {
        let counter = HitCounter::default();
        let screenshot_id = Uuid::new_v4();
        counter.record(screenshot_id);
        counter.record(screenshot_id);

        // Deliberately omit migrations so the screenshot update fails.
        let db = Database::open_in_memory().unwrap();
        assert!(counter.flush(&db).is_err());

        let pending = counter.pending.lock().unwrap();
        assert_eq!(pending.get(&screenshot_id), Some(&2));
    }
}

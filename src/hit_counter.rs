use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio::sync::Notify;
use uuid::Uuid;

use crate::{db::Database, Result};

pub const HIT_COUNT_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

struct PendingHits {
    counts: HashMap<Uuid, u64>,
    last_hit_at: Instant,
}

/// In-memory full-image hit accumulator.
///
/// Requests only update this map. A background task periodically drains it and
/// persists all distinct screenshot counts in one database transaction.
pub struct HitCounter {
    pending: Mutex<PendingHits>,
    idle_flush: Notify,
}

impl Default for HitCounter {
    fn default() -> Self {
        Self {
            pending: Mutex::new(PendingHits {
                counts: HashMap::new(),
                last_hit_at: Instant::now(),
            }),
            idle_flush: Notify::new(),
        }
    }
}

impl HitCounter {
    pub fn record(&self, screenshot_id: Uuid) {
        if self.record_at(screenshot_id, Instant::now()) {
            self.idle_flush.notify_one();
        }
    }

    fn record_at(&self, screenshot_id: Uuid, now: Instant) -> bool {
        let mut pending = self.pending.lock().unwrap();
        let was_idle =
            now.saturating_duration_since(pending.last_hit_at) >= HIT_COUNT_FLUSH_INTERVAL;
        pending.last_hit_at = now;
        let count = pending.counts.entry(screenshot_id).or_default();
        *count = count.saturating_add(1);
        was_idle
    }

    /// Wait until the first hit arrives after at least one idle flush interval.
    pub async fn idle_flush_requested(&self) {
        self.idle_flush.notified().await;
    }

    /// Persist all currently pending hits, returning the number of hits flushed.
    ///
    /// Hits recorded while the database transaction runs accumulate in a fresh
    /// map. If the transaction fails, the drained batch is merged back in.
    pub fn flush(&self, db: &Database) -> Result<u64> {
        let batch = {
            let mut pending = self.pending.lock().unwrap();
            pending.counts.drain().collect::<Vec<_>>()
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
                let pending_count = pending.counts.entry(screenshot_id).or_default();
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
        assert_eq!(pending.counts.get(&screenshot_id), Some(&8_000));
    }

    #[test]
    fn only_the_first_hit_after_an_idle_cycle_requests_an_immediate_flush() {
        let counter = HitCounter::default();
        let screenshot_id = Uuid::new_v4();
        let start = Instant::now();
        counter.pending.lock().unwrap().last_hit_at = start;

        assert!(!counter.record_at(
            screenshot_id,
            start + HIT_COUNT_FLUSH_INTERVAL - Duration::from_millis(1)
        ));
        assert!(counter.record_at(screenshot_id, start + (HIT_COUNT_FLUSH_INTERVAL * 2)));
        assert!(!counter.record_at(
            screenshot_id,
            start + (HIT_COUNT_FLUSH_INTERVAL * 2) + Duration::from_millis(1)
        ));
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
        assert_eq!(pending.counts.get(&screenshot_id), Some(&2));
    }
}

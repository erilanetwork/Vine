use std::sync::atomic::{AtomicU64, Ordering};

/// Counters the proxy bumps as things happen. Gauges like player counts are read straight from
/// the sessions when metrics are scraped, so they never drift.
#[derive(Default)]
pub struct Metrics {
    pub connections: AtomicU64,
    pub rate_limited: AtomicU64,
    pub status_pings: AtomicU64,
    pub logins: AtomicU64,
    pub logins_rejected: AtomicU64,
    pub server_switches: AtomicU64,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

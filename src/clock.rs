//! Pluggable time source.
//!
//! Time is represented as Unix milliseconds (`i64`). Inject [`ManualClock`] in
//! tests or for deterministic simulations.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock provider used for expiry, regeneration and timestamps.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Production clock backed by the operating system.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        system_now_ms()
    }
}

/// Read the current Unix time in milliseconds.
pub fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Fully controllable clock, useful in tests and deterministic replays.
#[derive(Debug)]
pub struct ManualClock {
    now: AtomicI64,
}

impl ManualClock {
    pub fn new(now_ms: i64) -> Self {
        Self {
            now: AtomicI64::new(now_ms),
        }
    }

    pub fn set(&self, now_ms: i64) {
        self.now.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, delta_ms: i64) -> i64 {
        self.now.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

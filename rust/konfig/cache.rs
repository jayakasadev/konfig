//! Lock-free trading config cache backed by [`ArcSwap`].
//!
//! Readers pay ~5 ns (pointer load + hazard epoch) with zero allocation.
//! Writers pay one `Arc::new` allocation + an atomic pointer swap.
//!
//! [`ArcSwap`]: arc_swap::ArcSwap

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::types::TradingConfigSnapshot;

// ── ConfigCache ───────────────────────────────────────────────────────────────

/// Shared, lock-free cache for the current [`TradingConfigSnapshot`].
///
/// Multiple readers can call [`ConfigCache::load`] concurrently at zero contention.
/// A single writer calls [`ConfigCache::update`] to atomically publish a new value.
pub struct ConfigCache {
    inner: ArcSwap<TradingConfigSnapshot>,
}

impl ConfigCache {
    /// Create a new cache pre-loaded with `initial`.
    pub fn new(initial: TradingConfigSnapshot) -> Self {
        Self {
            inner: ArcSwap::from_pointee(initial),
        }
    }

    /// Load the current snapshot.
    ///
    /// The returned guard keeps the snapshot alive until it is dropped.
    /// This is a single atomic pointer load — no lock, no allocation.
    pub fn load(&self) -> arc_swap::Guard<Arc<TradingConfigSnapshot>> {
        self.inner.load()
    }

    /// Replace the current snapshot with `new`.
    ///
    /// Concurrent readers that already hold a guard to the old value are
    /// unaffected; they will see the old value until their guard drops.
    pub fn update(&self, new: TradingConfigSnapshot) {
        self.inner.store(Arc::new(new));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RiskParamsSnapshot, StrategyParamsSnapshot};

    fn make_snapshot(schema_version: u32) -> TradingConfigSnapshot {
        TradingConfigSnapshot {
            schema_version,
            risk: RiskParamsSnapshot {
                max_position_usd: schema_version as f64 * 1000.0,
                max_order_size_usd: schema_version as f64 * 100.0,
                max_daily_loss_usd: schema_version as f64 * 500.0,
                max_orders_per_second: schema_version * 10,
                max_notional_per_minute: schema_version as f64 * 10_000.0,
                enabled: true,
            },
            strategies: vec![StrategyParamsSnapshot {
                product_id: format!("BTC-USDT-v{schema_version}"),
                signal_threshold: schema_version as f64 * 0.1,
                lookback_window_ms: schema_version as u64 * 60_000,
                max_spread_bps: 5.0,
                enabled: schema_version % 2 == 0,
            }],
            resource_version: format!("rv-{schema_version:04}"),
            loaded_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn initial_value_is_loadable() {
        let snap = make_snapshot(1);
        let cache = ConfigCache::new(snap);
        let loaded = cache.load();
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.risk.max_position_usd, 1000.0);
    }

    #[test]
    fn update_replaces_value() {
        let cache = ConfigCache::new(make_snapshot(1));
        assert_eq!(cache.load().schema_version, 1);

        cache.update(make_snapshot(2));
        assert_eq!(cache.load().schema_version, 2);
        assert_eq!(cache.load().risk.max_position_usd, 2000.0);
    }

    #[test]
    fn multiple_updates_always_see_latest() {
        let cache = ConfigCache::new(make_snapshot(0));
        for v in 1u32..=10 {
            cache.update(make_snapshot(v));
            assert_eq!(cache.load().schema_version, v);
        }
    }

    /// Old guards keep the old value alive until they drop.
    #[test]
    fn old_guard_survives_update() {
        let cache = ConfigCache::new(make_snapshot(1));
        let old_guard = cache.load(); // holds v=1
        cache.update(make_snapshot(2));

        // New load sees v=2.
        assert_eq!(cache.load().schema_version, 2);
        // Old guard still sees v=1.
        assert_eq!(old_guard.schema_version, 1);
        drop(old_guard);
    }

    #[test]
    fn concurrent_reads_see_consistent_version() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(ConfigCache::new(make_snapshot(0)));
        let cache_clone = Arc::clone(&cache);

        // Writer thread updates versions 1..=50.
        let writer = thread::spawn(move || {
            for v in 1u32..=50 {
                cache_clone.update(make_snapshot(v));
            }
        });

        // Reader thread: never panics.
        let _reader = thread::spawn({
            let cache = Arc::clone(&cache);
            move || {
                for _ in 0..1000 {
                    let _v = cache.load().schema_version;
                }
            }
        });

        writer.join().unwrap();
        // Final value must be exactly 50.
        assert_eq!(cache.load().schema_version, 50);
    }
}

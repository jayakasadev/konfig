//! Integration tests for the K8s ConfigMap watcher.
//!
//! These tests spin up an isolated K3s cluster via Testcontainers — no external
//! kind cluster or pre-existing K8s context is required.  Docker must be running
//! on the test host (CI or local).
//!
//! Run with:
//! ```sh
//! cargo test --test integration_watcher --features integration -p konfig
//! ```
//!
//! The `integration` feature gate prevents these tests from running in the
//! default `cargo test` invocation (which has no Docker dependency).

#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::ConfigMap;
use kube::api::Api;
use tokio::time::timeout;

use konfig::cache::ConfigCache;
use konfig::types::TradingConfigSnapshot;
use konfig::watcher::Watcher;

use common::{k3s_client, maybe_delete, poll_until, upsert_config_map};

// ── Test constants ────────────────────────────────────────────────────────────

const NAMESPACE: &str = "default";

/// Unique ConfigMap name per test to avoid cross-test interference when tests
/// run concurrently.
const CM_APPLY: &str = "trading-config-integ-apply";
const CM_DELETE: &str = "trading-config-integ-delete";

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Watcher picks up an Applied ConfigMap event and updates the cache.
///
/// Flow:
/// 1. Spin up a K3s container via Testcontainers.
/// 2. Spawn `run_watcher` against `CM_APPLY` in background.
/// 3. Create ConfigMap schema_version=1 with `strategy.enabled=true`.
/// 4. Poll up to 15 s for `cache.schema_version == 1`.
/// 5. Assert risk + strategy fields are correct.
/// 6. Apply ConfigMap schema_version=2 with `strategy.enabled=false`.
/// 7. Poll up to 15 s for `cache.schema_version == 2`.
/// 8. Assert version and flag are updated.
#[tokio::test]
async fn watcher_applies_config_map_to_cache() {
    let (_container, client) = k3s_client().await;

    let cms: Api<ConfigMap> = Api::namespaced(client.clone(), NAMESPACE);

    // Clean slate.
    maybe_delete(&cms, CM_APPLY).await;

    // Start watcher in a background task.
    let cache = Arc::new(ConfigCache::new(TradingConfigSnapshot::default()));
    let watcher_cache = Arc::clone(&cache);
    let watcher_client = client.clone();
    let watcher = tokio::spawn(async move {
        Watcher::new(watcher_client)
            .run(watcher_cache, NAMESPACE.to_string(), CM_APPLY.to_string())
            .await
            .expect("watcher exited with error");
    });

    // ── Apply schema_version=1, strategy.enabled=true ─────────────────────────
    upsert_config_map(&cms, NAMESPACE, CM_APPLY, 1, true)
        .await
        .expect("failed to create ConfigMap schema_version=1");

    let cache_ref = Arc::clone(&cache);
    timeout(Duration::from_secs(15), async move {
        poll_until(Duration::from_secs(15), Duration::from_millis(250), || {
            cache_ref.load().schema_version == 1
        })
        .await;
    })
    .await
    .expect("timed out waiting for cache schema_version=1");

    {
        let snap = cache.load();
        assert_eq!(snap.schema_version, 1, "schema_version must be 1");
        assert_eq!(snap.risk.max_position_usd, 100_000.0);
        assert_eq!(snap.risk.max_order_size_usd, 5_000.0);
        assert_eq!(snap.risk.max_daily_loss_usd, 2_000.0);
        assert_eq!(snap.risk.max_orders_per_second, 100);
        assert!(snap.risk.enabled, "risk.enabled must be true");
        assert_eq!(snap.strategies.len(), 1, "must have one strategy");
        assert_eq!(snap.strategies[0].product_id, "BTC-USDT");
        assert!(snap.strategies[0].enabled, "strategy.enabled must be true");
        assert_eq!(snap.strategies[0].signal_threshold, 0.75);
        assert_eq!(snap.strategies[0].max_spread_bps, 20.0);
        assert_eq!(snap.strategies[0].lookback_window_ms, 60_000);
    }

    // ── Apply schema_version=2, strategy.enabled=false ────────────────────────
    upsert_config_map(&cms, NAMESPACE, CM_APPLY, 2, false)
        .await
        .expect("failed to update ConfigMap to schema_version=2");

    let cache_ref2 = Arc::clone(&cache);
    timeout(Duration::from_secs(15), async move {
        poll_until(Duration::from_secs(15), Duration::from_millis(250), || {
            cache_ref2.load().schema_version == 2
        })
        .await;
    })
    .await
    .expect("timed out waiting for cache schema_version=2");

    {
        let snap = cache.load();
        assert_eq!(snap.schema_version, 2, "schema_version must be 2 after update");
        assert!(
            !snap.strategies[0].enabled,
            "strategy.enabled must be false at schema_version=2"
        );
    }

    // ── Cleanup ───────────────────────────────────────────────────────────────
    maybe_delete(&cms, CM_APPLY).await;
    watcher.abort();
    // _container is dropped here, stopping the K3s container.
}

/// Watcher retains last-known-good config after the ConfigMap is deleted.
///
/// Flow:
/// 1. Spin up a K3s container via Testcontainers.
/// 2. Spawn `run_watcher` against `CM_DELETE`.
/// 3. Create ConfigMap schema_version=5.
/// 4. Poll until `cache.schema_version == 5`.
/// 5. Delete the ConfigMap.
/// 6. Wait 1 s for the Deleted event to be processed.
/// 7. Assert cache still holds schema_version=5 (last-known-good retained on delete).
#[tokio::test]
async fn watcher_retains_cache_on_config_map_delete() {
    let (_container, client) = k3s_client().await;

    let cms: Api<ConfigMap> = Api::namespaced(client.clone(), NAMESPACE);

    maybe_delete(&cms, CM_DELETE).await;

    let cache = Arc::new(ConfigCache::new(TradingConfigSnapshot::default()));
    let watcher_cache = Arc::clone(&cache);
    let watcher_client = client.clone();
    let watcher = tokio::spawn(async move {
        Watcher::new(watcher_client)
            .run(watcher_cache, NAMESPACE.to_string(), CM_DELETE.to_string())
            .await
            .expect("watcher exited with error");
    });

    // Seed the cache with schema_version=5.
    upsert_config_map(&cms, NAMESPACE, CM_DELETE, 5, true)
        .await
        .expect("failed to create ConfigMap schema_version=5");

    let cache_ref = Arc::clone(&cache);
    timeout(Duration::from_secs(15), async move {
        poll_until(Duration::from_secs(15), Duration::from_millis(250), || {
            cache_ref.load().schema_version == 5
        })
        .await;
    })
    .await
    .expect("timed out waiting for cache to be seeded at schema_version=5");

    // Delete the ConfigMap and let the Deleted event be processed.
    use kube::api::DeleteParams;
    cms.delete(CM_DELETE, &DeleteParams::default())
        .await
        .expect("failed to delete ConfigMap");

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Cache must retain last-known-good (watcher logs a warning but does not clear the cache).
    assert_eq!(
        cache.load().schema_version,
        5,
        "cache must retain schema_version=5 after ConfigMap deletion (last-known-good)"
    );

    watcher.abort();
    // _container is dropped here, stopping the K3s container.
}

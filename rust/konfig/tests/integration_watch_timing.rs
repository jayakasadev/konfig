//! Watch-stream timing integration test.
//!
//! Verifies that the watcher propagates a ConfigMap apply event to the cache
//! within 500 ms of the `kubectl apply` (create or update) on a live K3s cluster.
//!
//! Run with:
//! ```sh
//! cargo test --test integration_watch_timing --features integration -p konfig
//! ```
//!
//! The `integration` feature gate prevents this test from running in the
//! default `cargo test` invocation (which has no Docker dependency).

#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::ConfigMap;
use kube::api::Api;
use tokio::time::timeout;

use konfig::cache::ConfigCache;
use konfig::types::TradingConfigSnapshot;
use konfig::watcher::Watcher;

use common::{k3s_client, maybe_delete, poll_until, upsert_config_map};

const NAMESPACE: &str = "default";
const CM_TIMING: &str = "trading-config-timing";

/// Watch-stream timing: subscribe → apply → cache update ≤ 500 ms.
///
/// Flow:
/// 1. Spin up a K3s container and connect a watcher in the background.
/// 2. Record `t0` immediately before applying a ConfigMap.
/// 3. Poll the cache at 10 ms intervals until `schema_version` is observed.
/// 4. Assert the elapsed time from `t0` to observation is ≤ 500 ms.
/// 5. Repeat for an *update* to the same ConfigMap to cover the MODIFIED event path.
///
/// The 500 ms budget is intentionally generous to account for container startup
/// variability on shared CI runners.  The watcher itself processes events in
/// microseconds; this budget covers K3s watch-stream delivery latency only.
#[tokio::test]
async fn watch_stream_update_propagates_within_500ms() {
    let (_container, client) = k3s_client().await;

    let cms: Api<ConfigMap> = Api::namespaced(client.clone(), NAMESPACE);

    // Clean slate.
    maybe_delete(&cms, CM_TIMING).await;

    // Start the watcher before applying the ConfigMap so it is already
    // subscribed to the watch stream when the event fires.
    let cache = Arc::new(ConfigCache::new(TradingConfigSnapshot::default()));
    let watcher_cache = Arc::clone(&cache);
    let watcher_client = client.clone();
    let watcher = tokio::spawn(async move {
        Watcher::new(watcher_client)
            .run(
                watcher_cache,
                NAMESPACE.to_string(),
                CM_TIMING.to_string(),
            )
            .await
            .expect("watcher exited with error");
    });

    // Give the watcher a moment to establish the watch stream before we apply.
    // This avoids a race where the ConfigMap arrives before the LIST+WATCH
    // handshake completes and the event is replayed as an ADDED event rather
    // than a MODIFIED event (both are fine, but the timing window shrinks).
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ── CREATE event ──────────────────────────────────────────────────────────
    let t0 = Instant::now();
    upsert_config_map(&cms, NAMESPACE, CM_TIMING, 10, true)
        .await
        .expect("failed to create ConfigMap schema_version=10");

    let cache_ref = Arc::clone(&cache);
    timeout(Duration::from_secs(5), async move {
        poll_until(Duration::from_secs(5), Duration::from_millis(10), || {
            cache_ref.load().schema_version == 10
        })
        .await;
    })
    .await
    .expect("timed out waiting for CREATE event within 5 s");

    let elapsed_create = t0.elapsed();
    assert!(
        elapsed_create <= Duration::from_millis(500),
        "CREATE event propagation took {elapsed_create:?} — must be ≤ 500 ms"
    );

    // ── MODIFIED event ────────────────────────────────────────────────────────
    let t1 = Instant::now();
    upsert_config_map(&cms, NAMESPACE, CM_TIMING, 11, false)
        .await
        .expect("failed to update ConfigMap to schema_version=11");

    let cache_ref2 = Arc::clone(&cache);
    timeout(Duration::from_secs(5), async move {
        poll_until(Duration::from_secs(5), Duration::from_millis(10), || {
            cache_ref2.load().schema_version == 11
        })
        .await;
    })
    .await
    .expect("timed out waiting for MODIFIED event within 5 s");

    let elapsed_modified = t1.elapsed();
    assert!(
        elapsed_modified <= Duration::from_millis(500),
        "MODIFIED event propagation took {elapsed_modified:?} — must be ≤ 500 ms"
    );

    // ── Cleanup ───────────────────────────────────────────────────────────────
    maybe_delete(&cms, CM_TIMING).await;
    watcher.abort();
    // _container is dropped here, stopping the K3s container.
}

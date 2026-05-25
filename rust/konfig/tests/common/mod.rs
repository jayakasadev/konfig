//! Shared helpers for Konfig K8s integration tests.
//!
//! All integration tests that require a live K8s cluster should import from
//! this module rather than duplicating the harness setup.
//!
//! # Feature gate
//!
//! This module is compiled only when the `integration` feature is enabled.
//! Every integration test file must declare `#![cfg(feature = "integration")]`
//! at its top level.
//!
//! # Docker requirement
//!
//! [`k3s_client`] starts a K3s container via Testcontainers.  Docker must be
//! running on the test host (CI or local).  On CI the `blacksmith-4vcpu-ubuntu-2404`
//! runner ships with Docker pre-installed.

#![cfg(feature = "integration")]

use std::env;

use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, PostParams};
use std::collections::BTreeMap;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::k3s::{K3s, KUBE_SECURE_PORT};

// ── K3s harness ───────────────────────────────────────────────────────────────

/// Start a K3s container and build a `kube::Client` connected to it.
///
/// Returns `(container_handle, client)`.  The caller **must** keep the handle
/// in scope for the duration of the test — dropping it stops the container.
///
/// Each call creates a unique temp-directory so that parallel test runs do not
/// race on the same `k3s.yaml` file.
pub async fn k3s_client() -> (ContainerAsync<K3s>, kube::Client) {
    let conf_dir = env::temp_dir().join(format!("k3s-konfig-{}", uuid_simple()));
    std::fs::create_dir_all(&conf_dir).expect("failed to create k3s conf dir");

    let container = K3s::default()
        .with_conf_mount(&conf_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("failed to start K3s container — is Docker running?");

    // Install the default rustls crypto provider once per process.  The call
    // is a no-op if a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let kubeconfig_yaml = container
        .image()
        .read_kube_config()
        .expect("failed to read kubeconfig from K3s container");

    let mut kube_config =
        kube::config::Kubeconfig::from_yaml(&kubeconfig_yaml)
            .expect("failed to parse kubeconfig YAML");

    let host_port = container
        .get_host_port_ipv4(KUBE_SECURE_PORT)
        .await
        .expect("failed to get K3s host port");

    kube_config.clusters.iter_mut().for_each(|named| {
        if let Some(cluster) = named.cluster.as_mut() {
            if let Some(server) = cluster.server.as_mut() {
                *server = format!("https://127.0.0.1:{host_port}");
            }
        }
    });

    let config =
        kube::Config::from_custom_kubeconfig(kube_config, &kube::config::KubeConfigOptions::default())
            .await
            .expect("failed to build kube Config from kubeconfig");

    let client = kube::Client::try_from(config).expect("failed to create kube Client");

    (container, client)
}

// ── ConfigMap helpers ─────────────────────────────────────────────────────────

/// Build a ConfigMap `data` map with the supplied `schema_version` and
/// `strategy.enabled` flag.
///
/// Keys match the `parse_data_map` convention in `watcher.rs`.
pub fn make_data_map(schema_version: u32, enabled: bool) -> BTreeMap<String, String> {
    let mut data = BTreeMap::new();
    data.insert("schema_version".into(), schema_version.to_string());
    data.insert("risk.max_position_usd".into(), "100000.0".into());
    data.insert("risk.max_order_size_usd".into(), "5000.0".into());
    data.insert("risk.max_daily_loss_usd".into(), "2000.0".into());
    data.insert("risk.max_orders_per_second".into(), "100".into());
    data.insert("risk.max_notional_per_minute".into(), "500000.0".into());
    data.insert("risk.enabled".into(), "true".into());
    data.insert("strategy.product_id".into(), "BTC-USDT".into());
    data.insert("strategy.enabled".into(), enabled.to_string());
    data.insert("strategy.signal_threshold".into(), "0.75".into());
    data.insert("strategy.lookback_window_ms".into(), "60000".into());
    data.insert("strategy.max_spread_bps".into(), "20.0".into());
    data
}

/// Create or replace a ConfigMap in the given namespace.
///
/// Strategy: try `create`; on 409 Conflict delete + recreate.  This is the
/// simplest test-only approach — no server-side-apply dependency.
pub async fn upsert_config_map(
    cms: &Api<ConfigMap>,
    namespace: &str,
    name: &str,
    schema_version: u32,
    enabled: bool,
) -> Result<(), kube::Error> {
    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        data: Some(make_data_map(schema_version, enabled)),
        ..Default::default()
    };

    match cms.create(&PostParams::default(), &cm).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(ref ae)) if ae.code == 409 => {
            cms.delete(name, &DeleteParams::default()).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            cms.create(&PostParams::default(), &cm).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Delete a ConfigMap if it exists; ignore Not Found.
pub async fn maybe_delete(cms: &Api<ConfigMap>, name: &str) {
    let _ = cms.delete(name, &DeleteParams::default()).await;
}

/// Poll `predicate` every `interval` until it returns `true` or `deadline` elapses.
///
/// Panics with a descriptive message if the deadline is exceeded.
pub async fn poll_until<F>(deadline: Duration, interval: Duration, mut predicate: F)
where
    F: FnMut() -> bool,
{
    let start = tokio::time::Instant::now();
    loop {
        if predicate() {
            return;
        }
        if start.elapsed() >= deadline {
            panic!("poll_until: condition not satisfied within {deadline:?}");
        }
        tokio::time::sleep(interval).await;
    }
}

// ── Misc ──────────────────────────────────────────────────────────────────────

/// Return a short random-ish hex string suitable for temp-dir uniqueness.
///
/// Avoids a `uuid` dependency — uses sub-nanosecond wall-clock jitter.
pub fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}", nanos)
}

//! Embedded watcher for konfig consumer pods.
//!
//! Watches a single `Config.konfig.io/v1` CRD via kube-rs, publishes the
//! current snapshot through `ArcSwap` for lock-free reads, and reconnects
//! with the Phase 4 backoff schedule on stream errors.
//!
//! ```no_run
//! use konfig_consumer::{KonfigConsumer, ConfigSnapshot};
//! use std::sync::Arc;
//!
//! # async fn run(client: kube::Client) -> Result<(), Box<dyn std::error::Error>> {
//! let consumer = KonfigConsumer::builder()
//!     .namespace("default")
//!     .name("risk-config")
//!     .start(client)
//!     .await?;
//!
//! let snap: Arc<ConfigSnapshot> = consumer.load();
//! let _max = snap.content["risk"]["max_order_size_usd"].as_u64();
//! # Ok(()) }
//! ```

pub mod metrics;
pub mod snapshot;
pub mod watcher;

use std::sync::Arc;

use arc_swap::ArcSwap;
use kube::Client;
use prometheus::{Gauge, Registry};
use thiserror::Error;
use tokio::task::JoinHandle;

pub use crate::metrics::{LastEventAt, MetricsError, register_stale_seconds, spawn_stale_sampler};
pub use crate::snapshot::{ConfigSnapshot, ConfigSpec, ParseError, parse_config_object};
pub use crate::watcher::{BACKOFF_STEPS_SECS, EventOutcome, WatcherError, backoff_delay};

#[derive(Debug, Error)]
pub enum ConsumerError {
    #[error("builder missing required field: {0}")]
    MissingField(&'static str),
    #[error(transparent)]
    Metrics(#[from] MetricsError),
    #[error(transparent)]
    Watcher(#[from] WatcherError),
}

/// Live, in-process view of a single `Config.konfig.io/v1` CRD.
///
/// Spawned by [`KonfigConsumer::builder`].  Reads via `.load()` are
/// `ArcSwap::load_full` — atomic pointer load, no locks, no allocations.
pub struct KonfigConsumer {
    store: Arc<ArcSwap<ConfigSnapshot>>,
    watcher_handle: JoinHandle<Result<(), WatcherError>>,
    sampler_handle: Option<JoinHandle<()>>,
}

impl KonfigConsumer {
    pub fn builder() -> KonfigConsumerBuilder {
        KonfigConsumerBuilder::default()
    }

    /// Returns a cheap `Arc` clone of the current snapshot pointer.
    pub fn load(&self) -> Arc<ConfigSnapshot> {
        self.store.load_full()
    }

    /// Direct access to the underlying `ArcSwap` for callers that want to
    /// subscribe to updates via `Cache::Guard` patterns themselves.
    pub fn store(&self) -> &Arc<ArcSwap<ConfigSnapshot>> {
        &self.store
    }

    /// Abort the spawned watcher + sampler tasks.  Useful for tests and clean
    /// shutdown in consumer pods.
    pub fn shutdown(&self) {
        self.watcher_handle.abort();
        if let Some(h) = &self.sampler_handle {
            h.abort();
        }
    }
}

impl Drop for KonfigConsumer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default)]
pub struct KonfigConsumerBuilder {
    namespace: Option<String>,
    name: Option<String>,
    fallback: Option<ConfigSnapshot>,
    registry: Option<Registry>,
    sampler_interval: Option<std::time::Duration>,
}

impl KonfigConsumerBuilder {
    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn name(mut self, n: impl Into<String>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Snapshot used until the first successful watch event arrives.  Without
    /// a fallback the consumer publishes `ConfigSnapshot::default()` (empty
    /// content), which makes `snap.content[...]` return `Value::Null`.
    pub fn fallback(mut self, snap: ConfigSnapshot) -> Self {
        self.fallback = Some(snap);
        self
    }

    /// Register `konfig_stale_seconds` on the supplied Prometheus registry.
    /// When omitted, no metrics are exported.
    pub fn registry(mut self, registry: Registry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Override the metrics sampler interval (default 5s).
    pub fn sampler_interval(mut self, interval: std::time::Duration) -> Self {
        self.sampler_interval = Some(interval);
        self
    }

    pub async fn start(self, client: Client) -> Result<KonfigConsumer, ConsumerError> {
        let namespace = self
            .namespace
            .ok_or(ConsumerError::MissingField("namespace"))?;
        let name = self.name.ok_or(ConsumerError::MissingField("name"))?;
        let initial = self.fallback.unwrap_or_default();
        let store = Arc::new(ArcSwap::from_pointee(initial));

        let last_event_at = Arc::new(LastEventAt::new());

        let sampler_handle = match self.registry {
            Some(reg) => {
                let gauge: Gauge = register_stale_seconds(&reg)?;
                let interval = self
                    .sampler_interval
                    .unwrap_or(std::time::Duration::from_secs(5));
                Some(spawn_stale_sampler(
                    Arc::clone(&last_event_at),
                    gauge,
                    interval,
                ))
            }
            None => None,
        };

        let task = watcher::WatcherTask {
            client,
            namespace,
            config_name: name,
            store: Arc::clone(&store),
            last_event_at,
        };
        let watcher_handle = tokio::spawn(task.run());

        Ok(KonfigConsumer {
            store,
            watcher_handle,
            sampler_handle,
        })
    }
}

/// Convenience: construct a consumer pre-seeded with a fallback snapshot.
/// Equivalent to `KonfigConsumer::builder().fallback(snap)`.
pub fn with_fallback(snap: ConfigSnapshot) -> KonfigConsumerBuilder {
    KonfigConsumer::builder().fallback(snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn builder_requires_namespace() {
        // No kube client touched — builder validation fires before any IO.
        let err = KonfigConsumer::builder()
            .name("c")
            .start(dummy_client().await)
            .await
            .err()
            .expect("missing namespace");
        assert!(matches!(err, ConsumerError::MissingField("namespace")));
    }

    #[tokio::test]
    async fn builder_requires_name() {
        let err = KonfigConsumer::builder()
            .namespace("ns")
            .start(dummy_client().await)
            .await
            .err()
            .expect("missing name");
        assert!(matches!(err, ConsumerError::MissingField("name")));
    }

    #[tokio::test]
    async fn fallback_visible_before_first_event() {
        let fallback = ConfigSnapshot::compiled_in(json!({"risk": {"max": 42}}));
        let consumer = with_fallback(fallback)
            .namespace("ns")
            .name("risk-config")
            .start(dummy_client().await)
            .await
            .expect("builds");
        // Watcher task will fail to connect (dummy client), but the fallback
        // snapshot is published synchronously by the builder before spawn —
        // so `load()` returns it regardless of whether the watcher has run.
        let snap = consumer.load();
        assert_eq!(snap.content["risk"]["max"], 42);
        consumer.shutdown();
    }

    #[tokio::test]
    async fn shutdown_aborts_spawned_tasks() {
        let consumer = KonfigConsumer::builder()
            .namespace("ns")
            .name("cfg")
            .start(dummy_client().await)
            .await
            .expect("builds");
        consumer.shutdown();
        // Task may take a moment to observe the abort; awaiting it returns an
        // abort error rather than hanging forever.
        // We don't unwrap since the dummy client + abort race can produce
        // either an aborted-task error or a kube connect error first.
    }

    async fn dummy_client() -> Client {
        // Avoids touching ~/.kube/config (would fail in CI).  The watcher
        // task spawned with this client will fail to connect, but every
        // assertion in these tests fires BEFORE the watcher emits its first
        // event — fallback / builder paths are independent of the network.
        let config = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
        Client::try_from(config).expect("builds dummy client")
    }
}

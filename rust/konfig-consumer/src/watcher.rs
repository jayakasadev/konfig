//! K8s watch loop for a single `Config.konfig.io/v1` CRD.
//!
//! Streams events via `kube::runtime::watcher`, parses each ADDED/MODIFIED
//! into `ConfigSnapshot`, and publishes it through `ArcSwap`.
//!
//! Reconnect backoff schedule per Phase 4 contract: 1, 2, 4, 8, 16, 30s (cap).

use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use futures_util::{StreamExt, TryStreamExt};
use kube::api::ApiResource;
use kube::core::DynamicObject;
use kube::runtime::watcher::{self as kube_watcher, Event, watcher as kube_watch_stream};
use kube::{Api, Client};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::metrics::LastEventAt;
use crate::snapshot::{ConfigSnapshot, parse_config_object};

pub const GROUP: &str = "konfig.io";
pub const VERSION: &str = "v1";
pub const KIND: &str = "Config";
pub const PLURAL: &str = "configs";

/// Phase 4 reconnect backoff (seconds): 1, 2, 4, 8, 16, then cap at 30.
pub const BACKOFF_STEPS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

pub fn backoff_delay(attempt: usize) -> Duration {
    let secs = BACKOFF_STEPS_SECS
        .get(attempt)
        .copied()
        .unwrap_or(*BACKOFF_STEPS_SECS.last().unwrap());
    Duration::from_secs(secs)
}

pub fn config_api_resource() -> ApiResource {
    ApiResource {
        group: GROUP.to_string(),
        version: VERSION.to_string(),
        api_version: format!("{GROUP}/{VERSION}"),
        kind: KIND.to_string(),
        plural: PLURAL.to_string(),
    }
}

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("kube error: {0}")]
    Kube(#[from] kube::Error),
    #[error("watcher error: {0}")]
    Watcher(#[from] kube_watcher::Error),
}

/// Outcome of feeding one `watcher::Event` to the snapshot store.
///
/// Returned by `handle_event` so unit tests can assert that BOOKMARK / Init
/// events do NOT surface to callers (== `Bookmark`), while ADDED/MODIFIED
/// produce `Updated` and DELETED produces `Deleted`.
#[derive(Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Updated,
    Deleted,
    Bookmark,
    Unparseable,
}

pub(crate) fn handle_event(
    event: Event<DynamicObject>,
    store: &Arc<ArcSwap<ConfigSnapshot>>,
    config_name: &str,
) -> EventOutcome {
    match event {
        Event::Apply(obj) | Event::InitApply(obj) => {
            let obj_name = obj.metadata.name.as_deref().unwrap_or("");
            if !config_name.is_empty() && obj_name != config_name {
                return EventOutcome::Bookmark;
            }
            match parse_config_object(&obj) {
                Some(snap) => {
                    info!(
                        name = %obj_name,
                        schema_version = snap.schema_version,
                        rv = %snap.resource_version,
                        "konfig-consumer: snapshot updated"
                    );
                    store.store(Arc::new(snap));
                    EventOutcome::Updated
                }
                None => EventOutcome::Unparseable,
            }
        }
        Event::Delete(obj) => {
            let name = obj.metadata.name.as_deref().unwrap_or("<unknown>");
            warn!(name = %name, "konfig-consumer: Config deleted — retaining last-known-good");
            EventOutcome::Deleted
        }
        Event::Init | Event::InitDone => {
            debug!("konfig-consumer: watch stream init / bookmark");
            EventOutcome::Bookmark
        }
    }
}

pub struct WatcherTask {
    pub client: Client,
    pub namespace: String,
    pub config_name: String,
    pub store: Arc<ArcSwap<ConfigSnapshot>>,
    pub last_event_at: Arc<LastEventAt>,
}

impl WatcherTask {
    pub async fn run(self) -> Result<(), WatcherError> {
        let ar = config_api_resource();
        let mut attempt: usize = 0;

        loop {
            let api: Api<DynamicObject> =
                Api::namespaced_with(self.client.clone(), &self.namespace, &ar);
            let wc = kube_watcher::Config::default()
                .fields(&format!("metadata.name={}", self.config_name));
            let mut stream = kube_watch_stream(api, wc).boxed();

            info!(
                namespace = %self.namespace,
                name = %self.config_name,
                attempt,
                "konfig-consumer: Config watcher started"
            );

            loop {
                match stream.try_next().await {
                    Ok(Some(event)) => {
                        self.last_event_at.touch();
                        let _ = handle_event(event, &self.store, &self.config_name);
                        attempt = 0;
                    }
                    Ok(None) => {
                        info!("konfig-consumer: watcher stream ended cleanly");
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(attempt, "konfig-consumer: watch error: {e} — marking stale");
                        mark_stale(&self.store);
                        let delay = backoff_delay(attempt);
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        break;
                    }
                }
            }
        }
    }
}

fn mark_stale(store: &Arc<ArcSwap<ConfigSnapshot>>) {
    let current = store.load();
    let mut next = (**current).clone();
    next.stale_since = Some(Instant::now());
    store.store(Arc::new(next));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_obj(name: &str, schema_version: u32, content: serde_json::Value) -> DynamicObject {
        let mut obj = DynamicObject::new(name, &config_api_resource());
        obj.metadata.name = Some(name.to_string());
        obj.metadata.namespace = Some("default".to_string());
        obj.metadata.resource_version = Some("rv-1".to_string());
        obj.data = json!({"spec": {"schema_version": schema_version, "content": content}});
        obj
    }

    fn empty_store() -> Arc<ArcSwap<ConfigSnapshot>> {
        Arc::new(ArcSwap::from_pointee(ConfigSnapshot::default()))
    }

    #[test]
    fn backoff_schedule_matches_phase4_contract() {
        let want = [1u64, 2, 4, 8, 16, 30, 30, 30, 30];
        for (i, &s) in want.iter().enumerate() {
            assert_eq!(
                backoff_delay(i),
                Duration::from_secs(s),
                "attempt {i} expected {s}s"
            );
        }
    }

    #[test]
    fn apply_event_publishes_snapshot() {
        let store = empty_store();
        let outcome = handle_event(
            Event::Apply(make_obj("risk-config", 4, json!({"max": 99}))),
            &store,
            "risk-config",
        );
        assert_eq!(outcome, EventOutcome::Updated);
        let snap = store.load();
        assert_eq!(snap.schema_version, 4);
        assert_eq!(snap.content["max"], 99);
    }

    #[test]
    fn init_apply_event_publishes_snapshot() {
        let store = empty_store();
        let outcome = handle_event(
            Event::InitApply(make_obj("risk-config", 1, json!({}))),
            &store,
            "risk-config",
        );
        assert_eq!(outcome, EventOutcome::Updated);
        assert_eq!(store.load().schema_version, 1);
    }

    #[test]
    fn delete_event_retains_last_known_good() {
        let store = empty_store();
        handle_event(
            Event::Apply(make_obj("risk-config", 3, json!({}))),
            &store,
            "risk-config",
        );
        let outcome = handle_event(
            Event::Delete(make_obj("risk-config", 3, json!({}))),
            &store,
            "risk-config",
        );
        assert_eq!(outcome, EventOutcome::Deleted);
        assert_eq!(store.load().schema_version, 3);
    }

    #[test]
    fn init_and_init_done_do_not_surface_to_caller() {
        let store = empty_store();
        let snap_before = store.load().resource_version.clone();

        assert_eq!(
            handle_event(Event::Init, &store, "risk-config"),
            EventOutcome::Bookmark
        );
        assert_eq!(
            handle_event(Event::InitDone, &store, "risk-config"),
            EventOutcome::Bookmark
        );

        // Snapshot pointer must not have moved.
        assert_eq!(store.load().resource_version, snap_before);
    }

    #[test]
    fn apply_for_unrelated_name_is_ignored() {
        let store = empty_store();
        let outcome = handle_event(
            Event::Apply(make_obj("other-config", 9, json!({"x": 1}))),
            &store,
            "risk-config",
        );
        assert_eq!(outcome, EventOutcome::Bookmark);
        assert_eq!(store.load().schema_version, 0);
    }

    #[test]
    fn unparseable_apply_does_not_replace_snapshot() {
        let store = empty_store();
        handle_event(
            Event::Apply(make_obj("risk-config", 5, json!({"ok": true}))),
            &store,
            "risk-config",
        );
        // Mangle the next obj — bad schema_version type forces parse failure.
        let mut bad = make_obj("risk-config", 0, json!({}));
        bad.data = json!({"spec": {"schema_version": "not-an-int"}});
        let outcome = handle_event(Event::Apply(bad), &store, "risk-config");
        assert_eq!(outcome, EventOutcome::Unparseable);
        assert_eq!(
            store.load().schema_version,
            5,
            "last-known-good must be retained"
        );
    }

    #[test]
    fn mark_stale_sets_stale_since() {
        let store = empty_store();
        handle_event(
            Event::Apply(make_obj("risk-config", 1, json!({}))),
            &store,
            "risk-config",
        );
        assert!(store.load().stale_since.is_none());
        mark_stale(&store);
        assert!(store.load().stale_since.is_some());
    }
}

//! Konfig — generic K8s config distribution.
//!
//! # Modules
//!
//! - [`types`] — `ConfigSnapshot`, `ConfigSpec`, `SecretSnapshot`
//! - [`cache`] — DashMap-backed multi-key lock-free config cache
//! - [`secret_cache`] — DashMap-backed multi-key lock-free secret cache
//! - [`watcher`] — kube-rs watcher for `Config.konfig.io/v1` CRDs
//! - [`configmap_watcher`] — watcher for ConfigMaps (konfig.io/managed=true)
//! - [`secret_watcher`] — watcher for Secrets (konfig.io/managed=true)
//! - [`grpc`] — gRPC server (Protobuf, standard tonic codec)
//! - [`import`] — CLI helper: onboard existing ConfigMaps as Config CRDs

pub mod cache;
pub mod configmap_watcher;
pub mod grpc;
pub mod import;
pub mod secret_cache;
pub mod secret_watcher;
pub mod types;
pub mod watcher;

/// Re-export `kube::Client` so consumers that depend on `@konfig//rust/konfig:konfig`
/// can call `konfig::KubeClient::try_default()` using the same kube crate instance
/// as the watcher, avoiding a cross-universe type mismatch when passing the client
/// to `konfig::watcher::Watcher::new()`.
pub use kube::Client as KubeClient;

// Generated protobuf types (via build.rs + tonic-build).
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/konfig.v1.rs"));
}

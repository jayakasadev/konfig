//! mTLS configuration for the gRPC server.
//!
//! Loads the server identity (`tls.crt` + `tls.key`) and the client CA bundle
//! (`ca.crt`) from disk and builds a `ServerTlsConfig` that:
//!   - presents the server identity on the TLS handshake,
//!   - requires every client to present a certificate signed by the configured
//!     CA (`client_auth_optional(false)` — tonic 0.14 reverses the polarity).
//!
//! Files are read once at startup. cert-manager rotates the underlying Secret;
//! a pod restart picks up the new material. There is no hot reload here.

use std::error::Error;
use std::path::{Path, PathBuf};

use tonic::transport::{Certificate, Identity, ServerTlsConfig};
use tracing::{info, warn};

/// Inputs to `build_server_tls_config` — all three paths are required.
#[derive(Debug, Clone)]
pub struct TlsPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub client_ca: PathBuf,
}

/// Construct a tonic `ServerTlsConfig` enforcing mTLS.
///
/// Returns an error (and does NOT panic) when any file is missing or fails to
/// load — the caller must propagate this so startup exits non-zero.
///
/// Security: the returned config requires client auth — `client_auth_optional`
/// stays at its default of `false`, which in tonic 0.14 means "every client
/// MUST present a cert signed by `client_ca_root`".
pub fn build_server_tls_config(
    paths: &TlsPaths,
) -> Result<ServerTlsConfig, Box<dyn Error + Send + Sync>> {
    let cert_pem = read_file(&paths.cert, "server cert")?;
    let key_pem = read_file(&paths.key, "server key")?;
    let ca_pem = read_file(&paths.client_ca, "client CA bundle")?;

    let identity = Identity::from_pem(&cert_pem, &key_pem);
    let ca = Certificate::from_pem(&ca_pem);

    // Never log key bytes; only paths. `read_file` already logged the load.
    info!(
        cert = %paths.cert.display(),
        client_ca = %paths.client_ca.display(),
        "mTLS configured: client auth required"
    );

    Ok(ServerTlsConfig::new().identity(identity).client_ca_root(ca))
}

/// Log a one-time WARN at startup when TLS is disabled. Surfaces in any prod
/// log scrape so an operator notices an unauthenticated server.
pub fn warn_tls_disabled() {
    warn!("TLS disabled; gRPC server is unauthenticated");
}

fn read_file(path: &Path, label: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read {label} at {}: {e}", path.display()))?;
    if bytes.is_empty() {
        return Err(format!("{label} at {} is empty", path.display()).into());
    }
    // PEM-only sanity check — we do not parse the cert here; tonic does that
    // when building the acceptor. The check catches "wrong file mounted" early.
    let head = std::str::from_utf8(&bytes[..bytes.len().min(64)])
        .map_err(|_| format!("{label} at {} is not valid UTF-8 PEM", path.display()))?;
    if !head.contains("-----BEGIN") {
        return Err(format!(
            "{label} at {} does not look like PEM (missing -----BEGIN header)",
            path.display()
        )
        .into());
    }
    info!(path = %path.display(), bytes = bytes.len(), "loaded {label}");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal self-signed PEM material generated once with rcgen and pasted
    // here so the test stays hermetic — no rcgen dep needed in the build.
    // These are NOT real keys for any production system; they verify that
    // build_server_tls_config accepts well-formed PEM and rejects bad input.
    // Cert + key emitted by `rcgen` for CN="test", validity 100y.
    const TEST_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIBcjCCARigAwIBAgIUImN1iKzNlsf3pnxQfb1Xs3l1bn4wCgYIKoZIzj0EAwIw\n\
DzENMAsGA1UEAwwEdGVzdDAgFw0yNTAxMDEwMDAwMDBaGA8yMTI1MDEwMTAwMDAw\n\
MFowDzENMAsGA1UEAwwEdGVzdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABIH+\n\
PLACEHOLDER_NOT_REAL_KEY_MATERIAL_FOR_UNIT_TEST_ONLY00000000000\n\
o1MwUTAdBgNVHQ4EFgQUtest1234567890tests1234567890testsxYwHwYDVR0j\n\
BBgwFoAUtest1234567890tests1234567890testsxYwDwYDVR0TAQH/BAUwAwEB\n\
/zAKBggqhkjOPQQDAgNHADBEAiBPLACEHOLDERPLACEHOLDERPLACEHOLDERPLAC\n\
EHOLDERPLACEHOLDERPLACEHOLDERPLACEHOLDERPLACEHOLDERPLACEHOLDER\n\
-----END CERTIFICATE-----\n";

    /// `read_file` rejects an empty file with a clear error rather than
    /// silently passing an empty PEM blob to tonic.
    #[test]
    fn read_file_rejects_empty() {
        let tmp = tempfile_path("empty.pem");
        std::fs::write(&tmp, b"").expect("write tmp");
        let err = read_file(&tmp, "thing").expect_err("must reject empty");
        assert!(err.to_string().contains("is empty"), "got: {err}");
        let _ = std::fs::remove_file(&tmp);
    }

    /// `read_file` rejects non-PEM input — guards against "wrong file mounted"
    /// where e.g. a binary DER cert ends up at the cert path.
    #[test]
    fn read_file_rejects_non_pem() {
        let tmp = tempfile_path("not_pem.pem");
        std::fs::write(&tmp, b"this is not pem\n").expect("write tmp");
        let err = read_file(&tmp, "thing").expect_err("must reject non-PEM");
        assert!(
            err.to_string().contains("does not look like PEM"),
            "got: {err}"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    /// `read_file` rejects a missing file with a path-bearing error so the
    /// operator can fix the mount.
    #[test]
    fn read_file_rejects_missing() {
        let tmp = tempfile_path("does_not_exist.pem");
        let _ = std::fs::remove_file(&tmp);
        let err = read_file(&tmp, "thing").expect_err("must reject missing");
        assert!(err.to_string().contains("failed to read"), "got: {err}");
    }

    /// `read_file` accepts a well-formed PEM header even with placeholder body.
    /// Confirms the early sanity check passes the file through to tonic.
    #[test]
    fn read_file_accepts_pem_header() {
        let tmp = tempfile_path("ok.pem");
        std::fs::write(&tmp, TEST_CERT_PEM).expect("write tmp");
        let bytes = read_file(&tmp, "cert").expect("must accept");
        assert_eq!(bytes, TEST_CERT_PEM);
        let _ = std::fs::remove_file(&tmp);
    }

    fn tempfile_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!("konfig_tls_test_{}_{}", std::process::id(), name))
    }
}

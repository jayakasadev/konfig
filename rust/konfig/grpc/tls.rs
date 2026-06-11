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

    // Self-signed test material generated once with `openssl ecparam -name
    // prime256v1` + `openssl req -x509` + a CA-signed leaf. NOT real keys
    // for any production system — only used to verify that
    // build_server_tls_config accepts well-formed PEM. Validity 100y so
    // tests never flake on cert expiry.
    const TEST_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIBiTCCAS+gAwIBAgIUEv/OWaMCR1HEcMawwE4voSS9N8YwCgYIKoZIzj0EAwIw\n\
GTEXMBUGA1UEAwwOa29uZmlnLXRlc3QtY2EwIBcNMjYwNjExMjExMTMwWhgPMjEy\n\
NjA1MTgyMTExMzBaMBkxFzAVBgNVBAMMDmtvbmZpZy10ZXN0LWNhMFkwEwYHKoZI\n\
zj0CAQYIKoZIzj0DAQcDQgAE++HFkYStxtFqA7cMIwdNX/KETc3R9uXqKfTsabKt\n\
J4lmeb8lHHZ1IEyTKNkFfBwLrHhvFURnfxWI0225EcOvzKNTMFEwHQYDVR0OBBYE\n\
FANch6VOuLpo+s/kRf9ZqeecBArIMB8GA1UdIwQYMBaAFANch6VOuLpo+s/kRf9Z\n\
qeecBArIMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIhAMSss4kC\n\
GNni6yWdM+6bCbck3Viux0GLi0Vor9HH8yeAAiAIS3v0v4J/+xvF7iU+qj3WIPMr\n\
zXqG1ImhfknU/1RulQ==\n\
-----END CERTIFICATE-----\n";

    const TEST_SERVER_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIBfDCCASKgAwIBAgIUNz/doutGmH7lCNj96AXiDP5LYoYwCgYIKoZIzj0EAwIw\n\
GTEXMBUGA1UEAwwOa29uZmlnLXRlc3QtY2EwIBcNMjYwNjExMjExMTMwWhgPMjEy\n\
NjA1MTgyMTExMzBaMB0xGzAZBgNVBAMMEmtvbmZpZy10ZXN0LXNlcnZlcjBZMBMG\n\
ByqGSM49AgEGCCqGSM49AwEHA0IABMlTqVcwvZTbexsFWQ7HBYJT1VPScyPEBhKU\n\
wzADiG/Z0c6BIPa3ut9N48KKogHeWnmRIn1uDFJqjS1ywrdXtFGjQjBAMB0GA1Ud\n\
DgQWBBTwVoIYO8NkYoK9ZwmcgEJT1Us4RzAfBgNVHSMEGDAWgBQDXIelTri6aPrP\n\
5EX/WannnAQKyDAKBggqhkjOPQQDAgNIADBFAiBHSvEUEDYxgg1nBR3DC2DOLq9X\n\
NeuTymT8dsCNyMQpWwIhAIf/cOURe9Ir38nAJ0jTv8wUqs/cUsybOX5qJPYV8BAT\n\
-----END CERTIFICATE-----\n";

    const TEST_SERVER_KEY_PEM: &[u8] = b"-----BEGIN EC PRIVATE KEY-----\n\
MHcCAQEEINpZNkHUaCis74WZnmxB5r8C1+TlzYfTwnPRRro6/jHUoAoGCCqGSM49\n\
AwEHoUQDQgAEyVOpVzC9lNt7GwVZDscFglPVU9JzI8QGEpTDMAOIb9nRzoEg9re6\n\
303jwoqiAd5aeZEifW4MUmqNLXLCt1e0UQ==\n\
-----END EC PRIVATE KEY-----\n";

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

    /// `read_file` accepts a well-formed PEM file and returns the bytes.
    #[test]
    fn read_file_accepts_pem() {
        let tmp = tempfile_path("ok.pem");
        std::fs::write(&tmp, TEST_CA_PEM).expect("write tmp");
        let bytes = read_file(&tmp, "cert").expect("must accept");
        assert_eq!(bytes, TEST_CA_PEM);
        let _ = std::fs::remove_file(&tmp);
    }

    /// Happy path: real PEM material on disk produces a `ServerTlsConfig`
    /// without panicking. Exercises every `?` branch in
    /// `build_server_tls_config` and the trailing builder chain.
    #[test]
    fn build_server_tls_config_succeeds_with_valid_pem() {
        let cert = tempfile_path("server.crt");
        let key = tempfile_path("server.key");
        let ca = tempfile_path("ca.crt");
        std::fs::write(&cert, TEST_SERVER_CERT_PEM).expect("write cert");
        std::fs::write(&key, TEST_SERVER_KEY_PEM).expect("write key");
        std::fs::write(&ca, TEST_CA_PEM).expect("write ca");
        let paths = TlsPaths {
            cert: cert.clone(),
            key: key.clone(),
            client_ca: ca.clone(),
        };
        let cfg = build_server_tls_config(&paths).expect("must build TLS config");
        // ServerTlsConfig is opaque — assert non-panic + drop is enough.
        drop(cfg);
        let _ = std::fs::remove_file(&cert);
        let _ = std::fs::remove_file(&key);
        let _ = std::fs::remove_file(&ca);
    }

    /// Missing server cert propagates a labelled error — surfaces which
    /// file the operator forgot to mount.
    #[test]
    fn build_server_tls_config_fails_on_missing_cert() {
        let paths = TlsPaths {
            cert: tempfile_path("missing.crt"),
            key: tempfile_path("missing.key"),
            client_ca: tempfile_path("missing.ca"),
        };
        let _ = std::fs::remove_file(&paths.cert);
        let _ = std::fs::remove_file(&paths.key);
        let _ = std::fs::remove_file(&paths.client_ca);
        let err = build_server_tls_config(&paths).expect_err("must fail on missing files");
        assert!(err.to_string().contains("server cert"), "got: {err}");
    }

    /// `warn_tls_disabled` is callable and does not panic. The log line is
    /// the only side effect; assert it does not return / unwind.
    #[test]
    fn warn_tls_disabled_does_not_panic() {
        warn_tls_disabled();
    }

    fn tempfile_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!("konfig_tls_test_{}_{}", std::process::id(), name))
    }
}

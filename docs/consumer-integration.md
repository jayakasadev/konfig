# Consumer integration

Consumer pods connect to the konfig gRPC endpoint on `:50051` and call
`Subscribe` or `Get`. The konfig server handles all K8s API interaction —
consumers need no Kubernetes credentials.

## gRPC client

### Python

```bash
pip install konfig-client
```

```python
from konfig_client import KonfigClient, AsyncKonfigClient

# Sync
client = KonfigClient("konfig.konfig-system.svc.cluster.local:50051")

# Point read
config = client.get(namespace="default", name="app-config")
print(config.schema_version)     # int
print(config.content)            # dict — parsed JSON
print(config.stale_since_ms)     # -1 = fresh; ≥0 = ms since watcher disconnected

# Live stream — reconnects automatically on error
for event in client.subscribe("default", names=["app-config"]):
    # event.event_type: "MODIFIED" | "DELETED"
    new_config = event.config.content

# Async
async with AsyncKonfigClient("konfig.konfig-system.svc.cluster.local:50051") as client:
    config = await client.get("default", "app-config")
    async for event in client.subscribe("default", ["app-config"]):
        process(event.config.content)
```

### Rust

```toml
[dependencies]
konfig = { git = "https://github.com/jayakasadev/konfig.git" }
```

```rust
use konfig::proto::konfig_service_client::KonfigServiceClient;
use konfig::proto::{GetRequest, SubscribeRequest};

let mut client = KonfigServiceClient::connect(
    "http://konfig.konfig-system.svc.cluster.local:50051"
).await?;

// Point read
let config = client.get(GetRequest {
    namespace: "default".into(),
    name: "app-config".into(),
}).await?.into_inner();

let content: serde_json::Value = serde_json::from_str(&config.content_json)?;

// Live stream
let mut stream = client.subscribe(SubscribeRequest {
    namespace: "default".into(),
    names: vec!["app-config".into()],
    resume_resource_version: String::new(),
}).await?.into_inner();

while let Some(event) = stream.next().await {
    let event = event?;
    let config = event.config.unwrap();
    println!("schema_version={}", config.schema_version);
}
```

### grpcurl (ad-hoc debugging)

```bash
# gRPC reflection is enabled — no proto file needed
grpcurl -plaintext konfig.konfig-system.svc.cluster.local:50051 list

# Get a config
grpcurl -plaintext \
  -d '{"namespace":"default","name":"app-config"}' \
  konfig.konfig-system.svc.cluster.local:50051 \
  konfig.v1.KonfigService/Get

# Live stream
grpcurl -plaintext \
  -d '{"namespace":"default","names":["app-config"]}' \
  konfig.konfig-system.svc.cluster.local:50051 \
  konfig.v1.KonfigService/Subscribe
```

## Available RPCs

| RPC | Description |
|-----|-------------|
| `Get(namespace, name)` | Point read — returns the current snapshot |
| `GetAll(namespace)` | Stream all configs in a namespace |
| `Apply(namespace, name, yaml_content)` | Write — enforces `schema_version` monotonicity |
| `Subscribe(namespace, names, resume_resource_version)` | Live stream — reconnect with saved RV for zero missed events |
| `GetSecret(namespace, name)` | Read a managed Secret (values base64-encoded) |
| `GetAllSecrets(namespace)` | Stream all Secrets in a namespace |
| `ApplySecret(namespace, name, yaml_content)` | Write a managed Secret (server base64-encodes) |

## Resume on reconnect

The `Subscribe` RPC accepts `resume_resource_version`. Save the last
`resource_version` from received events and pass it on reconnect:

```python
last_rv = ""
while True:
    try:
        for event in client.subscribe("default", ["app-config"],
                                       resume_resource_version=last_rv):
            last_rv = event.config.resource_version
            process(event.config.content)
    except KonfigError:
        time.sleep(1)   # brief backoff; the client library does this automatically
```

The Rust client handles reconnect automatically.

## Subscribing to multiple configs

Pass multiple names in one stream — one connection, one watch per namespace:

```python
for event in client.subscribe("default", names=["app-config", "feature-flags"]):
    print(event.config.name, event.config.content)
```

## Staleness detection

Every response includes:

- `age_ms` — milliseconds since the snapshot was loaded into cache
- `stale_since_ms` — `-1` when fresh; milliseconds since the watcher lost its K8s connection when stale

```python
config = client.get("default", "app-config")
if config.stale_since_ms >= 0:
    alert(f"Config is stale for {config.stale_since_ms}ms — watcher disconnected")
```

## Error handling

| gRPC status | Meaning | Action |
|---|---|---|
| `UNAVAILABLE` | Cache stale or server unreachable | Retry with backoff; use last-known-good value |
| `FAILED_PRECONDITION` | Apply rejected — `schema_version` not increasing | Fix the version and retry |
| `NOT_FOUND` | Config does not exist | Create it first |
| `RESOURCE_EXHAUSTED` | Subscriber too slow (ring buffer wrapped) | Reconnect with `resume_resource_version` |

## mTLS client certs

The konfig gRPC server requires mutual TLS by default. Every consumer must
present a certificate signed by the same in-cluster CA the server uses
(`Issuer/konfig-ca-issuer` in the `konfig-system` namespace).

### 1. Issue a client `Certificate`

In the consumer's namespace, request a cert from the cluster's konfig CA.
Because the issuer lives in `konfig-system`, use a `ClusterIssuer` indirection
or copy the CA secret with `trust-manager` — the example below assumes a
`ClusterIssuer/konfig-ca-issuer` mirrored from the in-namespace `Issuer`:

```yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: my-app-konfig-client
  namespace: my-app
spec:
  secretName: my-app-konfig-client-tls
  # CN identifies the caller in konfig logs; pick something stable per
  # ServiceAccount, not per pod.
  commonName: my-app.my-app.svc
  duration: 2160h     # 90d
  renewBefore: 720h   # 30d
  privateKey:
    algorithm: ECDSA
    size: 256
    rotationPolicy: Always
  usages:
    - client auth
  issuerRef:
    kind: ClusterIssuer
    name: konfig-ca-issuer
```

### 2. Mount it in the consumer pod

```yaml
spec:
  containers:
    - name: my-app
      volumeMounts:
        - name: konfig-client-tls
          mountPath: /var/run/konfig-client-tls
          readOnly: true
  volumes:
    - name: konfig-client-tls
      secret:
        secretName: my-app-konfig-client-tls
        defaultMode: 0o400
```

The Secret contains `tls.crt`, `tls.key`, and `ca.crt`. `ca.crt` is the
trust anchor for the konfig server — clients need it to verify the server
cert.

### 3. Wire TLS into the client

**Python** (`grpcio`):

```python
import grpc

with open("/var/run/konfig-client-tls/ca.crt", "rb") as f:
    root_ca = f.read()
with open("/var/run/konfig-client-tls/tls.crt", "rb") as f:
    client_cert = f.read()
with open("/var/run/konfig-client-tls/tls.key", "rb") as f:
    client_key = f.read()

creds = grpc.ssl_channel_credentials(
    root_certificates=root_ca,
    private_key=client_key,
    certificate_chain=client_cert,
)
channel = grpc.secure_channel(
    "konfig.konfig-system.svc.cluster.local:50051",
    creds,
    # Required when CN != target host. Match the server cert's SAN.
    options=[("grpc.ssl_target_name_override",
              "konfig.konfig-system.svc.cluster.local")],
)
```

**Rust** (`tonic`):

```rust
use tonic::transport::{Certificate, ClientTlsConfig, Channel, Identity};

let ca = std::fs::read("/var/run/konfig-client-tls/ca.crt")?;
let cert = std::fs::read("/var/run/konfig-client-tls/tls.crt")?;
let key = std::fs::read("/var/run/konfig-client-tls/tls.key")?;

let tls = ClientTlsConfig::new()
    .ca_certificate(Certificate::from_pem(ca))
    .identity(Identity::from_pem(cert, key))
    .domain_name("konfig.konfig-system.svc.cluster.local");

let channel = Channel::from_static(
    "https://konfig.konfig-system.svc.cluster.local:50051"
)
.tls_config(tls)?
.connect()
.await?;
```

### Disabling TLS (local dev only)

For local-only testing the konfig binary accepts `--tls=false`. The
deployment manifests never pass this flag — production is always mTLS-on.

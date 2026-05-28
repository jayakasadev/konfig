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

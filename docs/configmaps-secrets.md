# Using existing ConfigMaps and Secrets

konfig watches K8s-native ConfigMaps and Secrets alongside Config CRDs. This is
the zero-friction path for teams that already have config stored there.

## Opt-in label

Both ConfigMaps and Secrets require the label:

```
konfig.io/managed: "true"
```

konfig ignores any resource without it.

```bash
kubectl label configmap my-config konfig.io/managed=true -n default
kubectl label secret my-secret konfig.io/managed=true -n production
```

## ConfigMap data format

**Structured** — if `data["content"]` exists, it is parsed as JSON or YAML:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
  namespace: default
  labels:
    konfig.io/managed: "true"
  annotations:
    konfig.io/schema-version: "3"   # optional monotonicity
data:
  content: |
    { "rate_limit": 100, "feature_flags": { "dark_mode": true } }
```

**Flat** — if no `content` key, all key-value pairs become a flat JSON object.
The key `schema_version` is extracted separately:

```yaml
data:
  schema_version: "2"
  rate_limit: "100"
  dark_mode: "true"
```

## Secret format

Values are stored **base64-encoded on the wire** — the server never decodes them:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: api-creds
  namespace: production
  labels:
    konfig.io/managed: "true"
  annotations:
    konfig.io/schema-version: "1"
type: Opaque
stringData:
  api_key: "sk-live-abc123"
  api_secret: "supersecret"
```

Decode in your consumer:

```python
secret = client.get_secret("production", "api-creds")
# secret.data is dict[str, bytes] — already decoded
api_key = secret.data["api_key"].decode()
```

## Enable in Helm

```bash
# ConfigMaps across all namespaces
helm upgrade konfig ./chart --set konfig.watchConfigMaps=true

# Secrets in specific namespaces
helm upgrade konfig ./chart \
  --set-string "konfig.secretNamespaces=production,staging"

# Both
helm upgrade konfig ./chart \
  --set konfig.watchConfigMaps=true \
  --set-string "konfig.secretNamespaces=production,staging"
```

> Config CRD and ConfigMap caches are unified. Secrets use a separate cache.
> Use `Get`/`GetAll` for configs and ConfigMaps; `GetSecret`/`GetAllSecrets` for Secrets.

## Migrate a ConfigMap to a Config CRD

Once you want schema_version enforcement and the Apply RPC write path:

```bash
# Import a ConfigMap as a Config CRD
konfig-cli import configmap --namespace default --name app-config

# Dry-run first
konfig-cli import configmap --namespace default --name app-config --dry-run
```

After importing, remove the `konfig.io/managed` label from the original
ConfigMap — the Config CRD becomes the source of truth.

# konfig-cli reference

`konfig-cli` talks directly to the Kubernetes API — it does not require the
konfig server to be running. Install from
[GitHub releases](https://github.com/jayakasadev/konfig/releases).

## Commands

### apply

Create or update a Config CRD. Enforces `schema_version` monotonicity.

```bash
konfig-cli apply default app-config config.yaml
```

`config.yaml`:
```yaml
schema_version: 3
content:
  rate_limit: 100
  feature_flags:
    dark_mode: true
```

### get

Print a Config CRD spec as YAML.

```bash
konfig-cli get default app-config
```

### get-secret

Print a managed Secret. Values are redacted by default.

```bash
konfig-cli get-secret production api-creds
# api_key: [REDACTED]

konfig-cli get-secret production api-creds --reveal
# api_key: sk-live-abc123
```

### apply-secret

Patch a managed Secret from a YAML file. Server base64-encodes values before patching.

```bash
konfig-cli apply-secret production api-creds creds.yaml
```

`creds.yaml`:
```yaml
schema_version: 2
api_key: sk-live-newkey
api_secret: newsecret
```

### import configmap

Onboard an existing ConfigMap as a Config CRD.

```bash
# Dry-run
konfig-cli import configmap default app-config --dry-run

# Apply
konfig-cli import configmap default app-config
```

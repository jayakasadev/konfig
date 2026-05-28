# Configuration reference

## Core

| Parameter | Default | Description |
|-----------|---------|-------------|
| `replicaCount` | `1` | Replicas. Use 2+ for HA (requires `pdb.enabled=true`). |
| `image.repository` | `kasa288/konfig` | Image repository. |
| `image.tag` | `""` (chart appVersion) | Image tag. |

## konfig settings

| Parameter | Default | Description |
|-----------|---------|-------------|
| `konfig.watchNamespace` | `default` | Namespace of the seed Config CRD the server watches at startup. Used only for the readiness gate — does not restrict which configs subscribers can request. |
| `konfig.watchName` | `app-config` | Name of the seed Config CRD. Create this resource before deploying so the pod becomes ready. |
| `konfig.watchConfigMaps` | `false` | Enable ConfigMap watching. Requires label `konfig.io/managed=true` on each ConfigMap. |
| `konfig.secretNamespaces` | `""` | Comma-separated list of namespaces to watch for Secrets. Requires label `konfig.io/managed=true`. |

## RBAC

| Parameter | Default | Description |
|-----------|---------|-------------|
| `rbac.createConfigRole` | `true` | ClusterRole for `configs.konfig.io` (get/list/watch/patch/create). |
| `rbac.createConfigMapRole` | `true` | ClusterRole for ConfigMap access. Only relevant when `konfig.watchConfigMaps=true`. |
| `rbac.createSecretRole` | `true` | Role + RoleBinding per namespace for Secret access. Only created when `konfig.secretNamespaces` is set. |

## High availability

| Parameter | Default | Description |
|-----------|---------|-------------|
| `pdb.enabled` | `true` | Create a PodDisruptionBudget. |
| `pdb.maxUnavailable` | `0` | Zero downtime during node drains. Set to `1` if `replicaCount=1`. |

## Resources

```yaml
resources:
  requests:
    cpu: 50m
    memory: 64Mi
  limits:
    cpu: 200m
    memory: 256Mi
```

At 100 concurrent subscribers and 10 Apply/min, measured usage is ~30m CPU / 40Mi.

## Telemetry

```yaml
telemetry:
  dial9:
    enabled: false              # set DIAL9_ENABLED=true
    traceDir: /tmp/dial9-traces # mount a PVC for persistence
    maxDiskUsageMb: 512
    rotationSecs: 60

  tokioConsole:
    enabled: false   # development only; exposes port 4242
    port: 4242
```

Run `dial9 serve --local-dir /tmp/dial9-traces` to view nanosecond-level async traces.
Connect `tokio-console` to `:4242` for live task inspection.

## ArgoCD

```yaml
argocd:
  createApplication: false
  namespace: argocd
  project: default
  repoURL: ""
  targetRevision: HEAD
  automated:
    enabled: false
    prune: false
    selfHeal: false
```

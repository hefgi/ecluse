# k3d

A per-slot Kubernetes dev cluster using [k3d](https://k3d.io).

Each ecluse session creates a dedicated k3s-in-Docker cluster named `ecluse-<slug>` with a load balancer bound to the slot's assigned `PORT`. This lets you run fully isolated Kubernetes environments simultaneously on one machine.

## Mode

`host` — k3d manages its own Docker containers; ecluse only runs the hooks.

## Prerequisites

```sh
brew install k3d helm helmfile
```

## Environment variables set by ecluse

| Variable      | Description                                        |
|---------------|----------------------------------------------------|
| `ECLUSE_SLUG` | Session slug — used as the cluster name suffix     |
| `ECLUSE_SLOT` | Slot number                                        |
| `PORT`        | Host port bound to the cluster's load balancer (`:80`) |

## Hooks

- `on_up`: `k3d cluster create ecluse-$ECLUSE_SLUG --port "$PORT:80@loadbalancer"` — provisions a fresh k3s cluster.
- `on_down`: `k3d cluster delete ecluse-$ECLUSE_SLUG` — destroys the cluster and all its resources.

## Usage

```sh
ecluse init
ecluse up my-feature       # creates the k3d cluster
ecluse shell my-feature    # opens a shell with KUBECONFIG set to the cluster

# Inside the session shell — deploy your app
kubectl get nodes
helmfile apply             # see helmfile.yaml for the release config

# Your app is reachable at http://localhost:$PORT

ecluse down my-feature     # deletes the cluster
```

## helmfile.yaml

The included `helmfile.yaml` shows how to reference `ECLUSE_SLUG` and `PORT` in your Helm values. Adapt the chart path and values to your application.

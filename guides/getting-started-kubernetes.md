# Getting started on Kubernetes

Deploy llama-scale on Kubernetes with a ConfigMap (or Secret) for config, the
official multi-arch image from GitHub Container Registry, and standard
liveness / readiness probes.

## Prerequisites

- A Kubernetes cluster (1.24+) and `kubectl` configured against it
- Permission to create Deployments, Services, ConfigMaps, and Secrets
- At least one OpenAI-compatible backend reachable from the cluster network
  (Ollama, llama.cpp, vLLM, LM Studio, etc.)

Prebuilt images support **linux/amd64** and **linux/arm64**:

- **Image:** [`ghcr.io/tokenring-ai/llama-scale`](https://github.com/tokenring-ai/llama-scale/pkgs/container/llama-scale)

The runtime image is **distroless** (no shell or `curl` inside the container).
Probe HTTP endpoints from the orchestrator, not with an exec healthcheck.

## 1. Namespace and config

```bash
kubectl create namespace llama-scale
```

Create a local `config.yaml` (or download the example):

```bash
curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
```

Edit backends so URLs resolve **from inside the cluster**:

| Where the backend runs | Typical `url` host |
|------------------------|--------------------|
| Same cluster (Service) | `http://ollama.default.svc.cluster.local:11434/v1` |
| Same namespace Service | `http://ollama:11434/v1` |
| Node / host network | Node IP, or a headless / ExternalName Service |
| Outside the cluster | That host’s DNS name or IP (firewall must allow pods) |

Example config for a Service named `ollama` in the same namespace:

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys:
    - "${ROUTER_API_KEY}"

log:
  destination: "console"

backends:
  - name: ollama
    url: "http://ollama:11434/v1"
    api_key: "ollama"
    health_path: "/health"
```

Load it as a ConfigMap:

```bash
kubectl -n llama-scale create configmap llama-scale-config \
  --from-file=config.yaml=./config.yaml
```

Store the client API key (and any backend secrets) in a Secret:

```bash
kubectl -n llama-scale create secret generic llama-scale-secrets \
  --from-literal=ROUTER_API_KEY='sk-change-me' \
  --from-literal=OPENAI_API_KEY=''   # optional; omit if unused
```

Missing `${VAR}` references are a hard startup error — every variable used in
the config must be present in the pod environment.

## 2. Deployment and Service

Save as `llama-scale.yaml`:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: llama-scale
  namespace: llama-scale
  labels:
    app: llama-scale
spec:
  replicas: 2
  selector:
    matchLabels:
      app: llama-scale
  template:
    metadata:
      labels:
        app: llama-scale
    spec:
      securityContext:
        runAsNonRoot: true
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: llama-scale
          image: ghcr.io/tokenring-ai/llama-scale:v1.0.4
          imagePullPolicy: IfNotPresent
          args:
            - --config
            - /etc/llama-scale/config.yaml
          ports:
            - name: http
              containerPort: 8080
          env:
            - name: ROUTER_API_KEY
              valueFrom:
                secretKeyRef:
                  name: llama-scale-secrets
                  key: ROUTER_API_KEY
            # Uncomment when the config references ${OPENAI_API_KEY}
            # - name: OPENAI_API_KEY
            #   valueFrom:
            #     secretKeyRef:
            #       name: llama-scale-secrets
            #       key: OPENAI_API_KEY
          volumeMounts:
            - name: config
              mountPath: /etc/llama-scale/config.yaml
              subPath: config.yaml
              readOnly: true
          securityContext:
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
            readOnlyRootFilesystem: true
          resources:
            requests:
              cpu: 50m
              memory: 64Mi
            limits:
              cpu: "1"
              memory: 256Mi
          livenessProbe:
            httpGet:
              path: /healthz
              port: http
            initialDelaySeconds: 5
            periodSeconds: 10
          readinessProbe:
            httpGet:
              path: /readyz
              port: http
            initialDelaySeconds: 5
            periodSeconds: 5
      volumes:
        - name: config
          configMap:
            name: llama-scale-config
---
apiVersion: v1
kind: Service
metadata:
  name: llama-scale
  namespace: llama-scale
  labels:
    app: llama-scale
spec:
  type: ClusterIP
  selector:
    app: llama-scale
  ports:
    - name: http
      port: 8080
      targetPort: http
```

Pin the image tag (`v1.0.4` above) instead of `latest` for reproducible deploys.
Check [GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases) /
[GHCR](https://github.com/tokenring-ai/llama-scale/pkgs/container/llama-scale)
for current tags.

Apply:

```bash
kubectl apply -f llama-scale.yaml
kubectl -n llama-scale rollout status deployment/llama-scale
```

| Probe | Meaning |
|-------|---------|
| `GET /healthz` | Process is up (liveness) |
| `GET /readyz` | At least one backend is healthy (`200`) or not yet (`503`) |

Use `/readyz` for readiness when you only want traffic after backends are
verified. Use `/healthz` alone if the router should stay Ready while backends
are temporarily down (clients still get errors on inference).

## 3. Verify

Port-forward and call the API:

```bash
kubectl -n llama-scale port-forward svc/llama-scale 8080:8080
```

In another terminal:

```bash
curl -s http://127.0.0.1:8080/healthz
curl -s http://127.0.0.1:8080/readyz

curl -s http://127.0.0.1:8080/v1/models \
  -H "Authorization: Bearer sk-change-me"

curl -s http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-change-me" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "llama3.2",
        "messages": [{"role": "user", "content": "Say hi."}]
      }'
```

Logs:

```bash
kubectl -n llama-scale logs -f deploy/llama-scale
```

## 4. Expose outside the cluster

### NodePort or LoadBalancer

```yaml
spec:
  type: LoadBalancer   # or NodePort
  # ...
```

### Ingress (TLS at the edge)

Example with a generic Ingress controller (adjust class and host):

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: llama-scale
  namespace: llama-scale
  annotations:
    # Optional: large/streaming bodies — tune for your controller
    nginx.ingress.kubernetes.io/proxy-read-timeout: "3600"
    nginx.ingress.kubernetes.io/proxy-send-timeout: "3600"
    nginx.ingress.kubernetes.io/proxy-buffering: "off"
spec:
  ingressClassName: nginx
  tls:
    - hosts:
        - llm.example.com
      secretName: llama-scale-tls
  rules:
    - host: llm.example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: llama-scale
                port:
                  number: 8080
```

Streaming chat completions need long proxy timeouts and buffering disabled
where the controller supports it. Clients should still send
`Authorization: Bearer <key>` unless you terminate auth elsewhere.

llama-scale can also terminate TLS itself via `server.tls` in the config (see
[main README — TLS](../README.md#tls-terminating-https-directly)); mounting
cert/key as a Secret is the usual pattern.

## 5. Config and secret updates

After editing the ConfigMap:

```bash
kubectl -n llama-scale create configmap llama-scale-config \
  --from-file=config.yaml=./config.yaml \
  --dry-run=client -o yaml | kubectl apply -f -

# Config is read at process start — restart pods to pick it up
kubectl -n llama-scale rollout restart deployment/llama-scale
```

Update the API key:

```bash
kubectl -n llama-scale create secret generic llama-scale-secrets \
  --from-literal=ROUTER_API_KEY='sk-new-key' \
  --dry-run=client -o yaml | kubectl apply -f -
kubectl -n llama-scale rollout restart deployment/llama-scale
```

## 6. Scaling and session affinity

llama-scale keeps multi-turn session→backend affinity in **process memory**.
With multiple replicas:

- New requests load-balance across pods; each pod has its own session map
- Sticky sessions at the Service (ClientIP) or Ingress can reduce cross-pod
  drift, but do not share affinity state between replicas
- For a single homelab cluster, `replicas: 1` is often enough; scale out when
  you need HA for the router itself and can tolerate independent affinity tables

Session TTL defaults (`session_ttl_secs`, `session_max_entries`) are documented
in the [main README](../README.md#configuration-reference).

## 7. Metrics

`GET /metrics` exposes Prometheus metrics. Optionally guard it with
`server.admin_token` in the config and scrape with a bearer token. Example
ServiceMonitor (Prometheus Operator):

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: llama-scale
  namespace: llama-scale
spec:
  selector:
    matchLabels:
      app: llama-scale
  endpoints:
    - port: http
      path: /metrics
      interval: 30s
```

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| CrashLoopBackOff | `kubectl logs` — missing `${VAR}`, bad YAML, or config path |
| Ready never true (`/readyz` 503) | Backend Service DNS / NetworkPolicy / wrong `url` from the pod |
| Liveness fails, readiness ok | Unlikely with default probes; check listen port matches Service |
| Works via port-forward, fails via Ingress | Timeouts, buffering, or TLS/path on the controller |
| Permission errors on config | Image runs as non-root (`65532`); ConfigMap mounts are fine as read-only |
| Image pull errors | Public GHCR; pin a real tag; check node arch (amd64/arm64 only) |

Debug backend reachability from a throwaway pod:

```bash
kubectl -n llama-scale run curl --rm -it --restart=Never \
  --image=curlimages/curl -- \
  curl -sS http://ollama:11434/health
```

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup: [README — Setting up backends](../README.md#setting-up-backends)
- Docker / Compose (same image): [Getting started with Docker](getting-started-docker.md)
- Bare-metal packages: [Debian/Ubuntu](getting-started-debian-ubuntu.md) · [Red Hat/Fedora](getting-started-redhat-fedora.md)
- Homelab hypervisor: [Proxmox](getting-started-proxmox.md)

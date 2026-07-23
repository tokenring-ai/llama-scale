# Getting started with Docker

Run llama-scale from the official multi-arch image on GitHub Container Registry
(GHCR), or build the image yourself from this repository.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) 20.10+ (Docker Desktop on Mac/Windows, or Engine on Linux)
- A `config.yaml` that points at your backends
- Optional: Docker Compose

Prebuilt images support **linux/amd64** and **linux/arm64**.

## 1. Create a config file

```bash
curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
```

Edit `config.yaml`. For a first run against Ollama (or another server) on the
**Docker host**, use `host.docker.internal` instead of `127.0.0.1` so the
container can reach host ports:

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys:
    - "sk-router-dev-key-change-me"

log:
  destination: "console"

backends:
  - name: ollama
    url: "http://host.docker.internal:11434/v1"
    api_key: "ollama"
    health_path: "/health"
```

| Where the backend runs | Typical `url` host |
|------------------------|--------------------|
| Same Docker Compose network | Service name, e.g. `http://ollama:11434/v1` |
| Docker host (Mac / Windows) | `host.docker.internal` |
| Docker host (Linux) | `host.docker.internal` with `--add-host` (below), or host network mode |
| Another machine | That machine’s hostname or IP |

Secrets can use `${ENV_VAR}` and be passed with `-e` / Compose `environment`.

## 2. Run the official image

```bash
docker run -d --name llama-scale \
  --restart unless-stopped \
  -p 8080:8080 \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  ghcr.io/tokenring-ai/llama-scale:latest
```

Pin a version instead of `latest` when you want reproducible deploys:

```bash
ghcr.io/tokenring-ai/llama-scale:v1.0.4
```

### Reach backends on the Linux host

On Linux, map `host.docker.internal` to the host gateway:

```bash
docker run -d --name llama-scale \
  --restart unless-stopped \
  -p 8080:8080 \
  --add-host=host.docker.internal:host-gateway \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  ghcr.io/tokenring-ai/llama-scale:latest
```

Alternatively, use host networking (Linux only; ports and `localhost` match the host):

```bash
docker run -d --name llama-scale \
  --restart unless-stopped \
  --network host \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  ghcr.io/tokenring-ai/llama-scale:latest
```

With `--network host`, set backend URLs to `http://127.0.0.1:<port>/v1` as you
would for a bare-metal install. `server.listen` still controls the bind address
inside the host network namespace (e.g. `0.0.0.0:8080`).

### Pass environment variables

```bash
docker run -d --name llama-scale \
  -p 8080:8080 \
  -e ROUTER_API_KEY="sk-your-real-key" \
  -e OPENAI_API_KEY \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  ghcr.io/tokenring-ai/llama-scale:latest
```

Reference them in config as `${ROUTER_API_KEY}`, `${OPENAI_API_KEY}`, etc.
Missing variables cause a hard startup failure.

## 3. Docker Compose example

`docker-compose.yaml`:

```yaml
services:
  llama-scale:
    image: ghcr.io/tokenring-ai/llama-scale:latest
    ports:
      - "8080:8080"
    volumes:
      - ./config.yaml:/etc/llama-scale/config.yaml:ro
    environment:
      ROUTER_API_KEY: ${ROUTER_API_KEY:-sk-router-dev-key-change-me}
    extra_hosts:
      - "host.docker.internal:host-gateway"
    restart: unless-stopped
```

> **Note:** The published image is **distroless** (no shell or `curl`). Probe
> `/healthz` and `/readyz` from the host or your orchestrator instead of an
> in-container healthcheck:
>
> ```bash
> curl -sf http://localhost:8080/readyz
> ```

Start:

```bash
docker compose up -d
docker compose logs -f llama-scale
```

### Compose with Ollama on the same network

```yaml
services:
  ollama:
    image: ollama/ollama:latest
    volumes:
      - ollama_data:/root/.ollama
    # optional GPU: see Ollama Docker docs for device reservations

  llama-scale:
    image: ghcr.io/tokenring-ai/llama-scale:latest
    ports:
      - "8080:8080"
    volumes:
      - ./config.yaml:/etc/llama-scale/config.yaml:ro
    depends_on:
      - ollama
    restart: unless-stopped

volumes:
  ollama_data:
```

In `config.yaml` for this layout:

```yaml
backends:
  - name: ollama
    url: "http://ollama:11434/v1"
    api_key: "ollama"
    health_path: "/health"
```

## 4. Verify

```bash
# From the host
curl -s http://localhost:8080/healthz
curl -s http://localhost:8080/readyz

curl -s http://localhost:8080/v1/models \
  -H "Authorization: Bearer sk-router-dev-key-change-me"

curl -s http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-router-dev-key-change-me" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "llama3.2",
        "messages": [{"role": "user", "content": "Say hi."}]
      }'
```

| Probe | Meaning |
|-------|---------|
| `GET /healthz` | Process is up |
| `GET /readyz` | At least one backend is healthy (`200`) or not yet (`503`) |

Use `/readyz` for load balancer / k8s readiness when you want traffic only after
backends are verified.

## 5. Logs and lifecycle

```bash
docker logs -f llama-scale
docker restart llama-scale
docker stop llama-scale && docker rm llama-scale
```

After editing `config.yaml` on the host, restart the container so the new file
is loaded (the mount is read at process start).

## Build the image yourself

From a clone of this repo:

```bash
docker build -t llama-scale:local .
docker run --rm -p 8080:8080 \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  llama-scale:local
```

Multi-arch with buildx:

```bash
docker buildx build --platform linux/amd64,linux/arm64 -t llama-scale:local .
```

The image entrypoint is:

```text
/usr/local/bin/llama-scale --config /etc/llama-scale/config.yaml
```

Override the config path if needed:

```bash
docker run ... ghcr.io/tokenring-ai/llama-scale:latest \
  --config /etc/llama-scale/config.yaml
```

## Kubernetes sketch

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: llama-scale
spec:
  replicas: 1
  selector:
    matchLabels:
      app: llama-scale
  template:
    metadata:
      labels:
        app: llama-scale
    spec:
      containers:
        - name: llama-scale
          image: ghcr.io/tokenring-ai/llama-scale:v1.0.4
          ports:
            - containerPort: 8080
          volumeMounts:
            - name: config
              mountPath: /etc/llama-scale/config.yaml
              subPath: config.yaml
              readOnly: true
          livenessProbe:
            httpGet:
              path: /healthz
              port: 8080
          readinessProbe:
            httpGet:
              path: /readyz
              port: 8080
      volumes:
        - name: config
          configMap:
            name: llama-scale-config
```

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| Container exits immediately | `docker logs llama-scale` — usually missing mount, bad YAML, or unresolved `${VAR}` |
| `/readyz` is 503 | Backends unreachable from the container — fix `url` (`host.docker.internal` vs service name) |
| Works on Mac, fails on Linux host backends | Add `--add-host=host.docker.internal:host-gateway` |
| Connection refused to published port | Confirm `-p 8080:8080` and `server.listen` is `0.0.0.0:8080` (not only `127.0.0.1`) |
| Permission errors on config mount | Mount is read-only as non-root (`nonroot`); ensure the file is world-readable (`chmod 644 config.yaml`) |

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup: [README — Setting up backends](../README.md#setting-up-backends)
- Bare-metal Linux packages: [Debian/Ubuntu](getting-started-debian-ubuntu.md) · [Red Hat/Fedora](getting-started-redhat-fedora.md)
- macOS without Docker: [macOS guide](getting-started-macos.md)

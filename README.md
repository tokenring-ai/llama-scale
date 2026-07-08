# llama-scale

<img src="assets/logo.webp" alt="TokenRing Logo" style="max-width: 350px; margin: 0 auto; background: white">

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/language-Rust-orange.svg?logo=rust)](https://www.rust-lang.org/)

A simple, Rust-based **OpenAI-compatible LLM router** for locally hosted inference
servers. Point your apps at one endpoint and llama-scale forwards each request to
Ollama, llama.cpp, vLLM, LM Studio, or any combination of them — load-balancing
across replicas and keeping multi-turn conversations pinned to the same backend
so context stays warm.

## Quick start

### Option 1: npx (no install)

Requires [Node.js](https://nodejs.org/) 18+. Downloads the correct prebuilt binary
for your platform on first run.

```bash
curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
# edit config.yaml — see "Sample configuration" below

npx llama-scale --config config.yaml
```

Install globally if you prefer:

```bash
npm install -g llama-scale
llama-scale --config config.yaml
```

The config path can also be set with `MODEL_ROUTER_CONFIG`.

### Option 2: Docker

Prebuilt multi-arch images are published to GitHub Container Registry on every
release:

```bash
curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
# edit config.yaml

docker run -d --name llama-scale \
  -p 8080:8080 \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  ghcr.io/tokenring-ai/llama-scale:latest
```

Backends running on the Docker host (Ollama, LM Studio, etc.) are usually reached
at `http://host.docker.internal:<port>/v1` on macOS and Windows. On Linux, add
`--add-host=host.docker.internal:host-gateway` or use `network_mode: host`.

### Option 3: .deb or .rpm (Linux packages)

Download the package for your architecture from the
[GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases) page:

| Package | Architectures |
|---------|---------------|
| `.deb`  | `amd64`, `arm64` |
| `.rpm`  | `x86_64`, `aarch64` |

**Debian / Ubuntu:**

```bash
# pick the file that matches your CPU (example: amd64)
sudo dpkg -i llama-scale_<version>_amd64.deb

sudo nano /etc/llama-scale/config.yaml   # configure backends
sudo systemctl enable --now llama-scale
sudo systemctl status llama-scale
```

**Fedora / RHEL / openSUSE:**

```bash
sudo rpm -i llama-scale_<version>_x86_64.rpm

sudo nano /etc/llama-scale/config.yaml
sudo systemctl enable --now llama-scale
```

The package installs a `llama-scale` systemd service, creates a `llama-scale`
system user, and places the default config at `/etc/llama-scale/config.yaml`.
Optional environment variables for `${...}` expansion can be set in
`/etc/default/llama-scale`.

Tarballs for macOS and Windows are also available on the Releases page if you
prefer a standalone binary.

### Try it

Once llama-scale is listening (default `0.0.0.0:8080`), point any OpenAI client
at `http://localhost:8080/v1`:

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-router-dev-key-change-me" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "llama3.2",
        "messages": [
          {"role": "system", "content": "You are helpful."},
          {"role": "user", "content": "Say hi."}
        ]
      }'
```

List merged models from all healthy backends:

```bash
curl http://localhost:8080/v1/models \
  -H "Authorization: Bearer sk-router-dev-key-change-me"
```

## Sample configuration

The example below routes across four common local inference servers. Adjust host
names, ports, and model aliases to match your setup.

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys:
    - "sk-router-dev-key-change-me"

log:
  destination: "console"   # or "file"
  # file: "/var/log/llama-scale/access.log"
  # level: "info"          # used when RUST_LOG is unset

models_refresh_interval_secs: 30
health_check_interval_secs: 15
health_check_timeout_secs: 5
session_ttl_secs: 3600
session_max_entries: 100000

backends:
  # Ollama — OpenAI-compatible API on port 11434
  - name: ollama
    url: "http://127.0.0.1:11434/v1"
    api_key: "ollama"          # Ollama ignores this; any non-empty value works
    health_path: "/health"     # Ollama root health endpoint

  # llama.cpp server — `llama-server --port 8081` (avoid clashing with llama-scale)
  - name: llama-cpp
    url: "http://127.0.0.1:8081/v1"
    api_key: "local"
    health_path: "/health"

  # vLLM — `vllm serve <model> --port 8000`
  - name: vllm
    url: "http://127.0.0.1:8000/v1"
    api_key: "local"
    # health_path defaults to /v1/models (works for vLLM)

  # LM Studio — enable the local server in the Developer tab (default port 1234)
  - name: lmstudio
    url: "http://127.0.0.1:1234/v1"
    api_key: "lm-studio"       # set to match LM Studio's API key if configured
```

A fully commented reference copy lives in [`config.example.yaml`](config.example.yaml).

## Configuration reference

### `server`

| Field | Description |
|-------|-------------|
| `listen` | Socket address to bind, e.g. `"0.0.0.0:8080"` or `"127.0.0.1:11435"`. |
| `api_keys` | Bearer tokens clients must send as `Authorization: Bearer <key>`. Leave empty to disable authentication (open access). |

### `log`

| Field | Description |
|-------|-------------|
| `destination` | `"console"` (stderr, default) or `"file"`. |
| `file` | Path appended to when `destination` is `"file"`. Required for file logging. |
| `level` | Fallback log level (`info`, `debug`, …) when the `RUST_LOG` environment variable is not set. |

HTTP access logs (method, path, status, routed backend, latency) use the same
destination.

### Router tuning

| Field | Default | Description |
|-------|---------|-------------|
| `models_refresh_interval_secs` | `30` | How often the merged `/v1/models` list is rebuilt from all backends. |
| `health_check_interval_secs` | `15` | How often each backend is probed. Unhealthy backends are skipped until they recover. |
| `health_check_timeout_secs` | `5` | Timeout for a single health or model-list probe. |
| `session_ttl_secs` | `3600` | How long a conversation stays pinned to one backend. |
| `session_max_entries` | `100000` | Maximum number of session bindings kept in memory. |

### `backends` (list)

Each entry describes one upstream OpenAI-compatible server.

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `name` | yes | — | Unique label used in logs and the merged `/models` `owned_by` field. |
| `url` | yes | — | Base URL of the upstream API, including the `/v1` prefix. Must be `http` or `https`. Trailing slashes are stripped. |
| `api_key` | no | `""` | Sent to the upstream as `Authorization: Bearer <api_key>`. |
| `timeout_secs` | no | `120` | Per-request upstream timeout in seconds. |
| `health_path` | no | `"/v1/models"` | Host-root path used for health checks. Replaces the path of `url` — see [Health checks](#health-checks). |
| `model_aliases` | no | `[]` | Optional `alias` → `real` name map. When set, only alias names are exposed and routable; requests are rewritten to `real` before forwarding. When omitted, every model reported by the upstream `/models` endpoint is exposed under its original id. |

You can list the same backend multiple times (different `name` values) to add more
capacity for the same models — the router load-balances between healthy replicas.

## Setting up backends

llama-scale speaks the OpenAI HTTP API (`/v1/chat/completions`, `/v1/embeddings`,
`/v1/models`, streaming SSE, etc.). Each backend must expose that API locally.

### Ollama

```bash
ollama serve          # listens on :11434 by default
ollama pull llama3.2
```

- **URL:** `http://127.0.0.1:11434/v1`
- **API key:** any placeholder (e.g. `"ollama"`)
- **Health:** `health_path: "/health"`

### llama.cpp (`llama-server`)

```bash
llama-server -m /path/to/model.gguf --port 8081
```

- **URL:** `http://127.0.0.1:8081/v1` (pick a port that does not conflict with llama-scale)
- **API key:** any value if you enabled `--api-key`; otherwise a placeholder
- **Health:** `health_path: "/health"`

### vLLM

```bash
vllm serve meta-llama/Llama-3.2-3B-Instruct --port 8000
```

- **URL:** `http://127.0.0.1:8000/v1`
- **API key:** set `--api-key` on vLLM and use the same value here, or use a placeholder when auth is off
- **Health:** default `/v1/models` works; no `health_path` override needed

### LM Studio

Enable **Local Server** in the Developer tab (default port `1234`).

- **URL:** `http://127.0.0.1:1234/v1`
- **API key:** match the key configured in LM Studio, or any placeholder if auth is disabled
- **Health:** default `/v1/models` works

### Model aliases

Use aliases when you want a stable client-facing name or when several backends
should share a single model id:

```yaml
backends:
  - name: ollama-fast
    url: "http://127.0.0.1:11434/v1"
    api_key: "ollama"
    health_path: "/health"
    model_aliases:
      - { alias: "llama3.2", real: "llama3.2:latest" }

  - name: vllm-fast
    url: "http://127.0.0.1:8000/v1"
    api_key: "local"
    model_aliases:
      - { alias: "llama3.2", real: "meta-llama/Llama-3.2-3B-Instruct" }
```

Both backends advertise `llama3.2`; the router load-balances new sessions between
them while keeping each conversation on one backend.

### Health checks

`health_path` is resolved against the **host root**, not relative to `/v1`:

| `url` | `health_path` | Probe target |
|-------|---------------|--------------|
| `http://host:11434/v1` | `/health` | `http://host:11434/health` |
| `http://host:8000/v1` | *(default)* `/v1/models` | `http://host:8000/v1/models` |

Ollama and llama.cpp expose a root `/health` endpoint — set `health_path`
accordingly. vLLM and LM Studio work with the default `/v1/models`.

## Environment variable substitution

Any string value in the config can reference process environment variables with
`${VAR_NAME}` syntax:

```yaml
server:
  api_keys:
    - "${ROUTER_API_KEY}"

backends:
  - name: vllm
    url: "http://127.0.0.1:8000/v1"
    api_key: "${VLLM_API_KEY}"
```

Rules:

- Expansion runs at startup on **string values only** (YAML comments are ignored).
- Missing variables are a **hard error** — llama-scale refuses to start rather than
  routing with an empty secret.
- `${VAR}` must be properly closed; unterminated placeholders are rejected.
- With the Debian/RPM packages, put variables in `/etc/default/llama-scale` so
  systemd exports them before launch.

## How routing works

1. **Session affinity** — repeated turns of one conversation stick to the backend
   that already serves them. The session id is
   `sha256(api_key + model + first_message)`; the first message (typically the
   system prompt) identifies a conversation without client changes.
2. **Least connections** — for a brand-new session, the healthy backend with the
   fewest in-flight requests wins.

Additional rules:

- The router reads `model` from the JSON body. Requests without a `model` field
  are rejected with HTTP 400 (except `GET /v1/models`, served from cache).
- A backend serves a model if it is a configured alias, or (with no aliases) the
  model appears in its upstream `/models` listing.
- If a backend connection fails, the router retries the next healthy candidate.
  Non-2xx HTTP responses from an upstream are passed through unchanged.

## API endpoints

| Path | Auth | Description |
|------|------|-------------|
| `GET /v1/models` | yes | Merged, deduplicated model list |
| `GET /models` | yes | Alias of `/v1/models` |
| `* /v1/*` | yes | Proxied to a chosen backend (streaming supported) |
| `GET /healthz` | no | Liveness probe |
| `GET /` | no | Service info |

## Features

- OpenAI-compatible passthrough for any `/v1/*` path
- Conversation-sticky routing with no client changes
- Least-connections balancing across replicas of the same model
- Merged `/models` endpoint with background refresh
- Per-backend model aliases
- Periodic health checking with automatic failover
- Bearer API key authentication
- `${ENV_VAR}` secret expansion in config
- Structured HTTP access logging to console or file

## Development

Build and run from source with [Rust](https://rust-lang.org/tools/install):

```bash
cp config.example.yaml config.yaml
cargo run -- --config config.yaml
```

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

MIT License — see [LICENSE](LICENSE).
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

Platform-specific walkthroughs (packages, systemd, Docker, Kubernetes, Proxmox, macOS):

| Platform         | Guide                                                                              |
|------------------|------------------------------------------------------------------------------------|
| Debian / Ubuntu  | [guides/getting-started-debian-ubuntu.md](guides/getting-started-debian-ubuntu.md) |
| Red Hat / Fedora | [guides/getting-started-redhat-fedora.md](guides/getting-started-redhat-fedora.md) |
| macOS            | [guides/getting-started-macos.md](guides/getting-started-macos.md)                 |
| Docker           | [guides/getting-started-docker.md](guides/getting-started-docker.md)               |
| Kubernetes       | [guides/getting-started-kubernetes.md](guides/getting-started-kubernetes.md)       |
| Proxmox VE       | [guides/getting-started-proxmox.md](guides/getting-started-proxmox.md)             |
| MikroTik         | [guides/getting-started-mikrotik.md](guides/getting-started-mikrotik.md)           |

### Option 1: install script (macOS / Linux)

One-liner that installs a pinned release (uses npm/bun when available, otherwise
downloads the prebuilt binary into `~/.local/bin`):

```bash
curl -fsSL https://github.com/tokenring-ai/llama-scale/releases/latest/download/install.sh | bash
```

Then create a config and run:

```bash
curl -fsSL https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml -o config.yaml
# edit config.yaml — see "Sample configuration" below
llama-scale --config config.yaml
```

Override the version pin for testing:

```bash
curl -fsSL https://github.com/tokenring-ai/llama-scale/releases/latest/download/install.sh \
  | LLAMA_SCALE_INSTALL_VERSION=1.0.4 bash
```

### Option 2: npx (no install)

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

### Option 3: Prebuilt binary (GitHub Releases)

Download a release tarball for your platform from the
[GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases) page.
Each archive contains the `llama-scale` binary (or `llama-scale.exe` on Windows),
`config.example.yaml`, and this README.

| Platform            | Tarball suffix                     |
|---------------------|------------------------------------|
| Linux x86_64        | `x86_64-unknown-linux-gnu.tar.gz`  |
| Linux arm64         | `aarch64-unknown-linux-gnu.tar.gz` |
| macOS Intel         | `x86_64-apple-darwin.tar.gz`       |
| macOS Apple Silicon | `aarch64-apple-darwin.tar.gz`      |
| Windows x86_64      | `x86_64-pc-windows-msvc.tar.gz`    |
| Windows arm64       | `aarch64-pc-windows-msvc.tar.gz`   |

Replace `<version>` with a release tag such as `v1.0.2`:

```bash
curl -LO https://github.com/tokenring-ai/llama-scale/releases/download/<version>/llama-scale-<version>-<target>.tar.gz
tar xzf llama-scale-<version>-<target>.tar.gz
cp config.example.yaml config.yaml
# edit config.yaml — see "Sample configuration" below

./llama-scale --config config.yaml
```

Example for macOS Apple Silicon:

```bash
curl -LO https://github.com/tokenring-ai/llama-scale/releases/download/v1.0.2/llama-scale-v1.0.2-aarch64-apple-darwin.tar.gz
tar xzf llama-scale-v1.0.2-aarch64-apple-darwin.tar.gz
./llama-scale --config config.example.yaml
```

Move the binary onto your `PATH` (e.g. `sudo mv llama-scale /usr/local/bin/`) if you
want it available system-wide.

### Option 4: Install from source with Cargo

Requires the [Rust toolchain](https://rust-lang.org/tools/install) (stable).

**Clone the repository and install locally:**

```bash
git clone https://github.com/tokenring-ai/llama-scale.git
cd llama-scale
cargo install --path .
```

**Or install directly from GitHub without cloning:**

```bash
cargo install --git https://github.com/tokenring-ai/llama-scale
```

Then configure and run:

```bash
cp config.example.yaml config.yaml
# edit config.yaml — see "Sample configuration" below

llama-scale --config config.yaml
```

`cargo install` places the binary in `~/.cargo/bin` — ensure that directory is on
your `PATH`.

To run the latest development code without installing:

```bash
git clone https://github.com/tokenring-ai/llama-scale.git
cd llama-scale
cp config.example.yaml config.yaml
cargo run --release -- --config config.yaml
```

### Option 5: Docker

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

### Option 6: .deb or .rpm (Linux packages)

Download the package for your architecture from the
[GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases) page:

| Package | Architectures       |
|---------|---------------------|
| `.deb`  | `amd64`, `arm64`    |
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

Scrape Prometheus metrics (no auth required):

```bash
curl http://localhost:8080/metrics
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
    max_connections: 2         # optional; cap concurrent in-flight requests (0 = unlimited)

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
    # timeout_secs: 120              # max wait for response headers
    # stream_idle_timeout_secs: 120  # max silence between stream chunks (0 = off)
    # stream_timeout_secs: 0         # max total stream body time (0 = unlimited)

  # LM Studio — enable the local server in the Developer tab (default port 1234)
  - name: lmstudio
    url: "http://127.0.0.1:1234/v1"
    api_key: "lm-studio"       # set to match LM Studio's API key if configured
    fallback: 1                # optional; try only when all fallback: 0 hosts are unavailable
```

A fully commented reference copy lives in [`config.example.yaml`](config.example.yaml).

## Configuration reference

### `server`

| Field         | Description                                                                                                                                                                                            |
|---------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `listen`      | Socket address to bind, e.g. `"0.0.0.0:8080"` or `"127.0.0.1:11435"`.                                                                                                                                  |
| `api_keys`    | Bearer tokens clients must send as `Authorization: Bearer <key>`. Leave empty to disable authentication (open access). Accepts either a flat list of keys, or a map for multi-user setups — see below. |
| `tls`         | Optional. Terminates TLS directly on the listen socket — see below. Omit to serve plain HTTP (e.g. behind a TLS-terminating reverse proxy).                                                            |
| `admin_token` | Optional. Bearer token guarding privileged endpoints (`/metrics`) — see below. Omit to leave them open.                                                                                                |

#### `api_keys`: single vs. multi-user

The simple form is a flat list; every key is equivalent, can call any model,
and has no concurrency cap:

```yaml
server:
  api_keys:
    - "sk-router-dev-key-change-me"
```

For multi-user setups, use a map from key to per-key settings instead:

```yaml
server:
  api_keys:
    sk-alice-key:
      id: alice               # non-secret label for logs (defaults to a
      # masked key prefix if omitted)
      allowed_models: # models this key may request; omit/empty = all
        - "gpt-4"
        - "llama-3*"          # trailing "*" matches by prefix
      concurrent_requests: 2  # max in-flight requests for this key; 0/omit = unlimited
    sk-bob-key:
      id: bob                 # no allowed_models/concurrent_requests -> unrestricted
```

A request for a model outside `allowed_models` gets `403 model_not_allowed`;
one that would exceed `concurrent_requests` gets `429 rate_limit_error`. Each
key's `id` (never the raw key) is recorded in the access log and, if
restricted, filters what that key sees from `/v1/models`.

#### `tls`: terminating HTTPS directly

By default llama-scale serves plain HTTP. To terminate TLS on the listen
socket itself (instead of putting a reverse proxy in front), set `server.tls`
to a PEM certificate (or full chain) and private key:

```yaml
server:
  listen: "0.0.0.0:8443"
  tls:
    cert_path: "/etc/llama-scale/tls/cert.pem"
    key_path: "/etc/llama-scale/tls/key.pem"
```

Both paths are validated at startup — llama-scale refuses to start if either
file is missing. Clients then connect with `https://`. `listen` is unaffected
by `tls`; pick whatever port you want (`8443` above is just a convention).

#### `admin_token`: protecting `/metrics`

`/metrics` (the Prometheus scrape endpoint) is unauthenticated by default,
which leaks active backends, connection counts, and traffic rates to anyone
who can reach the port. Set `server.admin_token` to require a bearer token on
it:

```yaml
server:
  admin_token: "${LLAMA_SCALE_ADMIN_TOKEN}"
```

```
curl -H "Authorization: Bearer $LLAMA_SCALE_ADMIN_TOKEN" http://localhost:8080/metrics
```

`admin_token` is independent of `api_keys` — it is not subject to model
allowlists or concurrency caps, and `api_keys` cannot be used to authenticate
to `/metrics` (and vice versa). Like other secrets in this config, reference
it via `${ENV_VAR}` rather than committing it in plaintext. `/healthz` and
`/readyz` remain unauthenticated regardless, since orchestrators (k8s,
Docker) need to probe them without credentials.

### `log`

| Field         | Description                                                                                  |
|---------------|----------------------------------------------------------------------------------------------|
| `destination` | `"console"` (stderr, default) or `"file"`.                                                   |
| `file`        | Path appended to when `destination` is `"file"`. Required for file logging.                  |
| `level`       | Fallback log level (`info`, `debug`, …) when the `RUST_LOG` environment variable is not set. |

HTTP access logs (method, path, status, routed backend, latency) use the same
destination.

### Router tuning

| Field                          | Default  | Description                                                                          |
|--------------------------------|----------|--------------------------------------------------------------------------------------|
| `models_refresh_interval_secs` | `30`     | How often the merged `/v1/models` list is rebuilt from all backends.                 |
| `health_check_interval_secs`   | `15`     | How often each backend is probed. Unhealthy backends are skipped until they recover. |
| `health_check_timeout_secs`    | `5`      | Timeout for a single health or model-list probe.                                     |
| `session_ttl_secs`             | `3600`   | How long a conversation stays pinned to one backend.                                 |
| `session_max_entries`          | `100000` | Maximum number of session bindings kept in memory.                                   |

### `backends` (list)

Each entry describes one upstream OpenAI-compatible server.

| Field                      | Required | Default        | Description                                                                                                                                                                                                                                          |
|----------------------------|----------|----------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `name`                     | yes      | —              | Unique label used in logs and the merged `/models` `owned_by` field.                                                                                                                                                                                 |
| `url`                      | yes      | —              | Base URL of the upstream API, including the `/v1` prefix. Must be `http` or `https`. Trailing slashes are stripped.                                                                                                                                  |
| `api_key`                  | no       | `""`           | Sent to the upstream as `Authorization: Bearer <api_key>`.                                                                                                                                                                                           |
| `timeout_secs`             | no       | `120`          | Max wait for **response headers** from the upstream (seconds). Does **not** bound the full stream body — see stream timeouts below.                                                                                                                  |
| `stream_idle_timeout_secs` | no       | `120`          | Max silence between body chunks while streaming (seconds). `0` disables the idle timeout.                                                                                                                                                            |
| `stream_timeout_secs`      | no       | `0`            | Max total time for the response body after headers (seconds). `0` means unlimited so long generations are not cut off.                                                                                                                               |
| `max_connections`          | no       | `0`            | Max concurrent in-flight proxied requests to this backend. `0` means unlimited. Saturated backends are skipped; load spills to the next candidate.                                                                                                   |
| `health_path`              | no       | `"/v1/models"` | Host-root path used for health checks. Replaces the path of `url` — see [Health checks](#health-checks).                                                                                                                                             |
| `fallback`                 | no       | `0`            | Routing priority for new (unpinned) requests. Lower values are preferred; higher tiers are tried only when no healthy backend exists at a lower tier. See [How routing works](#how-routing-works).                                                   |
| `model_aliases`            | no       | `[]`           | Optional `alias` → `real` name map. When set, only alias names are exposed and routable; requests are rewritten to `real` before forwarding. When omitted, every model reported by the upstream `/models` endpoint is exposed under its original id. |

You can list the same backend multiple times (different `name` values) to add more
capacity for the same models — the router load-balances between healthy,
non-saturated replicas at the same `fallback` tier.

#### Timeouts

Each proxied request uses three independent bounds:

1. **Header timeout** (`timeout_secs`) — time until the upstream returns HTTP
   status and headers (including first-token latency for many servers).
2. **Stream idle timeout** (`stream_idle_timeout_secs`) — maximum gap between
   consecutive body chunks once streaming has started. Protects against stalled
   generations.
3. **Stream total timeout** (`stream_timeout_secs`) — optional absolute cap on
   body duration after headers. Leave at `0` for long completions.

Connect timeout is fixed at 5 seconds on the shared HTTP client. Health probes
and model-list fetches use `health_check_timeout_secs` and `timeout_secs`
respectively (see router tuning above).

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

### Primary and fallback backends

Use `fallback` to express preference order across hosts that serve the same model.
Backends with `fallback: 0` are tried first; `fallback: 1` is used only when every
lower tier is unhealthy or fails to connect:

```yaml
backends:
  - name: ollama-primary
    url: "http://127.0.0.1:11434/v1"
    api_key: "ollama"
    health_path: "/health"
    fallback: 0

  - name: vllm-backup
    url: "http://127.0.0.1:8000/v1"
    api_key: "local"
    fallback: 1
```

Pinned conversations still return to their affinity backend even if it has a higher
`fallback` value.

### Health checks

`health_path` is resolved against the **host root**, not relative to `/v1`:

| `url`                  | `health_path`            | Probe target                 |
|------------------------|--------------------------|------------------------------|
| `http://host:11434/v1` | `/health`                | `http://host:11434/health`   |
| `http://host:8000/v1`  | *(default)* `/v1/models` | `http://host:8000/v1/models` |

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
   system prompt) identifies a conversation without client changes. A pinned
   backend is tried first when it is still healthy and not saturated, regardless
   of its `fallback` tier. Affinity is recorded only after a **2xx** upstream
   response — 4xx/5xx do not pin the session.
2. **Fallback tiers** — for a brand-new session, healthy backends with the lowest
   `fallback` value are considered first. Higher tiers are only used when every
   backend at a lower tier is unhealthy, saturated, or fails to connect during
   the request.
3. **Least connections** — within the same `fallback` tier, the healthy backend
   with the fewest in-flight requests wins.
4. **Concurrency caps** — when `max_connections` is set, backends at capacity are
   skipped. A slot is reserved atomically before the upstream call so concurrent
   requests cannot oversubscribe a backend.

Additional rules:

- The router reads `model` from the JSON body. Requests without a `model` field
  are rejected with HTTP 400 (except `GET /v1/models`, served from cache).
- A backend serves a model if it is a configured alias, or (with no aliases) the
  model appears in its upstream `/models` listing.
- If a backend connection fails, times out (header or stream), or is at
  `max_connections`, the router retries the next healthy candidate.
  Non-2xx HTTP responses from an upstream are passed through unchanged (no
  failover on application errors).

### Startup and readiness

On boot, backends start as **unhealthy** with an empty model cache. llama-scale
runs one health-check pass and one models refresh **before** listening for
traffic, then continues both on the configured intervals.

| Probe     | Path           | Meaning                                                             |
|-----------|----------------|---------------------------------------------------------------------|
| Liveness  | `GET /healthz` | Process is up (always `ok` when reachable).                         |
| Readiness | `GET /readyz`  | At least one backend is currently healthy (`200`); otherwise `503`. |

Use `/readyz` (not `/healthz`) in Kubernetes or Docker healthchecks when you want
to wait until upstreams are verified.

## API endpoints

| Path             | Auth | Description                                       |
|------------------|------|---------------------------------------------------|
| `GET /v1/models` | yes  | Merged, deduplicated model list                   |
| `GET /models`    | yes  | Alias of `/v1/models`                             |
| `* /v1/*`        | yes  | Proxied to a chosen backend (streaming supported) |
| `GET /healthz`   | no   | Liveness probe                                    |
| `GET /readyz`    | no   | Readiness probe (any healthy backend)             |
| `GET /metrics`   | no   | Prometheus metrics scrape endpoint                |
| `GET /`          | no   | Service info                                      |

## Prometheus metrics

`GET /metrics` exposes Prometheus text format and does not require authentication.
Scrape it from Prometheus, Grafana Agent, or any compatible collector.

| Metric                                    | Type      | Description                                                                      |
|-------------------------------------------|-----------|----------------------------------------------------------------------------------|
| `llama_scale_active_connections{backend}` | Gauge     | In-flight proxied requests per backend                                           |
| `llama_scale_requests_total{outcome}`     | Counter   | Requests by outcome: `success` (2xx), `auth_failure` (401), `server_error` (5xx) |
| `llama_scale_request_duration_seconds`    | Histogram | End-to-end HTTP request duration                                                 |
| `llama_scale_time_to_first_byte_seconds`  | Histogram | Time from upstream request start until the first response byte                   |
| `llama_scale_tokens_generated_total`      | Counter   | Completion tokens observed in proxied LLM responses                              |
| `llama_scale_tokens_per_second_avg`       | Gauge     | Exponential moving average of completion tokens per second                       |

Token counts prefer `usage.completion_tokens` from upstream JSON/SSE when present;
otherwise they are estimated from streaming `delta.content` chunks.

Example PromQL:

```promql
sum(llama_scale_active_connections)
rate(llama_scale_requests_total[5m])
histogram_quantile(0.95, rate(llama_scale_request_duration_seconds_bucket[5m]))
rate(llama_scale_tokens_generated_total[5m])
```

## Features

- OpenAI-compatible passthrough for any `/v1/*` path
- Conversation-sticky routing with no client changes (pin on 2xx only)
- Fallback-tier routing with least-connections balancing within each tier
- Per-backend `max_connections` concurrency caps with failover when saturated
- Stream-aware timeouts (header / idle / optional total)
- Merged `/models` endpoint with background refresh
- Per-backend model aliases
- Periodic health checking with automatic failover
- Startup health + models pass before listen; `/healthz` and `/readyz`
- Prometheus metrics at `/metrics`
- Bearer API key authentication
- `${ENV_VAR}` secret expansion in config
- Structured HTTP access logging to console or file

## Development

Contributing or hacking on the router? Clone the repo (see
[Option 4: Install from source with Cargo](#option-4-install-from-source-with-cargo))
and use:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- --config config.yaml
```

## License

MIT License — see [LICENSE](LICENSE).
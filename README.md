# llama-scale

<img src="assets/logo.webp" alt="TokenRing Logo" style="max-width: 350px; margin: 0 auto; background: white">

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/language-Rust-orange.svg?logo=rust)](https://www.rust-lang.org/)

A simple, Rust-based **OpenAI-compatible LLM router**. It presents a single
OpenAI-style endpoint to your clients and forwards each request to one of several
configured backends, choosing the backend with:

1. **Session affinity** – repeated turns of one conversation stick to the backend
   that already serves them, so the backend keeps the request context warm.
2. **Least connections** – for a brand-new session, the backend with the fewest
   in-flight requests wins.

Multiple backends may advertise the same model; the router load-balances between
them automatically.

## Features

- **OpenAI-compatible passthrough** for any `/v1/*` path (chat completions with
  SSE streaming, embeddings, moderations, etc.).
- **Conversation-sticky routing** with no client changes required. The session id
  is `sha256(api_key + model + first_message)`; the first message (typically the
  system prompt) identifies a conversation.
- **Least-connections** balancing for the first request of each session.
- **Merged `/models`** endpoint aggregating all backends, refreshed in the
  background (default every 30s).
- **Model aliases** per backend. If a backend defines `model_aliases`, only those
  alias names are exposed and the request's `model` is rewritten to the upstream
  "real" name on the wire. If **no** aliases are configured, the backend's models
  are served under their original names.
- **Health checking** – unhealthy backends are probed periodically and skipped.
- **Bearer API keys** – OpenAI-style `Authorization: Bearer <key>` entitlement.
- **`${ENV_VAR}` expansion** in the config so secrets stay out of the file.
- **HTTP access logging** to console or file (configurable), with one
  structured line per request (method, path, status, routed backend, latency).

## Getting started

```bash
cp config.example.yaml config.yaml
export OPENAI_API_KEY="sk-..."
cargo run -- --config config.yaml
```

Then point any OpenAI client at `http://localhost:8080/v1`:

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-router-dev-key-change-me" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "gpt-4o",
        "messages": [
          {"role": "system", "content": "You are helpful."},
          {"role": "user", "content": "Say hi."}
        ]
      }'
```

## Configuration

See [`config.example.yaml`](config.example.yaml) for a fully commented example.

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys: ["sk-router-dev-key-change-me"]

# Logging: "console" (stderr) or "file". Honors RUST_LOG; falls back to log.level.
log:
  destination: "console"
  # file: "/var/log/llama-scale/access.log"
  # level: "info"

models_refresh_interval_secs: 30   # /models merge cadence
health_check_interval_secs: 15
health_check_timeout_secs: 5
session_ttl_secs: 3600
session_max_entries: 100000

backends:
  - name: openai
    url: "https://api.openai.com/v1"
    api_key: "${OPENAI_API_KEY}"
    model_aliases:
      - { alias: "gpt-4o", real: "gpt-4o-2024-08-06" }

  - name: ollama
    url: "http://localhost:11434/v1"
    api_key: "ollama"
    health_path: "/health"   # host-root absolute; defaults to /v1/models
    # no model_aliases -> all upstream models exposed under original names
```

### Routing rules

- The router reads `model` from the JSON request body to pick a backend. A
  request without a JSON body / `model` field is rejected with HTTP 400 (except
  `GET /v1/models`, which is served from cache).
- A backend "serves" a model if either the model is one of its configured
  aliases, or (when it has no aliases) the model appears in its upstream
  `/models` listing.
- If a backend connection fails, the router transparently retries the next
  healthy candidate for that model. Non-2xx HTTP responses from an upstream are
  passed through unchanged (they are not retried).

### Endpoints

| Path             | Auth | Description                              |
|------------------|------|------------------------------------------|
| `GET /v1/models` | yes  | Merged, deduplicated model list          |
| `GET /models`    | yes  | Alias of `/v1/models`                    |
| `*   /v1/*`      | yes  | Proxied to a chosen backend (streaming)  |
| `GET /healthz`   | no   | Liveness probe                           |
| `GET /`          | no   | Service info                             |

## Development

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- --config config.yaml
```

## License

MIT License - see LICENSE file for details.


# Getting started on Debian / Ubuntu

Install llama-scale from a prebuilt `.deb`, configure one or more OpenAI-compatible
backends, and run it under systemd.

## Prerequisites

- Debian 11+ or Ubuntu 20.04+ (amd64 or arm64)
- A local or remote OpenAI-compatible inference server (Ollama, llama.cpp, vLLM, LM Studio, etc.)
- Root (or `sudo`) for package install and service management

## 1. Download the package

Grab the `.deb` that matches your CPU from
[GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases):

| Architecture | Package name pattern |
|--------------|----------------------|
| x86_64 / amd64 | `llama-scale_<version>_amd64.deb` |
| arm64 / aarch64 | `llama-scale_<version>_arm64.deb` |

Example (replace `<version>` with a release tag such as `1.0.4`):

```bash
# amd64
curl -LO "https://github.com/tokenring-ai/llama-scale/releases/download/v<version>/llama-scale_<version>_amd64.deb"

# arm64
# curl -LO "https://github.com/tokenring-ai/llama-scale/releases/download/v<version>/llama-scale_<version>_arm64.deb"
```

Check your architecture with:

```bash
dpkg --print-architecture
```

## 2. Install

```bash
sudo dpkg -i llama-scale_<version>_amd64.deb
```

If `dpkg` reports missing dependencies:

```bash
sudo apt-get install -f
```

The package installs:

| Path | Purpose |
|------|---------|
| `/usr/bin/llama-scale` | Binary |
| `/lib/systemd/system/llama-scale.service` | systemd unit |
| `/etc/llama-scale/config.yaml.default` | Default config template |
| `llama-scale` system user | Service account |
| `/var/log/llama-scale/` | Log directory |

## 3. Create and edit the config

The service expects `/etc/llama-scale/config.yaml`. Copy the packaged template:

```bash
sudo cp /etc/llama-scale/config.yaml.default /etc/llama-scale/config.yaml
sudo nano /etc/llama-scale/config.yaml
```

The packaged default listens on **`127.0.0.1:11435`** (loopback only) and points
at a local Ollama instance. Change the API key and backends for your setup:

```yaml
server:
  listen: "127.0.0.1:11435"
  api_keys:
    - "sk-change-me-after-install"   # change this

backends:
  - name: ollama
    url: "http://127.0.0.1:11434/v1"
    api_key: "ollama"
    health_path: "/health"
```

To accept connections from other hosts, set `listen` to `"0.0.0.0:8080"` (or
another port) and restrict access with a firewall or reverse proxy.

### Secrets via environment variables

Reference secrets as `${VAR_NAME}` in the config. Export them for the service in
`/etc/default/llama-scale`:

```bash
sudo tee /etc/default/llama-scale >/dev/null <<'EOF'
ROUTER_API_KEY=sk-your-real-key
# OPENAI_API_KEY=sk-...
EOF
sudo chmod 600 /etc/default/llama-scale
```

```yaml
server:
  api_keys:
    - "${ROUTER_API_KEY}"
```

Missing variables are a hard startup error — llama-scale will not start with an
empty secret.

## 4. Start the service

```bash
sudo systemctl enable --now llama-scale
sudo systemctl status llama-scale
```

After config changes:

```bash
sudo systemctl restart llama-scale
```

Logs (when using the packaged file destination):

```bash
sudo journalctl -u llama-scale -f
# and/or
sudo tail -f /var/log/llama-scale/app.log
```

## 5. Verify

```bash
# Liveness
curl -s http://127.0.0.1:11435/healthz

# List models (use the key from your config)
curl -s http://127.0.0.1:11435/v1/models \
  -H "Authorization: Bearer sk-change-me-after-install"

# Chat completion
curl -s http://127.0.0.1:11435/v1/chat/completions \
  -H "Authorization: Bearer sk-change-me-after-install" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "llama3.2",
        "messages": [{"role": "user", "content": "Say hi."}]
      }'
```

Point any OpenAI-compatible client at `http://127.0.0.1:11435/v1` (or whatever
`server.listen` you configured).

## Alternative installs

If you prefer not to use the `.deb`:

**npx (no permanent install):**

```bash
curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
# edit config.yaml
npx llama-scale --config config.yaml
```

**Prebuilt tarball:**

```bash
curl -LO https://github.com/tokenring-ai/llama-scale/releases/download/v<version>/llama-scale-v<version>-x86_64-unknown-linux-gnu.tar.gz
tar xzf llama-scale-v<version>-x86_64-unknown-linux-gnu.tar.gz
./llama-scale --config config.example.yaml
```

**From source:**

```bash
# requires Rust stable: https://rust-lang.org/tools/install
cargo install --git https://github.com/tokenring-ai/llama-scale
```

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| `systemctl status` shows inactive / failed | `journalctl -u llama-scale -e` — often a missing `/etc/llama-scale/config.yaml` or unresolved `${VAR}` |
| `ConditionPathExists` failure | Copy `config.yaml.default` → `config.yaml` (see step 3) |
| `/readyz` returns 503 | No healthy backends yet — start Ollama/vLLM/etc. and confirm `url` / `health_path` |
| Connection refused | Confirm `listen` address/port and that the service is active |
| 401 from clients | `Authorization: Bearer` must match a key in `server.api_keys` |

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup (Ollama, llama.cpp, vLLM, LM Studio): [README — Setting up backends](../README.md#setting-up-backends)
- Routing, affinity, and metrics: [README](../README.md)

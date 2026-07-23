# Getting started on Red Hat / Fedora

Install llama-scale from a prebuilt `.rpm`, configure backends, and run it under
systemd. These steps also apply to RHEL, CentOS Stream, Rocky Linux, AlmaLinux,
and openSUSE (with `rpm` / `zypper` as appropriate).

## Prerequisites

- Fedora, RHEL 8+, Rocky/Alma 8+, or a compatible RPM-based distro (x86_64 or aarch64)
- A local or remote OpenAI-compatible inference server (Ollama, llama.cpp, vLLM, LM Studio, etc.)
- Root (or `sudo`) for package install and service management

## 1. Download the package

Grab the `.rpm` that matches your CPU from
[GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases):

| Architecture | Package name pattern |
|--------------|----------------------|
| x86_64 | `llama-scale_<version>_x86_64.rpm` |
| aarch64 | `llama-scale_<version>_aarch64.rpm` |

Example (replace `<version>` with a release tag such as `1.0.4`):

```bash
# x86_64
curl -LO "https://github.com/tokenring-ai/llama-scale/releases/download/v<version>/llama-scale_<version>_x86_64.rpm"

# aarch64
# curl -LO "https://github.com/tokenring-ai/llama-scale/releases/download/v<version>/llama-scale_<version>_aarch64.rpm"
```

Check your architecture with:

```bash
uname -m
```

## 2. Install

**Fedora / RHEL / Rocky / Alma:**

```bash
sudo dnf install -y ./llama-scale_<version>_x86_64.rpm
# or, if you prefer raw rpm:
# sudo rpm -i llama-scale_<version>_x86_64.rpm
```

**openSUSE:**

```bash
sudo zypper install -y ./llama-scale_<version>_x86_64.rpm
```

The package installs:

| Path | Purpose |
|------|---------|
| `/usr/bin/llama-scale` | Binary |
| `/usr/lib/systemd/system/llama-scale.service` | systemd unit |
| `/etc/llama-scale/config.yaml.default` | Default config template |
| `llama-scale` system user | Service account |
| `/var/log/llama-scale/` | Log directory |

The unit is enabled at install time but will not stay running until a valid
`config.yaml` exists (see below).

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
another port) and restrict access with firewalld or a reverse proxy:

```bash
# example: open port 8080 with firewalld
sudo firewall-cmd --permanent --add-port=8080/tcp
sudo firewall-cmd --reload
```

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
sudo systemctl daemon-reload
sudo systemctl enable --now llama-scale
sudo systemctl status llama-scale
```

After config changes:

```bash
sudo systemctl restart llama-scale
```

Logs:

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

## SELinux notes

On enforcing SELinux systems, if the service fails to bind or write logs after a
custom config change, check the audit log:

```bash
sudo ausearch -m avc -ts recent
```

The default layout (`/usr/bin/llama-scale`, `/etc/llama-scale`,
`/var/log/llama-scale`) is intended to work with standard confinement. Prefer
keeping the binary and config under those paths rather than relocating them.

## Alternative installs

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
| Connection refused | Confirm `listen` address/port; check firewalld if binding non-loopback |
| 401 from clients | `Authorization: Bearer` must match a key in `server.api_keys` |

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup (Ollama, llama.cpp, vLLM, LM Studio): [README — Setting up backends](../README.md#setting-up-backends)
- Routing, affinity, and metrics: [README](../README.md)

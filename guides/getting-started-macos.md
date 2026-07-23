# Getting started on macOS

Run llama-scale on Intel or Apple Silicon Macs. The fastest path is a prebuilt
binary or `npx`; use Cargo if you want to build from source.

## Prerequisites

- macOS 12+ (Intel or Apple Silicon)
- A local OpenAI-compatible inference server (Ollama, LM Studio, llama.cpp, etc.)
- Optional: [Node.js](https://nodejs.org/) 18+ for the npm / `npx` install
- Optional: [Rust](https://rust-lang.org/tools/install) stable for building from source

## Choose an install method

| Method | When to use |
|--------|-------------|
| **Prebuilt binary** | Simple standalone install from GitHub Releases |
| **npx / npm** | No permanent install, or you already use Node |
| **Cargo** | Developing llama-scale or wanting the latest git tip |

---

## Option A: Prebuilt binary (recommended)

1. Download the archive for your chip from
   [GitHub Releases](https://github.com/tokenring-ai/llama-scale/releases):

| Mac | Tarball suffix |
|-----|----------------|
| Apple Silicon (M1/M2/M3/M4) | `aarch64-apple-darwin.tar.gz` |
| Intel | `x86_64-apple-darwin.tar.gz` |

Check your chip:

```bash
uname -m
# arm64  -> Apple Silicon
# x86_64 -> Intel
```

2. Extract and run (replace `<version>` with a release tag such as `v1.0.4`):

```bash
# Apple Silicon example
curl -LO "https://github.com/tokenring-ai/llama-scale/releases/download/<version>/llama-scale-<version>-aarch64-apple-darwin.tar.gz"
tar xzf llama-scale-<version>-aarch64-apple-darwin.tar.gz

cp config.example.yaml config.yaml
# edit config.yaml — see "Configure" below

./llama-scale --config config.yaml
```

3. Optional: put the binary on your `PATH`:

```bash
sudo mv llama-scale /usr/local/bin/
# Apple Silicon Homebrew users often prefer:
# sudo mv llama-scale /opt/homebrew/bin/
```

---

## Option B: npx / npm

No permanent install — downloads the correct binary on first run:

```bash
curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
# edit config.yaml

npx llama-scale --config config.yaml
```

Or install the CLI globally:

```bash
npm install -g llama-scale
llama-scale --config config.yaml
```

The config path can also be set with `MODEL_ROUTER_CONFIG`.

---

## Option C: Cargo (from source)

```bash
# Install Rust if needed: https://rust-lang.org/tools/install
cargo install --git https://github.com/tokenring-ai/llama-scale
# Ensure ~/.cargo/bin is on your PATH (rustup usually configures this)

curl -O https://raw.githubusercontent.com/tokenring-ai/llama-scale/main/config.example.yaml
mv config.example.yaml config.yaml
# edit config.yaml

llama-scale --config config.yaml
```

Or clone and run without installing:

```bash
git clone https://github.com/tokenring-ai/llama-scale.git
cd llama-scale
cp config.example.yaml config.yaml
# edit config.yaml
cargo run --release -- --config config.yaml
```

---

## Configure

Minimal config for a local Ollama backend (common on Mac):

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys:
    - "sk-router-dev-key-change-me"

log:
  destination: "console"

backends:
  - name: ollama
    url: "http://127.0.0.1:11434/v1"
    api_key: "ollama"
    health_path: "/health"
```

**LM Studio** (enable Local Server in the Developer tab, default port `1234`):

```yaml
backends:
  - name: lmstudio
    url: "http://127.0.0.1:1234/v1"
    api_key: "lm-studio"
```

Secrets can use environment substitution:

```bash
export ROUTER_API_KEY="sk-your-real-key"
```

```yaml
server:
  api_keys:
    - "${ROUTER_API_KEY}"
```

A fully commented example lives in
[`config.example.yaml`](../config.example.yaml).

## Run in the foreground

```bash
llama-scale --config config.yaml
```

You should see a log line that the server is listening (default `0.0.0.0:8080`).

## Run in the background

Unix daemon mode (`-D` / `--daemon`) is supported. Prefer file logging so output
is not discarded after detach:

```yaml
log:
  destination: "file"
  file: "/tmp/llama-scale.log"
  level: "info"
```

```bash
llama-scale --config config.yaml --daemon
tail -f /tmp/llama-scale.log
```

For a login-item style service, create a LaunchAgent (optional). Example
`~/Library/LaunchAgents/ai.tokenring.llama-scale.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>ai.tokenring.llama-scale</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/llama-scale</string>
    <string>--config</string>
    <string>/Users/YOU/.config/llama-scale/config.yaml</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/tmp/llama-scale.out.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/llama-scale.err.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/ai.tokenring.llama-scale.plist
```

Adjust the binary path (`/opt/homebrew/bin/llama-scale` on many Apple Silicon
machines) and config path to match your install.

## Verify

```bash
curl -s http://localhost:8080/healthz

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

Point clients at `http://localhost:8080/v1`.

## Gatekeeper / “unidentified developer”

If macOS blocks the downloaded binary the first time:

1. System Settings → Privacy & Security → allow the blocked app, **or**
2. Remove the quarantine attribute after you trust the release artifact:

```bash
xattr -d com.apple.quarantine ./llama-scale
```

Only do this for binaries you obtained from the official GitHub Releases page.

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| `command not found: llama-scale` | Binary not on `PATH`; use `./llama-scale` or move it to `/usr/local/bin` or `/opt/homebrew/bin` |
| `/readyz` returns 503 | Backend not running — start Ollama (`ollama serve`) or LM Studio local server |
| Connection refused to Ollama | Confirm Ollama is on `11434`; try `curl http://127.0.0.1:11434/v1/models` |
| 401 from clients | Bearer token must match `server.api_keys` |
| Wrong binary arch | `uname -m` must match the release tarball (`arm64` vs `x86_64`) |

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup: [README — Setting up backends](../README.md#setting-up-backends)
- Docker on Mac: [Getting started with Docker](getting-started-docker.md)

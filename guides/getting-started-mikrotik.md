# Getting started on MikroTik (RouterOS containers)

Run llama-scale as a RouterOS container so one endpoint on your router can
load-balance OpenAI-compatible traffic across local or remote inference
backends (Ollama, llama.cpp, vLLM, LM Studio, and similar).

This guide follows the same shape as MikroTik’s official
[Container - mosquitto MQTT server](https://manual.mikrotik.com/docs/containers/user-guides/container-mosquitto-mqtt-server)
walkthrough.

## Introduction

RouterOS [containers](https://manual.mikrotik.com/docs/containers/) let you run
lightweight services on the router instead of a separate always-on host.
llama-scale is a good fit: it is a small Rust binary in a distroless image, and
it only needs a YAML config plus network reachability to your backends.

The image used here is published to GitHub Container Registry:

- **Image:** [`ghcr.io/tokenring-ai/llama-scale`](https://github.com/tokenring-ai/llama-scale/pkgs/container/llama-scale)
- **Architectures:** `linux/amd64`, `linux/arm64` (multi-arch)

## Summary

Study MikroTik’s [container](https://manual.mikrotik.com/docs/containers/) guide
before proceeding. Read the
[disclaimer](https://manual.mikrotik.com/docs/containers/#disclaimer) and
[requirements](https://manual.mikrotik.com/docs/containers/#requirements)
sections so you understand device-mode changes, storage needs, and security
trade-offs.

At the time of writing, the published llama-scale image targets **ARM64** and
**AMD64** (CHR / x86). It will **not** run on 32-bit ARM RouterBOARD models.
Confirm your device architecture before pulling the image.

> **Warning — keep this basic setup off the open internet without hardening.**
>
> - Use the [firewall](https://manual.mikrotik.com/docs/firewall-and-quality-of-service/firewall/)
>   so only trusted IPs can reach the container.
> - Change the default API key in `config.yaml` (and prefer `${ENV}` secrets).
> - Prefer TLS termination (RouterOS, reverse proxy, or llama-scale
>   [`server.tls`](../README.md#tls-terminating-https-directly)) for anything
>   beyond a lab LAN.
> - Mount the config **read-only** and avoid `privileged=yes`.

You will need:

- RouterOS with the **container** package installed
- Enough storage for image layers (USB / disk is strongly recommended)
- At least one reachable OpenAI-compatible backend

## Container configuration

**Sub-menu:** `/container`

*Note:* The **container** package is required.

### Container mode

Enable container mode:

```ros
/system/device-mode/update container=yes
```

Confirm the device-mode change with a press of the reset button, or a cold
reboot when using containers on x86 / CHR. See MikroTik’s container docs for
the exact confirmation flow on your platform.

### Networking

Add a veth interface for the container. This example uses a dedicated
`192.168.15.0/24` segment (adjust to fit your addressing plan):

```ros
/interface/veth/add name=veth1 address=192.168.15.7/24 gateway=192.168.15.1
```

Create a bridge for containers, give the router an address on that network, and
attach the veth:

```ros
/interface/bridge/add name=containers
/ip/address/add address=192.168.15.1/24 interface=containers
/interface/bridge/port/add bridge=containers interface=veth1
```

If the container must reach backends on your LAN (or the internet), either:

- NAT masquerade traffic from the container network out through your WAN/LAN, or
- Bridge/route the container network into an existing LAN as appropriate for
  your topology.

Example masquerade (when the container network is not already routed):

```ros
/ip/firewall/nat/add chain=srcnat src-address=192.168.15.0/24 \
  action=masquerade comment="llama-scale container egress"
```

#### Optional: publish the API on the router LAN IP

Forward a port on the router to the container (here the LAN IP is
`192.168.88.1` and the service listens on `11434` inside the container — pick
any free port, but keep NAT and `server.listen` in sync):

```ros
/ip/firewall/nat/add action=dst-nat chain=dstnat \
  dst-address=192.168.88.1 dst-port=11434 protocol=tcp \
  to-addresses=192.168.15.7 to-ports=11434 \
  comment="llama-scale LAN"
```

#### Optional: publish from WAN

Only do this if you understand the exposure. Restrict sources with firewall
filter rules; do not rely on NAT alone.

```ros
/ip/firewall/nat/add action=dst-nat chain=dstnat \
  in-interface-list=WAN dst-port=11434 protocol=tcp \
  to-addresses=192.168.15.7 to-ports=11434 \
  comment="llama-scale WAN"
```

### Mounts

llama-scale reads a single YAML file at
`/etc/llama-scale/config.yaml` (the image `CMD` default). Mount a host path to
that location.

Using a mount list (recommended; matches MikroTik’s mosquitto guide style):

```ros
/container/mounts/add list=llama-scale-config \
  src=/usb1/llama-scale/config \
  dst=/etc/llama-scale
```

You will place `config.yaml` inside `/usb1/llama-scale/config/` on the router so
the container sees `/etc/llama-scale/config.yaml`.

Inline mount on the container (also valid):

```text
mount=/usb1/llama-scale/config.yaml:/etc/llama-scale/config.yaml:ro
```

### Storage layout

Keep image layers and the container root on external storage when available
(USB / NVMe / disk). Example:

```text
/usb1/llama-scale/layer   → layer-dir
/usb1/llama-scale/root    → root-dir
/usb1/llama-scale/config  → mounted config directory
```

Create the directories from WinBox **Files**, FTP/SFTP, or:

```ros
/file/add name=usb1/llama-scale type=directory
/file/add name=usb1/llama-scale/layer type=directory
/file/add name=usb1/llama-scale/root type=directory
/file/add name=usb1/llama-scale/config type=directory
```

(Exact `/file` syntax can vary slightly by RouterOS version; creating the
folders over SFTP is always fine.)

### Getting the image

Point the container registry at GitHub Container Registry and set a pull
tmpdir (on disk/USB if internal flash is small):

```ros
/container/config/set registry-url=https://ghcr.io tmpdir=/usb1/pull
```

If you already use Docker Hub as `registry-url`, you can still pull by fully
qualified name (`ghcr.io/tokenring-ai/llama-scale:latest`) depending on
RouterOS version — when in doubt, set `registry-url` to `https://ghcr.io` for
this image.

### Pull image

```ros
/container/add \
  remote-image=tokenring-ai/llama-scale:latest \
  interface=veth1 \
  root-dir=/usb1/llama-scale/root \
  layer-dir=/usb1/llama-scale/layer \
  mountlists=llama-scale-config \
  logging=yes \
  check-certificate=yes \
  name=llama-scale \
  start-on-boot=yes
```

If your RouterOS build expects the host in `remote-image`, use:

```ros
remote-image=ghcr.io/tokenring-ai/llama-scale:latest
```

After running the command, RouterOS extracts the package. Watch progress with:

```ros
/container/print
```

Wait until the status is `stopped` (image ready) before starting. Pin a version
tag (for example `tokenring-ai/llama-scale:v1.0.4`) for production instead of
`latest`.

### Setting up the llama-scale configuration file

Prepare a `config.yaml` on your workstation. The listen address must be
reachable on the veth IP; bind all interfaces inside the container:

```yaml
server:
  listen: "0.0.0.0:11434"
  api_keys:
    - "sk-change-me-on-mikrotik"

log:
  destination: "console"
  level: "info"

models_refresh_interval_secs: 30
health_check_interval_secs: 15
health_check_timeout_secs: 5
session_ttl_secs: 3600
session_max_entries: 100000

backends:
  # Inference host on your LAN (not 127.0.0.1 — that would be the container itself)
  - name: ollama
    url: "http://192.168.88.50:11434/v1"
    api_key: "ollama"
    health_path: "/health"
```

Important MikroTik-specific points:

| Topic | Guidance |
|-------|----------|
| `server.listen` | Use `0.0.0.0:<port>`. Port must match `to-ports` in any dst-nat rule. |
| Backend `url` | Use a **LAN or public IP/hostname**. `127.0.0.1` inside the container is the container, not the router and not your PC. |
| API keys | Change the default; treat WAN exposure as hostile. |
| `${ENV}` secrets | Supported if you set container `env` / envlists for those variables. |

Upload the file with SFTP (SSH service must be enabled). From a workstation:

```powershell
sftp admin@192.168.88.1
sftp> cd usb1/llama-scale/config
sftp> put config.yaml
sftp> quit
```

With the mount list above, the container will see the file as
`/etc/llama-scale/config.yaml`.

If you use an **inline** single-file mount instead, upload to the `src` path you
configured (for example `/usb1/llama-scale/config.yaml`).

### Starting the container

```ros
/container/start llama-scale
```

Or by number from `/container/print`:

```ros
/container/start 0
```

With `logging=yes`, the system log should show startup lines similar to:

```text
container,info,debug starting llama-scale
container,info,debug listening on 0.0.0.0:11434
```

After config changes, restart:

```ros
/container/stop llama-scale
/container/start llama-scale
```

Wait until status is `stopped` before starting again.

## Verify

From a LAN host (or the router, if `fetch` can reach the veth IP):

```bash
# Liveness
curl -s http://192.168.15.7:11434/healthz

# Readiness (200 once at least one backend is healthy)
curl -s http://192.168.15.7:11434/readyz

# Models
curl -s http://192.168.15.7:11434/v1/models \
  -H "Authorization: Bearer sk-change-me-on-mikrotik"

# Chat
curl -s http://192.168.15.7:11434/v1/chat/completions \
  -H "Authorization: Bearer sk-change-me-on-mikrotik" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "llama3.2",
        "messages": [{"role": "user", "content": "Say hi."}]
      }'
```

If you published the service via dst-nat on the router LAN IP:

```bash
curl -s http://192.168.88.1:11434/healthz
```

Point OpenAI-compatible clients at:

```text
http://<router-or-veth-ip>:11434/v1
```

## Environment variables (optional)

To keep secrets out of the mounted YAML, define env on the container and
reference them as `${VAR}` in `config.yaml`:

```ros
/container/envs/add list=llama-scale-env name=ROUTER_API_KEY value="sk-your-real-key"
```

```ros
/container/set llama-scale envlists=llama-scale-env
```

```yaml
server:
  api_keys:
    - "${ROUTER_API_KEY}"
```

Missing variables are a hard startup error.

## Reference: compact RouterOS snippet

A complete minimal shape (addresses and paths are examples — align them with
your device):

```ros
# Device mode (confirm per MikroTik docs)
/system/device-mode/update container=yes

# Network
/interface/veth/add name=veth1 address=192.168.15.7/24 gateway=192.168.15.1
/interface/bridge/add name=containers
/ip/address/add address=192.168.15.1/24 interface=containers
/interface/bridge/port/add bridge=containers interface=veth1

# Mount + registry
/container/mounts/add list=llama-scale-config \
  src=/usb1/llama-scale/config dst=/etc/llama-scale
/container/config/set registry-url=https://ghcr.io tmpdir=/usb1/pull

# Image
/container/add \
  remote-image=tokenring-ai/llama-scale:latest \
  interface=veth1 \
  root-dir=/usb1/llama-scale/root \
  layer-dir=/usb1/llama-scale/layer \
  mountlists=llama-scale-config \
  logging=yes \
  check-certificate=yes \
  name=llama-scale \
  start-on-boot=yes

# Optional: expose on WAN (lock down with filter rules!)
/ip/firewall/nat/add action=dst-nat chain=dstnat \
  in-interface-list=WAN dst-port=11434 protocol=tcp \
  to-addresses=192.168.15.7 to-ports=11434 \
  comment="llama-scale WAN"

# After uploading config.yaml into /usb1/llama-scale/config/
/container/start llama-scale
```

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| Image pull fails | `registry-url`, DNS on the router, disk space under `tmpdir` / USB, architecture (arm64/amd64 only) |
| Status stuck / extract errors | Prefer USB for `root-dir` / `layer-dir`; free space; try a pinned version tag |
| Container starts then exits | `/log print where topics~"container"` — usually missing mount, bad YAML, or unresolved `${VAR}` |
| `/readyz` is 503 | Backend `url` wrong from the container’s network view; test with `/tool/fetch url="http://…"` from RouterOS |
| LAN clients cannot connect | dst-nat / firewall filter; confirm `server.listen` is `0.0.0.0:<port>` |
| Works on veth IP but not WAN | WAN dst-nat and **filter** accept rules; do not open WAN without auth + restricted sources |
| Backend timeouts | Ensure container egress (masquerade/routing) can reach the inference host |

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup: [README — Setting up backends](../README.md#setting-up-backends)
- Generic container image notes: [Getting started with Docker](getting-started-docker.md)
- MikroTik container docs: [https://manual.mikrotik.com/docs/containers/](https://manual.mikrotik.com/docs/containers/)

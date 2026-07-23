# Getting started on Proxmox VE

Run llama-scale on [Proxmox VE](https://www.proxmox.com/) as a lightweight
router in front of inference backends (Ollama, llama.cpp, vLLM, LM Studio, and
similar) that already live on VMs, LXC containers, or other hosts on your LAN.

This guide covers three common patterns:

| Pattern | Best for |
|---------|----------|
| **Debian/Ubuntu LXC + `.deb`** | Lowest overhead, systemd, no Docker |
| **LXC or VM + Docker image** | Match production container deploys |
| **Full VM + package or binary** | When you want a normal Linux guest |

GPU backends almost always run in a separate VM or privileged LXC with device
passthrough; llama-scale itself is CPU-only and is a good fit for an unprivileged
container.

## Prerequisites

- Proxmox VE host with storage for a CT or VM
- Network path from the llama-scale guest to each backend (`url` hosts must
  resolve and accept connections)
- One of: Debian/Ubuntu CT template, a Docker-capable guest, or a Linux VM

Optional: a reverse proxy (Nginx Proxy Manager, Caddy, Traefik, HAProxy) in
front for TLS and a stable hostname.

## Network model

llama-scale only needs:

1. **Inbound** — clients (apps, Open WebUI, etc.) → `server.listen` (default
   `0.0.0.0:8080` in the example config; packaged defaults may use loopback —
   change to `0.0.0.0` if you need LAN access)
2. **Outbound** — llama-scale → backend HTTP ports (e.g. Ollama `11434`)

Typical homelab layout:

```text
                  LAN / VLAN
                       │
     ┌─────────────────┼─────────────────┐
     │                 │                 │
 [clients]      [llama-scale CT]   [Ollama VM / CT]
                       │                 │
                       └── http://192.168.x.y:11434/v1 ──┘
```

Use static IPs or DHCP reservations for backends so `config.yaml` does not
break after reboots. Firewall (Proxmox datacenter/host, guest `ufw`, or VLAN
rules) should allow only trusted sources to the router port.

---

## Option A: Debian / Ubuntu LXC + package (recommended)

### 1. Create the container

In the Proxmox UI: **Create CT**

| Setting | Suggestion |
|---------|------------|
| Template | Debian 12 or Ubuntu 22.04+ |
| Unprivileged | Yes |
| Nesting | Not required for the `.deb` |
| CPU / RAM | 1 vCPU, 512 MiB is ample |
| Disk | 2–4 GiB |
| Network | Bridge (`vmbr0` or your LAN bridge), static IP preferred |
| Features | Default is fine |

Or from the Proxmox shell (adjust storage, bridge, IP):

```bash
# Example — pick a free CTID and matching template on your node
pct create 120 local:vztmpl/debian-12-standard_12.7-1_amd64.tar.zst \
  --hostname llama-scale \
  --memory 512 \
  --cores 1 \
  --rootfs local-lvm:4 \
  --net0 name=eth0,bridge=vmbr0,ip=192.168.1.50/24,gw=192.168.1.1 \
  --nameserver 192.168.1.1 \
  --unprivileged 1 \
  --features nesting=0 \
  --start 1
```

Enter the CT:

```bash
pct enter 120
```

### 2. Install llama-scale

Follow the same steps as bare metal — full detail in
[Getting started on Debian / Ubuntu](getting-started-debian-ubuntu.md):

```bash
# Inside the CT (amd64 example; use arm64 package on ARM hosts)
VERSION=1.0.4   # pin a real release
curl -LO "https://github.com/tokenring-ai/llama-scale/releases/download/v${VERSION}/llama-scale_${VERSION}_amd64.deb"
apt-get update
apt-get install -y ./llama-scale_${VERSION}_amd64.deb
# or: dpkg -i ... && apt-get install -f
```

### 3. Configure and start

```bash
cp /etc/llama-scale/config.yaml.default /etc/llama-scale/config.yaml
nano /etc/llama-scale/config.yaml
```

Point backends at other guests by **LAN IP or hostname**, not `127.0.0.1`
(unless the inference server is in the *same* CT):

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys:
    - "sk-change-me-after-install"

backends:
  - name: ollama
    url: "http://192.168.1.40:11434/v1"   # Ollama CT/VM on the LAN
    api_key: "ollama"
    health_path: "/health"
```

Secrets via `/etc/default/llama-scale` and `${VAR}` work the same as on Debian
packages — see the [Debian guide](getting-started-debian-ubuntu.md#secrets-via-environment-variables).

```bash
systemctl enable --now llama-scale
systemctl status llama-scale
```

### 4. Open the firewall if needed

If the CT or host firewall is strict:

```bash
# Example with ufw inside the CT
ufw allow 8080/tcp
ufw reload
```

Proxmox **Datacenter → Firewall** / **node firewall** rules may also need an
allow for the CT IP and port from trusted subnets only.

---

## Option B: Docker in an LXC or VM

Use the official image when you already run Docker, or want the same deploy as
cloud/Kubernetes.

### Guest requirements

- **VM:** install Docker Engine normally ([Docker guide](getting-started-docker.md))
- **LXC:** needs nesting (and often keyctl) for Docker; unprivileged + nesting
  is possible but more fiddly than a small Debian CT with the `.deb`. Prefer a
  **VM** for Docker if you are not already comfortable with nested containers
  on Proxmox.

### Run

On the guest, create `config.yaml` with backend URLs reachable from *that*
guest (other CT/VM IPs, or `host.docker.internal` only if the backend is on
the Docker host itself).

```bash
docker run -d --name llama-scale \
  --restart unless-stopped \
  -p 8080:8080 \
  -v "$(pwd)/config.yaml:/etc/llama-scale/config.yaml:ro" \
  -e ROUTER_API_KEY="sk-change-me" \
  ghcr.io/tokenring-ai/llama-scale:v1.0.4
```

Full options (Compose, host networking, env secrets):
[Getting started with Docker](getting-started-docker.md).

---

## Option C: Linux VM + binary or package

Create a normal Debian/Ubuntu/Fedora VM, then:

- **Debian/Ubuntu:** [package guide](getting-started-debian-ubuntu.md)
- **RHEL/Fedora:** [RPM guide](getting-started-redhat-fedora.md)
- **Any Linux:** [install script or release tarball](../README.md#quick-start)

Give the VM a static IP and treat backends the same as Option A.

---

## Verify from another machine

Replace `192.168.1.50` with the CT/VM address and the API key from your config:

```bash
curl -s http://192.168.1.50:8080/healthz
curl -s http://192.168.1.50:8080/readyz

curl -s http://192.168.1.50:8080/v1/models \
  -H "Authorization: Bearer sk-change-me-after-install"

curl -s http://192.168.1.50:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-change-me-after-install" \
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

Point Open WebUI, Continue, or any OpenAI-compatible client at:

```text
http://192.168.1.50:8080/v1
```

with the same bearer key.

## TLS and reverse proxy

For LAN-only lab use, HTTP plus a strong `api_keys` entry is often enough.
For anything wider:

1. Put Caddy / Nginx / Traefik on another CT (or the same guest) and terminate
   TLS there, **or**
2. Enable llama-scale `server.tls` with cert/key paths (see
   [main README — TLS](../README.md#tls-terminating-https-directly))

Disable proxy buffering and raise read timeouts for streaming completions when
using Nginx or similar.

## Backends on Proxmox (brief)

llama-scale does not require co-location with the model server. Common patterns:

| Backend | Proxmox notes |
|---------|----------------|
| **Ollama** | Often a VM with GPU passthrough, or privileged LXC + NVIDIA/AMD device |
| **llama.cpp** | Same as above; expose `--port` and OpenAI-compatible `/v1` |
| **vLLM** | Typically a GPU VM; open the serve port to the llama-scale guest only |
| **Remote cloud API** | Outbound HTTPS from the CT; put keys in `${ENV}` / `/etc/default` |

Ensure each backend’s `url` uses the address **as seen from the llama-scale
guest** (guest-to-guest on `vmbr0`, not the Proxmox host’s `127.0.0.1`).

## Updates

**Package (Option A):**

```bash
# Inside CT — download newer .deb and install over the previous package
dpkg -i llama-scale_<new-version>_amd64.deb
systemctl restart llama-scale
```

**Docker (Option B):**

```bash
docker pull ghcr.io/tokenring-ai/llama-scale:v1.0.5
docker stop llama-scale && docker rm llama-scale
# re-run docker run ... with the new tag
```

**Config changes:** edit `config.yaml` (or ConfigMap equivalent), then restart
the service or container — config is loaded at process start.

## Snapshots and backups

- Include the CT/VM in Proxmox **Backup** jobs
- Config lives at `/etc/llama-scale/config.yaml` (package) or your Docker host
  path — treat it as sensitive if it embeds keys (prefer `${ENV}` +
  `/etc/default/llama-scale` with mode `600`)
- Session affinity is in-memory only; restarts drop sticky bindings (clients
  re-pin on the next request)

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| `/readyz` is 503 | From the llama-scale CT: `curl http://<backend-ip>:<port>/health` — routing, firewall, wrong IP |
| Clients cannot connect | `listen` is `127.0.0.1` (change to `0.0.0.0`), CT firewall, Proxmox firewall |
| Works on CT, fails from laptop | Wrong bridge/VLAN; CT not on LAN-facing bridge |
| Docker in LXC fails to start | Nesting/AppArmor; use package install or a VM instead |
| High latency | Backend on slow storage or overloaded GPU — not the router (llama-scale is thin) |

```bash
# From Proxmox host
pct enter <CTID>
journalctl -u llama-scale -f

# From llama-scale guest to backend
curl -v http://192.168.1.40:11434/health
```

## Next steps

- Full config reference: [README — Configuration reference](../README.md#configuration-reference)
- Backend setup: [README — Setting up backends](../README.md#setting-up-backends)
- Debian package details: [Debian / Ubuntu](getting-started-debian-ubuntu.md)
- Containers: [Docker](getting-started-docker.md) · [Kubernetes](getting-started-kubernetes.md)

#!/usr/bin/env bash
# Build a .deb for llama-scale.
# Usage: build-deb.sh <version> <deb-arch> <binary-path> <output-path>
set -euo pipefail

VERSION="${1:?version required}"
DEB_ARCH="${2:?deb arch required (amd64 or arm64)}"
BINARY="${3:?binary path required}"
OUTPUT="${4:?output path required}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

STAGING="${ROOT}/llama-scale_${VERSION}_${DEB_ARCH}"
mkdir -p "${STAGING}/DEBIAN"
mkdir -p "${STAGING}/usr/bin"
mkdir -p "${STAGING}/lib/systemd/system"
mkdir -p "${STAGING}/etc/llama-scale"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

install -m 0755 "${BINARY}" "${STAGING}/usr/bin/llama-scale"
install -m 0644 "${SCRIPT_DIR}/llama-scale.service" "${STAGING}/lib/systemd/system/"
install -m 0644 "${SCRIPT_DIR}/config.yaml.default" "${STAGING}/etc/llama-scale/config.yaml.default"
install -m 0755 "${SCRIPT_DIR}/postinst" "${STAGING}/DEBIAN/"
install -m 0755 "${SCRIPT_DIR}/prerm" "${STAGING}/DEBIAN/"

echo "/etc/llama-scale/config.yaml" > "${STAGING}/DEBIAN/conffiles"

cat > "${STAGING}/DEBIAN/control" <<EOF
Package: llama-scale
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: llama-scale contributors <https://github.com/tokenring-ai/llama-scale>
Depends: ca-certificates, adduser | passwd
Description: OpenAI-compatible LLM router
 A Rust-based OpenAI-compatible LLM router with session affinity
 and least-connections load balancing.
Homepage: https://github.com/tokenring-ai/llama-scale
EOF

dpkg-deb --build --root-owner-group "${STAGING}" "${OUTPUT}"
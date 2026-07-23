#!/usr/bin/env bash
# llama-scale installer
#
# Usage:
#   curl -fsSL https://github.com/tokenring-ai/llama-scale/releases/latest/download/install.sh | bash
#
# Each published install.sh pins an explicit VERSION_PIN so installs from that
# script are deterministic. Bumpversion updates VERSION_PIN on release.
#
# Override the pin (for testing) with LLAMA_SCALE_INSTALL_VERSION=x.y.z
#
# Install strategy:
#   1. If bun or npm is available, install llama-scale@VERSION globally.
#   2. Otherwise on macOS/Linux, download the prebuilt release tarball for the
#      current platform and install the binary under ~/.local/bin.

set -euo pipefail

# Pinned release version (managed by bumpversion)
VERSION_PIN="1.0.6"

REPO="${LLAMA_SCALE_INSTALL_REPO:-tokenring-ai/llama-scale}"
VERSION="${LLAMA_SCALE_INSTALL_VERSION:-$VERSION_PIN}"
RELEASE_TAG="v${VERSION}"
RELEASE_BASE="${LLAMA_SCALE_RELEASE_BASE:-https://github.com/${REPO}/releases/download/${RELEASE_TAG}}"
NPM_PACKAGE="${LLAMA_SCALE_NPM_PACKAGE:-llama-scale}"
NPM_SPEC="${NPM_PACKAGE}@${VERSION}"

BIN_DIR="${LLAMA_SCALE_BIN_DIR:-${HOME}/.local/bin}"
SHARE_DIR="${LLAMA_SCALE_SHARE_DIR:-${HOME}/.local/share/llama-scale}"

RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
YELLOW=$'\033[0;33m'
BOLD=$'\033[1m'
RESET=$'\033[0m'

info() { printf '%s==>%s %s\n' "${GREEN}" "${RESET}" "$*"; }
warn() { printf '%sWarning:%s %s\n' "${YELLOW}" "${RESET}" "$*"; }
error() { printf '%sError:%s %s\n' "${RED}" "${RESET}" "$*" >&2; }
die() { error "$*"; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

download() {
  local url="$1"
  local dest="$2"

  if command -v curl >/dev/null 2>&1; then
    if ! curl -fsSL --retry 3 --retry-delay 1 -o "$dest" "$url"; then
      die "Failed to download: $url"
    fi
  elif command -v wget >/dev/null 2>&1; then
    if ! wget -q -O "$dest" "$url"; then
      die "Failed to download: $url"
    fi
  else
    die "Neither curl nor wget is available"
  fi
}

# Global so the EXIT trap can always see it (locals are out of scope on EXIT).
INSTALL_TMP=""

cleanup_install_tmp() {
  if [[ -n "${INSTALL_TMP}" && -d "${INSTALL_TMP}" ]]; then
    rm -rf "${INSTALL_TMP}"
  fi
  INSTALL_TMP=""
}

detect_rust_target() {
  local os arch

  case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    *) die "Unsupported operating system: $(uname -s). Install with npm, or use Docker / a GitHub Release package." ;;
  esac

  case "$(uname -m)" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) die "Unsupported architecture: $(uname -m)" ;;
  esac

  printf '%s-%s\n' "$arch" "$os"
}

path_contains() {
  case ":${PATH}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

ensure_bin_dir_on_path() {
  if path_contains "$BIN_DIR"; then
    return 0
  fi

  warn "${BIN_DIR} is not on your PATH."
  cat <<EOF

Add it to your shell profile, for example:

  # bash
  echo 'export PATH="${BIN_DIR}:\$PATH"' >> ~/.bashrc

  # zsh
  echo 'export PATH="${BIN_DIR}:\$PATH"' >> ~/.zshrc

  # fish
  fish_add_path ${BIN_DIR}

Then open a new terminal (or re-source your profile).
EOF
}

install_via_package_manager() {
  if command -v bun >/dev/null 2>&1; then
    info "Installing ${NPM_SPEC} globally with bun"
    bun install -g "$NPM_SPEC"
    return 0
  fi

  if command -v npm >/dev/null 2>&1; then
    info "Installing ${NPM_SPEC} globally with npm"
    npm install -g "$NPM_SPEC"
    return 0
  fi

  return 1
}

install_from_release() {
  local target archive_name archive_url
  target="$(detect_rust_target)"
  archive_name="llama-scale-${RELEASE_TAG}-${target}.tar.gz"
  archive_url="${RELEASE_BASE}/${archive_name}"

  INSTALL_TMP="$(mktemp -d "${TMPDIR:-/tmp}/llama-scale-install.XXXXXX")"
  trap cleanup_install_tmp EXIT

  info "Detected platform: ${target}"
  info "Downloading ${archive_name}"

  download "$archive_url" "${INSTALL_TMP}/${archive_name}"

  info "Extracting"
  tar -xzf "${INSTALL_TMP}/${archive_name}" -C "${INSTALL_TMP}"

  if [[ ! -f "${INSTALL_TMP}/llama-scale" ]]; then
    die "Archive did not contain a llama-scale binary: ${archive_name}"
  fi
  chmod 755 "${INSTALL_TMP}/llama-scale"

  info "Installing to ${BIN_DIR}"
  mkdir -p "$BIN_DIR" "$SHARE_DIR"
  install -m 755 "${INSTALL_TMP}/llama-scale" "${BIN_DIR}/llama-scale"

  if [[ -f "${INSTALL_TMP}/config.example.yaml" ]]; then
    install -m 644 "${INSTALL_TMP}/config.example.yaml" \
      "${SHARE_DIR}/config.example.yaml"
  fi

  cleanup_install_tmp
  trap - EXIT

  ensure_bin_dir_on_path

  info "Installed binary: ${BIN_DIR}/llama-scale"
  if [[ -f "${SHARE_DIR}/config.example.yaml" ]]; then
    info "Installed example config: ${SHARE_DIR}/config.example.yaml"
  fi
}

print_success() {
  local command_name="$1"
  local example_config="${SHARE_DIR}/config.example.yaml"

  cat <<EOF

${GREEN}${BOLD}llama-scale ${VERSION} is installed.${RESET}

Create a config and run:

  ${BOLD}curl -fsSL https://raw.githubusercontent.com/${REPO}/main/config.example.yaml -o config.yaml${RESET}
  ${BOLD}# edit config.yaml — set backends and API keys${RESET}
  ${BOLD}${command_name} --config config.yaml${RESET}

EOF

  if [[ -f "$example_config" ]]; then
    cat <<EOF
An example config was also installed at:

  ${BOLD}${example_config}${RESET}

EOF
  fi

  cat <<EOF
Optional: set the config path via environment variable:

  export MODEL_ROUTER_CONFIG=/path/to/config.yaml

Docs: https://github.com/${REPO}#readme
EOF
}

main() {
  info "Installing llama-scale ${VERSION}"

  if install_via_package_manager; then
    print_success "llama-scale"
    return 0
  fi

  case "$(uname -s)" in
    Darwin|Linux)
      need_cmd tar
      need_cmd install
      install_from_release
      print_success "llama-scale"
      ;;
    *)
      die "No bun/npm found and binary installs are only supported on macOS and Linux."
      ;;
  esac
}

main "$@"

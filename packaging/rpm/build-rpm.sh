#!/usr/bin/env bash
# Build an .rpm for llama-scale.
# Usage: build-rpm.sh <version> <rpm-arch> <binary-path> <output-path>
set -euo pipefail

VERSION="${1:?version required}"
RPM_ARCH="${2:?rpm arch required (x86_64 or aarch64)}"
BINARY="${3:?binary path required}"
OUTPUT="${4:?output path required}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEBIAN_DIR="${SCRIPT_DIR}/../debian"

TOPDIR="${ROOT}/rpmbuild"
STAGING="${TOPDIR}/BUILD/staging"
mkdir -p "${TOPDIR}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
mkdir -p "${STAGING}/usr/bin"
mkdir -p "${STAGING}/usr/lib/systemd/system"
mkdir -p "${STAGING}/etc/llama-scale"

install -m 0755 "${BINARY}" "${STAGING}/usr/bin/llama-scale"
install -m 0644 "${DEBIAN_DIR}/llama-scale.service" "${STAGING}/usr/lib/systemd/system/"
install -m 0644 "${DEBIAN_DIR}/config.yaml.default" "${STAGING}/etc/llama-scale/"

tar czf "${TOPDIR}/SOURCES/llama-scale-${VERSION}.tar.gz" -C "${TOPDIR}/BUILD" staging
cp "${SCRIPT_DIR}/llama-scale.spec" "${TOPDIR}/SPECS/"

rpmbuild -bb \
  --target "${RPM_ARCH}" \
  --define "_topdir ${TOPDIR}" \
  --define "version ${VERSION}" \
  "${TOPDIR}/SPECS/llama-scale.spec"

BUILT_RPM="$(find "${TOPDIR}/RPMS/${RPM_ARCH}" -name 'llama-scale-*.rpm' -print -quit)"
if [[ -z "${BUILT_RPM}" ]]; then
  echo "rpmbuild did not produce an RPM for ${RPM_ARCH}" >&2
  exit 1
fi

install -m 0644 "${BUILT_RPM}" "${OUTPUT}"
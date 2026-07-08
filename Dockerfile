# syntax=docker/dockerfile:1

# ---- Build stage ----
# The builder is pinned to the build host's platform ($BUILDPLATFORM) so the
# Rust compile always runs natively (never under QEMU). We cross-compile to the
# requested target arch instead, selected automatically from $TARGETARCH by
# buildx. This works because llama-scale is pure Rust (no native C deps), so
# only a cross-linker is needed for arm64.
#
# Standalone `docker build .` builds a native x86_64 image by default.
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS builder
ARG TARGETARCH
# ARGs aren't shell env vars; ENV makes TARGETARCH visible to the heredoc below.
ENV TARGETARCH=${TARGETARCH}
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    <<EOF
set -eu
case "${TARGETARCH}" in
  amd64) RUST_TARGET="x86_64-unknown-linux-gnu"; CROSS_APT=""; CROSS_LINKER="" ;;
  arm64) RUST_TARGET="aarch64-unknown-linux-gnu"; CROSS_APT="gcc-aarch64-linux-gnu"; CROSS_LINKER="aarch64-linux-gnu-gcc" ;;
  *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;;
esac

rustup target add "${RUST_TARGET}"

if [ -n "${CROSS_APT}" ]; then
  apt-get update
  apt-get install -y --no-install-recommends ${CROSS_APT}
  rm -rf /var/lib/apt/lists/*
fi

if [ -n "${CROSS_LINKER}" ]; then
  mkdir -p /usr/local/cargo
  printf '[target.%s]\nlinker = "%s"\n' "${RUST_TARGET}" "${CROSS_LINKER}" >> /usr/local/cargo/config.toml
fi

cargo build --release --locked --target "${RUST_TARGET}"
cp "target/${RUST_TARGET}/release/llama-scale" /llama-scale
EOF

# ---- Runtime stage ----
# Runs under the target platform. The only RUN here is the lightweight install
# of ca-certificates, so QEMU emulation (for arm64) is negligible.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1001 --create-home llama
WORKDIR /etc/llama-scale
COPY config.example.yaml /etc/llama-scale/config.example.yaml
COPY --from=builder /llama-scale /usr/local/bin/llama-scale
USER llama
EXPOSE 8080
ENTRYPOINT ["llama-scale"]
CMD ["--config", "/etc/llama-scale/config.yaml"]

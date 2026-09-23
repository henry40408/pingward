# syntax=docker/dockerfile:1

# ---- build: static musl binary via cargo-zigbuild -------------------------
# Runs on the native build platform; zig cross-compiles, so no qemu.
# The Rust version comes from rust-toolchain.toml. Don't change this to e.g.
# `rust:1.97`: un-suffixed tags resolve to trixie (a silent Debian major bump).
FROM --platform=$BUILDPLATFORM rust:bookworm AS build

# cmake: aws-lc-sys (rustls crypto) C build. curl + xz: fetch and unpack zig.
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake curl xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Not 0.15+: it adds a libc++-19 bindgen requirement.
ARG ZIG_VERSION=0.14.1
# >= 0.23.0 filters `-Wl,--fix-cortex-a53-843419` (passed by rustc >= 1.98 for
# aarch64 musl), which zig's linker rejects.
ARG ZIGBUILD_VERSION=0.23.0
RUN cargo install cargo-zigbuild --version "${ZIGBUILD_VERSION}" --locked
RUN set -eux; \
    case "$(uname -m)" in \
      x86_64) zarch=x86_64 ;; \
      aarch64) zarch=aarch64 ;; \
      *) echo "unsupported build arch $(uname -m)" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-${zarch}-linux-${ZIG_VERSION}.tar.xz" \
      | tar -xJ -C /opt; \
    ln -s "/opt/zig-${zarch}-linux-${ZIG_VERSION}/zig" /usr/local/bin/zig

WORKDIR /app

# Install the pinned toolchain in its own layer, so source edits don't re-download it.
COPY rust-toolchain.toml .
RUN cargo --version

COPY . .

# Map TARGETARCH to the musl triple and build with the pinned toolchain.
# GIT_VERSION comes from the workflow (`.git` is excluded from the context);
# without it the image is labelled `dev`.
ARG TARGETARCH
ARG GIT_VERSION=dev
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target,sharing=locked \
    set -eux; \
    case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-musl ;; \
      arm64) target=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported target arch $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    rustup target add "$target"; \
    GIT_VERSION="${GIT_VERSION}" cargo zigbuild --release --target "$target"; \
    install -Dm755 "target/${target}/release/pingward" /out/pingward

# ---- runtime: minimal static image (CA certs + tzdata, no shell) ------------
FROM gcr.io/distroless/static-debian12
COPY --from=build /out/pingward /pingward

VOLUME /data

# The default relative SQLite path then lands in the volume.
WORKDIR /data

EXPOSE 8080

# Listen on all interfaces (the app default is loopback); ENV keeps it overridable.
ENV PINGWARD_BIND=0.0.0.0:8080

ENTRYPOINT ["/pingward"]

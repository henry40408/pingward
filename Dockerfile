# syntax=docker/dockerfile:1

# ---- build: cross-compile a static musl binary with cargo-zigbuild ----------
# The builder is pinned to the native build platform; zig cross-compiles to the
# target arch's musl triple, so no qemu emulation is needed — an arm64 image
# builds at the host's native speed.
# No Rust version here: rust-toolchain.toml is the single source of truth and
# rustup installs it below. Do not "simplify" this to `rust:1.97` — the
# un-suffixed tag resolves to trixie, which would be a silent Debian major bump.
FROM --platform=$BUILDPLATFORM rust:bookworm AS build

# aws-lc-sys (the rustls/aws-lc-rs crypto backend behind sqlx's TLS) compiles
# its C sources through CMake; the SQLite (C) dep is built by zig cc. curl
# fetches zig; xz unpacks it.
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake curl xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Zig 0.14.1 avoids the libc++-19 bindgen requirement that 0.15+ introduces.
ARG ZIG_VERSION=0.14.1
# 0.23.0 is the first release whose zigcc wrapper filters
# `-Wl,--fix-cortex-a53-843419`, which rustc started passing for
# aarch64-unknown-linux-musl in 1.98 and zig's linker rejects.
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

# Install the pinned toolchain in a layer keyed on rust-toolchain.toml alone, so
# editing source does not re-download the compiler. Any rustup proxy invocation
# triggers the install.
COPY rust-toolchain.toml .
RUN cargo --version

COPY . .

# Map Docker's TARGETARCH onto the Rust musl triple and build. `rustup target
# add` runs after the source (and rust-toolchain.toml) is in place, so it
# resolves against the pinned toolchain rather than the base image's default.
#
# build.rs stamps the binary with GIT_VERSION. `.dockerignore` excludes `.git`,
# so its own `git describe` fallback cannot work here — the workflow passes the
# describe output in as a build arg. The literal "dev" default means an
# arg-less `docker build` still produces a working image, just labelled `dev`.
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

# Run from /data so the default SQLite path (pingward.sqlite3, relative) is
# created inside the mounted volume without an explicit DATABASE_URL override.
WORKDIR /data

EXPOSE 8080

# Bind the HTTP listener on all interfaces inside the container (the app default
# is loopback). Set via ENV rather than a hardcoded ENTRYPOINT arg so it stays
# overridable at runtime with `-e PINGWARD_BIND=...` or a compose `environment:`
# entry. PINGWARD_BASE_URL should also be set to the externally-reachable URL so
# rendered ping URLs are correct.
ENV PINGWARD_BIND=0.0.0.0:8080

ENTRYPOINT ["/pingward"]

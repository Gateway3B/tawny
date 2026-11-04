# syntax=docker/dockerfile:1.7
# `dx` comes from the CLI image for the Dioxus fork commit Cargo.lock pins,
# built once by the fork's "g3 CLI image" workflow; the image build passes that
# commit as DIOXUS_REV. A CLI from another commit fails to hydrate.
ARG DIOXUS_REV
FROM ghcr.io/gateway3b/dioxus-cli:${DIOXUS_REV} AS dioxus-cli

FROM node:22-bookworm-slim AS web-dependencies

WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev

FROM rust:1.95-bookworm AS chef

ARG CARGO_BUILD_JOBS=2

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        clang \
        cmake \
        curl \
        git \
        libclang-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# The cook and the final build must use the same toolchain. dx must come from
# the same Dioxus fork as the libraries, or the client fails to hydrate.
COPY rust-toolchain.toml ./
RUN rustup target add wasm32-unknown-unknown \
    && cargo install cargo-chef --version 0.1.78 --locked
COPY --from=dioxus-cli /dx /usr/local/cargo/bin/dx

# Keep local sibling overrides outside the image and fetch the portable lockfile.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    GIT_TERMINAL_PROMPT=0

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Cook what `dx build --fullstack --release` compiles: the web client as
# `wasm-release` for wasm32 with only `web`, and the server as `server-release`
# for the host triple (dx always passes `--target`) with `server,web`. Any other
# profile, target or feature set leaves dx recompiling every dependency.
RUN cargo chef cook --recipe-path recipe.json --locked \
        --profile wasm-release \
        --target wasm32-unknown-unknown \
        --no-default-features \
        --features web \
    && cargo chef cook --recipe-path recipe.json --locked \
        --profile server-release \
        --target "$(rustc -vV | sed -n 's/^host: //p')" \
        --no-default-features \
        --features server,web
COPY . .
COPY --from=web-dependencies /app/node_modules /app/node_modules
RUN cargo fetch --locked
RUN dx build --web --fullstack --release --locked --offline

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/dx/tawny/release/web/ /app/
# SurrealKit reads this path even when the app embeds its desired schema.
# Bake it in so non-root/read-only startup never tries to create it.
COPY database/setup.surql /app/database/setup.surql

# Not root: the server writes only under /data (caches, the WebSub secret,
# the suggestion model) and the system temp directory. A fixed id so a host
# can chown a bind mount or an existing volume to it; a new named volume
# copies /data's ownership from the image. See docs/docker.md.
RUN groupadd --system --gid 10001 tawny \
    && useradd --system --uid 10001 --gid tawny --home-dir /data --no-create-home \
        --shell /usr/sbin/nologin tawny \
    && mkdir -p /data \
    && chown tawny:tawny /data

ENV IP=0.0.0.0 \
    PORT=8080 \
    TAWNY_DATA_DIR=/data

USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/app/server"]

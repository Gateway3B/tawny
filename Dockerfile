# syntax=docker/dockerfile:1.7
FROM node:22-bookworm-slim AS web-dependencies

WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev

FROM rust:1.95-bookworm AS builder

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

RUN rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked

WORKDIR /app
COPY . .
COPY --from=web-dependencies /app/node_modules /app/node_modules
# Keep local sibling overrides outside the image. The portable lockfile pins
# g3-brand and the reviewed native source revision.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    GIT_TERMINAL_PROMPT=0 \
    GIT_CONFIG_COUNT=3 \
    GIT_CONFIG_KEY_0=credential.helper \
    GIT_CONFIG_VALUE_0="" \
    GIT_CONFIG_KEY_1=credential.helper \
    GIT_CONFIG_VALUE_1="!sh /app/ci/private-git-credential.sh" \
    GIT_CONFIG_KEY_2=credential.useHttpPath \
    GIT_CONFIG_VALUE_2=true

RUN --mount=type=secret,id=g3_brand_token,required=true cargo fetch --locked

ARG G3_CATALOG_ORIGIN=https://catalog.g3tech.net
ARG G3_RELEASE_CHANNEL=production
ENV G3_CATALOG_ORIGIN=${G3_CATALOG_ORIGIN} \
    G3_RELEASE_CHANNEL=${G3_RELEASE_CHANNEL}

RUN --mount=type=secret,id=g3_brand_token,required=true dx build --web --fullstack --release --locked --offline

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/dx/tawny/release/web/ /app/
# SurrealKit reads this path even when the app embeds its desired schema.
# Bake it in so non-root/read-only startup never tries to create it.
COPY database/setup.surql /app/database/setup.surql

ENV IP=0.0.0.0 \
    PORT=8080 \
    TAWNY_DATA_DIR=/data

EXPOSE 8080
ENTRYPOINT ["/app/server"]

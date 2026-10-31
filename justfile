set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

setup:
    npm install
    lefthook install

# --- Development ----------------------------------------------------------

# Dev server on http://localhost:8080. Needs the services: `just db-up`.
dev:
    dx serve

# Android emulator/device.
dev-android:
    dx serve --platform android

# iOS simulator (macOS only).
dev-ios:
    dx serve --platform ios

# The desktop app.
dev-desktop:
    dx serve --platform desktop

# Start SurrealDB and the extractor sidecars in the background. The server
# seeds a demo catalog into an empty database on its first start.
db-up:
    docker compose up -d --build

db-down:
    docker compose down

# Stop the services and delete their volumes: the library, subscriptions and
# history on this machine are gone.
[confirm("Delete all local Tawny data?")]
db-reset:
    docker compose down -v

# --- Quality --------------------------------------------------------------

format:
    cargo fmt --all
    npm run format:web

format-check:
    cargo fmt --all -- --check
    npm run format:web:check

check: check-web check-server check-mobile check-desktop

check-web:
    cargo check

check-server:
    cargo check --no-default-features --features server

check-mobile:
    cargo check --no-default-features --features mobile

check-desktop:
    cargo check --no-default-features --features desktop

lint:
    cargo clippy --all-targets --no-deps
    cargo clippy --all-targets --no-default-features --features server --no-deps
    npm run lint:web

lint-local:
    cargo clippy --all-targets --no-deps
    npm run lint:web

lint-strict:
    cargo clippy --all-targets --no-deps -- -D warnings
    cargo clippy --all-targets --no-default-features --features server --no-deps -- -D warnings
    cargo clippy --all-targets --no-default-features --features mobile --no-deps -- -D warnings
    cargo clippy --all-targets --no-default-features --features desktop --no-deps -- -D warnings
    npm run lint:web

test-node:
    npm run test:unit

test-rust:
    cargo nextest run
    cargo nextest run --no-default-features --features server

test: test-node test-rust

test-local:
    npm run test:unit:js
    cargo nextest run

test-ui:
    npm run test:ui

spell:
    typos

security:
    cargo deny check

pre-push: format-check check-web lint-local test-local spell

quality: format-check check lint test spell

ci: quality security

# Readability rules inside RSX, checked without rewriting source.
style-check:
    node scripts/rust-style.mjs

style-report:
    node scripts/rust-style.mjs --report

style-test:
    node --test scripts/rust-style.test.mjs

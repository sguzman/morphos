default:
    @just --list

build:
    cargo build --workspace

test:
    cargo test --workspace

fmt:
    cargo fmt --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

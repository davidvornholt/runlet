set shell := ["bash", "-euo", "pipefail", "-c"]

check:
    @cargo fmt; \
    cargo clippy --all-targets --all-features -- -D warnings; \
    cargo test --all-features

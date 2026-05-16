set shell := ["bash", "-euo", "pipefail", "-c"]

check:
    @if [ ! -f Cargo.toml ]; then \
        echo "No Cargo.toml found; Rust checks are not available yet."; \
        exit 0; \
    fi; \
    cargo fmt --check; \
    cargo clippy --all-targets --all-features -- -D warnings; \
    cargo test --all-features

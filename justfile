set shell := ["bash", "-euo", "pipefail", "-c"]

check:
    @cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

check-fix:
    @cargo fmt
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

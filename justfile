set shell := ["bash", "-cu"]

default: ci

# --- formatting ---

fmt:
    cargo +nightly fmt --all
    taplo format

fmt-check:
    cargo +nightly fmt --all -- --check

fmt-toml:
    taplo format

fmt-toml-check:
    taplo format --check

# --- lint / test / deny ---

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

lint-fix:
    cargo clippy --workspace --all-targets --all-features --fix --allow-dirty -- -D warnings

test:
    cargo test --workspace --all-features

deny:
    cargo deny --all-features check advisories bans sources

# --- Python facade ---

python-build:
    cd bindings/python && uv run maturin develop

python-test:
    cd bindings/python && uv run pytest tests

python-unsafe-check:
    ! rg -n "\\bunsafe\\b" bindings/python/src

python-ci: python-build python-test python-unsafe-check

# --- benches, miri, kani ---

bench:
    cargo bench --workspace

miri:
    cargo +nightly miri test --workspace

kani:
    cargo kani --workspace

# --- aggregate ---

# Fast gate — seconds-to-minutes. Runs in prek pre-commit and on every change.
ci: fmt-check fmt-toml-check lint deny test

# Heavy verification — miri for UB, kani for invariant proofs. Runs before
# major PRs, not per-commit.
verify: miri kani

# --- hooks ---

hooks-install:
    prek install --install-hooks

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
    cargo clippy --workspace --all-targets --all-features --exclude oxgraph-pgrx -- -D warnings

lint-fix:
    cargo clippy --workspace --all-targets --all-features --exclude oxgraph-pgrx --fix --allow-dirty -- -D warnings

test:
    cargo test --workspace --all-features

test-default:
    cargo test --workspace --all-features --exclude oxgraph-pgrx

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
    cargo bench --workspace --all-features

miri:
    cargo +nightly miri test --workspace

kani:
    cargo kani --workspace

# --- aggregate ---

# Fast gate — seconds-to-minutes. Runs in prek pre-commit and on every change.
ci: fmt-check fmt-toml-check lint deny test-default

# --- postgres extension (pgrx; macOS needs scripts/postgres-env.sh / .cargo/config.toml) ---
# One-time: `brew install pkgconf` then `just postgres-init`. Tests: `just postgres-test`.

postgres-init:
    ./scripts/postgres-init.sh

postgres-test:
    ./scripts/postgres-test.sh

postgres-check:
    bash -c 'source scripts/postgres-env.sh && cargo check -p oxgraph-pgrx --features pg16'

postgres-bench-engine:
    ./scripts/postgres-bench-engine.sh

postgres-bench-extension:
    ./scripts/postgres-bench-extension.sh

postgres-bench:
    ./scripts/postgres-bench.sh

postgres-bench-sandbox:
    ./scripts/postgres-bench-sandbox.sh

postgres-bench-repeat:
    ./scripts/postgres-bench-repeat.sh

postgres-verify: postgres-test postgres-bench-engine postgres-bench-extension

# Heavy verification — miri for UB, kani for invariant proofs. Runs before
# major PRs, not per-commit.
verify: miri kani

# --- hooks ---

hooks-install:
    prek install --install-hooks

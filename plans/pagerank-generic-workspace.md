# Plan: finish generic PageRank rank-scalar and workspace APIs

## Context

The generic foundation overhaul removed the accidental `f64` lock from property layers, builders, snapshots, and weighted input views, but the PageRank portion is only partially complete. `crates/oxgraph-algo/src/pagerank.rs` now accepts non-`f64` relation/incidence weights through `IntoPageRankScalar<f64>`, but the public PageRank rank state is still `f64`-specific and every entry point allocates scratch internally.

Current gaps found in code:

- `PageRankConfig`, `PageRankReport`, and `PageRankError` store `f64` scalar values.
- `pagerank`, `pagerank_weighted`, `hypergraph_pagerank`, and `hypergraph_pagerank_weighted` accept `Option<&[f64]>` personalization and `&mut [f64]` output ranks.
- The implementation allocates `Vec<f64>` scratch (`teleport`, `next`, `next_elements`, `next_relations`) inside the convenience APIs.
- There is no `PageRankWorkspace` / `HypergraphPageRankWorkspace` or borrowed scratch API analogous to the BFS tier pattern in `crates/oxgraph-algo/src/bfs/workspace.rs`.

The remaining work is to make PageRank ranks/config/personalization/output generic over an algorithm scalar `S`, while keeping topology weights semantic-free and only converting selected weights at the PageRank boundary.

## Approach

Implement PageRank in three API tiers:

1. **Allocating convenience APIs** retain the existing function names but become generic over rank scalar `S` inferred from config/personalization/output rank slices.
2. **Borrowed scratch APIs** accept caller-provided scratch slices/structs and perform no heap allocation after validation.
3. **Owned workspace APIs** mirror BFS workspace ergonomics with reusable `Vec<S>` storage branded to the view type and scalar type.

Use an OxGraph-owned `PageRankScalar` trait for native arithmetic in `S` instead of computing in `f64` and converting at the end. Implement the trait for `f32` and `f64` first, and keep downstream scalar support possible through public trait impls. `IntoPageRankScalar<S>` remains the explicit boundary for topology relation/incidence weights.

No topology trait should gain arithmetic bounds. Python remains a concrete `f64` adapter by choosing `PageRankConfig<f64>`, `&[f64]`, and `Vec<f64>` at the boundary.

## Files to modify

- `.shapes/amendments/...` — add a focused PageRank completion amendment before code edits.
- `.shapes/shapes/22-pagerank-algorithms.yaml` — mark PageRank rank scalar and workspace APIs as part of the actual shape.
- `.shapes/shapes/23-python-facade.yaml` — note Python remains a `f64` specialization over generic PageRank.
- `.shapes/shapes/24-architecture-documentation.yaml` and `docs/architecture.md` — document PageRank scalar/workspace tiers.
- `plans/generic-foundation-overhaul.md` — mark the PageRank checklist item completed once implemented.
- `crates/oxgraph-algo/src/pagerank.rs` — generic scalar API, borrowed scratch APIs, owned workspace APIs, native scalar implementation.
- `crates/oxgraph-algo/src/lib.rs` — export new PageRank workspace/scratch types and functions.
- `crates/oxgraph-algo/tests/pagerank.rs` and `transition_fixture.rs` — f32 rank-scalar coverage, non-f64 input weight coverage, no-allocation scratch/workspace coverage.
- `crates/oxgraph-algo/benches/pagerank.rs` — benchmark allocating vs workspace PageRank.
- `crates/oxgraph-python/src/lib.rs` — adapt type annotations/helpers to `PageRankConfig<f64>` and generic functions.
- `crates/oxgraph-python/tests/test_oxgraph.py` — keep Python smoke tests on the f64 specialization.

## Reuse

- `crates/oxgraph-algo/src/bfs/workspace.rs` — reuse API structure: typed workspace branding via `PhantomData<fn() -> G>`, `new`, `for_graph`, capacity constructors, and grow-on-use semantics.
- Existing PageRank iteration helpers in `crates/oxgraph-algo/src/pagerank.rs` — reuse control flow, but replace hard-coded `f64` arithmetic with scalar `S` operations.
- Existing `IntoPageRankScalar<S>` trait — keep as the selected topology-weight conversion boundary.
- Existing Python facade helper pattern — keep Python concrete and thin.

## Steps

- [x] Add a shape amendment for generic PageRank scalar/output/workspace completion.
- [x] Redesign `PageRankScalar` to support native scalar arithmetic required by PageRank (`ZERO`, `ONE`, infinity/default construction, finite checks, absolute value, conversion from `usize`, and basic arithmetic bounds).
- [x] Make `PageRankConfig<S>`, `PageRankReport<S>`, and `PageRankError<S>` generic over `S` without requiring topology weights to be numeric.
- [x] Change graph PageRank APIs so `personalization: Option<&[S]>`, `ranks: &mut [S]`, scratch, deltas, and invalid scalar values use `S`.
- [x] Change hypergraph PageRank APIs the same way for both element and relation rank slices.
- [x] Update weighted graph/hypergraph bounds from `IntoPageRankScalar<f64>` to `IntoPageRankScalar<S>`.
- [x] Add borrowed scratch types/functions for graph and hypergraph PageRank so callers can supply reusable `teleport`/`next` arrays with no allocation.
- [x] Add owned `PageRankWorkspace<G, S>` and `HypergraphPageRankWorkspace<H, S>` types with `new`, `for_graph`, capacity constructors, and grow-on-use behavior.
- [x] Keep existing allocating function names as convenience wrappers that allocate/fill a temporary workspace internally.
- [x] Update Python to explicitly use the `f64` scalar specialization and keep Python API behavior unchanged.
- [x] Expand tests for `f32` rank output, `f64` rank output, non-f64 input weights, borrowed scratch reuse, owned workspace reuse, and hypergraph workspace reuse.
- [x] Expand benches to compare allocating vs workspace PageRank paths.
- [x] Update docs/plans to remove the PageRank follow-up caveat after verification.

## Verification

- `shapes validate`
- `cargo fmt --all` / `cargo +nightly fmt --all -- --check` as available
- `cargo check --workspace --all-targets --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cd crates/oxgraph-python && uv run --reinstall-package oxgraph --with pytest --with maturin python -m pytest tests`
- Targeted assertions:
  - allocating and workspace PageRank produce identical ranks for graph and hypergraph fixtures;
  - `f32` rank output compiles and converges on small fixtures;
  - non-f64 builder/property weights can drive weighted PageRank through `IntoPageRankScalar<S>`;
  - borrowed scratch APIs reject undersized scratch before iteration;
  - owned workspaces grow once and can be reused across repeated calls for the same view/scalar type.

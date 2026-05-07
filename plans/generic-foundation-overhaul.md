# Plan: generic foundation overhaul for weights, properties, builders, and PageRank

## Context

The current Python-enabling branch is mechanically green, but it violates the intended generic substrate architecture by baking `f64` into public Rust layers above the newly-generic topology traits. The topology additions (`ElementWeight`, `RelationWeight`, `IncidenceWeight`) correctly use an associated `Weight: Copy`; the problem is that property layers, builders, snapshots, algorithms, and Python adapters collapse that generic design back to `f64`-only APIs.

Key findings from the current unstaged/untracked changes:

- `crates/oxgraph-property/src/lib.rs` exposes `DenseF64Layer`, `SparseF64Layer`, `MissingPolicy::DefaultF64`, and f64-only snapshot payload validation.
- `crates/oxgraph-graph-build/src/lib.rs` and `crates/oxgraph-hyper-build/src/lib.rs` store builder weights as `Vec<f64>` / `Box<[f64]>` and only export built-in f64 weight layers.
- `crates/oxgraph-algo/src/pagerank.rs` requires weighted views with `Weight = f64` and allocates internal rank/scratch buffers without a reusable workspace API.
- `crates/oxgraph-python/src/lib.rs` can keep Python-float conveniences, but it must be a boundary adapter over generic Rust APIs, not the source of the Rust model.
- `docs/architecture.md` and `plans/python-bindings-birdsong.md` currently mark f64-specific implementation as complete even though it is not foundational enough.

This is greenfield. The fix should remove f64-specialized public Rust surfaces rather than preserving deprecated aliases or compatibility shims.

## Approach

Make the Rust substrate generic end-to-end, with f64 only as an optional concrete specialization at the Python/numeric-convenience boundary.

1. **Shape first.** Add a new amendment covering the generic overhaul before implementation. Update the affected shape intent/realization notes so the graph says `f64` is not the property/build/algorithm model.
2. **Keep topology traits as-is.** The topology weight traits are already right: direct total value returns with associated `Weight: Copy` and no semantics. Do not add arithmetic, probability, or PageRank semantics to topology weights.
3. **Separate topology weight type from algorithm rank scalar.** PageRank consumes topology weight capabilities, but the computed ranks are algorithm state, not topology input weights. Integer or fixed-point edge weights still need fractional rank output, so PageRank should use a generic rank scalar `S` and convert `W::Weight` into `S` at the algorithm boundary.
4. **Define OxGraph-owned numeric traits.** Avoid committing the public API to `num-traits` for now. Add minimal OxGraph traits such as `PageRankScalar` and `IntoPageRankScalar<S>` in `oxgraph-algo`, with impls for `f32`, `f64`, and documented primitive weight conversions. Downstream-defined scalar support remains possible by implementing these traits.
5. **Replace f64 property APIs with generic Arrow-backed property layers.** Move from `DenseF64Layer` / `SparseF64Layer` to generic dense/sparse typed layers over Arrow arrays/native scalar views. Selected weight adapters should be generic over the selected layer value type.
6. **Make builders generic over element/relation/incidence weight types and support named property layers.** Builders should not require `One`, `Default`, or `f64`; callers provide default element/relation/incidence weights at construction. Builders also attach named Arrow property layers before freeze/export.
7. **Use arrow-rs for property persistence.** Replace the f64-only descriptor/data payload with Arrow IPC/schema-backed property sections so OxGraph does not reinvent Arrow data encoding. OxGraph metadata should index layers by ID family/name/role and validate section consistency; Arrow-rs owns schema and array payload fidelity.
8. **Make PageRank generic over rank scalar and input weight conversion.** Weighted PageRank should not require `Weight = f64`; it should accept relation/incidence weights convertible into the chosen rank scalar and keep rank/config/personalization/output typed by that scalar. Add reusable workspace/scratch variants following the BFS tier discipline.
9. **Keep Python as a thin specialization.** Python can expose `float`/Float64 convenience constructors because CPython floats are f64, but those classes/functions should wrap generic Rust types. If Python exposes typed properties, use explicit names for each supported family rather than pretending f64 is the whole model.
10. **Remove dead/backcompat code.** Since the branch is not landed, rename/remove f64-specific Rust public symbols instead of keeping deprecated aliases.

## Files to modify

Shape/docs first:

- `.shapes/amendments/...` — new amendment targeting shapes 19/20/21/22/23 and possibly 24.
- `.shapes/shapes/19-arrow-property-layers.yaml` — revise from f64-first completion to generic typed Arrow property layers.
- `.shapes/shapes/20-construction-builders.yaml` — revise builders to generic weight/property construction.
- `.shapes/shapes/21-snapshot-identity-property-sections.yaml` — revise property sections to generic typed/Arrow-compatible payloads.
- `.shapes/shapes/22-pagerank-algorithms.yaml` — revise PageRank to generic scalar/weight-conversion contracts.
- `.shapes/shapes/23-python-facade.yaml` — document Python as a specialization boundary.
- `docs/architecture.md` — replace f64-specific snapshot/property wording with generic architecture.
- `plans/python-bindings-birdsong.md` — change completion status/punch list to reflect this overhaul.

Rust implementation:

- `Cargo.toml` and relevant crate manifests — add only minimal Arrow dependencies needed for Arrow-backed property persistence; avoid `num-traits` unless later explicitly justified.
- `crates/oxgraph-property/src/lib.rs` — generic Arrow-backed property descriptors/layers, selected weight views, validation, Arrow IPC/schema snapshot encode/validate.
- `crates/oxgraph-property/tests/proptest.rs` and `benches/property.rs` — cover generic typed layers and selected weights.
- `crates/oxgraph-graph-build/src/lib.rs` — generic `GraphBuilder<EW, RW>` and `FrozenGraph<EW, RW>`; named property attachment and export.
- `crates/oxgraph-hyper-build/src/lib.rs` — generic `HypergraphBuilder<EW, RW, IW>` and `FrozenHypergraph<EW, RW, IW>`; named property attachment and export.
- `crates/oxgraph-algo/src/pagerank.rs` — generic rank scalar/config/workspace and weighted input conversion.
- `crates/oxgraph-algo/tests/pagerank.rs`, `transition_fixture.rs`, `benches/pagerank.rs` — run at least f32 and f64/rank-scalar coverage plus non-f64 input weights.
- `crates/oxgraph-python/src/lib.rs`, `python/oxgraph/__init__.py`, `tests/test_oxgraph.py` — adapt Python to generic Rust APIs with explicit Python concrete typed facades.
- `crates/oxgraph/src/lib.rs` and `Cargo.toml` — re-export generic names/features only.

## Reuse

- `crates/oxgraph-topology/src/lib.rs` — keep and reuse associated-type weight capability traits.
- `crates/oxgraph-graph/src/lib.rs` / `crates/oxgraph-hyper/src/lib.rs` — keep vocabulary/capability bundles; only adjust if PageRank needs reusable typed policies.
- `crates/oxgraph-csr/src/lib.rs` and `crates/oxgraph-hyper-bcsr/src/internal/traits.rs` — retain storage/layout traversal; do not add Arrow/property deps.
- `crates/oxgraph-property/src/lib.rs` — reuse descriptor concepts (`LayerId`, `LayerName`, `IdFamily`, `LayerRole`, `StorageMode`) while replacing f64-specific layer/value/default machinery.
- `crates/oxgraph-algo/src/bfs/*` — reuse BFS scratch/workspace tier pattern for PageRank workspace APIs.
- `crates/oxgraph-snapshot` — keep topology-agnostic section container; property sections remain registered above it.

## Steps

- [x] Add/update Shapes amendment and constraints so generic typed weights/properties are the explicit intent before code changes.
- [x] Update architecture docs and plan status to reject f64-only Rust surfaces as incomplete.
- [x] Replace `MissingPolicy::DefaultF64` with Arrow scalar/default representation that works for typed property layers.
- [x] Replace `DenseF64Layer` / `SparseF64Layer` with generic Arrow-backed dense/sparse layers and generic selected weight adapters for element/relation/incidence families.
- [x] Replace f64-only property snapshot records with Arrow IPC/schema-backed property sections that preserve real schema/type information and reject missing/overlapping/trailing section data.
- [x] Make graph and hypergraph builders generic over weight types, with caller-supplied default weights and no f64 default type parameters unless explicitly approved.
- [x] Add named Arrow property attachment/export APIs to graph and hypergraph builders in this slice.
- [x] Add minimal OxGraph-owned `PageRankScalar` and `IntoPageRankScalar<S>` traits; implement f32/f64 rank scalars and documented primitive weight conversions.
- [x] Make PageRank generic over rank scalar and accepted weight input type; add reusable borrowed scratch and owned workspace variants following BFS tier discipline.
- [x] Update Python to call concrete specializations of the generic Rust APIs; fix partial mutation on duplicate labels before returning errors.
- [x] Remove f64-specialized public Rust symbols/aliases and any dead compatibility code introduced only to preserve the current branch API.
- [x] Update umbrella re-exports and feature flags to expose the generic names.
- [x] Expand tests/proptests/benches/docs to match the new generic contracts.

## Verification

- `shapes validate`
- `cargo +nightly fmt --all -- --check`
- `taplo format --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `just ci`
- Python smoke tests: `cd crates/oxgraph-python && uv run --reinstall-package oxgraph --with pytest --with maturin python -m pytest tests`
- Proptests:
  - generic dense/sparse property selection across at least multiple Arrow primitive scalar types;
  - builder freeze/export for non-f64 weights and named attached properties;
  - PageRank invariants for f32 and f64 rank scalars plus non-f64 input weights where supported.
- Criterion benches:
  - generic selected-weight lookup overhead;
  - builder ingest/freeze with generic weights;
  - PageRank workspace iteration throughput.
- Kani:
  - keep existing bounded proofs where reachable;
  - add proof/skip rationales for new algebraic contracts and generic conversion helpers.

## Resolved design answers

1. **PageRank uses topology weights as inputs, but ranks are algorithm state.** The topology `Weight` associated type should stay semantic-free and arithmetic-free. PageRank adds local algorithm bounds: selected relation/incidence weights convert into a rank scalar `S`; ranks, damping, tolerance, personalization, and deltas are all typed as `S`.
2. **Use OxGraph-owned minimal numeric traits first.** Because the project is foundational and the exact dependency posture is undecided, define the smallest public traits needed by PageRank in `oxgraph-algo` instead of exposing `num-traits` in the API.
3. **Use arrow-rs for property persistence.** Property sections should use Arrow schema/IPC/array machinery rather than a custom f64 or primitive-only payload format.
4. **Builders support named property layers.** Graph and hypergraph builders should attach named Arrow property layers before freeze/export; generic topology weights are only the total weight capability path, not the entire builder property story.

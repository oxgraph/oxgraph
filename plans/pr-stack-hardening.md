# PR stack hardening plan

## Context

The open PR stack (#1-#9) adds index-width CSR support, architecture decisions, topology capabilities, Arrow-backed properties, builders, PageRank, umbrella re-exports, and a Python facade. The bottom of the stack is directionally good, but review found correctness bugs and architectural entropy in the upper layers.

The goal of this plan is to re-cut the stack so OxGraph remains:

- substrate-agnostic: topology traits do not encode domain/property/Python semantics;
- dependency-clean: no Arrow/PyO3/std leakage into foundation crates;
- wire-safe: snapshot bytes are explicitly endian/width/version defined;
- generic but simple: generic where it buys real substrate value, not bloat;
- testable: public data-taking APIs have proptests, existing bounded layout contracts keep Kani proofs, and Kani-skipped crates name their replacement proptest/fixture coverage.

This is a bottom-up stacked-PR repair plan. Each lower PR must be fixed and rebased before fixing the PRs above it.

## Approach

1. **Preserve the good foundation.** Keep PR #2 architecture intent and PR #3 topology capability traits, with only clarifying docs and shape updates.
2. **Fix wire/index fundamentals before higher layers.** PR #1 must make CSR memory and snapshot widths explicit, portable, and future-proof.
3. **Shrink property/build surfaces.** PR #4-#6 must make graph construction Arrow-free in the core feature set and must repair property/identity mapping after layout reordering.
4. **Re-cut PageRank around a correct visible-state contract.** PR #7 must not leak rank mass, must reject duplicate states, and must split no-alloc scratch from alloc convenience APIs.
5. **Curate aggregation layers.** PR #8 must use explicit re-exports and explicit feature costs. PR #9 is reviewed after PR #1-#8 as a standalone Python facade under `bindings/python`.
6. **Update Shapes first in the implementation phase.** Before code edits, add/amend Shapes to record the architecture corrections below.

## Fixed implementation decisions

These decisions remove discretion from the implementation handoff.

- CSR memory views use two logical widths: `NodeIndex` for node IDs/target entries and `EdgeIndex` for edge IDs/offset entries.
- CSR persisted snapshots support only `u16`, `u32`, and `u64` wire widths. `usize` remains native-memory-only.
- CSR persisted snapshots use width-specific section kinds. There is no generic widthless CSR snapshot section kind.
- Property layer IDs, sparse indexes, descriptor metadata words, and identity-map canonical IDs are generic over explicit unsigned widths. No Rust property API defaults to `u64`.
- Property snapshots use Arrow IPC schema as the only Arrow type/schema source of truth. `ArrowValueFamily` is removed.
- Sparse-default property snapshots store the sparse values stream and the one-value default stream as two explicit data ranges.
- Property layers in snapshots are keyed by snapshot-local IDs. Exporters must reorder property arrays when snapshot-local order differs from canonical builder order.
- Core graph/hyper builders are `no_std + alloc` crates in their base feature set and do not depend on Arrow, `oxgraph-property`, or PyO3.
- Core graph/hyper builders are generic over builder ID width and do not default to `u32`.
- Core unweighted builders do not carry weight fields. Weighted builders require explicit weights on add operations and do not use default weights.
- Builder snapshot/property export lives behind explicit `snapshot` and `property-arrow` features inside the builder crates.
- Builder generation counters are removed from this stack; no cache invalidation API is introduced.
- PageRank uses induced visible-state semantics: transitions to invisible states are ignored, and rows with no visible outgoing targets are dangling.
- `PageRankScalar` remains a generic public Rust trait with explicit documented numeric laws; Rust PageRank APIs do not default to `f64`.
- Umbrella crate features use explicit names for Arrow/property costs: `property-arrow`, `graph-property-arrow`, and `hyper-property-arrow`.
- PR #9 Python facade is a standalone follow-up branch after PR #1-#8, outside the root Rust workspace.

## Genericity policy

These rules apply to every phase below.

- Do not add default generic type parameters on public Rust types in this stack.
- Do not add `Default` implementations that choose semantic numeric values, ID widths, index widths, or damping/tolerance policy for Rust users.
- Do not force `u32`, `u64`, `usize`, `f64`, or `()` on public Rust APIs when the value can be expressed as a generic associated type or explicit type parameter.
- Fixed-width wire records are allowed only when the chosen width is selected explicitly through a type parameter or width-specific section kind.
- Python may choose `f64` and string labels later because Python is a facade; that choice must not leak into Rust substrate APIs.

## Files to modify

### Shapes and docs

- `.shapes/amendments/*.yaml`
- `.shapes/shapes/5-csr-graph-layout.yaml`
- `.shapes/shapes/19-arrow-property-layers.yaml`
- `.shapes/shapes/20-construction-builders.yaml`
- `.shapes/shapes/21-snapshot-identity-property-sections.yaml`
- `.shapes/shapes/22-pagerank-algorithms.yaml`
- `.shapes/shapes/23-python-facade.yaml`
- `.shapes/shapes/24-architecture-documentation.yaml`
- `docs/architecture.md`
- `docs/downstream-migration.md`
- `plans/oxgraph-design-decisions.md` — update to mark superseded decisions and point to `docs/architecture.md` for the active contract

### Workspace and CI

- `Cargo.toml`
- `Cargo.lock`
- `justfile`
- `deny.toml` — verify the existing duplicate/dependency policy still passes after the re-cut; no planned policy relaxation
- `.gitignore`

### CSR, BCSR, and snapshots

- `crates/oxgraph-csr/Cargo.toml`
- `crates/oxgraph-csr/src/lib.rs`
- `crates/oxgraph-csr/src/proofs.rs`
- `crates/oxgraph-csr/tests/csr.rs`
- `crates/oxgraph-csr/tests/snapshot_section.rs`
- `crates/oxgraph-csr/benches/*.rs`
- `crates/oxgraph-csr/examples/*.rs`
- `crates/oxgraph-hyper-bcsr/Cargo.toml`
- `crates/oxgraph-hyper-bcsr/src/**/*.rs`
- `crates/oxgraph-hyper-bcsr/tests/*.rs`
- `crates/oxgraph-hyper-bcsr/benches/*.rs`
- `crates/oxgraph-hyper-bcsr/examples/*.rs`
- `crates/oxgraph-snapshot/src/container/builder.rs` — add typed little-endian helper APIs used by snapshot exporters
- `crates/oxgraph-snapshot/tests/*` — add tests for little-endian typed helper APIs

### Property layers and property/identity snapshot sections

- `crates/oxgraph-property/Cargo.toml`
- `crates/oxgraph-property/src/lib.rs`
- `crates/oxgraph-property/tests/proptest.rs`
- `crates/oxgraph-property/benches/property.rs`

### Builders

- `crates/oxgraph-graph-build/Cargo.toml`
- `crates/oxgraph-graph-build/src/lib.rs`
- `crates/oxgraph-graph-build/tests/proptest.rs`
- `crates/oxgraph-graph-build/benches/builder.rs`
- `crates/oxgraph-hyper-build/Cargo.toml`
- `crates/oxgraph-hyper-build/src/lib.rs`
- `crates/oxgraph-hyper-build/tests/proptest.rs`
- `crates/oxgraph-hyper-build/benches/builder.rs`

### Algorithms

- `crates/oxgraph-algo/Cargo.toml`
- `crates/oxgraph-algo/src/lib.rs`
- `crates/oxgraph-algo/src/pagerank.rs`
- `crates/oxgraph-algo/tests/pagerank.rs`
- `crates/oxgraph-algo/tests/transition_fixture.rs`
- `crates/oxgraph-algo/benches/pagerank.rs`

### Umbrella crate

- `crates/oxgraph/Cargo.toml`
- `crates/oxgraph/src/lib.rs`
- `crates/oxgraph/README.md`

### Python facade

- `bindings/python/Cargo.toml`
- `bindings/python/src/lib.rs`
- `bindings/python/SAFETY.md`
- `bindings/python/pyproject.toml`
- `bindings/python/python/oxgraph/__init__.py`
- `bindings/python/tests/test_oxgraph.py`

## Reuse

- `CsrIndex`, `CsrWord`, `CsrSnapshotWord`, `CsrGraph` validation: `crates/oxgraph-csr/src/lib.rs`
- `SnapshotBuilder`, `SnapshotPlan`, typed section view validation: `crates/oxgraph-snapshot/src/container/*`
- Existing topology capability traits: `crates/oxgraph-topology/src/lib.rs`
- Existing graph/hyper vocabulary wrappers: `crates/oxgraph-graph/src/lib.rs`, `crates/oxgraph-hyper/src/lib.rs`
- `IdentityModeRecord`, property encode/validate skeletons, dense/sparse selected-weight adapters: `crates/oxgraph-property/src/lib.rs`
- Builder freeze reorder maps: `crates/oxgraph-graph-build/src/lib.rs`, `crates/oxgraph-hyper-build/src/lib.rs`
- BFS allocation-tier pattern and workspace model: `crates/oxgraph-algo/src/bfs/*`
- Existing PageRank config/report/error/scratch/workspace API ideas: `crates/oxgraph-algo/src/pagerank.rs`
- Existing transition tests as migration fixtures: `crates/oxgraph-algo/tests/transition_fixture.rs`

## Steps

### Phase 0 — stack governance and Shapes updates

- [ ] Create `.shapes/amendments/12-pr-stack-hardening.yaml` summarizing the discovered blockers and the decision to re-cut rather than patch blindly.
- [ ] In amendment 12, update shape 5 CSR graph layout to require separate memory widths and explicit snapshot wire widths.
- [ ] In amendment 12, update shape 6 bipartite-CSR hypergraph layout to require separate vertex, relation, and incidence widths with explicit snapshot wire widths.
- [ ] In amendment 12, update shape 21 snapshot identity/property sections to state that property arrays are keyed by **snapshot-local IDs**, and identity maps are mandatory whenever local and canonical order differ.
- [ ] In amendment 12, update shape 20 construction builders to split core append/freeze builders from Arrow/property/snapshot export helpers.
- [ ] In amendment 12, update shape 22 PageRank algorithms to define visible-state semantics and duplicate-state rejection.
- [ ] Create `.shapes/amendments/13-python-facade-blocked.yaml` for the initial PR #1-#8 deferral and `.shapes/amendments/14-python-facade-review-ready-follow-up.yaml` for the standalone PR #9 review scope.
- [ ] Update `docs/architecture.md` with the corrected dependency graph and snapshot/property/PageRank contracts.
- [ ] Update each PR body after code changes so the review focus matches the new scope.

Completion criteria:

- [ ] `shapes tree shape` shows updated/amended relevant shapes.
- [ ] `docs/architecture.md` describes the final dependency graph without contradicting Cargo manifests.
- [ ] Every changed PR body states what changed, what was removed, and how to verify it.

---

### Phase 1 — PR #1: CSR index widths and snapshot wire format

#### 1.1 Split node/target width from edge/offset width

Replace the single-width CSR public model with separate logical widths:

- `NodeIndex`: node IDs and target entries.
- `EdgeIndex`: edge IDs and offset entries.

Implementation checklist:

- [ ] Change `CsrGraph<'view, Index, StorageWord>` to exactly `CsrGraph<'view, NodeIndex, EdgeIndex, OffsetWord, TargetWord>`.
- [ ] Define `CsrNodeId<NodeIndex>` and `CsrEdgeId<EdgeIndex>`.
- [ ] Require `NodeIndex: CsrIndex` and `EdgeIndex: CsrIndex`.
- [ ] Require `TargetWord: CsrWord<Index = NodeIndex>`.
- [ ] Require `OffsetWord: CsrWord<Index = EdgeIndex>`.
- [ ] Add `pub type CsrNativeGraph<'view, NodeIndex, EdgeIndex> = CsrGraph<'view, NodeIndex, EdgeIndex, EdgeIndex, NodeIndex>` for native borrowed slices.
- [ ] Do not add a single-width default alias such as `CsrU32Graph`; examples must spell the chosen index widths explicitly.
- [ ] Update `validate` so:
  - [ ] `node_count` is `NodeIndex`;
  - [ ] offset monotonicity and final offset use `EdgeIndex`;
  - [ ] target bounds compare `NodeIndex` values against `node_count`;
  - [ ] all slice indexing remains checked through `usize` conversions.
- [ ] Update topology trait impls so element IDs use `NodeIndex` and relation IDs use `EdgeIndex`.
- [ ] Update examples, tests, benches, and Kani proofs for the new type parameters.
- [ ] Add tests for mixed-width memory views, especially `NodeIndex = u32`, `EdgeIndex = u64`.

Completion criteria:

- [ ] CSR still supports `u16`, `u32`, `u64`, and `usize` for native in-memory node/edge indexes.
- [ ] Mixed node/edge widths compile and pass traversal tests.
- [ ] Existing CSR behavior for single-width `u32` remains available by explicitly writing `CsrNativeGraph<'view, u32, u32>`.

#### 1.2 Make CSR snapshot widths self-describing and portable

Snapshot sections must not rely on out-of-band width selection, and persisted snapshots must not use `usize`.

Implementation checklist:

- [ ] Remove `usize` from supported snapshot wire words.
- [ ] Keep `usize` only for native in-memory CSR views.
- [ ] Delete the generic `SNAPSHOT_KIND_CSR_OFFSETS` / `SNAPSHOT_KIND_CSR_TARGETS` constants.
- [ ] Add exactly these width-specific section kinds:
  - [ ] `SNAPSHOT_KIND_CSR_OFFSETS_U16 = 0x0001`
  - [ ] `SNAPSHOT_KIND_CSR_OFFSETS_U32 = 0x0002`
  - [ ] `SNAPSHOT_KIND_CSR_OFFSETS_U64 = 0x0003`
  - [ ] `SNAPSHOT_KIND_CSR_TARGETS_U16 = 0x0004`
  - [ ] `SNAPSHOT_KIND_CSR_TARGETS_U32 = 0x0005`
  - [ ] `SNAPSHOT_KIND_CSR_TARGETS_U64 = 0x0006`
- [ ] Define sealed trait `CsrSnapshotIndex: CsrIndex` with associated `LittleEndianWord`, `OFFSETS_KIND`, and `TARGETS_KIND`.
- [ ] Implement `CsrSnapshotIndex` only for `u16`, `u32`, and `u64`.
- [ ] Do not implement `CsrSnapshotIndex` for `usize`.
- [ ] Define `pub type CsrSnapshotGraph<'view, NodeIndex, EdgeIndex> = CsrGraph<'view, NodeIndex, EdgeIndex, <EdgeIndex as CsrSnapshotIndex>::LittleEndianWord, <NodeIndex as CsrSnapshotIndex>::LittleEndianWord>`.
- [ ] Change `from_snapshot` to read `EdgeIndex::OFFSETS_KIND` for offsets and `NodeIndex::TARGETS_KIND` for targets.
- [ ] Make wrong-width opens fail with missing/width-mismatch errors instead of silently reinterpreting bytes.
- [ ] Update snapshot docs to state that wire widths are encoded in section kinds.
- [ ] Update snapshot tests to cover:
  - [ ] u16/u16
  - [ ] u32/u32
  - [ ] u32/u64
  - [ ] u64/u64
  - [ ] wrong target width rejected
  - [ ] wrong offset width rejected
  - [ ] no `usize` snapshot path exists

Completion criteria:

- [ ] No public snapshot alias or test opens persisted CSR bytes as `usize`.
- [ ] Snapshot readers never need external knowledge of CSR word widths beyond the typed API they requested.
- [ ] The snapshot section kind list is enough for generic tooling to identify payload widths.

#### 1.3 Make BCSR vertex, relation, and incidence widths generic

BCSR must not force `u32` vertex IDs, hyperedge IDs, participant IDs, or offsets.

Implementation checklist:

- [ ] Add sealed `BcsrIndex` implemented for `u16`, `u32`, `u64`, and `usize` for native in-memory views.
- [ ] Add sealed `BcsrSnapshotIndex` implemented only for `u16`, `u32`, and `u64` for persisted snapshots.
- [ ] Change `BcsrVertexId`, `BcsrHyperedgeId`, and `BcsrParticipantId` to `BcsrVertexId<VertexIndex>`, `BcsrHyperedgeId<RelationIndex>`, and `BcsrParticipantId<IncidenceIndex>`.
- [ ] Change `BcsrHypergraph<'view, Word>` to `BcsrHypergraph<'view, VertexIndex, RelationIndex, IncidenceIndex, OffsetWord, VertexWord, RelationWord>`.
- [ ] Require `OffsetWord: BcsrWord<Index = IncidenceIndex>` for all offset arrays.
- [ ] Require `VertexWord: BcsrWord<Index = VertexIndex>` for head/tail participant arrays.
- [ ] Require `RelationWord: BcsrWord<Index = RelationIndex>` for vertex outgoing/incoming hyperedge arrays.
- [ ] Update all hypergraph/topology trait impls so element IDs use `VertexIndex`, relation IDs use `RelationIndex`, and incidence IDs use `IncidenceIndex`.
- [ ] Delete widthless BCSR snapshot section constants.
- [ ] Add width-specific BCSR snapshot constants `0x0020..=0x0037` in this exact order and with these exact name stems plus suffix `_U16`, `_U32`, `_U64`: `SNAPSHOT_KIND_BCSR_HEAD_OFFSETS`, `SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS`, `SNAPSHOT_KIND_BCSR_TAIL_OFFSETS`, `SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS`, `SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS`, `SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES`, `SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS`, `SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES`.
- [ ] Define `BcsrSnapshotHypergraph<'view, VertexIndex, RelationIndex, IncidenceIndex>` using little-endian offset words from `IncidenceIndex`, participant words from `VertexIndex`, and hyperedge words from `RelationIndex`.
- [ ] Update validation, strict cross-checks, tests, benches, examples, and Kani proofs for mixed widths.
- [ ] Add tests for `VertexIndex = u32`, `RelationIndex = u32`, `IncidenceIndex = u64`.

Completion criteria:

- [ ] BCSR native memory views support separate vertex/relation/incidence widths.
- [ ] BCSR persisted snapshots never use `usize` wire words.
- [ ] BCSR wrong-width snapshot opens fail instead of reinterpreting bytes.

---

### Phase 2 — PR #2/#3: architecture docs and topology capabilities

PR #2 and PR #3 stay in the stack. Make the exact clarifying changes listed below and no additional topology traits.

Implementation checklist:

- [ ] Keep `ElementWeight`, `RelationWeight`, and `IncidenceWeight` as semantic-free optional capabilities.
- [ ] Keep canonical/local identity as opt-in per ID family.
- [ ] Clarify in docs that topology weights are one selected view at a time, not a named property registry.
- [ ] Clarify that PageRank and property layers live above topology.
- [ ] In `crates/oxgraph-hyper/src/lib.rs`, replace PageRank-specific motivation in directed hypergraph trait docs with algorithm-neutral wording: “directed traversal consumers” and “source-to-target expansion”.
- [ ] Keep graph/hyper crates no-std and dependency-light.

Completion criteria:

- [ ] `oxgraph-topology`, `oxgraph-graph`, and `oxgraph-hyper` remain `#![no_std]`.
- [ ] Foundation crates do not depend on Arrow, property, snapshot, PyO3, or Python crates.
- [ ] Static-dispatch tests still demonstrate the capability vocabulary without concrete storage dependencies.

---

### Phase 3 — PR #4: property layers and property/identity snapshot sections

#### 3.0 Make property identifiers and indexes generic

Implementation checklist:

- [ ] Replace `pub struct LayerId(pub u64)` with `pub struct LayerId<Id>(pub Id)`; do not provide a default `Id` type parameter.
- [ ] Add sealed trait `PropertyIndex` implemented for `u16`, `u32`, and `u64`.
- [ ] Give `PropertyIndex` associated Arrow unsigned type, little-endian word type, and checked conversions to/from `usize`.
- [ ] Change sparse property layers to store indexes as `PrimitiveArray<I::ArrowType>` where `I: PropertyIndex`, not fixed `UInt64Array`.
- [ ] Change `PropertyLayerDescriptor` to `PropertyLayerDescriptor<Id, I>` where `Id` is the layer ID type and `I: PropertyIndex` is the sparse/logical index width for that layer.
- [ ] Change `PropertyLayer` to `PropertyLayer<Id, I>` and thread those generics through dense/sparse constructors, validation, selected-weight adapters, snapshot encoding, and tests.
- [ ] Add `GraphPropertyLayers<'a, Id, NodeIndex, EdgeIndex>` with `element: &'a [PropertyLayer<Id, NodeIndex>]` and `relation: &'a [PropertyLayer<Id, EdgeIndex>]`.
- [ ] Add `HyperPropertyLayers<'a, Id, VertexIndex, RelationIndex, IncidenceIndex>` with `element`, `relation`, and `incidence` slices using the matching index width for each family.
- [ ] Do not provide `type` aliases that silently choose `u64` IDs or `u64` sparse indexes for Rust users.
- [ ] In examples and tests, spell the chosen ID/index widths explicitly.

Completion criteria:

- [ ] Rust property APIs do not force `u64` layer IDs or `UInt64Array` sparse indexes.
- [ ] Dense and sparse property tests cover `I = u16`, `I = u32`, and `I = u64`.

#### 3.1 Remove redundant/weak Arrow metadata

The current `ArrowValueFamily` abstraction adds bloat without enforcing exact schema fidelity. Remove it from the public API and from snapshot descriptors. Do not replace it with another coarse Arrow-classification enum in this stack.

Implementation checklist:

- [ ] Remove public `ArrowValueFamily`.
- [ ] Remove `arrow_family`, duplicated Arrow field name, and duplicated nullability from property snapshot records.
- [ ] Treat Arrow IPC schema/type information as the source of truth for stored values.
- [ ] Add sealed trait `PropertySnapshotMetaWord` implemented only for `u16`, `u32`, and `u64`.
- [ ] Give `PropertySnapshotMetaWord` associated little-endian word type and section kinds for descriptor and data sections.
- [ ] Add exactly these width-specific section kind pairs:
  - [ ] `SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U16 = 0x0100` and `SNAPSHOT_KIND_PROPERTY_DATA_U16 = 0x0101`;
  - [ ] `SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U32 = 0x0102` and `SNAPSHOT_KIND_PROPERTY_DATA_U32 = 0x0103`;
  - [ ] `SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U64 = 0x0104` and `SNAPSHOT_KIND_PROPERTY_DATA_U64 = 0x0105`.
- [ ] Define `PropertySnapshotRecord<W: PropertySnapshotMetaWord>` with exactly these fields, all using `W::LittleEndianWord`:
  - [ ] `layer_id`;
  - [ ] `name_offset`;
  - [ ] `name_len`;
  - [ ] `id_family`;
  - [ ] `role`;
  - [ ] `storage`;
  - [ ] `missing_policy`;
  - [ ] `logical_len`;
  - [ ] `value_count`;
  - [ ] `value_data_offset`;
  - [ ] `value_data_len`;
  - [ ] `default_data_offset`;
  - [ ] `default_data_len`;
  - [ ] `reserved`.
- [ ] Encode graph property snapshots through `encode_graph_property_snapshot<W, Id, NodeIndex, EdgeIndex>(layers: GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>)` where `W: PropertySnapshotMetaWord`, `Id: TryInto<W> + Copy`, and both index widths implement `PropertyIndex`.
- [ ] Encode hypergraph property snapshots through `encode_hyper_property_snapshot<W, Id, VertexIndex, RelationIndex, IncidenceIndex>(layers: HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>)` where `W: PropertySnapshotMetaWord`, `Id: TryInto<W> + Copy`, and all three index widths implement `PropertyIndex`.
- [ ] Return a typed error when any layer ID, length, string offset, data offset, or count does not fit the selected `W`.
- [ ] During validation, read Arrow IPC payloads and validate exact structural requirements for dense/sparse/default payloads.
- [ ] Delete tests that only check coarse Arrow family classification.
- [ ] Add tests that validate exact Arrow IPC structural behavior for bool/int/float/string property payloads; only int and float fixtures are used with selected-weight adapters.

Completion criteria:

- [ ] Snapshot validation no longer accepts or rejects based on a lossy duplicate Arrow family tag.
- [ ] The descriptor format is generic over the selected metadata word width.
- [ ] Public docs do not advertise Arrow conversion helpers that provide no real guarantee.

#### 3.2 Make identity snapshot sections generic over canonical ID width

Implementation checklist:

- [ ] Replace fixed `IdentityModeRecord` with `IdentityModeRecord<W: PropertySnapshotMetaWord>` where numeric fields use `W::LittleEndianWord`.
- [ ] Rename `IdentityMapMode::ExplicitU32Map` to `IdentityMapMode::ExplicitMap`.
- [ ] Add identity mode section kinds `SNAPSHOT_KIND_IDENTITY_MODES_U16 = 0x0110`, `SNAPSHOT_KIND_IDENTITY_MODES_U32 = 0x0111`, and `SNAPSHOT_KIND_IDENTITY_MODES_U64 = 0x0112`.
- [ ] Add element identity map section kinds `SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U16 = 0x0113`, `_U32 = 0x0114`, and `_U64 = 0x0115`.
- [ ] Add relation identity map section kinds `SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U16 = 0x0116`, `_U32 = 0x0117`, and `_U64 = 0x0118`.
- [ ] Add incidence identity map section kinds `SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U16 = 0x0119`, `_U32 = 0x011A`, and `_U64 = 0x011B`.
- [ ] Export identity maps using the canonical ID width selected by the exporting topology, not a fixed `u32`.
- [ ] Validate identity maps by the section kind's width and reject maps whose record count does not match the identity mode local length.

Completion criteria:

- [ ] Identity snapshot sections do not force `u32` canonical IDs.
- [ ] Tests cover u16, u32, and u64 identity maps.

#### 3.3 Fix sparse default encoding

Sparse default values must not be encoded as a length-one column in the same record batch as N sparse entries.

Implementation checklist:

- [ ] Encode dense layers as one Arrow IPC stream containing one record batch with exactly one column using `descriptor.arrow_field` and length `logical_len`.
- [ ] Encode sparse-null layers as one Arrow IPC stream containing one record batch with exactly two columns:
  - [ ] `Field::new("index", I::ARROW_DATA_TYPE, false)` where `I: PropertyIndex`;
  - [ ] `descriptor.arrow_field` for the values column, length equal to index length.
- [ ] Encode sparse-default layers as two Arrow IPC streams/ranges:
  - [ ] sparse values stream: the same two-column generic-index + descriptor value batch used by sparse-null layers;
  - [ ] default stream: one-column batch using `descriptor.arrow_field` and length 1, non-null.
- [ ] Add `default_data_offset` and `default_data_len` fields to `PropertySnapshotRecord<W>` using `W::LittleEndianWord`.
- [ ] Append property data in deterministic layer order: value stream first, then default stream for sparse-default layers; validate all value/default ranges cover the data section without gaps, overlaps, or trailing bytes.
- [ ] Add tests where sparse default has 0, 1, and many explicit values.
- [ ] Add malformed snapshot tests for missing default stream, default length not 1, default type mismatch, and overlapping ranges.

Completion criteria:

- [ ] `PropertyLayer::try_new_sparse` with default and more than one explicit value can encode and validate.
- [ ] Arrow `RecordBatch::try_new` is never asked to combine unequal-length columns.

#### 3.4 Enforce total selected-weight semantics

A selected weight view is total and non-null for every visible ID it covers.

Implementation checklist:

- [ ] Change selected-weight adapter types to carry property generics explicitly, for example `DenseElementWeights<'view, T, Id, I, P>` and `SparseRelationWeights<'view, T, Id, I, P>`.
- [ ] Dense primitive selected-weight validation continues to reject nulls.
- [ ] Sparse primitive selected-weight validation rejects explicit null values even if the underlying property field is nullable.
- [ ] Sparse primitive selected-weight validation rejects null/default-missing policies when no concrete default exists.
- [ ] Sparse primitive lookup remains `O(log k)` and returns default only for missing indexes, never for explicit nulls.
- [ ] Add tests for nullable sparse property layers that are valid as properties but invalid as selected weights.

Completion criteria:

- [ ] Every `Dense*Weights` and `Sparse*Weights` adapter returns a concrete `Copy` primitive without reading nulls as values.
- [ ] Property-layer validity and selected-weight validity are distinct and tested.

#### 3.5 Add property rekey helpers for snapshot-local exports

Property reordering must be generic over Arrow array types and must not hand-code only f64/int paths.

Implementation checklist:

- [ ] Add workspace dependency `arrow-select = "58.2.0"`.
- [ ] Add normal dependency `arrow-select = { workspace = true }` to `oxgraph-property`.
- [ ] Implement `rekey_layer_to_local<Id, I>(layer: &PropertyLayer<Id, I>, local_to_canonical: &[I]) -> Result<PropertyLayer<Id, I>, PropertyError>` in `oxgraph-property`.
- [ ] For dense layers, implement rekeying with `arrow_select::take::take` over the dense value array using `local_to_canonical` converted to an Arrow take-index array of type `I::ArrowType`.
- [ ] For sparse layers, build a `canonical_to_local: Vec<Option<I>>` from `local_to_canonical`.
- [ ] For sparse layers, remap each explicit canonical sparse index through `canonical_to_local`, sort remapped entries by local index, rebuild the sparse index array, and take values in the sorted remapped order.
- [ ] Preserve sparse default arrays unchanged after validating the default type still matches the value type.
- [ ] Set rekeyed layer logical length to `local_to_canonical.len()`.
- [ ] Return `PropertyError::SparseIndexOutOfBounds` when an explicit canonical sparse index is not present in `canonical_to_local`.
- [ ] Add tests for dense and sparse rekeying over `Int32Array`, `Float64Array`, and `StringArray`.

Completion criteria:

- [ ] Graph and hypergraph snapshot exporters do not duplicate Arrow reordering logic.
- [ ] Rekeying works for non-f64 property payloads.

#### 3.6 Strengthen property proptests

Implementation checklist:

- [ ] Add proptests for duplicate names and duplicate layer IDs.
- [ ] Add proptests for sparse index ordering, bounds, and value/default lookup.
- [ ] Add proptests for snapshot encode/validate roundtrips over generated dense and sparse primitive layers.
- [ ] Add generated malformed snapshot cases for gaps/overlaps/trailing bytes.
- [ ] Add proptests for `rekey_layer_to_local` dense and sparse layers using generated permutations.

Completion criteria:

- [ ] `cargo test -p oxgraph-property --all-features` covers deterministic and generated property/snapshot/rekey cases.

---

### Phase 4 — PR #5/#6: builders, dependency split, and snapshot export correctness

#### 4.1 Split core builders from Arrow/property export

Builders should be append/freeze construction systems first. Arrow/property support is an adapter/export layer, not a mandatory dependency of graph construction.

Implementation checklist:

- [ ] Make `oxgraph-graph-build` core compile without Arrow and without `oxgraph-property`.
- [ ] Make `oxgraph-hyper-build` core compile without Arrow and without `oxgraph-property`.
- [ ] Move direct `arrow-array` and `arrow-schema` dependencies from normal builder dependencies to builder dev-dependencies only.
- [ ] Add sealed `BuildIndex` trait implemented for `u16`, `u32`, `u64`, and `usize` with checked conversions to/from `usize`; do not provide a default index width.
- [ ] Define graph IDs as `GraphNodeId<NodeIndex>` and `GraphEdgeId<EdgeIndex>`.
- [ ] Define unweighted graph builder types `GraphBuilder<NodeIndex, EdgeIndex>` and `FrozenGraph<NodeIndex, EdgeIndex>`.
- [ ] Define weighted graph builder types `WeightedGraphBuilder<NodeIndex, EdgeIndex, EW, RW>` and `FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>`.
- [ ] `GraphBuilder::add_node()` takes no weight and returns `GraphNodeId<NodeIndex>`.
- [ ] `GraphBuilder::add_edge(source, target)` takes no weight and returns `GraphEdgeId<EdgeIndex>`.
- [ ] `WeightedGraphBuilder::add_node(weight: EW)` requires an explicit element weight.
- [ ] `WeightedGraphBuilder::add_edge(source, target, weight: RW)` requires an explicit relation weight.
- [ ] Define hypergraph IDs as `HyperVertexId<VertexIndex>`, `HyperedgeId<RelationIndex>`, and `HyperParticipantId<IncidenceIndex>`.
- [ ] Define unweighted hypergraph builder types `HypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex>` and `FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>`.
- [ ] Define weighted hypergraph builder types `WeightedHypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>` and `FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>`.
- [ ] `HypergraphBuilder::add_vertex()` takes no weight.
- [ ] `HypergraphBuilder::add_hyperedge(sources: &[HyperVertexId<VertexIndex>], targets: &[HyperVertexId<VertexIndex>])` takes no weights.
- [ ] `WeightedHypergraphBuilder::add_vertex(weight: EW)` requires an explicit element weight.
- [ ] `WeightedHypergraphBuilder::add_hyperedge(sources: &[(HyperVertexId<VertexIndex>, IW)], targets: &[(HyperVertexId<VertexIndex>, IW)], relation_weight: RW)` requires explicit incidence and relation weights.
- [ ] Remove `property_layers` fields from all core frozen and builder types.
- [ ] Set `default = []` in `oxgraph-graph-build` and `oxgraph-hyper-build`.
- [ ] Add `snapshot` feature to `oxgraph-graph-build`; it depends on `oxgraph-csr`, `oxgraph-snapshot/alloc`, and `zerocopy`.
- [ ] Add `snapshot` feature to `oxgraph-hyper-build`; it depends on `oxgraph-hyper-bcsr`, `oxgraph-snapshot/alloc`, and `zerocopy`.
- [ ] Add `property-arrow` feature to `oxgraph-graph-build`; it depends on `snapshot` and `oxgraph-property`.
- [ ] Add `property-arrow` feature to `oxgraph-hyper-build`; it depends on `snapshot` and `oxgraph-property`.
- [ ] Snapshot export helpers require their ID/index width parameters to implement the matching snapshot-width traits (`CsrSnapshotIndex`, `BcsrSnapshotIndex`, and `PropertySnapshotMetaWord`); builders using `usize` IDs remain in-memory-only and do not export snapshots.
- [ ] Implement exactly these export helpers:
  - [ ] `export_csr_snapshot<NodeIndex, EdgeIndex>(&FrozenGraph<NodeIndex, EdgeIndex>)` behind `snapshot` for topology-only graph snapshot export;
  - [ ] `export_weighted_csr_snapshot<NodeIndex, EdgeIndex, EW, RW>(&FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>)` behind `snapshot` for topology plus selected weights;
  - [ ] `export_csr_snapshot_with_properties<NodeIndex, EdgeIndex, Id>(&FrozenGraph<NodeIndex, EdgeIndex>, GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>)` behind `property-arrow`;
  - [ ] `export_weighted_csr_snapshot_with_properties<NodeIndex, EdgeIndex, EW, RW, Id>(&FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>, GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>)` behind `property-arrow`;
  - [ ] `export_bcsr_snapshot<VertexIndex, RelationIndex, IncidenceIndex>(&FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>)` behind `snapshot` for topology-only hypergraph snapshot export;
  - [ ] `export_weighted_bcsr_snapshot<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>(&FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>)` behind `snapshot` for topology plus selected weights;
  - [ ] `export_bcsr_snapshot_with_properties<VertexIndex, RelationIndex, IncidenceIndex, Id>(&FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>, HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>)` behind `property-arrow`;
  - [ ] `export_weighted_bcsr_snapshot_with_properties<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW, Id>(&FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>, HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>)` behind `property-arrow`.
- [ ] Make both builder crates `#![no_std]` and add `extern crate alloc`; use `core::error::Error` for error trait impls.

Completion criteria:

- [ ] A user can depend on graph/hyper builders without pulling Arrow.
- [ ] `cargo tree -p oxgraph-graph-build --no-default-features` and `cargo tree -p oxgraph-hyper-build --no-default-features` show no Arrow dependency.
- [ ] Builder examples still show simple append/freeze usage without property concepts.

#### 4.2 Write little-endian snapshot sections

Implementation checklist:

- [ ] Stop writing native `u32` slices with `add_section_typed` for persisted topology sections.
- [ ] Convert graph CSR offsets to `Vec<<EdgeIndex as CsrSnapshotIndex>::LittleEndianWord>` and graph CSR targets to `Vec<<NodeIndex as CsrSnapshotIndex>::LittleEndianWord>` before export.
- [ ] Convert every BCSR offset payload to `Vec<<IncidenceIndex as BcsrSnapshotIndex>::LittleEndianWord>` before export.
- [ ] Convert every BCSR participant payload to `Vec<<VertexIndex as BcsrSnapshotIndex>::LittleEndianWord>` before export.
- [ ] Convert every BCSR hyperedge payload to `Vec<<RelationIndex as BcsrSnapshotIndex>::LittleEndianWord>` before export.
- [ ] Add helper `fn csr_slice_to_le<I: CsrSnapshotIndex>(values: &[I]) -> Vec<I::LittleEndianWord>` for CSR export.
- [ ] Add helper `fn bcsr_slice_to_le<I: BcsrSnapshotIndex>(values: &[I]) -> Vec<I::LittleEndianWord>` for BCSR export.
- [ ] Add helper `fn identity_slice_to_le<I: PropertySnapshotMetaWord>(values: &[I]) -> Vec<I::LittleEndianWord>` for identity-map export.
- [ ] Add tests that inspect exported bytes for known little-endian values.
- [ ] Add roundtrip tests that would fail on a big-endian target if native bytes were used.

Completion criteria:

- [ ] Every persisted integer topology section is written in documented little-endian form.
- [ ] No snapshot writer relies on host endian for wire bytes.

#### 4.3 Reorder graph relation properties for CSR snapshot-local IDs

Property layers in snapshots are keyed by snapshot-local IDs. Graph relation property layers attached in canonical edge-ID order must be reordered when CSR local edge order differs.

Implementation checklist:

- [ ] Compute the CSR local-to-canonical relation map (`edge_ids: Vec<EdgeIndex>`) during graph freeze/export and store it in the snapshot-export local scope, not in the core frozen graph API.
- [ ] Export the relation identity map using the section kind selected by `EdgeIndex`.
- [ ] For every relation property layer, call `oxgraph_property::rekey_layer_to_local(layer, &edge_ids)` before encoding.
- [ ] The rekeyed local index `i` must read the original canonical value at edge ID `edge_ids[i]`.
- [ ] Element property layers need no reorder when element local == canonical.
- [ ] Reject property layers whose logical length is too short before export with `GraphBuildError::PropertyLayerTooShort`.
- [ ] Add deterministic test with insertion order different from CSR order and relation property values that prove reordering.
- [ ] Add proptest over generated graphs that exported relation properties match canonical values through the identity map.

Completion criteria:

- [ ] Opening an exported CSR snapshot and selecting relation weights reads the correct canonical edge values through the snapshot-local CSR edge IDs.
- [ ] Dense and sparse relation property exports are both tested.

#### 4.4 Fix hypergraph incidence identity/property mapping for BCSR snapshots

BCSR snapshot-local participant IDs differ from frozen builder canonical participant IDs. Export must record explicit incidence identity and reorder incidence properties.

Implementation checklist:

- [ ] Compute BCSR snapshot-local incidence order during export:
  - [ ] all head/source incidences in BCSR order;
  - [ ] then all tail/target incidences in BCSR order.
- [ ] Build `local_incidence_to_canonical_participant: Vec<IncidenceIndex>` for property rekeying.
- [ ] Build `local_incidence_to_canonical_participant_le: Vec<<IncidenceIndex as PropertySnapshotMetaWord>::LittleEndianWord>` from the same values for snapshot identity export.
- [ ] Export `IdentityModeRecord::<IncidenceIndex>::explicit_map(IdFamily::Incidence, len)` for incidence family.
- [ ] Export the incidence identity map section kind selected by `IncidenceIndex` (`SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U16`, `_U32`, or `_U64`) with `local_incidence_to_canonical_participant_le`.
- [ ] Export element identity as `LocalEqualsCanonical` and relation identity as `LocalEqualsCanonical`; this stack does not reorder BCSR vertices or hyperedges.
- [ ] For every incidence property layer, call `oxgraph_property::rekey_layer_to_local(layer, &local_incidence_to_canonical_participant)` before encoding.
- [ ] Add deterministic tests with multiple hyperedges and mixed source/target participant counts that prove identity map and incidence property values are correct.
- [ ] Add proptests over generated hypergraphs that exported incidence properties match canonical values through the explicit identity map.

Completion criteria:

- [ ] BCSR exported snapshots always declare incidence `ExplicitMap` using the selected `IncidenceIndex` width in this stack.
- [ ] Incidence properties survive export/open/selection without value corruption.

#### 4.5 Keep builder API intentionally narrow

Implementation checklist:

- [ ] Keep append/freeze/update-weight semantics.
- [ ] Do not add deletion, tombstones, ID reuse, compaction, overlay mutation, or stale borrowed views.
- [ ] Remove `generation` fields and `generation()` methods from both builders because this stack does not expose generation-checked caches.

Completion criteria:

- [ ] Builder public API remains a construction layer, not a mutation framework.
- [ ] Every public builder field/method has a demonstrated use in tests or examples.

---

### Phase 5 — PR #7: PageRank correctness, tiers, and verification

#### 5.1 Define and implement visible-state semantics

Use induced visible-state PageRank semantics:

- The caller-provided `elements` and `relations` define the visible state set.
- Duplicate visible states are errors.
- Transitions to invisible states are ignored.
- A row with no visible outgoing targets is a dangling row.
- Weighted row totals sum only visible outgoing targets.

Implementation checklist:

- [ ] Add `DuplicateElement` and `DuplicateRelation` error variants.
- [ ] Change `PageRankScratch<'scratch, S>` to contain `teleport: &mut [S]`, `next: &mut [S]`, and `visible_elements: &mut [u8]`.
- [ ] Change `HypergraphPageRankScratch<'scratch, S>` to contain `teleport: &mut [S]`, `next_elements: &mut [S]`, `next_relations: &mut [S]`, `visible_elements: &mut [u8]`, and `visible_relations: &mut [u8]`.
- [ ] During validation, clear visible buffers over the required bounds to `0`, set visible entries to `1`, and treat an existing `1` as a duplicate-state error.
- [ ] Extend `PageRankWorkspace` and `HypergraphPageRankWorkspace` with owned `Vec<u8>` visible buffers behind the `alloc` feature.
- [ ] Validate graph visible elements before initialization:
  - [ ] index in bounds;
  - [ ] no duplicates;
  - [ ] at least one visible element.
- [ ] Validate hypergraph visible elements and relations before initialization:
  - [ ] indexes in bounds;
  - [ ] no duplicates in either family;
  - [ ] combined visible state count > 0.
- [ ] In graph unweighted push, count only outgoing edges whose target is visible.
- [ ] In graph weighted push, sum and distribute only weights for outgoing edges whose target is visible.
- [ ] In hypergraph element -> relation push, consider only visible outgoing relations.
- [ ] In hypergraph relation -> element push, consider only target incidences whose target element is visible.
- [ ] Document the personalization layout for hypergraph as `[element_index..., element_bound + relation_index...]`.

Completion criteria:

- [ ] Total rank mass remains 1 within tolerance for generated visible subsets.
- [ ] Duplicate state inputs return typed errors.
- [ ] Invisible-target transitions no longer leak mass.

#### 5.2 Split no-alloc scratch/core from alloc conveniences

Implementation checklist:

- [ ] Make `pagerank` module compile without the `alloc` feature for borrowed scratch APIs.
- [ ] Keep allocating convenience functions and owned workspace types behind `alloc`.
- [ ] Mirror BFS tier documentation:
  - [ ] base no-alloc: caller-provided scratch;
  - [ ] `alloc`: allocating and reusable workspace APIs;
  - [ ] no `std` feature for PageRank in this stack.
- [ ] Update `crates/oxgraph-algo/src/lib.rs` exports accordingly.
- [ ] Add `cargo check -p oxgraph-algo --no-default-features` and `--features alloc` to verification docs.

Completion criteria:

- [ ] Borrowed scratch PageRank compiles without `alloc`.
- [ ] Allocating wrappers are unavailable unless the `alloc` feature is enabled.

#### 5.3 Document scalar laws and remove scalar defaults

Implementation checklist:

- [ ] Keep `PageRankScalar` publicly implementable; do not seal it.
- [ ] Add a `# Scalar laws` rustdoc section to `PageRankScalar` requiring additive/multiplicative identities, finite-preserving arithmetic for valid inputs, positive `from_usize` for non-zero values, total behavior for comparisons used by validation, and `abs(x) >= 0`.
- [ ] Remove default type parameters from `PageRankConfig`, `PageRankReport`, `PageRankError`, `PageRankWorkspace`, and `HypergraphPageRankWorkspace`.
- [ ] Remove `Default for PageRankConfig`; Rust callers must pass explicit damping, tolerance, and max iterations.
- [ ] Keep `IntoPageRankScalar<S>` generic for primitive topology weights.
- [ ] Implement `IntoPageRankScalar<f32>` for `f64` using an explicit lossy cast with a documented clippy expectation.
- [ ] Add tests for built-in `f32` and `f64` scalar behavior.
- [ ] Add one compile-pass test with a small downstream custom scalar implementing `PageRankScalar` to prove the API is not locked to `f32`/`f64`.

Completion criteria:

- [ ] Public scalar API is generic and has explicit laws.
- [ ] Rust PageRank APIs do not default to `f64`.
- [ ] Tests cover f32, f64, custom scalar compilation, and primitive weight conversions.

#### 5.4 Add PageRank proptests and stronger benches

Implementation checklist:

- [ ] Add graph PageRank proptests for generated directed graphs:
  - [ ] mass conservation;
  - [ ] non-negative finite ranks;
  - [ ] duplicate rejection;
  - [ ] invisible target handling;
  - [ ] weighted row normalization;
  - [ ] scratch/workspace/alloc equivalence.
- [ ] Add hypergraph PageRank proptests for generated directed hypergraphs:
  - [ ] element+relation mass conservation;
  - [ ] visible subset behavior;
  - [ ] duplicate element/relation rejection;
  - [ ] weighted incidence behavior;
  - [ ] scratch/workspace/alloc equivalence.
- [ ] Replace the regular stationary benchmark fixture with a non-regular directed graph and non-uniform personalization so the benchmark performs more than one power iteration.
- [ ] Benchmark scratch/workspace paths separately from allocating wrappers.
- [ ] Keep convergence-failure tests deterministic.

Completion criteria:

- [ ] PageRank benchmarks run more than one iteration for at least one benchmark case.
- [ ] Generated PageRank tests would have caught the visible-state mass leak.

---

### Phase 6 — PR #8: umbrella crate curation

Implementation checklist:

- [ ] Remove wildcard `pub use oxgraph_*::*` re-export modules.
- [ ] Replace wildcard modules with explicit curated re-export lists for every feature module.
- [ ] Rename umbrella feature `property` to `property-arrow`.
- [ ] Make `graph-build` and `hyper-build` depend only on core builder crates and not on snapshot/property features.
- [ ] Add umbrella feature `graph-snapshot = ["graph-build", "csr", "snapshot-alloc", "oxgraph-graph-build/snapshot"]`.
- [ ] Add umbrella feature `hyper-snapshot = ["hyper-build", "hyper-bcsr", "snapshot-alloc", "oxgraph-hyper-build/snapshot"]`.
- [ ] Add umbrella feature `graph-property-arrow = ["graph-snapshot", "property-arrow", "oxgraph-graph-build/property-arrow"]`.
- [ ] Add umbrella feature `hyper-property-arrow = ["hyper-snapshot", "property-arrow", "oxgraph-hyper-build/property-arrow"]`.
- [ ] Set umbrella `default = []`; users must opt into `topology`, `graph`, `hyper`, layouts, algorithms, builders, and property features explicitly.
- [ ] Add compile checks for:
  - [ ] `oxgraph --no-default-features`;
  - [ ] default feature set is empty and compiles;
  - [ ] `topology` only;
  - [ ] `graph`;
  - [ ] `hyper`;
  - [ ] `csr`;
  - [ ] `hyper-bcsr`;
  - [ ] `algo` no-alloc;
  - [ ] `algo-alloc`;
  - [ ] `property-arrow`;
  - [ ] `graph-snapshot`;
  - [ ] `hyper-snapshot`;
  - [ ] `graph-property-arrow`;
  - [ ] `hyper-property-arrow`;
  - [ ] `full`.

Completion criteria:

- [ ] `oxgraph` is a curated entrypoint, not an accidental mirror of every internal crate root.
- [ ] Enabling a feature has an obvious dependency cost.

---

### Phase 7 — PR #9: Python facade follow-up handling

PR #9 is reviewed after the Rust hardening stack as a standalone Python facade branch. No Python code lands in PR #1-#8, and the Python package stays outside the root Rust workspace member set.

Implementation checklist for the current stack:

- [ ] Mark PR #9 ready for review after `just python-ci` passes.
- [ ] Remove `oxgraph-python` from the active merge stack for PR #1-#8.
- [ ] Do not include `oxgraph-python` in root `Cargo.toml` workspace members for the PR #1-#8 merge path.
- [ ] Remove Python/PyO3 lockfile additions from the PR #1-#8 merge path.
- [ ] Update `docs/architecture.md` to state that Python is a follow-up facade after Rust contracts stabilize.
- [ ] Add a Python-facade Shapes amendment that records the review-ready standalone facade scope.

Required design for the Python follow-up PR:

- [ ] Place bindings under `bindings/python` with a standalone `Cargo.toml` and `pyproject.toml`; do not add it to the Rust workspace default member set.
- [ ] Add `just python-build` with exact command: `cd bindings/python && uv run maturin develop`.
- [ ] Add `just python-test` with exact command: `cd bindings/python && uv run pytest tests`.
- [ ] Add `just python-ci` that runs `python-build` then `python-test`.
- [ ] Add `just python-unsafe-check` with exact command `! rg -n "\\bunsafe\\b" bindings/python/src` and include it in `just python-ci`.
- [ ] Keep `unsafe_code = "allow"` only in the Python crate manifest and document in `SAFETY.md` that it exists solely for PyO3 macro expansion.
- [ ] Expose only these first Python classes/functions: `GraphBuilder`, `FrozenGraph`, `HypergraphBuilder`, `FrozenHypergraph`, `open_snapshot`, `open_csr_snapshot`, `open_bcsr_snapshot`, typed exceptions, BFS methods, and PageRank methods.
- [ ] Do not expose Python property-layer classes until property snapshots and builder property export have landed and have Python tests.
- [ ] Use string labels only in the first Python facade and document that labels are Python-facade-owned strings, not Rust topology IDs.
- [ ] Set Python package license metadata to `MIT` to match the Rust crate metadata.

Completion criteria:

- [ ] PR #1-#8 merge without PyO3 or Python build requirements.
- [ ] PR #9 can be reviewed independently after PR #1-#8 without adding PyO3 to the Rust workspace.
- [ ] The Python acceptance criteria are explicit and testable.

---

## Verification

### Required local gates after each re-cut PR

- [ ] `cargo +nightly fmt --all -- --check`
- [ ] `taplo format --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo deny --all-features check advisories bans sources`
- [ ] `cargo test --workspace --all-features`

### Required feature/no-std gates

- [ ] `cargo check -p oxgraph-topology --no-default-features`
- [ ] `cargo check -p oxgraph-graph --no-default-features`
- [ ] `cargo check -p oxgraph-hyper --no-default-features`
- [ ] `cargo check -p oxgraph-snapshot --no-default-features`
- [ ] `cargo check -p oxgraph-snapshot --no-default-features --features alloc`
- [ ] `cargo check -p oxgraph-csr --no-default-features`
- [ ] `cargo check -p oxgraph-hyper-bcsr --no-default-features`
- [ ] `cargo check -p oxgraph-algo --no-default-features`
- [ ] `cargo check -p oxgraph-algo --no-default-features --features alloc`
- [ ] `cargo check -p oxgraph --no-default-features`
- [ ] `cargo check -p oxgraph --no-default-features --features full`

### Required targeted tests

- [ ] `cargo test -p oxgraph-csr --all-features`
- [ ] `cargo test -p oxgraph-hyper-bcsr --all-features`
- [ ] `cargo test -p oxgraph-property --all-features`
- [ ] `cargo test -p oxgraph-graph-build --all-features`
- [ ] `cargo test -p oxgraph-hyper-build --all-features`
- [ ] `cargo test -p oxgraph-algo --all-features --test pagerank --test transition_fixture`

### Required property/proptest coverage

- [ ] CSR mixed-width proptests cover node/edge width separation.
- [ ] BCSR mixed-width proptests cover vertex/relation/incidence width separation.
- [ ] Property sparse/default proptests cover encode/validate/lookup for `I = u16`, `I = u32`, and `I = u64`.
- [ ] Builder proptests cover `u16`, `u32`, and `u64` builder index widths and verify identity maps/property reordering.
- [ ] PageRank proptests verify mass conservation, visible-state semantics, duplicate rejection, and API-tier equivalence.

### Required benches

- [ ] `cargo bench -p oxgraph-csr`
- [ ] `cargo bench -p oxgraph-property`
- [ ] `cargo bench -p oxgraph-graph-build`
- [ ] `cargo bench -p oxgraph-hyper-build`
- [ ] `cargo bench -p oxgraph-algo --features std`

### Required heavy verification before final merge

- [ ] `cargo kani --workspace` for crates with Kani harnesses.
- [ ] Keep Kani skips only for `oxgraph-property`, `oxgraph-graph-build`, `oxgraph-hyper-build`, and PageRank; each skip comment must name the exact proptest/fixture/bench files that cover the skipped contract.
- [ ] `cargo +nightly miri test --workspace` for the PR #1-#8 Rust workspace.

### Required Python gate for PR #9

- [ ] `just python-ci` from the Python follow-up plan.
- [ ] Python tests run from a clean environment and build/import the native extension.
- [ ] Python package metadata license is `MIT`.

## Final completion checklist

The PR stack is ready for merge only when all items below are true:

- [ ] Public Rust APIs added by this stack have no default generic type parameters.
- [ ] Public Rust APIs added by this stack do not choose default ID widths, index widths, or numeric scalar types for users.
- [ ] Foundation crates remain no-std and have no Arrow/PyO3/property dependency leakage.
- [ ] CSR memory views support mixed node/edge widths.
- [ ] BCSR memory views support mixed vertex/relation/incidence widths.
- [ ] CSR snapshot wire widths are self-describing and never use `usize`.
- [ ] Snapshot writers emit explicit little-endian payloads.
- [ ] Property Rust APIs are generic over layer ID and sparse index widths.
- [ ] Property snapshot descriptors are generic over selected metadata word width.
- [ ] Identity snapshot maps are generic over selected canonical ID width.
- [ ] Property snapshots do not duplicate weak Arrow metadata.
- [ ] Sparse default property encoding works for any explicit value count.
- [ ] Selected weight adapters reject nulls and expose total concrete weights.
- [ ] Graph relation properties are reordered into CSR snapshot-local order.
- [ ] Hypergraph incidence identity uses an explicit map when BCSR local order differs.
- [ ] Hypergraph incidence properties are reordered into BCSR snapshot-local order.
- [ ] Builder core APIs do not force Arrow dependencies.
- [ ] PageRank has no visible-state mass leak.
- [ ] PageRank rejects duplicate visible states.
- [ ] PageRank borrowed scratch APIs compile without `alloc`.
- [ ] PageRank scalar API is generic, law-documented, and has no default scalar type.
- [ ] PageRank has proptests for mass, visibility, weights, and tier equivalence.
- [ ] Umbrella re-exports are curated and feature dependency costs are explicit.
- [ ] Python PR #9 is standalone, review-ready, and not merged in PR #1-#8.
- [ ] All PR descriptions and docs match the implemented architecture.
- [ ] All verification commands listed above pass.

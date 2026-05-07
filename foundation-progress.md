# Foundation Progress

This document tracks implementation progress against `vision.md`. It records what has been built, what was intentionally deferred, and where the implementation has diverged from the original plan based on design pressure.

## Current Slice

The workspace now has the neutral topology substrate, graph and hypergraph specializations, concrete graph and hypergraph layouts, a substrate-agnostic algorithm crate, and a topology-agnostic snapshot container:

```text
crates/oxgraph-topology
crates/oxgraph-graph
crates/oxgraph-hyper
crates/oxgraph-csr
crates/oxgraph-hyper-bcsr
crates/oxgraph-algo
crates/oxgraph-snapshot
```

`oxgraph-topology` is a `no_std` crate that defines storage-agnostic read-view traits for discrete topology. It does not define concrete graph, hypergraph, snapshot, layout, storage, or domain types.

`oxgraph-graph` is a `no_std` crate that defines storage-agnostic read-view traits for ordinary binary graphs. It introduces node, edge, endpoint, outgoing, incoming, and direct-neighbor traversal vocabulary without defining concrete storage or forcing graph traversal through incidence APIs. It also re-exports the `oxgraph-topology` trait surface so downstream graph implementation crates only depend on `oxgraph-graph`.

`oxgraph-hyper` is a `no_std` crate that defines storage-agnostic read-view traits for hypergraphs. It introduces vertex, hyperedge, participant, incident-hyperedge, directed participant-set, and directed vertex-expansion vocabulary while mapping those concepts onto `oxgraph-topology` elements, relations, incidences, and roles. The wrapper layer is symmetric with topology — every topology trait used by hypergraph backends has a hypergraph-facing wrapper with a delegating default and a blanket impl, and the topology trait surface is re-exported so downstream hypergraph implementation crates only depend on `oxgraph-hyper`.

`oxgraph-csr` is a `no_std` crate that defines a borrowed CSR graph view. It validates CSR offsets and targets, implements outgoing edge and direct-neighbor graph traversal, and intentionally does not provide incoming traversal without a CSC or reverse index. It depends on `oxgraph-graph` and `oxgraph-snapshot` only — `oxgraph-topology` is reached through the `oxgraph-graph` re-export.

`oxgraph-hyper-bcsr` is a `no_std` crate that defines a borrowed bipartite-CSR hypergraph view (the hypergraph counterpart to `oxgraph-csr`). It depends on `oxgraph-hyper` and `oxgraph-snapshot` only — `oxgraph-topology` is reached through the `oxgraph-hyper` re-export. It validates eight section payloads — head/tail offsets and participants per hyperedge, plus outgoing/incoming offsets and hyperedge IDs per vertex — and implements every read-view trait in `oxgraph-hyper` (including `DirectedHyperedgeParticipants`, `DirectedVertexSuccessors`, `DirectedVertexPredecessors`, and the `Hypergraph` bundle) with `O(degree)` traversal in either direction. Validation is tiered through `BcsrValidation` (`Layout` for cheap structural invariants; `Strict` adds cross-direction multiset equality between the hyperedge-major and vertex-major indexes). v1.0 caps each of `vertex_count`, `hyperedge_count`, and per-direction participant totals at `2³² − 1`; section kinds `0x0018..=0x001F` are reserved for future `u64`-offset variants.

`oxgraph-algo` is a `no_std` crate with optional `alloc` and `std` traversal tiers. It defines substrate-agnostic graph algorithms over `oxgraph-topology` traits — BFS runs unchanged on `oxgraph-csr::CsrGraph` and `oxgraph-hyper-bcsr::BcsrHypergraph` (and any future view that exposes the required topology capabilities). Both forward and reverse BFS ship for every storage tier (`scratch`, `epoch_scratch`, `workspace`, `indexed`, `generic_btree`, `generic_hash`); forward variants bind on `ElementSuccessors`, reverse variants bind on `ElementPredecessors`. A single direction-parameterized `Bfs<…, D>` driver in `bfs/core.rs` carries either direction at zero runtime cost — the sealed `ExpansionDirection` policy monomorphizes to one direct call into `element_successors` or `element_predecessors` per yield. The reusable `BfsEpochScratch` and `BfsWorkspace` types are direction-agnostic and serve both traversal directions over the same view.

`oxgraph-snapshot` is a `no_std` crate that defines a topology-agnostic byte-level snapshot container (format v1.0). It validates a fixed header and a sectioned payload region, exposes borrowed section bytes via [`Snapshot::sections`] / [`Snapshot::section`], and lets consumers reinterpret payloads as typed slices via [`Section::try_as_slice`] (with actual-pointer alignment checked at the call site, not on the producer's declared metadata). The container holds no domain semantics — section kinds are opaque `u32`s registered by upper layers (e.g. `oxgraph-csr` defines `SNAPSHOT_KIND_CSR_OFFSETS` / `SNAPSHOT_KIND_CSR_TARGETS`). A no-`alloc` writer (`SnapshotPlan::encoded_len` / `write_into`) and an alloc-gated owning builder (`SnapshotBuilder`) emit identical bytes. Validation is tiered through [`ValidationLevel`] (`Header`, `SectionTable`, `Layout`); [`Snapshot::open`] defaults to `Layout` (O(s²) duplicate-kind walk) and [`Snapshot::open_with`] lets callers select a cheaper level. The format is explicitly not a stable ABI yet.

Current dependency hierarchy:

```text
oxgraph-topology
├── oxgraph-graph
│   └── oxgraph-csr
│       └── oxgraph-snapshot
├── oxgraph-hyper
│   └── oxgraph-hyper-bcsr
│       └── oxgraph-snapshot
└── oxgraph-algo

oxgraph-snapshot
```

`oxgraph-topology` has no direct downstream impl-crate consumers: `oxgraph-csr` lists only `oxgraph-graph` and `oxgraph-snapshot`; `oxgraph-hyper-bcsr` lists only `oxgraph-hyper` and `oxgraph-snapshot`. Both wrapper crates `pub use` the topology trait surface so impl crates can `use oxgraph_graph::ElementIndex` (or `oxgraph_hyper::IncidenceElement`, etc.) without naming `oxgraph-topology` in their `Cargo.toml` or source. `oxgraph-algo` is the one crate that depends on `oxgraph-topology` directly — it is substrate-agnostic by design and does not pick a domain (graph vs. hypergraph) at the trait-bound level.

`oxgraph-snapshot` is the topology-agnostic container; it has no graph or hypergraph deps. `oxgraph-csr` depends on `oxgraph-snapshot` to expose `CsrSnapshotGraph::<Index>::from_snapshot`, which reads typed little-endian CSR offsets and targets from snapshot sections without copying. `oxgraph-hyper-bcsr` likewise depends on `oxgraph-snapshot` to expose `BcsrHypergraph::from_snapshot`, which reads the eight bipartite-CSR sections without copying. The snapshot crate's own tests, benches, and example exercise only the agnostic surface (header parsing, section table, builder/reader roundtrip, typed-slice views).

## Vision Alignment

The current implementation directly supports these `vision.md` principles:

- **Topology here. Meaning elsewhere. Storage anywhere.**
- **Views, not ownership.**
- **Capabilities over assumptions.**
- **`no_std` core.**
- **Graph and hypergraph neutrality at the deepest layer.**
- **No domain semantics in the foundation.**
- **The ordinary directed graph hot path remains graph-specific.**

The implemented API models the shared vocabulary from the vision:

- **Element**: an item that participates in topology.
- **Relation**: a connection or higher-order relation among elements.
- **Incidence**: one element's participation in one relation.
- **Role**: implementation-defined participation metadata for an incidence.

## Implemented

### Workspace Restructure

The old context-layer skeleton was removed and replaced with `oxgraph-topology`.

This matches the vision's layer-zero direction more closely than preserving the earlier context-layer crate name and placeholder schema API.

### `oxgraph-topology`

Implemented traits:

- `TopologyId`
- `TopologyBase`
- `IncidenceBase`
- `TopologyCounts`
- `IncidenceCounts`
- `ElementIndex`
- `RelationIndex`
- `IncidenceIndex`
- `ContainsElement`
- `ContainsRelation`
- `ContainsIncidence`
- `RelationIncidences`
- `ElementIncidences`
- `IncidenceElement`
- `IncidenceRelation`
- `IncidenceRole`
- `RelationIncidenceCount`
- `ElementIncidenceCount`
- `IncidenceView`

The traits use associated types for IDs and roles. No concrete ID types are defined in the crate. `TopologyBase` only requires element and relation IDs; incidence IDs and roles live behind `IncidenceBase` so graph-only views are not forced to expose incidence concepts.

Traversal uses generic associated iterator types so implementations can return concrete iterators with static dispatch instead of `Box<dyn Iterator>`.

### `oxgraph-graph`

Implemented graph-facing aliases:

- `NodeId`
- `EdgeId`
- `EndpointId`
- `EndpointRole`

Implemented traits:

- `GraphBase`
- `GraphCounts`
- `NodeIndex`
- `EdgeIndex`
- `EndpointIndex`
- `ContainsNode`
- `ContainsEdge`
- `ContainsEndpoint`
- `EdgeSourceGraph`
- `EdgeTargetGraph`
- `EdgeEndpointGraph`
- `OutgoingGraph`
- `IncomingGraph`
- `OutgoingNeighborsGraph`
- `IncomingNeighborsGraph`
- `OutgoingEdgeCount`
- `IncomingEdgeCount`
- `DirectedGraph`
- `ForwardGraph`
- `ReverseGraph`

`oxgraph-graph` also `pub use`s the `oxgraph-topology` trait surface (`TopologyId`, `TopologyBase`, `IncidenceBase`, `ElementIndex`, `RelationIndex`, `IncidenceIndex`, `ContainsElement`, `ContainsRelation`, `ContainsIncidence`, `TopologyCounts`, `IncidenceCounts`, `RelationIncidences`, `ElementIncidences`, `IncidenceElement`, `IncidenceRelation`, `IncidenceRole`, `RelationIncidenceCount`, `ElementIncidenceCount`) so downstream graph implementation crates can name those traits without depending on `oxgraph-topology` directly.

Graph identity is not duplicated. `NodeId` and `EdgeId` are graph-facing aliases for `TopologyBase::ElementId` and `TopologyBase::RelationId`. `EndpointId` and `EndpointRole` are graph-facing aliases for `IncidenceBase::IncidenceId` and `IncidenceBase::Role` when a graph view also exposes incidence capabilities.

The graph traversal traits extend `TopologyBase` directly. Graph nodes map to topology elements and graph edges map to topology relations, while traversal remains separate from incidence traversal so ordinary graph users can stay on node/edge APIs without defining endpoint-incidence IDs.

No concrete graph, node, edge, CSR, CSC, COO, snapshot, builder, mutation, or payload types are defined in the crate.

Traversal uses generic associated iterator types so implementations can return concrete iterators with static dispatch instead of `Box<dyn Iterator>`.

ID containment is modeled as an optional capability instead of being folded into dense indexing. This keeps `node_bound` / `element_bound` as allocation bounds while giving boundary code a way to ask whether an externally supplied ID is valid and visible in a view.

### `oxgraph-hyper`

Implemented hypergraph-facing aliases:

- `VertexId`
- `HyperedgeId`
- `ParticipantId`
- `ParticipantRole`

Implemented traits:

- `HypergraphBase`
- `ParticipantBase`
- `HypergraphCounts`
- `ParticipantCounts`
- `VertexIndex`
- `HyperedgeIndex`
- `ParticipantIndex`
- `ContainsVertex`
- `ContainsHyperedge`
- `ContainsParticipant`
- `HyperedgeIncidences`
- `VertexIncidences`
- `ParticipantVertex`
- `ParticipantHyperedge`
- `ParticipantRoleOf`
- `HyperedgeParticipants`
- `IncidentHyperedges`
- `HyperedgeParticipantCount`
- `IncidentHyperedgeCount`
- `DirectedHyperedgeParticipants`
- `DirectedVertexSuccessors`
- `DirectedVertexPredecessors`
- `Hypergraph`

Every wrapper extends a topology trait, has a default-delegating method (or no methods, for naming-only base traits), and is provided to every topology impl through a blanket impl. The trait DAG roots in hypergraph wrappers: `HyperedgeParticipants` extends `HyperedgeIncidences + ParticipantVertex` and `IncidentHyperedges` extends `VertexIncidences + ParticipantHyperedge`, so backends only need topology trait impls and the wrapper hierarchy comes free.

`oxgraph-hyper` also `pub use`s the same `oxgraph-topology` trait surface that `oxgraph-graph` re-exports, so downstream hypergraph implementation crates (e.g. `oxgraph-hyper-bcsr`) can name those traits without depending on `oxgraph-topology` directly.

Hypergraph identity is not duplicated. `VertexId` / `HyperedgeId` / `ParticipantId` / `ParticipantRole` alias `TopologyBase::ElementId` / `TopologyBase::RelationId` / `IncidenceBase::IncidenceId` / `IncidenceBase::Role`. No concrete hypergraph, vertex, hyperedge, participant, layout, snapshot, builder, or payload types are defined in the crate.

### Examples

Every added crate is expected to include executable examples.

`oxgraph-topology` currently includes:

- `examples/directed_graph.rs`: models a directed graph through topology traits.
- `examples/hypergraph.rs`: models an oriented hypergraph through topology traits.

`oxgraph-graph` currently includes:

- `examples/graph_directed.rs`: models a directed graph through graph-specific node and edge traits.

`oxgraph-hyper` currently includes:

- `examples/hyper_directed.rs`: models a directed hypergraph through hypergraph-specific participant traits.

`oxgraph-csr` currently includes:

- `examples/csr_directed.rs`: validates borrowed CSR slices and traverses outgoing edges through graph traits.

`oxgraph-hyper-bcsr` currently includes:

- `examples/bcsr_directed.rs`: validates borrowed bipartite-CSR slices and traverses head, tail, incident hyperedges, and directed successors through hypergraph traits.
- `examples/open_bcsr_snapshot.rs`: builds a bipartite-CSR snapshot using `oxgraph-hyper-bcsr`'s eight registered section kinds, opens it via `BcsrHypergraph::from_snapshot`, and walks it without heap reconstruction.

`oxgraph-algo` currently includes:

- `examples/bfs.rs`: runs forward BFS over a borrowed CSR graph through topology trait bounds.

`oxgraph-snapshot` currently includes:

- `examples/pack_two_sections.rs`: builds a snapshot containing one little-endian `u32` payload and one byte payload, opens it, and views both back through `try_as_slice` / `bytes`. The example contains no graph or hypergraph code.

`oxgraph-csr` currently includes:

- `examples/open_snapshot.rs`: builds a `u32` CSR snapshot using `oxgraph-csr`'s registered section kinds, opens it, validates the CSR layout via `CsrSnapshotGraph::<u32>::from_snapshot`, and runs BFS over the borrowed sections without heap reconstruction.

The examples are intentionally educational. They are not optimized graph or hypergraph storage implementations.

### Tests

`tests/static_dispatch.rs` defines a small concrete topology fixture and verifies:

- generic consumers can use the trait surface through static dispatch;
- relation traversal can resolve participating elements;
- incidences can resolve their relation, element, and role;
- exact relation-incidence counts match traversal counts;
- dense element, relation, and incidence indexes stay within bounds and are distinct for visible IDs.

`oxgraph-graph` includes `tests/static_dispatch.rs`, which defines a small concrete directed graph fixture and verifies:

- generic consumers can use outgoing and incoming traversal through static dispatch;
- generic consumers can use direct outgoing and incoming neighbor traversal through static dispatch;
- endpoint lookup resolves sources and targets;
- exact in-degree and out-degree counts match traversal counts;
- combined endpoint lookup agrees with individual source and target lookup;
- graph-facing node, edge, and endpoint index aliases delegate to topology index capabilities.

`oxgraph-hyper` includes `tests/static_dispatch.rs`, which defines a small directed hypergraph fixture and verifies:

- generic consumers can use hyperedge participant traversal through static dispatch;
- generic consumers can use incident hyperedge traversal through static dispatch;
- hypergraph-facing counts map to topology counts;
- exact participant and incident counts match traversal counts;
- hypergraph-facing vertex, hyperedge, and participant index aliases delegate to topology index capabilities.
- hypergraph-facing containment aliases delegate to topology containment capabilities;
- directed source/target participant traversal remains separate from full incidence/role capabilities;
- directed successor/predecessor vertex expansion preserves documented multiplicity;
- the `Hypergraph` capability bundle is available through static dispatch.

`oxgraph-csr` includes `tests/csr.rs`, which verifies:

- valid CSR views traverse outgoing edges, outgoing neighbors, and resolve endpoints;
- CSR views expose dense element and relation indexes;
- invalid offset lengths, first offsets, monotonicity, final offsets, and target bounds are rejected;
- generated valid CSR inputs have out-degree counts matching traversal counts.
- empty graphs, isolated nodes, self-loops, and parallel edges validate and traverse correctly;
- node/edge containment and checked target lookup report invalid handles without entering the hot trait path;
- `CsrOutEdges` preserves exact-size iterator length as it advances;
- `CsrOutNeighbors` preserves exact-size iterator length as it advances;
- snapshot-style little-endian `zerocopy` words validate without copying.

`oxgraph-algo` includes `tests/bfs.rs`, which verifies:

- forward and reverse indexed BFS run over a hand-written topology-trait fixture;
- forward and reverse epoch-scratch, allocating, workspace, generic-btree, and generic-hash BFS variants all agree across the same fixture;
- forward BFS runs over `oxgraph-csr::CsrGraph` (CSR is forward-only by design — its lack of `ElementPredecessors` is a deliberate type-level constraint);
- forward and reverse BFS run over `oxgraph-hyper-bcsr::BcsrHypergraph` opened from `oxgraph-snapshot` sections, proving the algorithm is genuinely substrate-agnostic;
- a single `BfsEpochScratch` and a single `BfsWorkspace` can drive both forward and reverse traversals without a full mark clear between directions;
- forward indexed BFS runs over a CSR view opened from `oxgraph-snapshot` sections;
- generated valid CSR inputs produce matching generic and default indexed BFS orders;
- scratch-backed BFS validates construction inputs before producing an iterator;
- invalid starts are rejected through containment before dense indexing;
- scratch-size errors have deterministic precedence over start-element validation;
- expanded element indexes past `element_bound` panic with the structured `BfsError::NeighborIndexOutOfBounds` message.

`oxgraph-snapshot` includes the following integration test files (six total), all of which exercise only the topology-agnostic surface:

- `tests/container_open.rs`: header truncation, bad magic, format major mismatch, format minor too new, header size mismatch, header reserved non-zero, section count too large, section table truncation, per-entry checksum/flags/reserved zero invariants, alignment-log2 cap, payload bounds, monotonic-offset enforcement, and duplicate-kind rejection are each surfaced as typed `SnapshotError` variants.
- `tests/plan_writer.rs`: a no-`alloc` smoke test that builds a snapshot in a stack `[u8; N]` via `SnapshotPlan`, opens it back, and exercises `BufferTooSmall` / `DuplicateKind` / header-only validation paths.
- `tests/builder_reader_roundtrip.rs`: proptest — the alloc-gated `SnapshotBuilder` followed by `Snapshot::open` yields byte-equal section payloads for arbitrary kinds, versions, alignments, and payload sizes.
- `tests/determinism.rs`: proptest — the same logical input produces byte-equal output (deterministic padding).
- `tests/malformed_bytes.rs`: proptest — `Snapshot::open` over arbitrary `Vec<u8>` returns a `Result`, never panics. This is the §22.1 keystone invariant.
- `tests/section_view.rs`: `Section::try_as_slice<T>` succeeds when the actual borrowed pointer is aligned, returns `AlignmentMismatch` for sub-sliced misaligned payloads, and returns `LengthNotMultipleOfSize` for payloads whose length is not a multiple of `size_of::<T>()`. The declared `alignment_log2` is intentionally not consulted.

`oxgraph-csr` includes `tests/snapshot_section.rs`, which verifies:

- valid CSR snapshots open as typed `CsrSnapshotGraph::<Index>` views via `from_snapshot` (with `node_count` derived from `offsets.len() - 1`, no separate metadata section);
- directed BFS runs over a CSR view opened from snapshot sections;
- missing offsets, empty offsets, target-out-of-range, and non-monotonic offsets are surfaced as typed `CsrSnapshotError` variants.

`oxgraph-hyper-bcsr` includes `tests/bcsr.rs`, `tests/snapshot_section.rs`, and `tests/static_dispatch.rs`, which verify:

- the canonical fixture validates at `BcsrValidation::Layout` and exposes the right vertex / hyperedge / per-direction incidence counts;
- `DirectedHyperedgeParticipants` yields head and tail participants as separate sorted slices;
- `HyperedgeParticipants` chains head then tail without re-sorting;
- `IncidentHyperedges` chains outgoing then incoming for one vertex;
- `DirectedVertexSuccessors` and `DirectedVertexPredecessors` expand correctly through tail and head sets;
- canonical participant IDs round-trip through `IncidenceElement`, `IncidenceRelation`, and `IncidenceRole`;
- `ContainsElement` / `ContainsRelation` / `ContainsIncidence` reject out-of-range handles before any hot trait path;
- empty hypergraphs validate;
- offset-length mismatches, non-monotonic offsets, and out-of-range vertex IDs are surfaced as typed `BcsrError` variants;
- a hyperedge mirrored only in vertex-major data passes `Layout` and is rejected at `Strict` as `CrossDirectionMismatch`;
- snapshots round-trip through `BcsrHypergraph::from_snapshot` (eight `BCSR_*` section kinds);
- missing sections and validation failures inside a snapshot surface as typed `BcsrSnapshotError` variants;
- generic consumers can use every trait `oxgraph-hyper` exposes through static dispatch.

### Workspace Lints

The unwrap/expect policy was moved into the workspace lint configuration:

```toml
expect_used = "deny"
unwrap_used = "deny"
```

This keeps panic shortcut policy centralized instead of crate-local.

## Intentional Deviations

### No General Graph Layout Crate Yet

`vision.md` lists graph layout crates for CSR, CSC, COO, and edge-table storage. The current slice adds the narrow `oxgraph-csr` crate, but not a general graph-layout crate covering multiple physical indexes.

Reason: CSR is the first concrete physical layout and should pressure-test the graph traits before adding CSC, COO, edge tables, or a broader layout abstraction.

### No Hypergraph Algorithm Crate Yet

`oxgraph-hyper` and `oxgraph-hyper-bcsr` together cover the trait vocabulary and a concrete bipartite-CSR layout, but no `oxgraph-hyper-algo` crate exists yet.

Reason: hypergraph algorithms (random walks, propagation, hypergraph BFS variants) deserve their own slice once a concrete consumer surfaces; the layout itself is the wedge that proves the trait surface.

### No Concrete ID Newtypes In Core

The vision allows typed ID wrappers such as `ElementId`, `RelationId`, and `IncidenceId` in `oxgraph-topology`. The current implementation does not provide default concrete ID newtypes.

Reason: we chose associated types only. This allows each view or layer to choose its own ID representation and identity layer.

### Identity Is Layer-Specific

The implementation treats each view's associated ID types as the identity exposed by that view.

A logical element, relation, or incidence can have multiple representations across layers, such as layout-local IDs, snapshot-canonical IDs, or domain IDs. Mapping between those layers is intentionally explicit and not part of the first trait surface.

### Roles Are Associated Types

No concrete role enum is provided by `oxgraph-topology`.

Reason: graph roles, hypergraph roles, oriented incidence labels, and domain-specific participation labels are not the same concept. The core exposes the slot for role information but does not interpret it.

### Payloads Are Not In Core

The core does not define relation, element, or incidence payload traits.

Reason: arbitrary rich data is expected to live in optional layers keyed by topology IDs. For example, a morphism layer can attach metadata to `RelationId` without requiring all topology views to support relation payloads.

### Counts Are A Capability

`TopologyCounts` is separate from `TopologyBase`.

Reason: not every view can meaningfully or cheaply report global counts. A view may represent a full snapshot, a filtered projection, a generated topology, a page-sized window, or an overlay. Counts mean the number of items visible through that specific view.

`TopologyCounts` only covers elements and relations. Incidence counts are exposed separately through `IncidenceCounts` so graph-only views can count nodes and edges without defining incidence records.

### No Mutation Traits Yet

The vision says mutation capabilities should be modeled early. The current implementation does not define mutation methods or markers.

Reason: mutation affects ID stability, deletion semantics, tombstones, compaction, freezing, and stale handles. Adding mutation traits before concrete read views would likely overfit the API.

### CSR Only, No CSC Yet

`oxgraph-csr` implements outgoing traversal over CSR. It does not implement incoming traversal because that requires a CSC section or another reverse index.

Reason: CSR is the smallest useful concrete graph layout and keeps this slice focused on the first wedge.

### Topology-Agnostic Snapshot v1 (ABI candidate)

`oxgraph-snapshot` now validates a topology-agnostic byte-level snapshot container at format major 1, minor 0. The header carries magic / format version / `header_size` / `section_count` only — no graph- or hypergraph-specific metadata. Section entries carry `(offset, length, kind, version, reserved_checksum, alignment_log2, flags, reserved)` for 32 bytes each, with `u64` offsets and lengths so multi-GB mmap'd topologies are addressable.

Reason: the container is the topology-interchange layer per vision §20–§24.4; section semantics belong to upper layers (`oxgraph-csr`, future `oxgraph-hyper-layout`). Several deliberate cuts kept v1 small: no CRC verification (the 4-byte `reserved_checksum` field is reserved and must be zero in v1), no REQUIRED-section enforcement (the flags byte must be zero; required-vs-optional is consumer policy and the container has no kind registry), no multi-instance sections (one entry per kind), no mmap helpers (the `std` feature is reserved and unused). The bytes are not yet promised as a stable ABI, but the upgrade path for each deferred feature is preserved by the reserved fields.

### No Topology-General Algorithm Crate Yet

No `topology-algo` crate was added.

Reason: graph BFS and hypergraph expansion have different semantics. Directed BFS belongs in `oxgraph-algo` over `oxgraph-graph` traits. Hypergraph algorithms should wait for concrete hypergraph pressure.

## Current Meaning Of "View"

A topology view is any value that exposes topology through `oxgraph-topology` traits.

Examples of possible views:

- a full immutable snapshot;
- a borrowed in-memory layout;
- a CSR section;
- a page-sized window;
- a filtered projection;
- an overlay;
- a generated topology.

The view decides its own boundary. Trait implementations document what their IDs, counts, and traversal methods mean within that boundary.

## Verification

The current slice has passed:

```sh
just ci
cargo run -p oxgraph-topology --example directed_graph
cargo run -p oxgraph-topology --example hypergraph
cargo run -p oxgraph-graph --example graph_directed
cargo run -p oxgraph-hyper --example hyper_directed
cargo run -p oxgraph-csr --example csr_directed
cargo run -p oxgraph-algo --example bfs
cargo run -p oxgraph-snapshot --example pack_two_sections --features alloc
cargo run -p oxgraph-csr --example open_snapshot
cargo run -p oxgraph-hyper-bcsr --example bcsr_directed
cargo run -p oxgraph-hyper-bcsr --example open_bcsr_snapshot
cargo bench --workspace --no-run
cargo check --workspace --no-default-features
cargo test -p oxgraph-algo --no-default-features
```

`oxgraph-snapshot/src/proofs.rs` carries Kani proof harnesses for the container — header parsing totality, `Snapshot::open` not-panicking on arbitrary 64-byte input, two-section `SnapshotPlan` write/open roundtrip, and the `FORMAT_MAGIC` constant. `oxgraph-hyper-bcsr/src/proofs.rs` adds harnesses for bipartite-CSR validation totality at both `Layout` and `Strict`, participant-ID arithmetic non-overflow within the documented `u32` cap, and `u32 → usize` narrowing safety on every supported target. They run under `cargo kani` (heavy `just verify` gate, not the fast `just ci` path).

## Benchmarks

The current slice includes Criterion benchmarks for the first measurable graph path:

- `oxgraph-csr/benches/csr.rs`: CSR validation and outgoing traversal over deterministic regular graphs.
- `oxgraph-algo/benches/bfs.rs`: scratch, epoch-scratch, allocating indexed, and workspace directed BFS over `OutgoingNeighborsGraph` using CSR-backed graph shapes.
- `oxgraph-snapshot/benches/container.rs`: agnostic snapshot open over `s ∈ {1, 16, 256, 1024}` sections, last-section linear lookup, and `SnapshotBuilder::finish` over `s ∈ {16, 256, 1024}` × 1 KiB payloads.
- `oxgraph-csr/benches/snapshot.rs`: snapshot-backed CSR open and BFS over deterministic ring graphs at `n ∈ {64, 1024, 16_384}` nodes.
- `oxgraph-hyper-bcsr/benches/bcsr.rs`: bipartite-CSR validation at `Layout` and `Strict`, hyperedge-major head/tail iteration, vertex-major incident iteration, and directed successor expansion over deterministic regular hypergraphs at `v ∈ {1k, 16k, 256k}` (fan-in/out 4).
- `oxgraph-hyper-bcsr/benches/snapshot.rs`: snapshot-backed bipartite-CSR open and successor traversal over deterministic regular hypergraphs at `v ∈ {1k, 16k}`.

The synthetic benchmark sizes currently cover 10k, 100k, and 1M nodes for graphs, and up to 256k vertices for hypergraphs. They are scale smoke tests, not final performance contracts. They do not yet compare against raw slice baselines, mmap-backed files, CSC/incoming traversal, or larger 100M-edge workloads.

The latest short run used:

```sh
cargo bench -p oxgraph-csr --bench csr -- --sample-size 10 --warm-up-time 1 --measurement-time 2
cargo bench -p oxgraph-algo --bench bfs -- --sample-size 10 --warm-up-time 1 --measurement-time 2
cargo bench -p oxgraph-snapshot --bench container -- --sample-size 10 --warm-up-time 1 --measurement-time 2
cargo bench -p oxgraph-csr --bench snapshot -- --sample-size 10 --warm-up-time 1 --measurement-time 2
cargo bench -p oxgraph-hyper-bcsr --bench bcsr -- --sample-size 10 --warm-up-time 1 --measurement-time 2
cargo bench -p oxgraph-hyper-bcsr --bench snapshot -- --sample-size 10 --warm-up-time 1 --measurement-time 2
```

The latest short runs are directional numbers, not final published performance claims. They show that the dense-index capability removes the generic BFS bottleneck without removing the fallback path for arbitrary ID spaces.

## Open Questions

- Should `oxgraph-topology` eventually include optional default ID newtypes, or should it remain associated-type-only?
- Should payload access traits live in `oxgraph-topology`, a separate capability crate, or the first concrete graph/hypergraph crates that need them?
- Should graph slice capabilities such as outgoing target slices enter `oxgraph-graph`, or remain concrete `oxgraph-csr` APIs until another layout proves the need?
- Should indexed BFS use a packed bitset instead of `Vec<u8>` for visited state once benchmarked against cache and mutation costs?
- What role vocabulary should the hypergraph example use long term: oriented incidence signs, input/output/resource-style roles, or an unroleable hyperedge with `Role = ()`?
- When should mutation capability traits be introduced?
- Should identity-layer mapping traits be designed before snapshots, or only after a concrete snapshot format exists?

## Next Candidate Slices

1. Decide whether slice-level graph capabilities belong in `oxgraph-graph` based on `oxgraph-csr` pressure.
2. Add a minimal graph or bipartite-CSR builder only if snapshot examples/tests need too much hand-written byte construction.
3. Add CSC or reverse-index support when incoming traversal is needed by an algorithm or snapshot use case.
4. Promote the snapshot v1.0 bytes from "ABI candidate" to a stable ABI once a downstream consumer (e.g. mmap-backed graph store) has exercised them at scale.
5. Add explicit payload capability sketches once a concrete morphism or metadata use case needs them.
6. Add an `oxgraph-hyper-algo` crate once a concrete consumer surfaces (random walks, hypergraph BFS, propagation).
7. Promote the bipartite-CSR section kinds (`0x0010..=0x0017`) from layout-internal to a stable ABI once a downstream consumer has exercised them at scale; reserved kinds `0x0018..=0x001F` remain available for `u64`-offset variants.

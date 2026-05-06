# OxGraph architecture

OxGraph is a storage-agnostic topology substrate: **Topology here. Meaning elsewhere. Storage anywhere.** The lowest crates stay small, no-std where possible, and independent of Arrow, Python, and named properties. Higher layers select richer data into capability views for algorithms.

## Layer hierarchy

```text
oxgraph-topology        no_std shared capabilities: IDs, traversal, weights, identity
  ├─ oxgraph-graph      binary graph vocabulary wrappers
  └─ oxgraph-hyper      hypergraph vocabulary wrappers
oxgraph-csr            borrowed CSR graph layout
oxgraph-hyper-bcsr     borrowed directed bipartite-CSR hypergraph layout
oxgraph-snapshot       topology-agnostic section container
oxgraph-property       std + Arrow named typed layers and selected weight views
oxgraph-graph-build    append/update graph builder, freeze to owned view
oxgraph-hyper-build    append/update hypergraph builder, freeze to owned view
oxgraph-algo           BFS and PageRank over capability bounds
oxgraph-python         PyO3 facade exposing OxGraph concepts to Python
oxgraph                feature-gated umbrella re-exports
```

Foundation crates (`topology`, `graph`, `hyper`, `csr`, `hyper-bcsr`) do not depend on Arrow, PyO3, Python labels, or `oxgraph-property`.

## Weights versus properties

Topology weights are optional capabilities:

- element weight
- relation weight
- incidence weight

A weight capability is total for every visible ID in its family and returns a `Copy` representation. It carries no meaning: not probability, distance, cost, capacity, count, confidence, or metadata.

Named values live in `oxgraph-property`. A property layer is keyed by ID family, has a stable layer ID and name, carries an Arrow field, and records dense or sparse storage with an explicit missing policy. Algorithms consume property data only after a caller selects a layer into a topology weight view.

## Identity

Canonical identity is opt-in. A view that implements canonical identity guarantees local-to-canonical mapping for the selected ID family within the view generation, frozen view, or snapshot. Reverse canonical-to-local lookup is separate and optional because filtered/projected views may not have a total reverse map.

Builders assign dense append-only canonical IDs. Frozen views may keep local IDs equal to canonical IDs or may reorder locally as long as opted-in identity maps recover the canonical IDs.

Python labels are facade-owned domain maps. Rust algorithms never interpret them.

## Builders and freeze

The first builders are construction-time systems:

- add isolated elements;
- add graph edges or directed hyperedges;
- update typed weights/properties before freeze;
- maintain construction indexes;
- freeze/export to immutable owned views and snapshots.

Deletion, tombstones, ID reuse, compaction-after-delete, and overlay mutation are out of scope for this slice. Borrowed caches are generation-checked and invalidated by edits; Python exposes owned frozen views to avoid lifetime footguns.

## Snapshot direction

`oxgraph-snapshot` remains a topology-agnostic section container. Higher layers register section kinds for canonical identity maps and property descriptors/data. Snapshot validation checks structure: section consistency, ID-family compatibility, type tags, lengths, names, and layout. Algorithms validate numeric semantics such as finite/non-negative/normalizable values.

The internal v1 property/identity encoding used by the Python-enabling slice is Arrow-backed and ABI-candidate only:

- one identity-mode section contains fixed records for element, relation, and incidence families;
- a mode is either `local == canonical` or `explicit u32 local-to-canonical map`;
- optional element/relation/incidence map sections contain little-endian `u32` canonical IDs when local IDs differ;
- one property descriptor section contains fixed records plus UTF-8 layer-name and Arrow-field-name bytes;
- one property data section stores concatenated Arrow IPC streams, preserving each layer's Arrow schema/type information, dense/sparse values, sparse indexes, and optional Arrow scalar default;
- descriptor records carry the Arrow value family, ID family, role, storage, missing policy, logical length, and data offsets needed for structural validation, including missing, overlapping, gapped, or trailing data ranges.

Snapshot v1 bytes remain an ABI candidate, not a stable ABI.

## PageRank policies

`oxgraph-algo::pagerank` uses the canonical PageRank name.

Ordinary directed graph PageRank:

- unweighted edges default to weight `1`;
- rank configuration, reports, personalization, output ranks, teleport vectors, and scratch storage are generic over an OxGraph-owned `PageRankScalar`;
- weighted mode consumes selected relation weights convertible into the PageRank scalar;
- weights must be finite and non-negative after conversion;
- outgoing rows are normalized inside the algorithm;
- zero-total outgoing rows are dangling rows;
- dangling mass redistributes according to personalization;
- default damping is `0.85` for supported scalar implementations;
- convergence uses L1 rank delta with configurable tolerance and max iterations;
- invalid inputs, undersized scratch/output storage, and non-convergence return typed errors.

Directed hypergraph PageRank uses an incidence/bipartite state space of elements plus relations. Flow is source element → relation → target element. Relation weights choose outgoing relations from a source element when supplied; target incidence weights choose target participants when supplied. Weighted inputs convert through OxGraph-owned PageRank numeric traits instead of constraining topology weight types to `f64`. Dangling mass redistributes over one combined personalization vector.

PageRank follows the allocation-tier discipline used by BFS: allocating convenience APIs keep the canonical names, borrowed scratch APIs accept caller-owned teleport/next arrays, and owned workspace APIs reuse `Vec` storage branded to the topology view type and scalar.

Projected hypergraph PageRank is a future explicit policy, not the default.

## Python facade and safety

The Python package is `oxgraph`, backed by the `oxgraph-python` native module. It exposes builders, frozen views, identity lookup, weights/properties, BFS, PageRank, and snapshot helpers. It deliberately does not expose third-party graph library exporters in this slice.

Any PyO3-required unsafe or macro-generated unsafe is isolated to `oxgraph-python` and documented in `crates/oxgraph-python/SAFETY.md`. Foundation crates keep `unsafe_code = "forbid"`.

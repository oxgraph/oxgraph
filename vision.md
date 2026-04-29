# Foundational Topology Substrate for Rust: Vision Document

## Status

This document defines the central vision for a foundational topology substrate for Rust.

It is intended to be the reference document for the project: the thing we return to when deciding what belongs in the deepest substrate, what belongs in graph-specific or hypergraph-specific crates, what belongs in layouts, what belongs in snapshots, what belongs in mutation systems, and what should not be part of the system at all.

The project began from the desire to build a foundational graph crate. The current vision is more precise:

> Build a storage-agnostic, zero-copy-friendly topology access layer for Rust, starting with large immutable directed graphs and designed to generalize to hypergraphs and higher-order topology, with all domain meaning kept above the core.

The project is not merely a graph library.

It is not a graph database.

It is not a knowledge graph framework.

It is not an AI memory framework.

It is a layered substrate for topology-shaped data.

The first optimized path is ordinary directed graphs.

Hypergraphs are a first-class sibling path in the architecture, but not necessarily the first full implementation path.

Topology is the shared foundation beneath both.

The central principle is:

> **Topology here. Meaning elsewhere. Storage anywhere.**

---

# 1. The Vision

Graphs and graph-like structures are among the most universal structures in computing.

They appear in compilers, databases, build systems, package managers, dependency resolution, static analysis, distributed systems, simulations, robotics, knowledge graphs, AI systems, graph retrieval, agent memory, authorization systems, workflows, provenance systems, biology, networks, embedded devices, and many other domains.

But the deeper recurring structure is not always just a binary graph.

Many systems need:

* ordinary directed graphs,
* undirected graphs,
* multigraphs,
* dependency graphs,
* provenance graphs,
* state transition graphs,
* hypergraphs,
* directed hypergraphs,
* incidence structures,
* n-ary relations,
* higher-order context groups,
* and topology-like relations between many participating elements.

Yet infrastructure for these structures remains fragmented.

Every domain reinvents some version of:

* node or element identifiers,
* edge or relation identifiers,
* adjacency traversal,
* incidence traversal,
* topology storage,
* mutation rules,
* graph snapshots,
* serialization,
* validation,
* algorithms,
* physical indexes,
* metadata attachment,
* import/export,
* and interop.

The vision is to build the missing lower layer:

> **A storage-agnostic, high-performance, zero-copy-friendly topology substrate for Rust.**

This substrate should be useful anywhere topology needs to be represented, traversed, validated, archived, memory-mapped, embedded, transformed, indexed, mutated, or analyzed.

However, the adoption wedge must be concrete.

The first product story is not:

> Everyone needs topology.

The first product story is:

> Many systems repeatedly rebuild large topology-shaped structures into heap-owned graphs before they can traverse them. This project should let them validate and traverse those structures directly from compact snapshots, memory-mapped files, borrowed bytes, or other storage backends.

The strongest initial use case is:

> Build a large directed graph, freeze it into a compact validated snapshot, memory-map it, and begin traversal without deserializing it into a heap graph.

A strong headline demo is:

> Open a 100M-edge graph and begin traversing it without rebuilding the graph in memory.

That is the first wedge.

The larger topology vision remains, but the first proof must be narrow, measurable, and hard to fake.

---

# 2. Definition of Topology

The word **topology** can mean many things.

In this project, topology means:

> **Discrete relational connectivity: elements, relations, incidences, roles, identifiers, traversal, indexes, and validated access to those structures.**

This project does **not** use topology to mean general mathematical topology, open sets, manifolds, continuous spaces, or all possible higher-dimensional mathematical structures.

The substrate is concerned with discrete connectivity.

It provides a shared foundation for structures such as:

* binary graphs,
* directed graphs,
* multigraphs,
* hypergraphs,
* directed hypergraphs,
* incidence structures,
* relation-to-participant structures,
* and storage-backed traversal views.

It may later interoperate with richer mathematical structures such as simplicial complexes or cell complexes, but those are not the core definition.

The foundation knows only about discrete topology.

Meaning belongs above it.

---

# 3. The Mission

The mission is to make topology a reusable systems primitive.

The project should provide a common foundation that allows topology-producing systems and topology-consuming systems to interoperate without agreeing on a database, application domain, query language, storage engine, semantic model, or runtime.

The project should make it possible to:

* write algorithms once and run them over many storage backends,
* represent large graph-like structures compactly,
* open immutable topology snapshots without deserializing them into heap objects,
* traverse topology with predictable performance,
* support mutable topology as an explicit opt-in capability,
* support `no_std` environments at the deepest layers,
* allow embedded/static topology use cases,
* memory-map large topology snapshots,
* choose physical indexes based on workload,
* separate topology from node/edge/relation payloads,
* support ordinary graphs and hypergraphs as sibling specializations,
* allow downstream domain-specific systems to build on top without changing the substrate,
* and become a stable topology access layer that other systems can build on.

The project should play a foundational systems role similar in spirit to crates and systems such as:

* `bytes` for byte buffers,
* `serde` for serialization interfaces,
* `rkyv` for archived zero-copy data,
* `tokio` for async runtime infrastructure,
* Arrow for columnar memory,
* and `tracing` for structured instrumentation.

The goal is not to copy these projects.

The goal is to occupy a similarly foundational role for topology-shaped data.

But the project must earn that role by solving one painful repeated problem first.

The initial pain is:

> Large topology-shaped data often has to be parsed, allocated, reconstructed, and indexed before it can be traversed.

The initial answer is:

> Validated zero-copy graph snapshots with storage-agnostic traversal traits.

---

# 4. The Problem

Graph-shaped and topology-shaped data is everywhere, but the infrastructure is not shared.

Most systems choose one of four paths:

1. Use a domain-specific graph system.
2. Build an ad hoc graph representation.
3. Use a general-purpose graph library that owns the data structure.
4. Encode richer structures, such as hyperedges, into ordinary graphs manually.

Each path has limitations.

## 4.1 Domain-specific graph systems

Graph databases, RDF stores, knowledge graph systems, ML graph frameworks, compiler IRs, workflow engines, and provenance systems are useful, but they usually bring their own storage model, semantics, query layer, runtime assumptions, and application-specific interpretation.

They solve their domain problem, but they do not provide a neutral topology substrate.

## 4.2 Ad hoc graph representations

Many projects build custom adjacency lists, maps, edge arrays, relation tables, incidence lists, or index structures.

This works initially, but leads to repeated effort around:

* ID management,
* traversal APIs,
* serialization,
* validation,
* algorithm reuse,
* mutation handling,
* snapshotting,
* memory layout,
* and interoperability.

## 4.3 General-purpose graph libraries

General graph libraries are valuable, but they often focus on owned in-memory graphs and algorithm ergonomics.

They usually do not aim to be:

* storage-agnostic,
* zero-copy snapshot formats,
* `no_std` substrate layers,
* topology ABI candidates,
* mmap-friendly graph representations,
* mutation-capability abstractions,
* or domain-neutral topology indexes.

This project should not compete by merely being another graph crate.

It should compete by being the best lower layer for storage-agnostic, zero-copy-friendly topology traversal and validated snapshots.

## 4.4 Hypergraphs and higher-order topology

Many systems need relationships involving more than two participants.

Examples include:

* a meeting involving people, documents, decisions, and deadlines,
* an AI context episode involving a user request, tool call, source file, output, and validation result,
* a provenance event involving actors, inputs, outputs, and evidence,
* a constraint involving many variables,
* a directed hyperedge from a set of sources to a set of targets,
* or a higher-order relationship among multiple entities.

These can be encoded into ordinary graphs, but the encoding can lose clarity, introduce artificial nodes, or obscure the original higher-order relation.

This does not mean hypergraphs should be forced into the same API as ordinary graphs.

It means the architecture must recognize that binary graphs and hypergraphs are different specializations of a deeper topology concept.

---

# 5. The Core Thesis

The core thesis is:

> Topology should be separated from meaning, storage, physical layout, mutation strategy, and application logic.

A topology substrate should define the minimal common structure that graph-like systems need:

* elements,
* relations,
* incidences,
* identifiers,
* endpoints or participants,
* traversal,
* direction or role markers where needed,
* topology indexes,
* snapshots,
* validation,
* mutation capabilities,
* and storage-independent views.

It should not define what the elements and relations mean.

A node is not necessarily an entity.

An edge is not necessarily a semantic relation.

A hyperedge is not necessarily an event.

A label is not necessarily an ontology term.

A path is not necessarily a proof.

A payload is not necessarily a property.

A graph is not necessarily a database.

A topology is not necessarily an application model.

Those are domain interpretations.

The foundational layer should refuse to interpret.

The guiding phrase is:

> **Structure below. Meaning above.**

---

# 6. Non-Goals

The project needs explicit non-goals so the core does not become a dumping ground.

## 6.1 Non-goals for the foundation

The foundation is not:

* a graph database,
* a query language,
* an RDF store,
* a property graph engine,
* an ontology system,
* an AI memory framework,
* a GraphRAG framework,
* a provenance semantics framework,
* a distributed graph processing system,
* a full hypergraph mathematics library,
* a replacement for every in-memory graph crate,
* a universal graph algorithm library,
* a semantic modeling language,
* or a product/domain modeling framework.

The foundation may support systems that do those things.

It should not become those things.

## 6.2 Non-goals for v1

The first version should not promise:

* every graph layout,
* every hypergraph model,
* production-grade mutation engines,
* stable cross-language ABI guarantees,
* Python bindings,
* ML framework integration,
* distributed traversal,
* product-specific domain layers,
* or full interop with every graph ecosystem.

The first version should prove the core claim:

> A large directed graph can be built, frozen into a compact byte-level snapshot, validated from bytes, and traversed efficiently through storage-agnostic APIs without heap reconstruction.

---

# 7. The Layered Architecture

The project must be built hierarchically.

The architecture should make clear what is foundational, what is specialized, what is optional, and what is domain-specific.

The intended hierarchy is:

```text
Layer 0: oxgraph-topology
  Minimal shared topology primitives.
  Identity, elements, relations, incidences, roles, cursors, capabilities.
  Not a graph library.
  Not a hypergraph library.

Layer 1A: oxgraph-graph
  Binary graph specialization.
  Nodes, edges, source, target, outgoing traversal, incoming traversal.
  Fast and ergonomic for ordinary graph users.

Layer 1B: oxgraph-hyper
  Hypergraph specialization.
  Vertices, hyperedges, participants, incidences, roles, directed hyperedges.
  First-class architectural sibling of oxgraph-graph.
  Not necessarily fully implemented before graph snapshots are proven.

Layer 2A: graph-layout
  Physical layouts for ordinary graphs.
  CSR, CSC, COO, edge tables, optional reverse indexes.

Layer 2B: hyper-layout
  Physical layouts for hypergraphs.
  Incidence arrays, hyperedge offsets, participant arrays, source/target participant sets.

Layer 3: snapshot/container formats
  Byte-level validated topology containers.
  ABI-candidate design.
  Graph and hypergraph sections can be separate but share principles.

Layer 4: builders and mutation systems
  Construction, mutation, append-only deltas, overlays, freezing, compaction, validation.

Layer 5: algorithms
  Algorithms over capability traits.
  Graph algorithms and hypergraph algorithms may be separate.

Layer 6: interop and projections
  Bridges to existing ecosystems and explicit conversions between representations.
```

The hierarchy matters because not every layer is equally foundational.

The deepest layer should be small enough to remain stable.

The specialized layers should be ergonomic enough to be useful.

Downstream product and domain systems should be able to build on the library without contaminating the foundation.

The architecture is broad, but the library boundary stops at interop.

Domain-specific systems are downstream consumers, not library layers.

The MVP must be narrow.

---

# 8. Topology-Core

`oxgraph-topology` is the true lowest-level substrate.

It is not a graph library.

It is not a hypergraph library.

It provides the minimal shared vocabulary needed by graph-like and hypergraph-like structures.

## 8.1 Core concepts

The basic topology vocabulary is:

* **Element** — an item that participates in topology.
* **Relation** — a connection, link, edge, hyperedge, or other topological relation.
* **Incidence** — the participation of an element in a relation.
* **Role** — optional information describing how an element participates in a relation.

For an ordinary directed graph edge:

```text
A → B
```

The topology view can be described as:

```text
Relation E:
  incidence 1: element A, role Source
  incidence 2: element B, role Target
```

For a hyperedge:

```text
H = {A, B, C}
```

The topology view can be described as:

```text
Relation H:
  incidence 1: element A
  incidence 2: element B
  incidence 3: element C
```

For a directed hyperedge:

```text
{A, B} → {C, D}
```

The topology view can be described as:

```text
Relation H:
  incidence 1: element A, role Source
  incidence 2: element B, role Source
  incidence 3: element C, role Target
  incidence 4: element D, role Target
```

This general model allows ordinary graphs and hypergraphs to share a tiny conceptual substrate without pretending they are the same user-facing abstraction.

## 8.2 Topology-core inclusion rule

`oxgraph-topology` must be kept small.

A concept belongs in `oxgraph-topology` only if one of the following is true:

1. It is required by both `oxgraph-graph` and `oxgraph-hyper` without awkwardness.
2. It is required by the snapshot/container validation model.
3. It is required to express storage-agnostic traversal capabilities.
4. It is required to preserve identity boundaries across layouts.

A concept does not belong in `oxgraph-topology` merely because it is theoretically general.

The core should be validated by implementation pressure.

The rule is:

> Nothing enters `oxgraph-topology` until concrete graph and/or hypergraph implementations prove it is necessary.

## 8.3 What oxgraph-topology should contain

`oxgraph-topology` may contain:

* typed ID wrappers,
* element IDs,
* relation IDs,
* incidence IDs,
* identity layering concepts,
* endpoint or incidence role markers,
* cursor traits,
* storage-agnostic view traits,
* basic topology error types,
* capability marker traits,
* and low-level relation/incidence access contracts.

`oxgraph-topology` should remain `no_std` where possible.

It should be boring, stable, and dependency-light.

## 8.4 What oxgraph-topology should not contain

`oxgraph-topology` should not contain:

* CSR implementation,
* CSC implementation,
* hypergraph layout implementation,
* snapshot binary format implementation,
* builders,
* graph algorithms,
* hypergraph algorithms,
* RDF concepts,
* property graph concepts,
* AI concepts,
* provenance concepts,
* mutation storage engines,
* Python bindings,
* mmap support,
* or domain semantics.

`oxgraph-topology` is for implementers and advanced library authors.

Most users should not need to interact with it directly.

## 8.5 Incidence should not slow down ordinary graphs

Incidence is the shared conceptual model.

It should not become the mandatory performance path for ordinary graph traversal.

For ordinary graphs, the hot path should remain close to:

```text
offsets[node]..offsets[node + 1] → edge or target slice
```

A binary graph should be able to expose a topology/incidence view when needed, but graph traversal should not be forced to materialize or iterate through generalized incidence records.

The rule is:

> `oxgraph-topology` defines the common vocabulary. `oxgraph-graph` preserves the fast binary graph path.

---

# 9. Graph-Core

`oxgraph-graph` is the binary graph specialization built on the topology foundation.

It exists because ordinary graphs are the most common and performance-critical case.

A graph user should not need to think about general incidence topology just to run BFS or traverse dependencies.

## 9.1 Graph model

The graph model should support:

* nodes,
* binary edges,
* directed graphs,
* undirected graphs where appropriate,
* multigraphs where appropriate,
* self-loops where appropriate,
* outgoing traversal,
* incoming traversal,
* edge endpoint lookup,
* optional stable edge identity,
* and optional payload/domain identity layers above the core.

The optimized base case is:

```text
edge = source node → target node
```

## 9.2 Graph-core should expose ergonomic APIs

Graph users should be able to work with APIs like:

```rust
fn outgoing(node) -> OutEdges;
fn incoming(node) -> InEdges;
fn source(edge) -> NodeId;
fn target(edge) -> NodeId;
fn endpoints(edge) -> (NodeId, NodeId);
```

They should not be required to manually traverse incidence structures for ordinary graph use.

## 9.3 Graph-core is a specialization, not the whole universe

`oxgraph-graph` is not the only foundation.

It is one specialization of the deeper topology substrate.

Hypergraphs should not be forced to depend on binary graph semantics.

The correct relationship is:

```text
          oxgraph-topology
          /           \
   oxgraph-graph       oxgraph-hyper
```

Not:

```text
oxgraph-graph
  ↓
oxgraph-hyper
```

A hypergraph is not merely a graph with bigger edges.

It is a different specialization of topology.

## 9.4 Graph performance contract

The common graph path must remain efficient.

The project should not accept an abstraction that makes ordinary graph traversal meaningfully slower or more awkward than a direct layout-specific traversal.

The graph path must support:

* iterator-based traversal for generic algorithms,
* exact-size traversal where available,
* contiguous-slice traversal where the layout supports it,
* and low-level validated section access for snapshot-backed implementations.

A foundational graph substrate should compile down close to raw slice traversal for CSR/CSC-backed views.

---

# 10. Hyper-Core

`oxgraph-hyper` is the hypergraph specialization built on the topology foundation.

It exists because many systems need higher-order relations involving more than two participants.

## 10.1 Hypergraph model

The hypergraph model should support:

* vertices,
* hyperedges,
* participant sets,
* incidence traversal,
* optional endpoint roles,
* directed hyperedges,
* source participant sets,
* target participant sets,
* hyperedge identity,
* and opaque payload/domain identity layers above the core.

A hyperedge can represent:

```text
H = {A, B, C}
```

A directed hyperedge can represent:

```text
{A, B} → {C, D}
```

## 10.2 Hyper-core should not be an encoding trick

Hyperedges can be represented as ordinary graphs through projections, but that should not be the native model of `oxgraph-hyper`.

A hypergraph should be able to preserve the fact that a relation connects many participants as one relation.

This matters for:

* higher-order relationships,
* AI context engineering,
* grouped memories,
* provenance events,
* constraints,
* multi-input/multi-output transformations,
* workflows,
* factor graphs,
* and n-ary facts.

## 10.3 Hyper-core v1 scope

Hypergraphs are first-class in the architecture, but v1 should not attempt to model every mathematical structure related to higher-order connectivity.

`oxgraph-hyper` v1 should be narrow:

* vertices,
* hyperedges,
* participant traversal,
* incident hyperedge traversal,
* optional participant roles,
* directed source/target participant sets,
* and explicit projections to graph views.

It should not try to fully model:

* all oriented hypergraph variants,
* all factor graph semantics,
* all simplicial complex semantics,
* all cell complex semantics,
* all tensor representations,
* or all category-theoretic structures.

Those may be built above or beside the substrate later.

The core should make them possible without becoming them.

## 10.4 Hyper-core and oxgraph-graph should interoperate explicitly

Hypergraphs can be projected into ordinary graphs when useful.

Common projections include:

### Incidence graph / star expansion

A hyperedge becomes a graph node connected to each participant.

Example:

```text
Hypergraph:
  H1 = {A, B, C}

Graph projection:
  H1 -- A
  H1 -- B
  H1 -- C
```

This preserves hyperedge identity but converts the hyperedge into a graph node.

### Clique expansion

A hyperedge becomes pairwise edges between all participants.

Example:

```text
Hypergraph:
  H1 = {A, B, C}

Graph projection:
  A -- B
  A -- C
  B -- C
```

This is useful for ordinary graph algorithms, but it loses the fact that `A`, `B`, and `C` were connected by one shared hyperedge.

It can also produce many edges for large hyperedges.

These projections are interop tools.

They are not the native representation.

---

# 11. Identity Model

Identity must be layered.

The substrate should support three identity kinds:

1. **Topology-local identity**
2. **Canonical substrate identity**
3. **Domain identity**

A topology implementation should be able to opt into any combination of these identity kinds at once.

## 11.1 Topology-local identity

Topology-local identity is position-based identity inside a layout or index.

Examples:

* CSR edge slot,
* CSC edge slot,
* incidence row,
* participant array offset,
* local relation offset.

This identity is fast and compact.

It is useful for traversal and layout-local operations.

But it may not be stable across compaction, rebuilding, sorting, or conversion.

## 11.2 Canonical substrate identity

Canonical substrate identity is stable internal identity across topology indexes or layout sections.

Examples:

* `NodeId(42)`,
* `EdgeId(42)`,
* `ElementId(42)`,
* `RelationId(42)`,
* `HyperedgeId(42)`,
* `IncidenceId(42)`.

This identity is useful when multiple indexes need to agree on the same logical item.

For example, if CSR and CSC both contain the same edge, a canonical edge ID allows both indexes to refer to the same logical edge.

## 11.3 Domain identity

Domain identity is application-level identity.

Examples:

* URI,
* UUID,
* package name,
* document ID,
* database key,
* file path,
* RDF term,
* compiler symbol,
* external object ID.

The substrate may support opaque domain identity through mapping tables, custom sections, payload references, or application-owned extension layers.

But the core must not interpret domain identity.

The core should not know that a string is a package name, RDF URI, person, document, memory, or claim.

## 11.4 Identity invariants

The identity model needs explicit invariants.

### Local IDs

Local IDs are:

* dense where possible,
* layout-specific,
* optimized for traversal,
* cheap to store,
* and allowed to change after rebuild, sorting, compaction, or conversion.

Local IDs should not be treated as stable application references.

### Canonical substrate IDs

Canonical substrate IDs are:

* stable within a topology generation or snapshot,
* used to connect multiple indexes that describe the same logical item,
* not reused within a generation unless a mutation policy explicitly allows it,
* optionally mapped from layout-local slots,
* and not inherently meaningful to the application domain.

In immutable snapshots, canonical IDs can usually be dense integer IDs.

In mutable structures, canonical IDs may need generation counters or tombstone-aware allocation policies.

### Domain IDs

Domain IDs are:

* opaque,
* optional,
* application-owned,
* and never interpreted by the core.

The core may store, expose, or index domain ID mappings as bytes or typed extension data, but it must not assign semantic meaning to them.

## 11.5 Identity and mutation

Mutation changes identity guarantees.

The system must define what happens to local IDs, canonical IDs, and domain mappings under:

* insertion,
* deletion,
* tombstoning,
* compaction,
* sorting,
* freezing,
* overlay application,
* and snapshot rebuilding.

For mutable topologies, generational IDs may be useful:

```rust
pub struct NodeId {
    pub index: u32,
    pub generation: u32,
}
```

This is not necessarily required for immutable snapshots, but the mutation design must account for stale handles.

## 11.6 Identity principle

The guiding principle is:

> Local identity gives performance. Canonical identity gives substrate-level stability. Domain identity gives application meaning. The core supports the layers but only interprets topology-local and canonical substrate layers.

---

# 12. Design Principle: Structure Below, Meaning Above

The substrate owns topology.

Applications own meaning.

## 12.1 The substrate may know about

* topology views,
* element IDs,
* relation IDs,
* incidence IDs,
* node IDs,
* edge IDs,
* hyperedge IDs,
* endpoints or participants,
* outgoing adjacency,
* incoming adjacency,
* incidence traversal,
* topology indexes,
* graph snapshots,
* layout sections,
* validation,
* mutation capabilities,
* traversal,
* and storage backends.

## 12.2 The substrate must not know about

* people,
* documents,
* claims,
* evidence,
* provenance meaning,
* RDF triples,
* ontology classes,
* embeddings,
* GraphRAG chunks,
* agent memory,
* compiler symbols,
* package dependencies,
* authorization policies,
* timestamps,
* confidence scores,
* or any application-specific concept.

These may exist as opaque payloads, domain identity mappings, custom sections, or extension-layer semantics.

But the core must not interpret them.

---

# 13. Design Principle: Views, Not Ownership

The primary abstraction should not be an owned `Graph` or `Hypergraph` type.

The primary abstraction should be a view.

A view exposes topology regardless of where or how the topology is stored.

A view could be backed by:

* heap vectors,
* borrowed slices,
* memory-mapped files,
* archived bytes,
* embedded static bytes,
* flash memory,
* database pages,
* generated views,
* foreign memory,
* append-only logs,
* overlay layers,
* or remote/proxy-backed storage.

Owned structures are useful, but they are not foundational enough.

A substrate becomes foundational when algorithms and systems can target access contracts instead of concrete data structures.

The deepest question should not be:

> What graph type do we own?

It should be:

> What topology capabilities can this view provide?

---

# 14. Design Principle: Capabilities Over Assumptions

Different topology backends support different operations.

Some support outgoing traversal.

Some support incoming traversal.

Some support both.

Some support incidence traversal.

Some support mutable updates.

Some are append-only.

Some support deletion.

Some have stable canonical IDs.

Some only expose local positional IDs.

Some have domain identity mappings.

Some have payloads.

Some have no payloads at all.

Some are immutable snapshots.

Some are mutable builders.

The core should model this through capability traits, not universal assumptions.

Examples:

* `OutgoingGraph` for efficient outgoing graph traversal.
* `IncomingGraph` for efficient incoming graph traversal.
* `EdgeEndpointGraph` for binary edge endpoint lookup.
* `IncidenceView` for relation-to-element participation.
* `MutableTopology` for mutable topology.
* `AppendOnlyTopology` for append-only mutation.
* `FreezableTopology` for converting mutable topology into an immutable view or snapshot.
* `StableRelationIdentity` for canonical relation IDs.
* `DomainIdentityMap` for opaque external identity lookup.

Algorithms should declare the capabilities they require.

A forward BFS only needs outgoing traversal.

A reverse reachability query needs incoming traversal.

A bidirectional search benefits from both.

A hypergraph participant query needs incidence traversal.

A mutable algorithm needs mutation capabilities.

This keeps the core flexible without making every topology pay for every feature.

---

# 15. Design Principle: Capability Tiers

For performance-sensitive code, not every traversal capability is equally strong.

The project should define capability tiers so generic algorithms can be ergonomic while optimized algorithms can demand stronger access contracts.

## 15.1 Tier 1: generic iterator capability

Example:

```rust
pub trait OutgoingGraph: DirectedGraph {
    type OutEdges<'a>: Iterator<Item = Self::EdgeId>
    where
        Self: 'a;

    fn outgoing(&self, node: Self::NodeId) -> Self::OutEdges<'_>;
}
```

This is ergonomic and storage-agnostic.

It is good for general algorithms.

## 15.2 Tier 2: exact-size traversal capability

Some backends can cheaply report the number of outgoing edges.

Example:

```rust
pub trait OutgoingExactGraph: OutgoingGraph {
    fn out_degree(&self, node: Self::NodeId) -> usize;
}
```

This helps algorithms preallocate or optimize loops.

## 15.3 Tier 3: contiguous slice capability

CSR-backed graphs can expose contiguous slices.

Example:

```rust
pub trait OutgoingEdgeSliceGraph: DirectedGraph {
    fn outgoing_edge_slice(&self, node: Self::ElementId) -> &[Self::RelationId];
}
```

Or, for target-only traversal:

```rust
pub trait OutgoingTargetSliceGraph: TopologyBase {
    fn outgoing_target_slice(&self, node: Self::ElementId) -> &[Self::ElementId];
}
```

This allows optimized algorithms to compile down close to raw slice iteration.

## 15.4 Tier 4: validated section capability

Snapshot-backed implementations may expose low-level validated sections for specialized code.

This should remain carefully controlled.

The public safe API should preserve bounds and validation invariants.

The rule is:

> Generic algorithms use broad capabilities. Hot-path algorithms can require stronger capabilities.

---

# 16. Design Principle: Mutability Is Opt-In and Modeled Early

The system must support mutability as an explicit capability from the beginning.

However, mutability should not dominate every API.

The correct principle is:

> Immutable and mutable topology views are both first-class concepts, but mutation is opt-in and capability-based.

The system should support multiple mutation modes over time:

* owned mutable graphs,
* owned mutable hypergraphs,
* mutable builders,
* append-only topologies,
* base snapshot plus delta,
* overlay graphs,
* overlay hypergraphs,
* copy-on-write topology,
* and freeze-to-snapshot workflows.

The core should distinguish between:

```text
immutable view
mutable view
append-only mutable view
removable mutable view
overlay view
freezable builder
validated snapshot
```

This matters because mutation changes many things:

* ID stability,
* deletion behavior,
* compaction,
* concurrency,
* index maintenance,
* validation,
* snapshotting,
* memory layout,
* and traversal guarantees.

Mutation must be considered in the v1 capability model.

But production-grade mutation engines should not be required for the MVP.

The v1 rule is:

> Model mutation capabilities early. Implement mutation strategies incrementally.

The preferred flow is:

```text
mutable builder / overlay / delta
        ↓ freeze / compact / validate
immutable snapshot
        ↓ zero-copy view
algorithms over read capabilities
```

---

# 17. Design Principle: Layout Is a Cost Model, Not a Semantic Model

A foundational topology substrate should not force one physical layout.

The same logical topology can be indexed in different ways depending on workload.

Physical layout choices should be explicit.

They trade off:

* memory usage,
* traversal speed,
* reverse lookup speed,
* mutation cost,
* validation cost,
* build time,
* cache locality,
* snapshot size,
* and interop convenience.

The principle is:

> Layout is a cost model, not a semantic model.

## 17.1 CSR: outgoing adjacency

CSR, or compressed sparse row, is optimized for outgoing queries in binary graphs.

It answers:

```text
What nodes or edges does this node point to?
```

CSR is ideal for:

* forward traversal,
* BFS,
* dependency expansion,
* many graph algorithms,
* compact static graphs,
* and memory-mapped traversal.

## 17.2 CSC: incoming adjacency

CSC, or compressed sparse column, is optimized for incoming queries in binary graphs.

It answers:

```text
What nodes or edges point into this node?
```

CSC is ideal for:

* reverse dependency lookup,
* backlinks,
* reverse reachability,
* bidirectional search,
* impact analysis,
* provenance backtracking,
* and systems needing efficient in-neighbor traversal.

## 17.3 COO: edge-list representation

COO, or coordinate format, stores edges as source-target pairs.

It is useful for:

* construction,
* streaming,
* sorting,
* conversion,
* import/export,
* and interop with scientific or ML tooling.

COO is usually not the best final traversal layout, but it is an important interchange and build representation.

## 17.4 Incidence layouts

Hypergraphs and general topology views may need incidence-oriented layouts.

Examples:

* relation offsets,
* participant arrays,
* incidence arrays,
* role arrays,
* source participant arrays,
* target participant arrays,
* element-to-relation indexes,
* relation-to-element indexes.

These are not identical to ordinary graph CSR/CSC, though they may use similar compressed-array techniques.

## 17.5 Optional combinations

A topology snapshot or mutable topology may contain different combinations of indexes.

For binary graphs, it may contain:

* CSR only,
* CSC only,
* CSR + CSC,
* COO only,
* CSR + COO,
* CSC + COO,
* CSR + CSC + COO,
* edge tables,
* or domain identity mappings.

For hypergraphs, it may contain:

* relation-to-participant index,
* participant-to-relation index,
* source participant index,
* target participant index,
* incidence table,
* role table,
* projection indexes,
* or domain identity mappings.

All combinations have costs.

Users should explicitly choose the indexes they need.

A memory-constrained embedded graph may store only CSR.

A reverse lookup service may store only CSC.

A large analytical graph may store CSR + CSC.

A hypergraph context engine may store relation-to-participant and participant-to-relation indexes.

A mutable system may store append logs plus periodic compacted indexes.

The substrate separates logical topology from physical indexes.

---

# 18. Design Principle: Zero-Copy Friendly by Default

The substrate should make zero-copy topology traversal a primary capability.

The ideal immutable flow is:

```text
bytes → validate → topology view → traverse
```

Not:

```text
bytes → parse → allocate graph → copy edges → traverse
```

A snapshot should be readable from:

* a byte slice,
* a memory-mapped file,
* static embedded bytes,
* archived memory,
* or possibly foreign memory.

The system should validate the snapshot and then expose safe traversal APIs over the underlying bytes.

Zero-copy does not mean unsafe by default.

It means:

* explicit layout,
* explicit validation,
* bounded access,
* alignment checks,
* version checks,
* endian handling,
* and a minimal unsafe surface hidden behind safe APIs.

---

# 19. Snapshot Format Is Not Rust Serialization

The snapshot format must be a byte-level format.

It must not be Rust struct serialization.

The snapshot format should define:

* explicit integer widths,
* explicit offsets,
* explicit section tables,
* explicit alignment rules,
* explicit endian rules,
* explicit versioning,
* explicit compatibility rules,
* explicit validation invariants,
* and optional checksums.

This matters because Rust object layout is not the same thing as a portable long-term binary format.

If the snapshot format is ever to become a shared topology interchange format, it must be stable independently of Rust compiler layout choices.

The snapshot format should be specified independently from the Rust trait APIs.

Rust crates are one implementation of the format, not the format itself.

---

# 20. Snapshot Format as ABI Candidate

The snapshot format is one of the most important strategic assets of the project.

Traits make algorithms reusable.

Snapshots make topology portable.

A snapshot should be:

* compact,
* deterministic,
* versioned,
* extensible,
* mmap-friendly,
* `no_std` readable where possible,
* safe to validate,
* efficient to traverse,
* and independent of domain meaning.

The snapshot format should be designed as an ABI candidate.

That means:

> The snapshot format should aspire to become a stable topology interchange ABI, but v1 should not overpromise ABI stability before the format has been tested.

The correct ambition is:

```text
ABI candidate first.
Stable ABI only after real-world validation.
```

## 20.1 Snapshot as topology interchange

A future topology snapshot format could allow many systems to produce and consume shared topology structures.

A build system could emit it.

A database could export it.

An AI pipeline could mmap it.

An embedded system could include it.

A compiler could analyze it.

A Python tool could load it.

A hypergraph context engine could store higher-order relations in it.

No shared domain model is required.

## 20.2 Sectioned layout

The snapshot should likely use a sectioned layout.

Example:

```text
Header
Section table
Graph CSR section, optional
Graph CSC section, optional
Graph COO section, optional
Graph edge table section, optional
Hypergraph incidence section, optional
Hypergraph participant section, optional
Element identity section, optional
Relation identity section, optional
Domain mapping section, optional
Custom section, optional
```

Each section should include:

* kind,
* version,
* offset,
* length,
* alignment,
* flags,
* and possibly checksum.

Unknown sections should be safely skippable unless marked required.

## 20.3 Minimal graph snapshot

The smallest useful graph snapshot may support:

```text
Header
Section table
CSR offsets
CSR targets
```

That is enough for compact outgoing traversal.

A stronger graph snapshot may support optional CSC:

```text
CSC offsets
CSC sources
```

And optional COO:

```text
COO sources
COO targets
```

## 20.4 Minimal hypergraph snapshot

The smallest useful hypergraph snapshot may support:

```text
Header
Section table
Relation offsets
Participant element IDs
```

A stronger hypergraph snapshot may support:

```text
Participant-to-relation index
Incidence IDs
Endpoint roles
Source participant sections
Target participant sections
```

Graph and hypergraph snapshot sections should share container principles but not pretend to be the same physical layout.

---

# 21. Snapshot Compatibility Model

A snapshot format needs an explicit compatibility model from the beginning.

The compatibility model should define:

* magic bytes,
* format major version,
* format minor version,
* section kind identifiers,
* section version identifiers,
* required vs optional sections,
* skippable unknown sections,
* non-skippable required sections,
* endian policy,
* integer width policy,
* offset width policy,
* alignment policy,
* checksum policy,
* and feature flags.

## 21.1 Versioning principle

The snapshot format should allow evolution without breaking every reader.

A reader should be able to:

* reject unsupported major versions,
* accept compatible minor versions,
* skip unknown optional sections,
* reject unknown required sections,
* and expose metadata about unsupported features.

## 21.2 Required vs optional sections

Some sections are required for a particular view.

For example, a CSR graph view requires CSR offsets and targets.

Other sections may be optional.

For example, CSC may be optional unless the user requests incoming traversal.

The format should make this explicit.

## 21.3 Endian and integer policy

The format must choose an endian policy.

The format must also define which integer widths are allowed for:

* node IDs,
* edge IDs,
* offsets,
* counts,
* section lengths,
* and checksums.

Configurable ID widths may be useful, but they complicate readers.

The MVP may start with fixed widths and later generalize if benchmarks prove the need.

---

# 22. Safety Model

Zero-copy requires a serious safety model.

The project should make safety a first-class design area, not an implementation detail.

## 22.1 Safety principles

The safety model should include:

* all snapshot loading begins with validation,
* unsafe code is isolated to layout interpretation,
* every unsafe block documents its invariants,
* offset arithmetic is checked,
* section ranges are bounds-checked,
* integer overflow is checked,
* alignment is checked where required,
* all public traversal APIs preserve validation invariants,
* malformed snapshots cannot cause undefined behavior through safe APIs,
* fuzzing is part of the development process,
* and property tests compare builders, validators, and traversal results.

## 22.2 Validation levels

Large snapshots may need multiple validation modes:

* header-only validation,
* section table validation,
* layout-level validation,
* topology-level validation,
* full validation,
* and trusted unchecked construction for advanced unsafe contexts.

Unchecked paths must be explicit and unsafe.

The default path should be safe validation.

## 22.3 Validation invariants

For CSR, validation should check at least:

* section lengths are valid,
* offsets length is `node_count + 1`,
* offsets are monotonic,
* final offset equals target length,
* target node IDs are in range,
* optional edge IDs are in range,
* offset arithmetic cannot overflow,
* and all referenced sections are aligned or safely readable.

For CSC, equivalent reverse-index invariants apply.

For hypergraph incidence layouts, validation should check at least:

* relation offsets are monotonic,
* final relation offset equals participant length,
* participant element IDs are in range,
* role arrays match participant arrays where present,
* source/target sections are consistent where present,
* and incidence counts are valid.

---

# 23. Design Principle: `no_std` Core

The deepest core should support `no_std`.

This does not mean every crate must be `no_std`.

The foundation should be split so that:

* `oxgraph-topology` is `no_std`,
* `oxgraph-graph` is `no_std`,
* `oxgraph-hyper` is `no_std` where possible,
* traversal traits are `no_std`,
* snapshot reading can support `no_std` where possible,
* builders can require `alloc` or `std`,
* mmap support can require `std`,
* Python bindings can require `std`,
* interop crates can depend on external ecosystems.

`no_std` matters because it keeps the deepest layers minimal, portable, and usable in constrained environments.

But the project should not be marketed only as embedded software.

The deeper point is:

> If the core is clean enough for `no_std`, it is likely clean enough to serve as a true substrate.

---

# 24. Proposed Crate Ecosystem

The project should eventually be a family of crates organized by hierarchy.

However, the crate ecosystem should not be built all at once.

There is a difference between:

* architectural map,
* MVP crate set,
* and long-term ecosystem.

## 24.1 MVP crate set

The MVP should be small.

Recommended initial crates:

```text
oxgraph-topology
oxgraph-graph
oxgraph-csr
oxgraph-snapshot
graph-build
oxgraph-algo
```

A minimal `oxgraph-hyper` prototype may be included early to validate that `oxgraph-topology` is not accidentally graph-only, but it should not expand the first product promise.

## 24.2 Kernel crates

### `oxgraph-topology`

The minimal shared topology substrate.

Responsibilities:

* `no_std` topology primitives,
* typed IDs,
* element/relation/incidence vocabulary,
* identity layering concepts,
* cursor traits,
* capability traits,
* basic errors,
* low-level view contracts.

This crate should be extremely small.

### `oxgraph-graph`

The binary graph specialization.

Responsibilities:

* node and edge abstractions,
* directed and undirected graph traits,
* outgoing traversal,
* incoming traversal,
* endpoint lookup,
* graph capability traits,
* ergonomic graph APIs.

This crate should be what most ordinary graph users target.

### `oxgraph-hyper`

The hypergraph specialization.

Responsibilities:

* vertex and hyperedge abstractions,
* participant traversal,
* incidence traversal,
* directed hyperedge traits,
* source and target participant sets,
* role-aware endpoint APIs,
* hypergraph capability traits.

This crate should be first-class, not an afterthought, but it can mature after the graph snapshot path proves the architecture.

## 24.3 Layout crates

### `oxgraph-csr` or `graph-layout`

Canonical physical layouts for binary graphs.

Responsibilities:

* CSR,
* CSC,
* COO,
* packed adjacency arrays,
* optional edge tables,
* optional reverse indexes,
* graph layout validation helpers.

The first concrete layout crate should focus on CSR and optional CSC.

### `hyper-layout`

Canonical physical layouts for hypergraphs.

Responsibilities:

* relation offsets,
* participant arrays,
* incidence arrays,
* role arrays,
* source participant indexes,
* target participant indexes,
* participant-to-relation indexes,
* hypergraph layout validation helpers.

This should come after the graph snapshot path is proven.

## 24.4 Snapshot crates

### `topology-snapshot`

Shared snapshot container principles.

Responsibilities:

* binary header,
* section table,
* versioning,
* section metadata,
* validation framework,
* custom section handling,
* byte-slice loading,
* ABI-candidate discipline.

This can either be a separate crate or initially embedded inside `oxgraph-snapshot` until the abstraction proves itself.

### `oxgraph-snapshot`

Graph-specific snapshot sections.

Responsibilities:

* CSR sections,
* CSC sections,
* COO sections,
* edge table sections,
* graph zero-copy views.

### `hyper-snapshot`

Hypergraph-specific snapshot sections.

Responsibilities:

* relation participant sections,
* incidence sections,
* role sections,
* directed hyperedge sections,
* hypergraph zero-copy views.

This should not be built before the graph snapshot format has been validated.

## 24.5 Builder and mutation crates

### `graph-build`

Construction and freezing for ordinary graphs.

Responsibilities:

* ingest edge lists,
* remap external IDs to internal IDs,
* sort edges,
* deduplicate edges where requested,
* build CSR,
* build CSC,
* build COO,
* choose layout plans,
* write snapshots.

### `hyper-build`

Construction and freezing for hypergraphs.

Responsibilities:

* ingest hyperedge participant lists,
* ingest directed hyperedge source/target sets,
* remap external IDs,
* build incidence layouts,
* build participant indexes,
* write snapshots.

This should come later.

### `graph-mutate` and `hyper-mutate`

Optional mutable topology systems.

Responsibilities may include:

* owned mutable structures,
* append-only mutation,
* deletion models,
* overlays,
* delta layers,
* compaction,
* reindexing,
* freeze-to-snapshot workflows.

These should be designed conceptually early but implemented incrementally.

## 24.6 Algorithm crates

### `oxgraph-algo`

Algorithms over graph capability traits.

Initial algorithms:

* BFS,
* DFS,
* k-hop expansion,
* connected components,
* strongly connected components,
* topological sort,
* shortest paths,
* path enumeration,
* neighborhood extraction.

### `hyper-algo`

Algorithms over hypergraph capability traits.

Potential algorithms:

* incidence traversal,
* participant expansion,
* hyperedge neighborhood extraction,
* directed hyperedge reachability,
* hypergraph projection,
* higher-order connectivity,
* context group expansion.

Graph algorithms and hypergraph algorithms should not be forced into one crate if their semantics differ.

## 24.7 Interop crates

Interop crates bridge to existing ecosystems.

Possible targets:

* NetworkX,
* PyTorch Geometric,
* DGL,
* Arrow,
* Parquet,
* edge-list files,
* sparse matrix formats,
* GraphML,
* database-backed graph views,
* hypergraph formats,
* incidence matrix formats.

Interop is critical for adoption, but it is not part of the first substrate proof.

## 24.8 Python bindings

Python bindings are important for AI, data, and research users.

Potential features:

* open graph snapshots,
* open hypergraph snapshots,
* mmap snapshots,
* export edge indexes,
* export incidence indexes,
* convert to NetworkX,
* convert to PyTorch Geometric,
* run k-hop expansion,
* run path extraction,
* expose NumPy or Arrow views.

Researchers and AI engineers may not use Rust directly, but they may use a Rust-backed Python package if it solves performance and memory problems.

Python should not distort the core Rust design.

## 24.9 Downstream applications outside the library boundary

Domain-specific systems may be built on top of the library, but they are not part of the library hierarchy.

Examples include:

* property graph systems,
* RDF systems,
* knowledge graph systems,
* temporal graph systems,
* provenance systems,
* ML graph pipelines,
* dependency graph tools,
* AI context systems,
* agent memory systems,
* Shapes systems,
* and agent-native version-control systems.

These systems interpret topology.

The foundation does not.

The library should make these systems possible without naming them as internal layers or crate obligations.

---

# 25. Interop Between Graphs and Hypergraphs

Graphs and hypergraphs should not be merged into one awkward API.

But they should interoperate explicitly.

A hypergraph can be projected into a graph when needed.

A graph can be lifted into a hypergraph when needed.

These are explicit conversions with known tradeoffs.

## 25.1 Hypergraph to graph: incidence graph / star expansion

A hyperedge becomes a graph node connected to each participant.

Example:

```text
Hypergraph:
  H1 = {A, B, C}

Graph:
  H1 -- A
  H1 -- B
  H1 -- C
```

This preserves the hyperedge as an explicit object, but it changes it into a graph node.

## 25.2 Hypergraph to graph: clique expansion

A hyperedge becomes pairwise graph edges among all participants.

Example:

```text
Hypergraph:
  H1 = {A, B, C}

Graph:
  A -- B
  A -- C
  B -- C
```

This is useful for ordinary graph algorithms, but it loses the fact that the participants belonged to one shared hyperedge.

It can also be expensive: a hyperedge with `k` participants can create `k * (k - 1) / 2` undirected pairwise edges.

## 25.3 Graph to hypergraph

A binary graph can be lifted into a hypergraph by treating every edge as a two-participant hyperedge.

Example:

```text
Graph:
  A → B

Hypergraph:
  H1:
    source: A
    target: B
```

This is lossless for basic binary edge structure, but it may not add value unless the system wants to use hypergraph APIs uniformly.

## 25.4 Interop principle

The principle is:

> Graphs and hypergraphs are sibling topology specializations. They can be projected into one another, but those projections are explicit and may lose or transform information.

---

# 26. Relationship to AI and Context Engineering

AI is not the foundation’s domain, but AI increases the need for the foundation.

Recent AI systems increasingly use graph-like structures for:

* retrieval,
* memory,
* context organization,
* tool traces,
* provenance,
* reasoning paths,
* entity relationships,
* and multi-hop context expansion.

Ordinary graphs are useful for pairwise structure:

```text
document mentions entity
entity related_to entity
function calls function
package depends_on package
claim supported_by evidence
agent action produced artifact
memory contradicts memory
```

Hypergraphs are useful for higher-order context:

```text
conversation episode = {user, request, files, tool calls, answer, validation}
claim = {source A, source B, assumption, conclusion}
experiment = {dataset, model, prompt, metric, result}
workflow step = {inputs, actors, tools, outputs, evidence}
```

This matters for context engineering because context is often not merely pairwise.

A context event may involve many participants that should be retrieved as one unit.

The project should therefore support both:

```text
binary graphs for pairwise traversal
+
hypergraphs for higher-order context
```

But the core must still not know about AI.

AI context engineering belongs in downstream application or extension crates outside the core library hierarchy.

---

# 27. Relationship to Shapes and Agent-Native VCS

The substrate can eventually support Shapes, agent-native version control, provenance graphs, and structured intent systems.

But those should not define the core.

A Shapes system may use the topology substrate to represent:

* shape nodes,
* constraints,
* evidence,
* attestations,
* realizations,
* proposals,
* project states,
* state transitions,
* actors,
* intent links,
* and higher-order events.

Some of these may fit ordinary graphs.

Some may fit hypergraphs better.

For example, a proposal event involving actors, files, shapes, attestations, and target states may be modeled as a hyperedge or higher-order relation.

But to the substrate, these are still just topology.

This is the correct separation.

The topology substrate should be general enough that Shapes can use it, but Shapes should not be baked into it.

---

# 28. API Shape

The API should be minimal and capability-oriented.

The sketches below are illustrative, not final.

The most important rule is:

> Ordinary graph users should get ordinary graph APIs. Implementers may opt into topology-general APIs. Algorithms should declare the capabilities they require.

## 28.1 Topology-core sketch

A possible low-level topology abstraction:

```rust
pub trait TopologyBase {
    type ElementId: Copy + Eq;
    type RelationId: Copy + Eq;
}

pub trait IncidenceBase: TopologyBase {
    type IncidenceId: Copy + Eq;
    type Role;
}

pub trait TopologyCounts: TopologyBase {
    fn element_count(&self) -> usize;
    fn relation_count(&self) -> usize;
}

pub trait IncidenceCounts: IncidenceBase {
    fn incidence_count(&self) -> usize;
}

pub struct Endpoint<T: IncidenceBase> {
    pub incidence: T::IncidenceId,
    pub element: T::ElementId,
    pub relation: T::RelationId,
    pub role: T::Role,
}

pub trait RelationEndpointView: IncidenceBase {
    type Endpoints<'a>: Iterator<Item = Endpoint<Self>>
    where
        Self: 'a;

    fn endpoints(&self, relation: Self::RelationId) -> Self::Endpoints<'_>;
}
```

This is not meant to be the everyday graph user API.

It is the low-level topology contract.

## 28.2 Graph-core sketch

Graph users should get graph-specific APIs:

```rust
pub type NodeId<G> = <G as TopologyBase>::ElementId;
pub type EdgeId<G> = <G as TopologyBase>::RelationId;
pub type EndpointId<G> = <G as IncidenceBase>::IncidenceId;
pub type EndpointRole<G> = <G as IncidenceBase>::Role;

pub trait EdgeEndpointGraph: TopologyBase {
    fn source(&self, edge: Self::RelationId) -> Self::ElementId;
    fn target(&self, edge: Self::RelationId) -> Self::ElementId;
}

pub trait OutgoingGraph: TopologyBase {
    type OutEdges<'a>: Iterator<Item = Self::RelationId>
    where
        Self: 'a;

    fn outgoing(&self, node: Self::ElementId) -> Self::OutEdges<'_>;
}

pub trait IncomingGraph: TopologyBase {
    type InEdges<'a>: Iterator<Item = Self::RelationId>
    where
        Self: 'a;

    fn incoming(&self, node: Self::ElementId) -> Self::InEdges<'_>;
}
```

The aliases provide graph vocabulary without introducing a second identity
layer. Graph-specific traits add graph operations over topology IDs.

Algorithms then require only what they need:

```rust
fn bfs<G>(graph: &G, start: NodeId<G>)
where
    G: OutgoingGraph,
{
    // forward traversal
}

fn reverse_reachable<G>(graph: &G, start: NodeId<G>)
where
    G: IncomingGraph,
{
    // reverse traversal
}
```

## 28.3 Graph-core optimized capability sketch

For hot-path layouts, stronger capabilities may be exposed:

```rust
pub trait OutgoingTargetSliceGraph: TopologyBase {
    fn outgoing_targets(&self, node: Self::ElementId) -> &[Self::ElementId];
}

pub trait IncomingSourceSliceGraph: TopologyBase {
    fn incoming_sources(&self, node: Self::ElementId) -> &[Self::ElementId];
}
```

This lets high-performance algorithms avoid unnecessary abstraction overhead.

## 28.4 Hyper-core sketch

Hypergraph users should get hypergraph-specific APIs:

```rust
pub trait HypergraphBase {
    type VertexId: Copy + Eq;
    type HyperedgeId: Copy + Eq;

    fn vertex_count(&self) -> usize;
    fn hyperedge_count(&self) -> usize;
}

pub trait HyperedgeParticipants: HypergraphBase {
    type Participants<'a>: Iterator<Item = Self::VertexId>
    where
        Self: 'a;

    fn participants(&self, hyperedge: Self::HyperedgeId) -> Self::Participants<'_>;
}

pub trait IncidentHyperedges: HypergraphBase {
    type Incident<'a>: Iterator<Item = Self::HyperedgeId>
    where
        Self: 'a;

    fn incident_hyperedges(&self, vertex: Self::VertexId) -> Self::Incident<'_>;
}

pub trait DirectedHypergraph: HypergraphBase {
    type Sources<'a>: Iterator<Item = Self::VertexId>
    where
        Self: 'a;

    type Targets<'a>: Iterator<Item = Self::VertexId>
    where
        Self: 'a;

    fn sources(&self, hyperedge: Self::HyperedgeId) -> Self::Sources<'_>;
    fn targets(&self, hyperedge: Self::HyperedgeId) -> Self::Targets<'_>;
}
```

The important principle is:

> The substrate may be topology-general internally, but common graph use must remain graph-simple.

---

# 29. MVP

The MVP should be narrow enough to build, but broad enough to validate the architecture.

The MVP should prove:

> A topology can be represented through storage-agnostic views, frozen into a compact snapshot, validated from bytes, and traversed efficiently without domain semantics or heap reconstruction.

The MVP should focus on ordinary directed graphs first.

Hypergraphs should influence the design, but should not expand the first product surface beyond what can be proven.

## 29.1 MVP hierarchy

The MVP should include:

1. `oxgraph-topology`
2. `oxgraph-graph`
3. `oxgraph-csr` or `graph-layout` with CSR and optional CSC
4. `oxgraph-snapshot`
5. `graph-build`
6. minimal mutation capability traits
7. `oxgraph-algo`
8. optional minimal `oxgraph-hyper` prototype for architecture validation

The first optimized implementation should be ordinary directed graphs.

## 29.2 MVP graph features

### Core

* `no_std` topology primitives,
* typed node IDs,
* typed edge IDs,
* typed element/relation IDs where needed,
* outgoing capability trait,
* incoming capability trait,
* endpoint capability trait,
* cursor-based or iterator-based traversal,
* identity layering design,
* mutation capability traits.

### Layout

* CSR as first-class outgoing index,
* CSC as optional incoming index,
* optional COO as construction/interchange representation,
* optional edge table,
* fixed ID width initially unless benchmarks prove configurable widths are needed.

### Snapshot

* versioned binary header,
* section table,
* CSR section,
* optional CSC section,
* validation,
* zero-copy view over bytes,
* optional mmap support through a `std` feature or separate crate.

### Builder

* ingest edge list,
* compact external IDs into internal IDs,
* build CSR,
* optionally build CSC,
* write snapshot.

### Mutation

* mutable graph trait design,
* owned mutable builder prototype,
* append-only or builder mutation path,
* freeze into immutable graph snapshot.

Mutation engines beyond this are post-MVP.

### Algorithms

* BFS,
* DFS,
* k-hop expansion,
* reverse reachability if CSC exists,
* connected components,
* topological sort.

## 29.3 MVP demo

The best demo:

> Build a large graph, freeze it into a snapshot, memory-map it, validate it, and immediately run traversal queries without deserializing the graph into heap objects.

A strong headline:

> Open a 100M-edge graph and begin traversing it without rebuilding the graph in memory.

## 29.4 MVP success criteria

The MVP is successful only if:

* open time is near O(1) plus chosen validation cost,
* no heap reconstruction is required for traversal,
* CSR traversal is close to raw slice traversal,
* snapshot size is close to raw CSR arrays plus bounded metadata overhead,
* validation catches malformed offsets and out-of-range IDs,
* algorithms can run over traits without depending on concrete storage,
* the API remains understandable to ordinary graph users,
* and the design leaves room for hypergraphs without forcing graph users through hypergraph abstractions.

---

# 30. Benchmarks and Proof

A foundation-level crate needs proof.

The project should benchmark:

* snapshot build time,
* mutation overhead where implemented,
* freeze time,
* snapshot size,
* open time,
* validation time,
* allocation count on open,
* outgoing traversal throughput,
* incoming traversal throughput,
* incidence traversal throughput where applicable,
* BFS throughput,
* reverse traversal throughput,
* k-hop expansion throughput,
* memory usage,
* cache locality,
* and performance against simple baselines.

Benchmarks should compare against:

* hand-written adjacency vectors,
* flat CSR arrays,
* naive deserialization,
* custom domain-specific layouts,
* and, where relevant, hypergraph-specific libraries or incidence representations.

The benchmarks must be honest.

If a simpler representation wins, understand why.

The goal is not marketing.

The goal is to force the abstraction to stay real.

## 30.1 Benchmark contract

The project should publish benchmark contracts such as:

```text
Graph snapshot open:
  Input: validated or partially validated snapshot bytes
  Requirement: no graph heap reconstruction

CSR traversal:
  Input: CSR snapshot view
  Requirement: within an acceptable overhead of raw slice traversal

Validation:
  Input: malformed and valid snapshots
  Requirement: safe rejection of malformed data through public APIs

Algorithm genericity:
  Input: multiple graph backends implementing the same capability
  Requirement: algorithms run without backend-specific code
```

This prevents the project from hiding behind abstraction.

---

# 31. Adoption Strategy

Adoption should happen in layers.

The project should not begin by trying to convince every graph-like domain to adopt a universal topology model.

It should win one concrete wedge first.

## 31.1 First wedge: large immutable or build-once directed graphs

The first wedge should be concrete:

> Large directed graphs that need fast traversal without heap reconstruction.

Examples:

* dependency graphs,
* package graphs,
* build graphs,
* static analysis graphs,
* provenance graphs,
* precomputed AI retrieval graphs,
* embedded state graphs.

This wedge proves the performance and storage claims.

## 31.2 First users: Rust systems developers

Initial users are likely to be people building:

* compilers,
* static analyzers,
* package managers,
* build tools,
* dependency resolvers,
* workflow engines,
* database internals,
* embedded systems,
* graph analytics tools,
* and AI infrastructure.

These users value performance, memory layout, `no_std`, and predictable APIs.

## 31.3 Second wedge: mutable topology

The next wedge should show that mutability is not an afterthought.

Examples:

* mutable build graph,
* mutable dependency graph,
* append-only provenance graph,
* snapshot plus overlay,
* incremental static analysis graph.

## 31.4 Third wedge: hypergraph and higher-order context

The hypergraph path should prove the oxgraph-topology abstraction.

Examples:

* AI context episodes,
* grouped memories,
* multi-input/multi-output provenance events,
* workflow joins/splits,
* constraints,
* directed hyperedges.

## 31.5 Broader users: graph/data/AI ecosystems

Broader adoption requires interop.

The project should eventually support:

* Python bindings,
* NetworkX conversion,
* PyTorch Geometric conversion,
* DGL conversion,
* Arrow export,
* Parquet export,
* edge-list import/export,
* sparse tensor interop,
* and incidence matrix interop.

AI researchers may not use the Rust core directly, but they may use a Rust-backed Python package if it solves performance and memory problems.

## 31.6 Downstream domain systems

Once the substrate is stable, domain-specific systems can build on it outside the library hierarchy.

Examples:

* knowledge graph systems,
* property graph systems,
* provenance systems,
* temporal graph systems,
* dependency graph tools,
* AI memory systems,
* AI context engineering systems,
* Shapes systems,
* and agent-native version-control systems.

These downstream systems should validate the substrate’s neutrality.

If many domains can build on it without requiring changes to the core, the design is working.

---

# 32. Strategic Differentiation

The project is differentiated by the combination of:

1. a tiny shared topology substrate,
2. ergonomic binary graph specialization,
3. first-class hypergraph specialization in the architecture,
4. storage-agnostic views,
5. zero-copy-friendly snapshots,
6. `no_std` core layers,
7. optional topology indexes,
8. canonical CSR/CSC/COO layouts for graphs,
9. incidence layouts for hypergraphs,
10. layered identity model,
11. capability-based APIs,
12. opt-in mutability,
13. algorithms over views,
14. safe validation,
15. and interop with existing ecosystems.

Any one of these is not enough.

Together, they form a foundation.

But the project should not try to win by having the most features.

It should win by having the cleanest boundary and the strongest first wedge.

The sharpest wedge is:

> Validated, zero-copy, storage-agnostic graph snapshots and traversal APIs for large directed graphs.

The broader topology substrate grows from that proof.

---

# 33. Non-Negotiables

These are the rules the project should not violate.

## 33.1 The core must not interpret domain meaning

No domain semantics in the foundation.

## 33.2 The hierarchy must remain clear

`oxgraph-topology` is the deepest substrate.

`oxgraph-graph` and `oxgraph-hyper` are sibling specializations.

Product-specific domain systems are downstream consumers, outside the library hierarchy.

## 33.3 The core must remain small

The deepest crates should be boring, stable, and dependency-light.

## 33.4 Storage must remain abstract

The view APIs must not assume heap ownership.

## 33.5 Common graph traversal must remain efficient

Ordinary graph traversal should remain close to raw slice traversal.

Hypergraph generality must not make the common graph path slow or awkward.

## 33.6 Layout choices must be explicit

Users should choose physical indexes based on workload.

## 33.7 Mutation capabilities must be designed into v1

Mutation should be capability-based and opt-in.

Production-grade mutation engines can come later.

## 33.8 Safety must be designed in

Zero-copy requires strong validation, clear invariants, fuzzing, and minimal unsafe code.

## 33.9 Snapshot format must be byte-level

The snapshot format must not be Rust struct serialization.

## 33.10 Stable ABI must be earned

The snapshot format can be an ABI candidate early.

It should only be called stable after real-world validation.

## 33.11 Interop matters

The project should become a substrate, not a silo.

## 33.12 The MVP must stay narrow

The first version should prove one hard thing well.

The first hard thing is validated zero-copy traversal for large directed graph snapshots.

---

# 34. Open Design Questions

The following questions remain open and should be resolved through prototypes and benchmarks.

## 34.1 How minimal should `oxgraph-topology` be?

Too small, and it is useless.

Too large, and it becomes abstract and confusing.

The right balance must be found through real graph and hypergraph implementations.

The inclusion rule should be strict:

> A concept belongs in `oxgraph-topology` only if concrete implementations prove it is necessary.

## 34.2 Should oxgraph-graph expose oxgraph-topology traits directly?

Graph users should not need to think in terms of incidence.

Graph-core should be a specialization of oxgraph-topology, not a second identity substrate.

The current rule is: graph nodes are topology elements, graph edges are topology relations, and oxgraph-graph may provide graph-facing aliases for those associated types. Graph users should not need to think in terms of incidence, but oxgraph-graph should not introduce independent `NodeId` and `EdgeId` associated types that shadow `ElementId` and `RelationId`.

## 34.3 How should canonical IDs interact with layout-local IDs?

The system should support topology-local identity, canonical substrate identity, and domain identity.

But the exact API for converting between them must be designed carefully.

## 34.4 How should mutation preserve or invalidate IDs?

Mutation affects identity.

The system must define what happens to local IDs, canonical IDs, and domain mappings under insertion, deletion, compaction, sorting, and freezing.

## 34.5 Should mutable IDs be generational?

Generational IDs can prevent stale handle bugs, but they add storage and API complexity.

The design should decide whether generational IDs belong in mutable crates, core identity traits, or optional ID policies.

## 34.6 How should validation levels work?

Large snapshots may need different validation modes:

* header-only,
* section-level,
* topology-level,
* full validation,
* trusted unchecked unsafe construction.

The API must make these modes explicit.

## 34.7 How should custom sections be exposed?

Custom sections are important for extension, but the core must not interpret them.

The API should expose them as typed or untyped byte regions with safe bounds.

## 34.8 How should external ID mapping work?

External IDs are essential for applications, but they do not belong in the core topology model.

Builders and downstream application layers should handle mapping tables.

## 34.9 How much should the crate optimize for Python and ML?

Python and ML interop are important for adoption, but they should not distort the core design.

The foundation should remain Rust-first and substrate-first.

## 34.10 How much hypergraph functionality belongs in v1?

Hypergraphs should be first-class in the architecture.

But the first optimized implementation should focus on ordinary directed graphs.

A minimal `oxgraph-hyper` prototype may be useful early to ensure the topology abstraction is not accidentally graph-only.

## 34.11 What is the exact snapshot versioning policy?

The format needs a precise answer for:

* major versions,
* minor versions,
* section versions,
* required sections,
* optional sections,
* unknown sections,
* and feature flags.

## 34.12 What is the acceptable abstraction overhead?

The project needs an explicit benchmark target for how close CSR traversal through traits must be to raw slice traversal.

If the abstraction overhead is too high, the substrate will not earn its systems-level claim.

---

# 35. The Long-Term Vision

The long-term vision is that graph-like systems stop reinventing their topology substrate.

A future system should be able to say:

```text
We expose a graph view.
```

or:

```text
We expose a hypergraph view.
```

or:

```text
We emit a topology snapshot.
```

or:

```text
We consume any topology implementing the required capabilities.
```

That should be enough for algorithms, tools, interop layers, and domain systems to compose.

In the long term, this project could support:

* memory-mapped graph datasets,
* embedded static graphs,
* graph database snapshot exports,
* compiler graph analysis,
* build graph tooling,
* dependency graph tooling,
* mutable graph overlays,
* AI graph retrieval,
* hypergraph context memory,
* knowledge graph engines,
* provenance graph systems,
* topology diffs,
* distributed topology views,
* and downstream domain-specific topology systems.

But the foundation remains the same:

> identity, topology, traversal, layout, mutation capabilities, validation, and storage abstraction.

The long-term ambition is broad.

The first proof is narrow.

That is the strategic discipline of the project.

---

# 36. The One-Sentence Vision

> Build a storage-agnostic, zero-copy-friendly topology access layer for Rust: a high-performance, `no_std`-capable foundation for graph-like structures, traversal, indexing, snapshots, mutation capabilities, and algorithms, starting with large immutable directed graphs and keeping all domain meaning above the core.

---

# 37. The Short Manifesto

Graphs are everywhere, but graph infrastructure is fragmented.

Hypergraphs and higher-order relations are increasingly important, but they are often forced into ordinary graph encodings.

Every domain reinvents identity, topology storage, traversal, serialization, mutation, snapshots, algorithms, and interop.

Graph databases solve database problems.

Knowledge graph systems solve semantic problems.

ML frameworks solve tensor problems.

Application code solves local problems.

But systems software still lacks a common topology substrate.

This project defines that substrate.

It separates topology from meaning.

It separates access from storage.

It separates physical indexes from logical semantics.

It separates binary graph APIs from hypergraph APIs while grounding both in a shared topology foundation.

It lets algorithms target capabilities instead of concrete data structures.

It lets snapshots be validated and traversed without deserialization.

It lets mutable systems opt into mutation without making every topology mutable.

It lets domains build their own meaning above a shared foundation.

The core does not know about AI, RDF, GraphRAG, provenance, databases, or application objects.

The core knows topology.

Build semantics above it.

Build storage below it.

Share the topology layer in between.

Start with the sharpest proof:

> large directed graphs, compact snapshots, validated bytes, zero-copy traversal.

Then grow outward.

---

# 38. Final Guiding Phrase

> **Graphs everywhere. Topology here. Meaning elsewhere.**

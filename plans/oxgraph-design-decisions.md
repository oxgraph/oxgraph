# OxGraph foundational design decision log

## Context

We are pausing implementation to align on the long-lived primitives before adding Python exposure, builders, weights, ranking, or hypergraph expansion behavior. The goal is to keep OxGraph foundational, substrate-agnostic, and consistent with the vision: **Topology here. Meaning elsewhere. Storage anywhere.**

This file is a working decision log. Each open question should end as either:

- a concrete decision to fold back into `plans/python-bindings-birdsong.md`, `.shapes/`, and `docs/architecture.md`, or
- a research gate with explicit scope and evidence needed before implementation.

## Approach

Work through one design question at a time. For each question:

1. State the design pressure and why it matters.
2. List the durable options.
3. Recommend one option.
4. Confirm the decision with the user.
5. Record the decision and its consequences.

## Files to modify later

- `plans/python-bindings-birdsong.md` — revise implementation phases after decisions are made.
- `.shapes/...` — add/amend shapes before any code changes.
- `docs/architecture.md` — record the finalized architecture and rationale.
- Rust crates only after shape approval.

## Reuse

- `vision.md` — especially the constraints: topology here, meaning elsewhere, storage anywhere; capabilities over assumptions; ordinary graphs and hypergraphs as siblings; Python must not distort Rust substrate design.
- `.shapes/shapes/2-topology-substrate.yaml` — minimal no_std topology vocabulary.
- `.shapes/shapes/13-capabilities-over-assumptions.yaml` — optional traits for concrete capabilities.
- `.shapes/shapes/16-mutation-capabilities.yaml` — mutation/builder boundary is deliberately pending.
- `.shapes/shapes/9-substrate-agnostic-algorithms.yaml` — algorithms bind on topology capabilities.

## Decision queue

1. **Foundation inclusion rule** — what test must a primitive pass before entering `oxgraph-topology`?
2. **Weight primitive shape** — exact `ElementWeight`, `RelationWeight`, and `IncidenceWeight` contracts.
3. **Weighted expansion shape** — whether direct weighted successor/predecessor capabilities belong in topology or only in algorithm adapters.
4. **Probability/stochastic policy boundary** — how algorithms accept normalization/damping/priors without turning topology into a probabilistic model.
5. **PageRank/ranking scope** — ordinary graph first, topology-generic kernel, and hypergraph research gate.
6. **Canonical/local/domain identity semantics** — stability, query APIs, cache generations, and invalidation.
7. **Builder vs mutation boundary** — append-only builders, freeze/export, and what remains out of scope.
8. **Snapshot/sidecar persistence for weights and identity maps** — what becomes a section vs borrowed sidecar vs custom section.
9. **Python facade boundary** — expose OxGraph concepts without adding third-party exporters or cloning another API.
10. **Unsafe boundary policy** — exact crate-local exception shape if PyO3/maturin requires unsafe glue.
11. **Verification contract** — docs, proptests, Kani, benchmarks, golden fixtures, and performance contracts.

## Decisions

### Decision 1: foundation inclusion rule

Accepted rule:

> A concept belongs in `oxgraph-topology` only if it is a shared primitive of graph-like topology families: domain-neutral, storage-agnostic, `no_std`-compatible, useful to both ordinary graph and hypergraph families, expressible as an optional capability trait, and not forcing all backends to pay for it.

Interpretation:

- `oxgraph-topology` is the primitive substrate for all graph families that share discrete topology concepts.
- Weights can enter topology as optional capabilities because they are neutral typed values keyed by element/relation/incidence IDs.
- Probabilities do not enter topology because they imply stochastic interpretation and modeling parameters.
- PageRank does not enter topology because it is an algorithm over capabilities.
- Python facades, builders, snapshots, metadata, and domain labels stay outside topology.

## Current question

Question 2: define the exact weight primitive shape for `oxgraph-topology`.

Candidate decision under review:

> Add `ElementWeight`, `RelationWeight`, and `IncidenceWeight` as independent optional capability traits in `oxgraph-topology`. Each trait has its own associated weight type and a lookup method keyed by the corresponding topology ID. Weight traits do not imply what the number means; algorithms add numeric constraints only where needed.

Confirmed so far:

- Weight lookup should return a value directly, not `Option<Weight>`.
- Implementing a weight capability means the view can answer a weight for every visible ID of that kind.
- Sparse/default behavior belongs in adapters or higher layers.

Open details:

- Should each trait have an independent associated type (`ElementWeight`, `RelationWeight`, `IncidenceWeight`), or should a view have one shared `Weight` associated type for all weighted IDs?
- Should topology require `Copy` weights, or leave weight bounds entirely to algorithms?
- How should multiple possible weights/statistics be represented without turning topology into a property graph? Candidate direction: the topology trait represents one typed weight view at a time; multiple named/derived weights are separate sidecar views or adapters that each implement the same capability.
- Architecture sketch for many weight layers:
  - The base topology view owns/connects elements, relations, incidences, ids, and traversal.
  - A weight layer is a typed sidecar or computed view keyed by element/relation/incidence ids.
  - A weighted topology view wraps `(&base_topology, &weight_layer)` and delegates topology traits to the base while implementing exactly one weight capability for that layer.
  - Many weight layers are many wrappers over the same base topology, not many key/value fields inside `oxgraph-topology`.
  - In Rust, callers choose the layer by choosing which sidecar/wrapper they pass to an algorithm.
  - If an algorithm needs multiple inputs at once, a derived/composite weight view can read several layers and expose the exact value or tuple the algorithm requires. Examples: `0.7 * distance + 0.3 * risk`, `(cost, capacity)`, or an algorithm-specific feature struct.
  - Algorithms that are inherently multi-objective should declare that explicitly in `oxgraph-algo` with algorithm-specific input traits/types rather than forcing `oxgraph-topology` to become a named property map.
  - In Python, the facade may keep a registry of named layers for ergonomics, but the native algorithm call receives an explicit selected layer, derived layer, or multi-input policy at a time.
- Research note needed: survey graph-theory and major graph-library conventions for single edge weight vs multiple attributes/weight functions, multi-objective graph algorithms, and how algorithms select or derive weights.

Decision 2a: key/value properties and named weight layers do not belong in `oxgraph-topology`.

Accepted split:

- Many graph libraries expose properties/attributes for vertices, edges, and sometimes incidences, and OxGraph needs a serious ecosystem story for these if it is to be the graph layer other systems sit on.
- However, key/value properties introduce concerns that are not pure topology: keys/names, schemas, value typing, allocation, string interning, namespaces, runtime lookup errors, serialization format, and possible application meaning.
- Naming a weight layer is useful at builder/Python/application boundaries, but an algorithm does not need the name; it needs the selected weight function or selected typed view.
- Therefore:
  - `oxgraph-topology`: primitive capability shape only — IDs, traversal, incidence, and one selected typed element/relation/incidence weight view.
  - future `oxgraph-payload` / `oxgraph-property` / build-layer registries: named typed columns/layers keyed by element/relation/incidence IDs.
  - `oxgraph-snapshot`: optional/custom sections for persisted named layers, with the container not interpreting domain meaning.
  - `oxgraph-algo`: consumes explicit typed views or algorithm-specific input policies, not runtime property names.
  - Python facade: may expose ergonomic names/registries over OxGraph-owned layers while passing one selected/composed typed view to native algorithms.

Decision 2b: element, relation, and incidence weights are independent capability traits.

Accepted shape:

```rust
pub trait ElementWeight: TopologyBase {
    type Weight;

    fn element_weight(&self, element: Self::ElementId) -> Self::Weight;
}

pub trait RelationWeight: TopologyBase {
    type Weight;

    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight;
}

pub trait IncidenceWeight: IncidenceBase {
    type Weight;

    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight;
}
```

Rationale:

- Each capability can be implemented independently.
- Algorithms can require only the weighted ID family they need.
- Element, relation, and incidence weights can have different types.
- This matches the established capability-over-assumption pattern.

Decision 2c: topology weight return representations must be `Copy`.

Accepted rule:

> `ElementWeight::Weight`, `RelationWeight::Weight`, and `IncidenceWeight::Weight` must be `Copy` return representations.

Rationale:

- Topology traversal must stay cheap and predictable.
- Non-`Copy` owned values such as `BigRational`, symbolic expressions, uncertainty distributions, or high-dimensional vectors are valid upper-layer payloads, but returning them by value from a topology hot path would force clone/allocation or hide non-constant work.
- `Copy` does not restrict the underlying stored value to a scalar. The returned representation may be a numeric scalar, compact handle, or shared reference into a sidecar/property layer.
- Algorithms add stronger bounds such as finite, non-negative, ordered, additive, `Into<f64>`, semiring-like, etc.

Accepted weight capability sketch:

```rust
pub trait ElementWeight: TopologyBase {
    type Weight: Copy;

    fn element_weight(&self, element: Self::ElementId) -> Self::Weight;
}

pub trait RelationWeight: TopologyBase {
    type Weight: Copy;

    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight;
}

pub trait IncidenceWeight: IncidenceBase {
    type Weight: Copy;

    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight;
}
```

Question 2 is now decided unless later shape review finds a trait-lifetime issue for borrowed/reference weights.

### Decision 3: weighted expansion stays out of topology for now

Accepted rule:

> Do not add `WeightedElementSuccessors` / `WeightedElementPredecessors` or equivalent direct weighted-expansion traits to `oxgraph-topology` in this slice.

Rationale:

- Primitive weights (`ElementWeight`, `RelationWeight`, `IncidenceWeight`) are shared graph-family concepts.
- Direct `(successor, weight)` expansion may be derived differently for ordinary graphs, directed hypergraphs, projections, stochastic traversals, or algorithm-specific policies.
- For hypergraphs especially, successor weight may depend on relation weight, incidence weight, source/target participant cardinalities, projection choice, or normalization policy.
- Algorithms can compose existing topology traversal and primitive weight capabilities through algorithm-layer adapters.
- Add weighted expansion later only if concrete graph and hypergraph implementations prove it is a reusable primitive with a durable contract.

## Current question

### Decision 4: probabilities and stochastic policies live above topology

Accepted rule:

> Probabilities are not topology primitives. A probability is a normalized or modeled interpretation of topology + weights under caller/domain parameters.

Boundary:

- `oxgraph-topology` exposes connectivity, ids, incidence, and primitive element/relation/incidence weights.
- `oxgraph-algo` may provide generic normalization and stochastic traversal/ranking policy types such as row normalization, damping, teleport/priors, and dangling-node handling.
- Computed probability vectors/layers may be represented as ordinary typed weight layers once computed, but topology does not know they are probabilities.
- Algorithms receive explicit policy parameters and typed weight views; they should not inspect arbitrary metadata or dynamic property names.

## Current question

Question 5: define PageRank/ranking scope holistically.

User-aligned direction:

> OxGraph should fully support canonical PageRank semantics and should not land a half-baked ordinary-graph-only design that blocks hypergraph support later. The implementation may still be phased, but the design must cover ordinary graphs and directed hypergraphs before code begins.

Open details:

- Define "canonical PageRank" for ordinary directed graphs: directed Markov-chain interpretation, damping, teleport/personalization vector, dangling-node handling, weighted edge normalization, convergence norm, tolerance, max iterations, initialization, and error behavior.
- Decide whether PageRank uses push-style outgoing traversal as an implementation strategy while preserving canonical semantics.
- Decide the topology capability bundles required for ordinary graphs: likely counts/indexing, outgoing edge traversal, edge endpoint lookup, and optional relation weights.
- Research directed-hypergraph PageRank/projection approaches before implementation. Hypergraph ranking should not be treated as secondary; it should be designed alongside graph PageRank even if the first executable adapter lands later.
- Decide which hypergraph projection/lift semantics OxGraph treats as built-in primitives vs algorithm-layer policies: clique/star expansion, incidence bipartite walk, source-to-target directed expansion, cardinality-normalized hyperedge walks, relation-weighted and incidence-weighted walks, or tensor/higher-order approaches.
- Define how PageRank accepts stochastic policy parameters without moving probabilities into topology.

### Decision 5a: canonical ordinary directed PageRank baseline

Accepted baseline for ordinary directed graph PageRank:

- Support unweighted PageRank with implicit edge weight `1`.
- Support weighted PageRank over relation weights.
- Edge weights used for PageRank must be finite and non-negative after conversion to the algorithm's numeric type.
- Outgoing edge weights are row-normalized into transition probabilities inside `oxgraph-algo`.
- Zero-total outgoing rows are treated as dangling rows.
- Dangling nodes distribute according to the personalization/teleport vector.
- Damping defaults to `0.85` and is configurable.
- Personalization/teleport vector is optional; default is uniform over visible elements.
- Convergence uses an L1 rank-delta criterion unless later research/compatibility review changes this.
- Tolerance and maximum iterations are configurable.
- Failure to converge returns a typed convergence error, not a panic.
- Invalid weights or invalid policy parameters return typed errors.
- Deterministic iteration order is required where the view exposes deterministic dense indexes.

Remaining Question 5b:

- Which directed-hypergraph transition/projection policies should OxGraph support for PageRank/ranking, and which should be built-in vs research-gated? This requires literature review before implementation.

Current implementation context:

- `oxgraph-hyper-bcsr` is explicitly a borrowed bipartite CSR layout.
- It stores hyperedge-major head/tail participant sections and vertex-major outgoing/incoming hyperedge sections.
- It already implements incidence/hypergraph capabilities such as `RelationIncidences`, `ElementIncidences`, `IncidentHyperedges`, and `DirectedHyperedgeParticipants`.
- It also implements `ElementSuccessors` / `ElementPredecessors` as a projected directed source-to-target expansion, so projected walks are possible, but the most layout-native hypergraph-aware walk is the bipartite/incidence walk.

Tentative alignment:

> For the current hypergraph ranking path, the first built-in hypergraph-aware PageRank policy should be incidence/bipartite PageRank because it matches `oxgraph-hyper-bcsr`, preserves hyperedge identity, and avoids pretending every hyperedge is only a set of binary edges.

Decision 5b-1: incidence/bipartite PageRank ranks both vertices and hyperedges.

Accepted rule:

> The first hypergraph-aware PageRank policy should model the bipartite/incidence walk with both element states and relation states. It returns rank scores for vertices/elements and hyperedges/relations, not only vertex scores with hyperedges as hidden transient routing states.

Rationale:

- This matches the BCSR representation: one side is vertices/elements, the other is hyperedges/relations, and incidences connect them.
- It preserves hyperedge identity and lets hyperedges receive importance scores directly.
- Vertex-only scores can still be derived by selecting/projecting the element side of the result.
- Relation scores can support later use cases such as important event/group/context ranking without changing topology semantics.

Decision 5b-2: directed hypergraph PageRank is directional by default.

Accepted rule:

> Canonical PageRank over ordinary directed graphs follows outgoing directed edges. The directed-hypergraph analogue should therefore follow source/head participants into a relation and then relation out to target/tail participants.

For a directed hyperedge with sources `[A, B]` and targets `[C, D]`, the default directed incidence walk is:

```text
A -> H
B -> H
H -> C
H -> D
```

not the reverse unless a reverse relation exists or a separate undirected/all-participant policy is selected.

Rationale:

- This preserves the canonical PageRank interpretation that rank flows along directed links.
- It maps cleanly to BCSR's vertex outgoing hyperedges and hyperedge target participants.
- Undirected/all-participant incidence PageRank can exist later as a separate policy, not the directed default.

Decision 5b-3: default directed hypergraph weighting policy

Accepted default for directed incidence/bipartite PageRank:

- The walk has two row-normalized steps: source element -> relation, then relation -> target element.
- At a source element, relation weights choose among outgoing hyperedges. If no relation weights are supplied, outgoing hyperedges are uniform.
- At a relation, target incidence weights choose among target elements. If no target incidence weights are supplied, target elements are uniform.
- Source incidence weights do not participate in the default formula. They may be supported later through custom/algorithm-specific policies if needed.

Concrete example:

```text
H1: sources [A, B], targets [C, D]
H2: sources [A], targets [E]
relation_weight(H1) = 9
relation_weight(H2) = 1
target_incidence_weight(H1, C) = 3
target_incidence_weight(H1, D) = 1
```

Then:

```text
P(A -> H1) = 0.9
P(A -> H2) = 0.1
P(H1 -> C) = 0.75
P(H1 -> D) = 0.25
P(A -> H1 -> C) = 0.675
P(A -> H1 -> D) = 0.225
```

Decision 5b-4: bipartite dangling mass uses full-state personalization

Accepted rule:

> Incidence/bipartite PageRank runs over one combined state space: elements + relations. Dangling mass from either an element state with no outgoing relations or a relation state with no target elements redistributes according to one personalization vector over the full bipartite state space.

Defaults:

- If no personalization vector is supplied, use uniform mass over all visible element and relation states.
- Caller-supplied personalization may weight elements and relations differently, but must define a valid non-negative, normalizable distribution over the combined state space.

Rationale:

- This preserves the canonical Markov-chain interpretation of PageRank over the actual bipartite state graph.
- It avoids special-case rank sinks for either side of the bipartite layout.
- Element-only or relation-only summaries can still be derived by selecting the relevant side after ranking.

Open design details for PageRank/ranking:

- Should OxGraph also expose a separate projected source-to-target PageRank policy later because `BcsrHypergraph` already supports `ElementSuccessors`?
- How early should PageRank land relative to other foundational algorithms such as DFS, SCC, topological sort, shortest paths, connected components, and k-hop expansion?
Decision 5c: use the canonical name PageRank.

Accepted naming:

- Use `pagerank` / `PageRank` for the canonical algorithm.
- Naming research is recorded in `plans/ranking-naming-research.md`: major libraries/databases use `PageRank`/`pagerank` for the canonical algorithm.
- Broader conceptual names such as stationary distribution, random-surfer/random-walk ranking, Markov-chain stationary ranking, eigenvector centrality variant, personalized PageRank, random walk with restart, ArticleRank, Katz, HITS, and graph diffusion are useful context but should not replace the canonical PageRank name.
- A broad `rank` module may still make sense as an organization point, but the public algorithm should be named `pagerank`.

Decision 5d: projected hypergraph PageRank is a separate explicit policy

Accepted rule:

> OxGraph should eventually support projected directed-hypergraph PageRank over element successors, but it must be an explicit separate policy from incidence/bipartite PageRank and not the default hypergraph-aware policy.

Rationale:

- `BcsrHypergraph` already implements `ElementSuccessors` / `ElementPredecessors`, so projected source-to-target ranking is possible.
- Projection gives users graph-like vertex/element ranks that are easier to compare with ordinary graph PageRank.
- Projection loses relation/hyperedge rank and some one-relation-many-participants semantics, so it should not replace incidence/bipartite PageRank.
- The default hypergraph-aware policy remains incidence/bipartite ranking over elements + relations.

## Current question

Question 6: canonical/local/domain identity semantics.

Candidate decision under review:

> OxGraph should define rigorous identity invariants before implementing builders/Python. Layout-local IDs are dense and layout-specific; canonical substrate IDs are stable within a builder generation or snapshot; domain IDs/labels are opaque application/facade mappings. Freeze/export creates local-to-canonical maps whenever layout order may differ from canonical order. Builder edits after freeze create a new generation and invalidate borrowed frozen caches unless those caches were materialized as owned snapshots/views.

Decision 6a: global/canonical IDs are opt-in, and guaranteed when present

Accepted rule:

> Global/canonical substrate IDs are an optional capability. Views/builders that opt into canonical identity must guarantee the documented mapping and stability contract. Views that do not opt in expose only layout-local IDs and make no global identity promise.

Consequences:

- Layout-local IDs remain dense and optimized for traversal.
- Canonical substrate IDs are not required for every minimal topology view.
- If a view implements canonical identity, users must be able to ask for the canonical ID corresponding to a local element/relation/incidence slot where that ID family exists.
- Local reordering is acceptable for canonical-identity views only if the local -> canonical mapping is available and correct.
- `local == canonical` is allowed as a zero-cost identity-map implementation when the layout preserves canonical order.
- Domain IDs/labels remain opaque and outside topology.

Decision 6b: first canonical identity scope is one generation/view/snapshot

Accepted rule:

> The first canonical identity guarantee is stable within one builder generation, frozen view, or snapshot. Cross-snapshot or cross-dataset lineage identity is a later optional capability/policy.

Rationale:

- This keeps the first builder/freeze/snapshot layer rigorous without solving dataset versioning up front.
- Canonical IDs can connect multiple layouts/caches describing the same logical topology inside one generation or snapshot.
- Cross-snapshot stability may require lineage metadata, mutation logs, tombstone policies, compaction policies, or external ID mapping and should not be smuggled into the first canonical identity capability.

Decision 6c: first builder canonical IDs are dense append-only integers

Accepted rule:

> The first graph/hypergraph builders assign canonical element, relation, and incidence IDs as dense append-only integer sequences with no reuse within a generation.

Meaning:

- First element gets canonical element ID `0`, next gets `1`, and so on.
- First relation gets canonical relation ID `0`, next gets `1`, and so on.
- First incidence gets canonical incidence ID `0`, next gets `1`, and so on.
- Adding a new item never renumbers existing canonical IDs.
- Deletion, tombstones, compaction, and generational reuse are out of scope for this first builder layer.

Rationale:

- Dense IDs index sidecar arrays directly.
- Append-only assignment makes Python/facade handles stable during one builder generation.
- Freeze/export may reorder layout-local IDs for traversal, but local -> canonical maps preserve identity.

Decision 6d: canonical -> local lookup is a separate optional capability

Accepted rule:

> For views that opt into canonical identity, local -> canonical lookup is part of the canonical identity capability. Reverse canonical -> local lookup is a separate optional capability because it may require extra memory or may be partial for filtered views.

Implications:

- `canonical_element_id(local)` / `canonical_relation_id(local)` / `canonical_incidence_id(local)` are guaranteed by canonical identity capabilities.
- `local_element_id(canonical)` / `local_relation_id(canonical)` / `local_incidence_id(canonical)` are separate capabilities and should return `Option<LocalId>` because filtered/projected views may not contain every canonical ID.
- `local == canonical` remains a valid zero-cost implementation when layout order preserves canonical order.

Decision 6e: canonical identity trait names

Accepted trait naming:

```rust
pub trait CanonicalElementIdentity: TopologyBase {
    type CanonicalElementId: TopologyId;

    fn canonical_element_id(&self, element: Self::ElementId) -> Self::CanonicalElementId;
}

pub trait CanonicalRelationIdentity: TopologyBase {
    type CanonicalRelationId: TopologyId;

    fn canonical_relation_id(&self, relation: Self::RelationId) -> Self::CanonicalRelationId;
}

pub trait CanonicalIncidenceIdentity: IncidenceBase {
    type CanonicalIncidenceId: TopologyId;

    fn canonical_incidence_id(&self, incidence: Self::IncidenceId) -> Self::CanonicalIncidenceId;
}
```

Reverse lookup capabilities:

```rust
pub trait LocalElementIdentity: CanonicalElementIdentity {
    fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId>;
}

pub trait LocalRelationIdentity: CanonicalRelationIdentity {
    fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId>;
}

pub trait LocalIncidenceIdentity: CanonicalIncidenceIdentity {
    fn local_incidence_id(&self, canonical: Self::CanonicalIncidenceId) -> Option<Self::IncidenceId>;
}
```

Rationale:

- `Identity` avoids confusing trait names with associated type names.
- Local -> canonical and canonical -> local directions are explicit.
- Reverse lookup remains optional and partial.

Decision 6f: Python labels are facade/domain identity maps

Accepted rule:

> Python labels map to canonical handles in the Python facade/build layer. They are opaque domain identity mappings and are not part of `oxgraph-topology`. Rust algorithms never inspect labels.

Implications:

- Python can accept arbitrary hashable labels for ergonomic construction.
- The facade maintains `label -> canonical_id` and optionally `canonical_id -> label` maps.
- Canonical handles remain queryable separately from labels.
- Snapshot/property layers may persist opaque label maps later, but topology does not interpret them.

Question 6 is now decided for the first builder/Python slice.

## Current question

### Decision 7a: first builders are construction-time append/update-only

Accepted rule:

> First implement producer-side construction builders, not long-lived general mutation engines. Builders support construction-time mutation: append topology items and update sidecars before freeze. They do not support deletion, tombstones, ID reuse, compaction, stale-handle policies, or overlay/delta views in this slice.

Supported in first builders:

- add elements/vertices;
- add graph relations/edges;
- add hypergraph relations/hyperedges and incidences/participants;
- set/update typed element/relation/incidence weights;
- maintain construction-time indexes useful for freeze/export;
- freeze/export to immutable views/snapshots.

Out of scope for first builders:

- delete element/relation/incidence;
- remove participant from hyperedge;
- tombstones;
- canonical ID reuse;
- compaction after deletion;
- stable long-lived mutable read views;
- overlay/delta views over frozen snapshots.

Rationale:

- Append/update-only construction proves the core build -> freeze -> snapshot path without solving all mutation semantics.
- Deletion is solvable later, but requires explicit stale-handle, tombstone, compaction, and identity policies.
- Overlay/delta views are a separate consumer-side mutable layer over frozen snapshots and should not be conflated with the producer-side builder.

Decision 7b: builders support isolated elements

Accepted rule:

> Graph and hypergraph builders support adding isolated elements/vertices before any relation mentions them. Element count is independent from relation/incidence count.

Rationale:

- Ordinary graph libraries support isolated nodes.
- Topology snapshots must be able to represent zero-degree elements.
- Algorithms such as PageRank need dangling/zero-degree handling.
- Users often construct elements first and relations later.

Decision 7c: graph builder supports parallel edges by default

Accepted rule:

> The first graph builder supports parallel relations/edges with the same `(source, target)` endpoints by default. No reject/dedup/simple-graph policy is needed in this slice.

Rationale:

- Topology relations have identity; multiple relations between the same elements are valid topology.
- Deduping by default would destroy relation identity and can merge weights incorrectly.
- Simple-graph policies can be added later as optional builder/facade policies if needed.

Decision 7d: freeze/cache invalidation model

Accepted rule:

> Borrowed frozen caches/views derived from a builder are invalidated by the next builder edit. Owned frozen views/snapshots are independent artifacts and remain valid after later builder edits.

Implications:

- Rust may expose borrowed freeze/cache paths where lifetimes can enforce validity.
- Python-facing freeze should prefer owned frozen views/snapshot-like objects to avoid lifetime footguns.
- If a borrowed cache is exposed across an API boundary that cannot enforce Rust lifetimes, it must carry a generation check and fail with a typed stale-view error after builder edits.
- Builder edits after freeze create a new cache generation.

Question 7 is now decided for the first builder slice.

## Current question

Question 8: snapshot/sidecar persistence for weights and identity maps.

Accepted direction:

> First-class substrate capabilities need a first-class snapshot story. Canonical identity maps and topology weights should be designed as snapshot-compatible capabilities, not only in-memory sidecars. Arbitrary key/value properties still need a careful separate property/payload-layer design and should not be smuggled into topology.

Decision 8a: canonical identity maps are first-class snapshot capabilities

Accepted rule:

> If a snapshot opts into canonical identity and local IDs may differ from canonical IDs, it must persist enough information to recover local -> canonical mappings for the opted-in ID families. If local IDs equal canonical IDs, the snapshot may encode that as an identity-map mode/metadata instead of storing redundant arrays.

Implications:

- Canonical identity maps are substrate-level, not domain metadata.
- Required mappings depend on which identity capabilities the snapshot claims: element, relation, and/or incidence.
- Reverse canonical -> local maps remain optional indexes and may be materialized on load or persisted as optional acceleration sections.

Open details:

- What should first-class weight snapshot sections look like?
- Should named weights be first-class in the snapshot/property-layer architecture even though names are not part of `oxgraph-topology`?
- How should named weight layers persist without putting names in `oxgraph-topology`?
- How should arbitrary properties differ from first-class weights in snapshot design?
- What validation is required for weight sidecars: length, type, endian, finite/non-negative only for algorithms or at load time?

Decision 8b: named weights/properties need a first-class OxGraph property-layer design

Accepted rule:

> Named weights should be first-class OxGraph layer/snapshot concepts, but not part of the deepest `oxgraph-topology` trait surface. We will flesh out an `oxgraph-property` (or final name) design after the current foundational questions and include it in the architecture before implementation.

Rationale:

- Weight is a first-class topology capability, so persisted weights should not feel bolted on.
- Real systems often have many weight layers and users need names to select them.
- Names/properties are not topology primitives, but they are critical for OxGraph's ecosystem role.
- A sibling property/layer crate can define named typed columns/layers keyed by element/relation/incidence IDs without turning topology into a property graph.
- Snapshot design must include descriptors/data sections for these layers so named weights and later arbitrary properties can persist durably.

Deferred design topic:

- After the remaining current questions, define `oxgraph-property`: named layers, schemas/value types, dense vs sparse storage, required vs optional layers, weight-layer specialization, snapshot sections, validation, and Python facade exposure.

Decision 8c: snapshot validates structure; algorithms validate numeric semantics

Accepted rule:

> Snapshot/property-layer validation checks structural correctness of persisted layers. Algorithm-specific numeric meaning is validated by the algorithm that consumes the layer.

Snapshot/property validation should check:

- layer descriptor/data section consistency;
- ID family compatibility: element/relation/incidence;
- data length matches the declared count or bound;
- byte length matches value type and count;
- value type is known/supported or safely opaque;
- endian/layout/alignment rules are satisfied;
- required layers are present if declared;
- duplicate layer IDs/names are rejected within their namespace unless a future policy allows them.

Algorithm validation should check requirements such as:

- finite values;
- non-negative values;
- normalizable row sums;
- no NaN/Inf for floating algorithms;
- ordered/additive/metric/semiring contracts;
- any PageRank-specific stochastic policy constraints.

Rationale:

- A valid OxGraph weight/property layer may contain negative, signed, categorical, exact, or opaque values.
- PageRank, shortest paths, flow, ranking, and future algorithms impose different numeric contracts.
- Snapshot validation should not reject data merely because one algorithm cannot consume it.

Question 8 is partially decided; full `oxgraph-property` design remains a dedicated follow-up topic before implementation.

### Decision 9: Python facade exposes OxGraph concepts, including existing algorithms

Accepted rule:

> Python should expose OxGraph concepts directly: builders, frozen views, snapshots, identity, weights/properties once designed, and algorithms. It should not initially expose specific third-party library exporters or clone another Python graph API.

Python facade scope:

- package/module name: `oxgraph`;
- graph and hypergraph builders;
- frozen graph and frozen hypergraph views;
- snapshot open/write helpers;
- canonical/local/domain-label lookup through facade maps;
- typed weight/property-layer APIs once designed;
- existing BFS algorithm exposure;
- PageRank exposure once designed/implemented;
- future algorithms exposed by OxGraph names and capability contracts.

Boundary:

- Python labels are facade/domain identity maps, not topology semantics.
- Python ergonomics can include named layer selection, but native Rust algorithms receive selected typed views/policies.
- Third-party Python library exporters/converters are out of scope for this plan.

## Current question

### Decision 10a: `oxgraph-python` lives inside the workspace

Accepted rule:

> `oxgraph-python` should live inside the Rust workspace. Foundation/substrate crates keep the workspace `unsafe_code = "forbid"` policy. If PyO3/maturin requires unsafe generated/glue code, isolate that exception to `oxgraph-python` with an explicit shape/constraint amendment, crate-local lint policy, and safety documentation. Do not weaken the workspace policy globally.

Decision 10b: Python FFI needs explicit safety documentation

Accepted rule:

> `oxgraph-python` must include explicit safety documentation, either as `crates/oxgraph-python/SAFETY.md`, crate-level docs, or both, before exposing Python bindings.

Minimum safety documentation must cover:

- what unsafe/generated glue is introduced by PyO3/maturin, if any;
- why the exception is isolated to `oxgraph-python`;
- ownership/lifetime rules for Rust objects exposed to Python;
- policy that Python-facing frozen views are owned or generation-checked, not unsafely borrowed from mutating builders;
- generation/stale-view checks where borrowed caches are exposed;
- thread/GIL behavior;
- panic-to-Python-exception behavior;
- snapshot/raw-buffer validation before opening views;
- confirmation that foundation/substrate crates keep `unsafe_code = "forbid"`.

Decision 10c: run a PyO3/maturin compatibility spike after shape approval

Accepted rule:

> Run a minimal PyO3/maturin compatibility spike before implementing real bindings, but only after shape approval. The spike decides the exact crate-local lint exception and packaging setup for `oxgraph-python`.

Spike goals:

- create/import a minimal `oxgraph` Python module;
- verify workspace lint behavior;
- identify whether PyO3 macros/generated glue require an unsafe exception;
- confirm maturin packaging shape inside the workspace;
- document the result in `oxgraph-python` safety docs and shapes.

Question 10 is now decided.

## Current question

### Decision 11: verification contract by layer

Accepted rule:

> Every new public capability, builder, algorithm, snapshot/property layer, and Python surface needs verification as part of design. The verification mode depends on the layer.

Topology traits:

- docs for every public/private item;
- explicit performance contracts;
- compile tests/examples where useful;
- property tests mostly on concrete implementations, not bare traits.

Builders:

- unit tests for add/freeze behavior;
- proptests for random edge/hyperedge streams;
- canonical ID stability tests;
- local <-> canonical map roundtrip tests;
- freeze output validates as CSR/BCSR.

Snapshot/property/weights:

- validation tests for length/type/section mismatch;
- proptests for random valid/invalid descriptors;
- Kani for offset/section arithmetic where applicable;
- roundtrip tests.

PageRank/ranking:

- hand-computed small fixtures;
- known examples from references/libraries where semantics match;
- weighted and unweighted cases;
- dangling nodes;
- personalization;
- zero-weight rows;
- invalid negative/NaN/Inf weights;
- convergence failure;
- deterministic output.

Python:

- import smoke test;
- builder/freeze/BFS/PageRank tests;
- identity/label mapping tests;
- stale generation error tests if relevant;
- Python exceptions for Rust errors.

Benchmarks:

- weight lookup overhead;
- builder ingest/freeze;
- BFS regression;
- PageRank iteration throughput;
- Python overhead for batch ingest vs per-edge append.

Question 11 is now decided.

## Next dedicated design topic

`oxgraph-property` / named layer architecture remains the major unresolved design area before implementation.

Working explanation:

- `oxgraph-topology` exposes primitive capability traits such as `RelationWeight`.
- `oxgraph-property` stores many named typed layers keyed by topology ID family.
- A selected property/weight layer becomes the active `RelationWeight` / `ElementWeight` / `IncidenceWeight` capability for a specific view or algorithm call.

Concrete relation example:

```text
topology:
  relation 0: A -> B
  relation 1: A -> C
  relation 2: B -> C

relation layers:
  "count"       -> [10, 2, 5]
  "distance"    -> [3.5, 9.0, 1.2]
  "probability" -> [0.83, 0.17, 1.0]

algorithm call:
  pagerank(weight = "probability")

selected weighted view:
  RelationWeight::relation_weight(relation 0) = 0.83
```

Thus `RelationWeight` is the selected capability exposed by a view; `oxgraph-property` is the registry/storage for many possible named layers.

### Decision P1: `oxgraph-property` uses typed layers/columns

Accepted rule:

> `oxgraph-property` should model properties as typed layers/columns keyed by topology ID family, not as per-element/per-edge dictionaries.

Meaning:

```text
relation id | count | distance | probability
0           | 10    | 3.5      | 0.83
1           | 2     | 9.0      | 0.17
2           | 5     | 1.2      | 1.0
```

is stored column-wise:

```text
count       = [10, 2, 5]
distance    = [3.5, 9.0, 1.2]
probability = [0.83, 0.17, 1.0]
```

Rationale:

- Matches dense topology IDs.
- Fast lookup: `values[id]`.
- Snapshot/mmap friendly.
- Easy structural validation by ID family, type, and length.
- Easy to select one layer as the active weight capability for an algorithm.
- Python facade can still expose ergonomic property lookup if desired.

### Decision P2: support dense and sparse property layers with explicit defaults

Accepted rule:

> `oxgraph-property` should support both dense and sparse typed layers. Sparse layers must carry an explicit missing/default policy before they can be selected as total topology weight capabilities.

Layer storage modes:

- Dense layer: one value for every visible ID in the declared ID family.
- Sparse layer: values for selected IDs plus an explicit default/missing policy.

Weight-selection rule:

- A dense weight layer can directly back `ElementWeight`, `RelationWeight`, or `IncidenceWeight`.
- A sparse layer can back a topology weight capability only through a totalizing view that has an explicit default value or explicit missing-value behavior.
- If no default is provided and a required weight is missing, algorithm selection should fail with a typed error before the hot loop.

Rationale:

- Real systems often have sparse properties.
- Topology weight traits return a value, not `Option`, so any selected sparse layer must become total before it is exposed as `*Weight`.
- Defaults belong to the property/algorithm selection layer, not `oxgraph-topology`.

### Decision P3: property layer type system should be fully generic/extensible

Accepted direction:

> `oxgraph-property` should be designed as a fully generic/extensible typed layer system, not a numeric-only sidecar. Numeric layers are important for weights, but the property architecture must support arbitrary property value families over time.

Implications:

- Weight-compatible numeric layers are a specialization of the property system, not the whole system.
- Property descriptors need a value-type/schema model, not just `f64` arrays.
- The design must support fixed-width scalar values, variable-width values, nested/grouped values, and extension/opaque values.
- Heavy/non-`Copy` property values can live in `oxgraph-property`; when selected as topology weights they must be exposed through a totalizing weight view whose returned representation is `Copy` (for example scalar, handle, or shared reference).
- Snapshot design must allow property layer descriptors plus one or more data sections per layer.
- This remains outside `oxgraph-topology` while still being a first-class OxGraph layer.

Candidate value families to include in the design:

- booleans;
- signed/unsigned integers of common widths;
- floats;
- fixed-size binary values;
- variable-size binary values;
- UTF-8 strings;
- dictionary/categorical encodings;
- fixed-size lists/vectors, useful for embeddings or feature vectors;
- variable-size lists;
- structs/records or grouped columns;
- opaque extension values with a type/schema identifier.

### Decision P4: `oxgraph-property` is a `std` Arrow-backed layer

Accepted rule:

> `oxgraph-property` is a higher-level `std` crate that depends on `arrow-rs` for named generic typed property layers. Foundation/no_std topology, graph/hyper, CSR, BCSR, and core traversal crates must not depend on `oxgraph-property` or Arrow.

Arrow research status:

- Research recorded in `plans/arrow-rs-property-research.md`.
- `arrow-rs` is strongly aligned with generic typed property layers and zero-copy columnar arrays.
- `default-features = false` does not by itself imply `no_std`. Inspected Arrow crates are not documented as no-std and use `std`/`Arc`/allocation and unsafe internally, though safe APIs include validation and zero-copy slicing.
- OxGraph's current Rust toolchain (`1.95.0`) is above the inspected `arrow-rs` master MSRV (`1.85`), so MSRV is not an immediate blocker.

Architecture consequences:

- `oxgraph-property` depends downward on topology identity/capability vocabulary and Arrow arrays/schemas.
- `oxgraph-topology`, `oxgraph-graph`, `oxgraph-hyper`, `oxgraph-csr`, and `oxgraph-hyper-bcsr` remain independent of Arrow/property.
- `oxgraph-algo` should not require Arrow directly; algorithms consume selected topology views/capabilities. Arrow-backed property layers provide adapters/views that implement weight capabilities when selected.
- `oxgraph-python` can depend on `oxgraph-property` for rich named layers.
- Umbrella crate feature gates must keep property/Arrow optional so minimal users do not pay for it.

### Decision P5: property layers have explicit descriptors

Accepted rule:

> Every property layer has an explicit descriptor combining Arrow schema/type information with OxGraph topology-alignment metadata.

Candidate descriptor shape:

```rust
PropertyLayerDescriptor {
    name: LayerName,
    id_family: IdFamily,       // Element | Relation | Incidence
    semantic_class: LayerClass, // Weight | Property | Label | ExternalId? final set TBD
    storage: StorageMode,       // Dense | Sparse { default? }
    arrow_field: arrow_schema::Field,
}
```

Arrow `Field` provides:

- layer name/type/nullability;
- Arrow data type;
- Arrow metadata.

OxGraph metadata provides:

- topology ID family the layer is keyed by;
- whether it is intended as a weight-compatible layer or ordinary property layer;
- dense/sparse/default policy;
- whether indexing follows canonical IDs, local IDs, or a declared identity map/generation.

Decision P6: first layer roles are `Weight` and `Property`

Accepted rule:

> `oxgraph-property` starts with two layer roles: `Weight` and `Property`.

Meaning:

- `Weight`: intended to be selectable as an element/relation/incidence weight capability if type/default rules allow.
- `Property`: generic named typed layer with no algorithm-specific meaning.

Deferred:

- Labels, external IDs, categories, annotations, and domain-specific concepts remain ordinary property layers or metadata for now.
- Add more roles only after concrete implementation pressure proves they are substrate-level property roles.

Decision P7: property layers have both stable layer IDs and string names

Accepted rule:

> Property layers have both a stable internal `LayerId` and a human/facade-facing string name.

Implications:

- `LayerId` is the internal handle used by registries, snapshots, and fast lookup.
- String names are used for ergonomics in builders/Python/configuration.
- Names should be unique at least within an ID-family namespace so element, relation, and incidence layers can each have natural names without conflict.
- Exact namespace rule remains to be finalized during implementation, but duplicate ambiguous lookup must be rejected.

Decision P8: sparse layers support null and explicit default policies

Accepted rule:

> Sparse property layers support both nullable/missing values and explicit defaults. A sparse layer can become a topology weight view only after it is totalized by a layer default or caller/algorithm-supplied default.

Candidate storage policy shape:

```rust
enum StorageMode {
    Dense,
    Sparse { missing: MissingPolicy },
}

enum MissingPolicy {
    Null,
    Default(ArrowScalarValue),
}
```

Rules:

- Generic property layers may use `Null` to represent absence.
- Weight layers selected for algorithms must be total:
  - dense layer: already total if non-nullable or validated as no missing values;
  - sparse with `Default(value)`: totalized by descriptor default;
  - sparse/null with caller-supplied default: totalized by selection policy;
  - sparse/null with no default: selection as a topology weight fails before the hot loop.
- Algorithm-specific numeric validation still happens after totalization.

Question P8 is decided.

Open `oxgraph-property` follow-up details for shape work:

- Exact Arrow scalar/default representation.
- Exact sparse physical representation: Arrow nullable dense array, index/value sparse arrays, or both.
- Snapshot section layout for descriptors and Arrow-backed data.

## Verification

- Every decision should be reflected in shapes before code.
- `shapes validate` should pass after shape changes.
- Each new public Rust item needs documentation and a performance contract per repo constraints.

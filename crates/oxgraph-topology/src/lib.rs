//! Storage-agnostic traits for discrete topology views.
//!
//! `oxgraph-topology` defines the minimal vocabulary shared by graph,
//! hypergraph, snapshot, and layout crates. It does not define concrete node,
//! edge, vertex, hyperedge, incidence, storage, or role types. Implementations
//! provide those through associated types.
//!
//! This crate defines read-view capabilities. Mutation belongs in explicit
//! capability traits that define identity stability, deletion, compaction, and
//! stale-handle semantics.
//!
//! A topology view is any value that exposes topology through these traits. The
//! view decides its own boundary: an entire snapshot, one layout section, a
//! generated projection, a page-sized window, or an overlay can all be views if
//! they provide the requested capabilities.
#![no_std]

#[cfg(kani)]
extern crate kani;

/// Marker trait for compact topology identity handles.
///
/// IDs are handles into topology storage, not the storage itself. They are
/// passed by value so traversal APIs can remain simple and static-dispatch
/// friendly. A logical element, relation, or incidence can have different ID
/// representations at different layers. Implementations should document which
/// identity layer each ID type represents and how it maps to other exposed
/// identity layers.
///
/// # Performance
///
/// Implementations are expected to be small `Copy` handles. Copying,
/// comparing, ordering, hashing, and formatting for debug should be `O(1)`.
pub trait TopologyId: Copy + Eq + Ord + core::fmt::Debug + core::hash::Hash {}

/// Blanket implementation for compact ID handle types.
///
/// Any type satisfying the `TopologyId` bounds is a valid topology ID. This
/// keeps the crate trait-based without requiring implementations to write empty
/// marker impls for every local handle type.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the concrete ID type.
impl<T> TopologyId for T where T: Copy + Eq + Ord + core::fmt::Debug + core::hash::Hash {}

/// Common element and relation identity vocabulary for a topology view.
///
/// The associated ID types deliberately avoid graph- or hypergraph-specific
/// names. Graph crates can map elements to nodes and relations to edges;
/// hypergraph crates can map elements to vertices and relations to hyperedges.
///
/// # Performance
///
/// Associated ID values should be compact `O(1)` handles. Arbitrary relation or
/// element payloads should be exposed by separate traits keyed by the associated
/// ID types.
pub trait TopologyBase {
    /// Identity of a topology element.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy, compare, order, hash, and debug-format.
    type ElementId: TopologyId;

    /// Identity of a topology relation.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy, compare, order, hash, and debug-format.
    type RelationId: TopologyId;
}

/// Substrate-neutral alias for a topology view's element ID type.
///
/// Mirrors the substrate-specific `NodeId<G>` / `VertexId<H>` aliases exposed
/// by graph and hypergraph wrapper crates and gives substrate-agnostic code
/// (algorithms, snapshot tooling) a short way to name the element identity of
/// a topology view in generic signatures and return types.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`TopologyBase::ElementId`] type.
pub type ElementId<T> = <T as TopologyBase>::ElementId;

/// Substrate-neutral alias for a topology view's relation ID type.
///
/// Mirrors the substrate-specific `EdgeId<G>` / `HyperedgeId<H>` aliases exposed
/// by graph and hypergraph wrapper crates.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`TopologyBase::RelationId`] type.
pub type RelationId<T> = <T as TopologyBase>::RelationId;

/// Optional total weight capability for topology elements.
///
/// A view implements this trait only when every visible element has a weight
/// representation. The topology layer does not interpret the value: it is not a
/// probability, cost, distance, count, or property name. Algorithms state their
/// own numeric contracts when they consume a selected weight capability.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait ElementWeight: TopologyBase {
    /// Copyable weight representation attached to each visible element.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy.
    type Weight: Copy;

    /// Returns the weight attached to `element`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn element_weight(&self, element: Self::ElementId) -> Self::Weight;
}

/// Optional total weight capability for topology relations.
///
/// A view implements this trait only when every visible relation has a weight
/// representation. The topology layer does not interpret the value; algorithms
/// define any finite, non-negative, additive, ordered, or normalization
/// requirements separately.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait RelationWeight: TopologyBase {
    /// Copyable weight representation attached to each visible relation.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy.
    type Weight: Copy;

    /// Returns the weight attached to `relation`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight;
}

/// Optional local-to-canonical element identity capability.
///
/// Views implement this trait only when they guarantee a stable canonical ID for
/// every visible element in the view's documented identity scope. The canonical
/// ID is a substrate identity, not a Python label or domain identifier.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait CanonicalElementIdentity: TopologyBase {
    /// Canonical element ID guaranteed by this view.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy, compare, order, hash, and debug-format.
    type CanonicalElementId: TopologyId;

    /// Returns the canonical ID for a visible local `element`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn canonical_element_id(&self, element: Self::ElementId) -> Self::CanonicalElementId;
}

/// Optional canonical-to-local element identity capability.
///
/// This reverse lookup is separate from [`CanonicalElementIdentity`] because it
/// may require extra memory or may be partial for filtered and projected views.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait LocalElementIdentity: CanonicalElementIdentity {
    /// Returns the visible local element for `canonical`, if present.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId>;
}

/// Optional local-to-canonical relation identity capability.
///
/// Views implement this trait only when they guarantee a stable canonical ID for
/// every visible relation in the view's documented identity scope.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait CanonicalRelationIdentity: TopologyBase {
    /// Canonical relation ID guaranteed by this view.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy, compare, order, hash, and debug-format.
    type CanonicalRelationId: TopologyId;

    /// Returns the canonical ID for a visible local `relation`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn canonical_relation_id(&self, relation: Self::RelationId) -> Self::CanonicalRelationId;
}

/// Optional canonical-to-local relation identity capability.
///
/// This reverse lookup is separate from [`CanonicalRelationIdentity`] because it
/// may require extra memory or may be partial for filtered and projected views.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait LocalRelationIdentity: CanonicalRelationIdentity {
    /// Returns the visible local relation for `canonical`, if present.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId>;
}

/// Incidence identity and role vocabulary for topology views with incidences.
///
/// Incidence support is separate from [`TopologyBase`] so graph-only views can
/// expose nodes and edges without inventing endpoint identities or roles. Views
/// that support relation-to-element participation records implement this trait.
///
/// # Performance
///
/// Associated ID values should be compact `O(1)` handles. `Role` describes how
/// an element participates in a relation. Arbitrary incidence payloads should be
/// exposed by separate traits keyed by [`Self::IncidenceId`].
pub trait IncidenceBase: TopologyBase {
    /// Identity of one element's participation in one relation.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy, compare, order, hash, and debug-format.
    type IncidenceId: TopologyId;

    /// Implementation-defined participation role.
    ///
    /// A role is a topology-level label for an incidence. Rich metadata can be
    /// attached by separate payload traits or storage layers keyed by
    /// [`Self::IncidenceId`], [`Self::RelationId`], or [`Self::ElementId`].
    ///
    /// # Performance
    ///
    /// `perf: unspecified`. Implementations should prefer structural role
    /// values or compact role handles; rich metadata should be reached through
    /// separate payload access traits.
    type Role;
}

/// Optional total weight capability for topology incidences.
///
/// A view implements this trait only when every visible incidence has a weight
/// representation. The topology layer does not interpret the value; algorithms
/// define their own numeric contracts separately.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait IncidenceWeight: IncidenceBase {
    /// Copyable weight representation attached to each visible incidence.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy.
    type Weight: Copy;

    /// Returns the weight attached to `incidence`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight;
}

/// Optional local-to-canonical incidence identity capability.
///
/// Views implement this trait only when they guarantee a stable canonical ID for
/// every visible incidence in the view's documented identity scope.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait CanonicalIncidenceIdentity: IncidenceBase {
    /// Canonical incidence ID guaranteed by this view.
    ///
    /// # Performance
    ///
    /// Values should be `O(1)` to copy, compare, order, hash, and debug-format.
    type CanonicalIncidenceId: TopologyId;

    /// Returns the canonical ID for a visible local `incidence`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn canonical_incidence_id(&self, incidence: Self::IncidenceId) -> Self::CanonicalIncidenceId;
}

/// Optional canonical-to-local incidence identity capability.
///
/// This reverse lookup is separate from [`CanonicalIncidenceIdentity`] because
/// it may require extra memory or may be partial for filtered and projected
/// views.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait LocalIncidenceIdentity: CanonicalIncidenceIdentity {
    /// Returns the visible local incidence for `canonical`, if present.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn local_incidence_id(
        &self,
        canonical: Self::CanonicalIncidenceId,
    ) -> Option<Self::IncidenceId>;
}

/// Count capability for a topology view.
///
/// This trait is separated from [`TopologyBase`] because counts are a capability.
/// Some views can report global counts cheaply; others expose only local,
/// paged, generated, or filtered topology and should define counts only when the
/// result is meaningful for that view.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait TopologyCounts: TopologyBase {
    /// Returns the number of elements visible in this topology view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn element_count(&self) -> usize;

    /// Returns the number of relations visible in this topology view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn relation_count(&self) -> usize;
}

/// Count capability for incidence-capable topology views.
///
/// This trait is separated from [`TopologyCounts`] because not every topology
/// view exposes incidence records. Graph-only views can count nodes and edges
/// without defining endpoint-incidence identities.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait IncidenceCounts: IncidenceBase {
    /// Returns the number of incidences visible in this topology view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_count(&self) -> usize;
}

/// Dense element-index capability for topology views.
///
/// This capability maps visible element IDs to compact array indexes usable by
/// algorithms that need visited sets, distance arrays, parent arrays, or other
/// per-element scratch storage. The index is a property of the view, not the ID
/// type itself: a view may expose opaque IDs while also providing a dense index
/// mapping.
///
/// `element_bound` is an allocation bound, not necessarily the exact visible
/// element count. Immutable compact layouts usually have `element_bound() ==
/// element_count()`. Mutable layouts with tombstones, overlays, or stable slots
/// may have `element_bound() >= element_count()`.
///
/// Implementations must ensure every valid visible element maps to an index less
/// than `element_bound`, distinct visible elements map to distinct indexes, and
/// indexes remain stable for the lifetime of the view operation using them.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait ElementIndex: TopologyBase {
    /// Returns the exclusive upper bound for element indexes in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn element_bound(&self) -> usize;

    /// Returns the dense index for `element` in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn element_index(&self, element: Self::ElementId) -> usize;
}

/// Dense relation-index capability for topology views.
///
/// This capability maps visible relation IDs to compact array indexes usable by
/// algorithms that need per-relation scratch storage. The index is a property of
/// the view, not the ID type itself.
///
/// `relation_bound` is an allocation bound, not necessarily the exact visible
/// relation count. Immutable compact layouts usually have `relation_bound() ==
/// relation_count()`. Mutable layouts may expose a larger bound.
///
/// Implementations must ensure every valid visible relation maps to an index
/// less than `relation_bound`, distinct visible relations map to distinct
/// indexes, and indexes remain stable for the lifetime of the view operation
/// using them.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait RelationIndex: TopologyBase {
    /// Returns the exclusive upper bound for relation indexes in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn relation_bound(&self) -> usize;

    /// Returns the dense index for `relation` in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn relation_index(&self, relation: Self::RelationId) -> usize;
}

/// Dense incidence-index capability for topology views.
///
/// This capability maps visible incidence IDs to compact array indexes usable by
/// algorithms that need per-incidence scratch storage. Views that do not expose
/// incidences do not implement this trait.
///
/// `incidence_bound` is an allocation bound, not necessarily the exact visible
/// incidence count. Immutable compact layouts usually have `incidence_bound() ==
/// incidence_count()`. Mutable layouts may expose a larger bound.
///
/// Implementations must ensure every valid visible incidence maps to an index
/// less than `incidence_bound`, distinct visible incidences map to distinct
/// indexes, and indexes remain stable for the lifetime of the view operation
/// using them.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait IncidenceIndex: IncidenceBase {
    /// Returns the exclusive upper bound for incidence indexes in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_bound(&self) -> usize;

    /// Returns the dense index for `incidence` in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_index(&self, incidence: Self::IncidenceId) -> usize;
}

/// Element-ID containment capability for a topology view.
///
/// This capability answers whether an element ID is valid and visible in this
/// view at the time of the call. It is intentionally separate from
/// [`ElementIndex`]: an index bound is an allocation bound, while containment is
/// an ID-validity predicate. Mutable, filtered, tombstoned, or overlay views may
/// have indexes below the bound that are not visible elements.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsElement: TopologyBase {
    /// Returns whether `element` is valid and visible in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_element(&self, element: Self::ElementId) -> bool;
}

/// Relation-ID containment capability for a topology view.
///
/// This capability answers whether a relation ID is valid and visible in this
/// view at the time of the call. It does not answer graph-specific questions
/// such as whether an edge between two nodes exists, nor hypergraph-specific
/// questions such as whether a vertex participates in a hyperedge.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsRelation: TopologyBase {
    /// Returns whether `relation` is valid and visible in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_relation(&self, relation: Self::RelationId) -> bool;
}

/// Incidence-ID containment capability for an incidence-capable topology view.
///
/// This capability answers whether an incidence ID is valid and visible in this
/// view at the time of the call.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsIncidence: IncidenceBase {
    /// Returns whether `incidence` is valid and visible in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_incidence(&self, incidence: Self::IncidenceId) -> bool;
}

/// Capability for traversing incidences attached to a relation.
///
/// The generic associated iterator type lets each backend return its own
/// concrete iterator without allocating or using dynamic dispatch.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` incidences should be `O(k)` because the output
/// itself has length `k`.
pub trait RelationIncidences: IncidenceBase {
    /// Iterator over incidence IDs for one relation.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Incidences<'view>: Iterator<Item = Self::IncidenceId>
    where
        Self: 'view;

    /// Returns incidences attached to `relation`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` incidences is
    /// expected `O(k)`.
    fn relation_incidences(&self, relation: Self::RelationId) -> Self::Incidences<'_>;
}

/// Capability for traversing incidences attached to an element.
///
/// This is the topology-general form of asking which relations mention an
/// element.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` incidences should be `O(k)` because the output
/// itself has length `k`.
pub trait ElementIncidences: IncidenceBase {
    /// Iterator over incidence IDs for one element.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Incidences<'view>: Iterator<Item = Self::IncidenceId>
    where
        Self: 'view;

    /// Returns incidences attached to `element`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` incidences is
    /// expected `O(k)`.
    fn element_incidences(&self, element: Self::ElementId) -> Self::Incidences<'_>;
}

/// Capability for resolving the element side of an incidence.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait IncidenceElement: IncidenceBase {
    /// Returns the element participating through `incidence`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_element(&self, incidence: Self::IncidenceId) -> Self::ElementId;
}

/// Capability for resolving the relation side of an incidence.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait IncidenceRelation: IncidenceBase {
    /// Returns the relation containing `incidence`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_relation(&self, incidence: Self::IncidenceId) -> Self::RelationId;
}

/// Capability for resolving a role attached to an incidence.
///
/// Roles are implementation-defined topology labels. The core never
/// interprets them.
///
/// # Performance
///
/// Lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait IncidenceRole: IncidenceBase {
    /// Returns the role for `incidence`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incidence_role(&self, incidence: Self::IncidenceId) -> Self::Role;
}

/// Exact relation-incidence count capability.
///
/// This pairs with [`RelationIncidences`] for backends that can report a
/// relation's incidence count without traversal. The trait does not require
/// traversal support by itself.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait RelationIncidenceCount: IncidenceBase {
    /// Returns the number of incidences attached to `relation`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn relation_incidence_count(&self, relation: Self::RelationId) -> usize;
}

/// Exact element-incidence count capability.
///
/// This pairs with [`ElementIncidences`] for backends that can report an
/// element's incidence count without traversal. The trait does not require
/// traversal support by itself.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ElementIncidenceCount: IncidenceBase {
    /// Returns the number of incidences attached to `element`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn element_incidence_count(&self, element: Self::ElementId) -> usize;
}

/// Convenience trait for views that can resolve complete incidence records.
///
/// This trait has no methods of its own. It names the common capability bundle
/// needed by generic code that wants to traverse relations and inspect each
/// incidence's element, relation, and role.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
pub trait IncidenceView:
    RelationIncidences + IncidenceElement + IncidenceRelation + IncidenceRole
{
}

/// Blanket implementation for complete incidence views.
///
/// Any type that can traverse relation incidences and resolve each incidence's
/// element, relation, and role automatically implements [`IncidenceView`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
impl<T> IncidenceView for T where
    T: RelationIncidences + IncidenceElement + IncidenceRelation + IncidenceRole
{
}

/// Capability for expanding an element to its directed successor elements.
///
/// This is the substrate-neutral form of "follow outgoing connections from
/// this element." For binary graphs the successors are the targets of
/// outgoing edges; for hypergraphs they are the vertices reachable through
/// outgoing hyperedges. Substrate-agnostic algorithms (forward BFS, forward
/// reachability) bind on this trait so the same code drives any topology
/// view that can answer the question.
///
/// Implementations define whether parallel connections produce repeated
/// successor elements. Implementations that preserve multiplicity should
/// document that behavior; consumers that need set semantics should
/// deduplicate at their own level.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` successors should be `O(k)` and should not
/// allocate unless the implementation documents otherwise.
pub trait ElementSuccessors: TopologyBase {
    /// Iterator over successor element IDs reached from one element.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Successors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns elements reachable through outgoing connections from `element`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` successors is
    /// expected `O(k)`.
    fn element_successors(&self, element: Self::ElementId) -> Self::Successors<'_>;
}

/// Capability for expanding an element to its directed predecessor elements.
///
/// This is the substrate-neutral form of "follow incoming connections to this
/// element." For binary graphs the predecessors are the sources of incoming
/// edges; for hypergraphs they are the vertices that reach this vertex
/// through outgoing hyperedges. Substrate-agnostic algorithms (reverse BFS,
/// reverse reachability) bind on this trait so the same code drives any
/// topology view that can answer the question.
///
/// Implementations define whether parallel connections produce repeated
/// predecessor elements. Implementations that preserve multiplicity should
/// document that behavior; consumers that need set semantics should
/// deduplicate at their own level.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` predecessors should be `O(k)` and should not
/// allocate unless the implementation documents otherwise.
pub trait ElementPredecessors: TopologyBase {
    /// Iterator over predecessor element IDs reaching one element.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Predecessors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns elements that reach `element` through outgoing connections.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` predecessors is
    /// expected `O(k)`.
    fn element_predecessors(&self, element: Self::ElementId) -> Self::Predecessors<'_>;
}

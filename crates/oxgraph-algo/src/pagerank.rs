//! `PageRank` algorithms over `OxGraph` capability views.
//!
//! The module provides ordinary directed graph `PageRank` and directed
//! hypergraph incidence/bipartite `PageRank`. Property layers are not read here;
//! callers select named layers into topology weight capability views before
//! invoking weighted variants.
// kani-skip: PageRank uses unbounded caller-supplied iteration counts and floating-point
// convergence; deterministic unit tests and Criterion benches cover this slice.
#![cfg_attr(
    not(test),
    expect(
        clippy::missing_docs_in_private_items,
        reason = "PageRank helper functions are private implementation details behind documented public API tiers"
    )
)]

#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};
#[cfg(feature = "alloc")]
use core::marker::PhantomData;
use core::{
    error::Error,
    fmt,
    ops::{Add, AddAssign, Div, Mul, Sub},
};

use oxgraph_graph::ForwardGraph;
use oxgraph_hyper::{DirectedHyperedgeIncidences, DirectedVertexHyperedges};

/// Capability bundle the hypergraph `PageRank` surface requires.
///
/// Bundles directed vertex-to-hyperedge adjacency, directed hyperedge
/// incidences, incidence-to-vertex resolution, and dense element/relation
/// indexing. Blanket-implemented for every topology providing the parts, so
/// callers never implement it directly.
///
/// # Performance
///
/// `perf: unspecified`; this is a bound bundle.
pub trait PageRankHypergraph:
    DirectedVertexHyperedges
    + DirectedHyperedgeIncidences
    + IncidenceElement
    + DenseElementIndex
    + DenseRelationIndex
{
}

impl<T> PageRankHypergraph for T where
    T: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + DenseElementIndex
        + DenseRelationIndex
{
}
use oxgraph_topology::{
    DenseElementIndex, DenseIncidenceIndex, DenseRelationIndex, ElementId, IncidenceElement,
    IncidenceWeight, RelationId, RelationWeight,
};

/// Rank scalar accepted by `OxGraph` `PageRank` entry points.
///
/// The trait is owned by `OxGraph` so topology weights do not inherit arithmetic
/// semantics and public algorithms do not expose a broad numeric dependency.
/// Implementations define the numeric operations `PageRank` needs for rank state,
/// damping, tolerance, personalization, and converted weights.
///
/// # Scalar laws
///
/// Implementations must use ordinary finite numeric ordering and arithmetic:
/// `ZERO` is the additive identity, `ONE` is the multiplicative identity,
/// division by a positive count is finite for supported topology sizes, and
/// `abs(a - b)` is non-negative and finite whenever `a` and `b` are finite.
pub trait PageRankScalar:
    Copy
    + fmt::Debug
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + AddAssign
    + 'static
{
    /// Additive identity.
    const ZERO: Self;
    /// Multiplicative identity.
    const ONE: Self;
    /// Positive infinity sentinel used before the first iteration delta.
    const INFINITY: Self;

    /// Converts a row degree or visible-state count into this scalar.
    fn from_usize(value: usize) -> Self;

    /// Converts a Rust float literal/default into this scalar.
    fn from_f64(value: f64) -> Self;

    /// Absolute value.
    #[must_use]
    fn abs(self) -> Self;

    /// Returns whether this value is finite.
    fn is_finite(self) -> bool;
}

impl PageRankScalar for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    #[expect(
        clippy::use_self,
        reason = "primitive inherent infinity constant is clearer here"
    )]
    const INFINITY: Self = f64::INFINITY;

    #[expect(
        clippy::cast_precision_loss,
        reason = "PageRank degree conversion is a documented scalar-boundary conversion"
    )]
    fn from_usize(value: usize) -> Self {
        value as Self
    }

    fn from_f64(value: f64) -> Self {
        value
    }

    fn abs(self) -> Self {
        self.abs()
    }

    fn is_finite(self) -> bool {
        self.is_finite()
    }
}

impl PageRankScalar for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    #[expect(
        clippy::use_self,
        reason = "primitive inherent infinity constant is clearer here"
    )]
    const INFINITY: Self = f32::INFINITY;

    #[expect(
        clippy::cast_precision_loss,
        reason = "PageRank degree conversion is a documented scalar-boundary conversion"
    )]
    fn from_usize(value: usize) -> Self {
        value as Self
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "f32 PageRank callers explicitly select f32 rank/config output"
    )]
    fn from_f64(value: f64) -> Self {
        value as Self
    }

    fn abs(self) -> Self {
        self.abs()
    }

    fn is_finite(self) -> bool {
        self.is_finite()
    }
}

/// Explicit conversion from a topology weight into a `PageRank` rank scalar.
///
/// Implementations are deliberately limited to documented primitive conversions;
/// downstream topology weights stay semantic-free and algorithms opt into the
/// numeric interpretation at this boundary.
pub trait IntoPageRankScalar<S: PageRankScalar> {
    /// Converts `self` into rank scalar `S`.
    fn into_pagerank_scalar(self) -> S;
}

impl<S: PageRankScalar> IntoPageRankScalar<S> for S {
    fn into_pagerank_scalar(self) -> S {
        self
    }
}

/// Implements lossless primitive conversions into a `PageRank` scalar.
macro_rules! impl_weight_into_pagerank_scalar_from {
    ($target:ty; $($type:ty),* $(,)?) => {
        $(
            impl IntoPageRankScalar<$target> for $type {
                fn into_pagerank_scalar(self) -> $target { <$target>::from(self) }
            }
        )*
    };
}

/// Implements explicitly lossy primitive conversions into a `PageRank` scalar.
macro_rules! impl_weight_into_pagerank_scalar_cast {
    ($target:ty; $($type:ty),* $(,)?) => {
        $(
            impl IntoPageRankScalar<$target> for $type {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "PageRank primitive weight conversions are explicit algorithm-boundary numeric interpretations"
                )]
                fn into_pagerank_scalar(self) -> $target { self as $target }
            }
        )*
    };
}

impl_weight_into_pagerank_scalar_from!(f64; u8, u16, u32, i8, i16, i32, f32);
impl_weight_into_pagerank_scalar_cast!(f64; u64, usize, i64, isize);
impl_weight_into_pagerank_scalar_from!(f32; u8, u16, i8, i16);
impl_weight_into_pagerank_scalar_cast!(f32; u32, u64, usize, i32, i64, isize);

impl IntoPageRankScalar<f32> for f64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "f32 PageRank callers explicitly select f32 rank and configuration output"
    )]
    fn into_pagerank_scalar(self) -> f32 {
        self as f32
    }
}

/// Per-element outgoing rank-distribution rule used by graph `PageRank`.
///
/// Implementations decide how an element's current `rank` is split across its
/// outgoing visible neighbors and how much of it (if any) flows to the
/// dangling reservoir. Built-in [`Uniform`] divides equally over visible
/// out-degree; built-in [`Weighted`] divides proportionally to relation
/// weights. User crates can implement this trait for custom rules — for
/// example, attenuated decay, top-k filtering, or caller-supplied
/// per-element shares — without forking the kernel.
///
/// # Contract
///
/// On success, `distribute_outgoing` writes per-target shares into `next`
/// (visible-masked) and returns the dangling contribution. The sum of `next`
/// increments plus the returned dangling contribution must equal `rank` when
/// at least one visible target exists; when no visible targets exist, the
/// implementation must return `rank` and write nothing. Failures are surfaced
/// as [`PageRankError`].
///
/// # Performance
///
/// `perf: unspecified`; built-in implementations document their per-element
/// cost. Most do `O(degree)` work per element with one or two passes over
/// the outgoing-edge iterator.
pub trait OutgoingDistribution<G, S>
where
    G: ForwardGraph + DenseElementIndex,
    S: PageRankScalar,
{
    /// Distributes `rank` from `element` to its outgoing visible neighbors.
    ///
    /// # Errors
    ///
    /// Returns [`PageRankError`] when a topology index is invalid, when a
    /// caller-supplied weight is invalid, or when an arithmetic boundary is
    /// crossed.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; implementations document their cost.
    #[expect(
        clippy::too_many_arguments,
        reason = "distribution kernel needs graph, element, rank, output buffer, and visibility mask explicitly"
    )]
    fn distribute_outgoing(
        &self,
        graph: &G,
        element: G::ElementId,
        rank: S,
        next: &mut [S],
        visible: &[u8],
    ) -> Result<S, PageRankError<S>>;
}

/// Distributes outgoing rank uniformly across visible out-degree.
///
/// Built-in [`OutgoingDistribution`] implementation matching the unweighted
/// `PageRank` semantics. Two passes over the outgoing-edge iterator: one to
/// count visible degree, one to write the share `rank / degree`.
///
/// # Performance
///
/// Each call is `O(degree)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Uniform;

impl<G, S> OutgoingDistribution<G, S> for Uniform
where
    G: ForwardGraph + DenseElementIndex,
    S: PageRankScalar,
{
    fn distribute_outgoing(
        &self,
        graph: &G,
        element: G::ElementId,
        rank: S,
        next: &mut [S],
        visible: &[u8],
    ) -> Result<S, PageRankError<S>> {
        let mut degree = 0_usize;
        for edge in graph.outgoing_edges(element) {
            let target = graph.target(edge);
            let target_index = checked_element_index(graph, target)?;
            if is_visible(visible, target_index) {
                degree += 1;
            }
        }
        if degree == 0 {
            return Ok(rank);
        }
        let share = rank / S::from_usize(degree);
        for edge in graph.outgoing_edges(element) {
            let target = graph.target(edge);
            let target_index = checked_element_index(graph, target)?;
            if is_visible(visible, target_index) {
                next[target_index] += share;
            }
        }
        Ok(S::ZERO)
    }
}

/// Distributes outgoing rank proportionally to caller-supplied weights.
///
/// Built-in [`OutgoingDistribution`] implementation matching the weighted
/// `PageRank` semantics. Holds a borrowed [`RelationWeight`] adapter and uses
/// two passes over the outgoing-edge iterator: one to compute the visible
/// outgoing-weight total, one to write per-edge shares `rank * weight /
/// total`. Dangling rows (zero or non-positive total) flow the entire `rank`
/// to the dangling reservoir.
///
/// # Performance
///
/// Each call is `O(degree)`.
#[derive(Clone, Copy, Debug)]
pub struct Weighted<'w, W> {
    /// Borrowed relation-weight adapter.
    weights: &'w W,
}

impl<'w, W> Weighted<'w, W> {
    /// Constructs a [`Weighted`] distribution borrowing `weights`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(weights: &'w W) -> Self {
        Self { weights }
    }
}

impl<G, W, S> OutgoingDistribution<G, S> for Weighted<'_, W>
where
    G: ForwardGraph + DenseElementIndex + DenseRelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    fn distribute_outgoing(
        &self,
        graph: &G,
        element: G::ElementId,
        rank: S,
        next: &mut [S],
        visible: &[u8],
    ) -> Result<S, PageRankError<S>> {
        let total = outgoing_weight_total(graph, self.weights, element, visible)?;
        if total <= S::ZERO {
            return Ok(rank);
        }
        for edge in graph.outgoing_edges(element) {
            let target = graph.target(edge);
            let target_index = checked_element_index(graph, target)?;
            if is_visible(visible, target_index) {
                let weight = checked_relation_weight(graph, self.weights, edge)?;
                next[target_index] += rank * (weight / total);
            }
        }
        Ok(S::ZERO)
    }
}

/// Bipartite outgoing rank-distribution rule used by hypergraph `PageRank`.
///
/// Implementations decide how an element's current `rank` is split across its
/// outgoing visible relations, and how a relation's current `rank` is split
/// across its visible target elements. Built-in [`Uniform`] divides each leg
/// evenly over the visible out-degree; built-in [`HyperWeighted`] divides
/// element-to-relation by [`RelationWeight`] values and relation-to-element by
/// [`IncidenceWeight`] values. User crates can implement this trait for custom
/// bipartite policies — e.g. attenuated decay, top-k filtering, or
/// caller-supplied per-state shares — without forking the kernel.
///
/// # Contract
///
/// On success, both methods write per-target shares into the appropriate next
/// buffer (visible-masked) and return the dangling contribution for the
/// source state. The sum of `next_*` increments plus the returned dangling
/// contribution must equal `rank` when at least one visible target exists;
/// when no visible target exists, the implementation must return `rank` and
/// write nothing. Failures are surfaced as [`PageRankError`].
///
/// # Performance
///
/// `perf: unspecified`; built-in implementations document their per-state
/// cost. Most do `O(degree)` work per state with one or two passes over the
/// outgoing-incidence iterator.
pub trait HypergraphOutgoingDistribution<H, S>
where
    H: PageRankHypergraph,
    S: PageRankScalar,
{
    /// Distributes `rank` from `element` to its outgoing visible relations.
    ///
    /// # Errors
    ///
    /// Returns [`PageRankError`] when a topology index is invalid, when a
    /// caller-supplied weight is invalid, or when an arithmetic boundary is
    /// crossed.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; implementations document their cost.
    #[expect(
        clippy::too_many_arguments,
        reason = "distribution kernel needs hypergraph, element, rank, output buffer, and visibility mask explicitly"
    )]
    fn distribute_from_element(
        &self,
        hypergraph: &H,
        element: H::ElementId,
        rank: S,
        next_relations: &mut [S],
        visible_relations: &[u8],
    ) -> Result<S, PageRankError<S>>;

    /// Distributes `rank` from `relation` to its visible target elements.
    ///
    /// # Errors
    ///
    /// Returns [`PageRankError`] when a topology index is invalid, when a
    /// caller-supplied weight is invalid, or when an arithmetic boundary is
    /// crossed.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; implementations document their cost.
    #[expect(
        clippy::too_many_arguments,
        reason = "distribution kernel needs hypergraph, relation, rank, output buffer, and visibility mask explicitly"
    )]
    fn distribute_from_relation(
        &self,
        hypergraph: &H,
        relation: H::RelationId,
        rank: S,
        next_elements: &mut [S],
        visible_elements: &[u8],
    ) -> Result<S, PageRankError<S>>;
}

impl<H, S> HypergraphOutgoingDistribution<H, S> for Uniform
where
    H: PageRankHypergraph,
    S: PageRankScalar,
{
    fn distribute_from_element(
        &self,
        hypergraph: &H,
        element: H::ElementId,
        rank: S,
        next_relations: &mut [S],
        visible_relations: &[u8],
    ) -> Result<S, PageRankError<S>> {
        let mut degree = 0_usize;
        for relation in hypergraph.outgoing_hyperedges(element) {
            let relation_index = checked_relation_index_for(hypergraph, relation)?;
            if is_visible(visible_relations, relation_index) {
                degree += 1;
            }
        }
        if degree == 0 {
            return Ok(rank);
        }
        let share = rank / S::from_usize(degree);
        for relation in hypergraph.outgoing_hyperedges(element) {
            let relation_index = checked_relation_index_for(hypergraph, relation)?;
            if is_visible(visible_relations, relation_index) {
                next_relations[relation_index] += share;
            }
        }
        Ok(S::ZERO)
    }

    fn distribute_from_relation(
        &self,
        hypergraph: &H,
        relation: H::RelationId,
        rank: S,
        next_elements: &mut [S],
        visible_elements: &[u8],
    ) -> Result<S, PageRankError<S>> {
        let mut degree = 0_usize;
        for incidence in hypergraph.target_incidences(relation) {
            let target = hypergraph.incidence_element(incidence);
            let target_index = checked_element_index(hypergraph, target)?;
            if is_visible(visible_elements, target_index) {
                degree += 1;
            }
        }
        if degree == 0 {
            return Ok(rank);
        }
        let share = rank / S::from_usize(degree);
        for incidence in hypergraph.target_incidences(relation) {
            let target = hypergraph.incidence_element(incidence);
            let target_index = checked_element_index(hypergraph, target)?;
            if is_visible(visible_elements, target_index) {
                next_elements[target_index] += share;
            }
        }
        Ok(S::ZERO)
    }
}

/// Distributes bipartite outgoing rank proportionally to caller-supplied
/// weights.
///
/// Built-in [`HypergraphOutgoingDistribution`] implementation matching the
/// weighted incidence/bipartite `PageRank` semantics. Holds borrowed
/// [`RelationWeight`] and [`IncidenceWeight`] adapters: element-to-relation
/// distribution uses relation weights, and relation-to-element distribution
/// uses target-incidence weights. Source-incidence weights are intentionally
/// not used by the default policy. Dangling rows (zero or non-positive total)
/// flow the entire `rank` to the dangling reservoir.
///
/// # Performance
///
/// Each call is `O(degree)` for the source state's visible neighborhood.
#[derive(Clone, Copy, Debug)]
pub struct HyperWeighted<'rw, 'iw, RW, IW> {
    /// Borrowed relation-weight adapter driving element → relation transitions.
    relation_weights: &'rw RW,
    /// Borrowed incidence-weight adapter driving relation → element transitions.
    incidence_weights: &'iw IW,
}

impl<'rw, 'iw, RW, IW> HyperWeighted<'rw, 'iw, RW, IW> {
    /// Constructs a [`HyperWeighted`] distribution borrowing the relation and
    /// incidence weight adapters.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(relation_weights: &'rw RW, incidence_weights: &'iw IW) -> Self {
        Self {
            relation_weights,
            incidence_weights,
        }
    }
}

impl<H, RW, IW, S> HypergraphOutgoingDistribution<H, S> for HyperWeighted<'_, '_, RW, IW>
where
    H: PageRankHypergraph + DenseIncidenceIndex,
    RW: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    RW::Weight: IntoPageRankScalar<S>,
    IW: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    IW::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    fn distribute_from_element(
        &self,
        hypergraph: &H,
        element: H::ElementId,
        rank: S,
        next_relations: &mut [S],
        visible_relations: &[u8],
    ) -> Result<S, PageRankError<S>> {
        let total = hyper_outgoing_relation_weight(
            hypergraph,
            self.relation_weights,
            element,
            visible_relations,
        )?;
        if total <= S::ZERO {
            return Ok(rank);
        }
        for relation in hypergraph.outgoing_hyperedges(element) {
            let relation_index = checked_relation_index_for(hypergraph, relation)?;
            if !is_visible(visible_relations, relation_index) {
                continue;
            }
            let weight = checked_relation_weight(hypergraph, self.relation_weights, relation)?;
            next_relations[relation_index] += rank * (weight / total);
        }
        Ok(S::ZERO)
    }

    fn distribute_from_relation(
        &self,
        hypergraph: &H,
        relation: H::RelationId,
        rank: S,
        next_elements: &mut [S],
        visible_elements: &[u8],
    ) -> Result<S, PageRankError<S>> {
        let total = hyper_target_incidence_weight(
            hypergraph,
            self.incidence_weights,
            relation,
            visible_elements,
        )?;
        if total <= S::ZERO {
            return Ok(rank);
        }
        for incidence in hypergraph.target_incidences(relation) {
            let target = hypergraph.incidence_element(incidence);
            let target_index = checked_element_index(hypergraph, target)?;
            if !is_visible(visible_elements, target_index) {
                continue;
            }
            let weight = checked_incidence_weight(hypergraph, self.incidence_weights, incidence)?;
            next_elements[target_index] += rank * (weight / total);
        }
        Ok(S::ZERO)
    }
}

/// `PageRank` configuration shared by graph and hypergraph policies.
///
/// # Performance
///
/// Copying and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageRankConfig<S> {
    /// Damping factor, usually `0.85`.
    pub damping: S,
    /// L1 convergence tolerance.
    pub tolerance: S,
    /// Maximum power-iteration count.
    pub max_iterations: usize,
}

impl<S> PageRankConfig<S> {
    /// Constructs a `PageRank` configuration.
    ///
    /// Validation is performed by the `PageRank` entry point so callers can build
    /// configs before deciding which algorithm variant to invoke.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(damping: S, tolerance: S, max_iterations: usize) -> Self {
        Self {
            damping,
            tolerance,
            max_iterations,
        }
    }
}

impl<S: PageRankScalar> Default for PageRankConfig<S> {
    /// The conventional defaults — damping `0.85`, L1 tolerance `1e-6`, and `100`
    /// power iterations — so consumers stop hardcoding the same triple.
    fn default() -> Self {
        Self {
            damping: S::from_f64(0.85),
            tolerance: S::from_f64(1e-6),
            max_iterations: 100,
        }
    }
}

/// Successful `PageRank` convergence report.
///
/// # Performance
///
/// Copying and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageRankReport<S> {
    /// Number of iterations executed.
    pub iterations: usize,
    /// Final L1 rank delta.
    pub delta: S,
}

/// `PageRank` input, numeric, scratch, and convergence errors.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PageRankError<S> {
    /// `PageRank` is undefined for an empty visible state set.
    EmptyState,
    /// Damping must be finite and in `[0, 1]`.
    InvalidDamping {
        /// Invalid damping value.
        damping: S,
    },
    /// Tolerance must be finite and non-negative.
    InvalidTolerance {
        /// Invalid tolerance value.
        tolerance: S,
    },
    /// At least one iteration is required.
    InvalidMaxIterations,
    /// Output rank storage was shorter than the topology index bound.
    OutputTooShort {
        /// Required length.
        required: usize,
        /// Actual length.
        actual: usize,
    },
    /// Scratch storage was shorter than the required bound.
    ScratchTooShort {
        /// Scratch slice name.
        name: &'static str,
        /// Required length.
        required: usize,
        /// Actual length.
        actual: usize,
    },
    /// Personalization storage was shorter than the topology index bound.
    PersonalizationTooShort {
        /// Required length.
        required: usize,
        /// Actual length.
        actual: usize,
    },
    /// A personalization entry was negative or non-finite.
    InvalidPersonalization {
        /// Invalid index.
        index: usize,
        /// Invalid value.
        value: S,
    },
    /// Personalization sum was zero over visible states.
    ZeroPersonalization,
    /// A topology element mapped outside the advertised element bound.
    ElementIndexOutOfBounds {
        /// Invalid index.
        index: usize,
        /// Advertised bound.
        bound: usize,
    },
    /// A visible element was provided more than once.
    DuplicateElement {
        /// Duplicate dense element index.
        index: usize,
    },
    /// A visible relation was provided more than once.
    DuplicateRelation {
        /// Duplicate dense relation index.
        index: usize,
    },
    /// A topology relation mapped outside the advertised relation bound.
    RelationIndexOutOfBounds {
        /// Invalid index.
        index: usize,
        /// Advertised bound.
        bound: usize,
    },
    /// A topology incidence mapped outside the advertised incidence bound.
    IncidenceIndexOutOfBounds {
        /// Invalid index.
        index: usize,
        /// Advertised bound.
        bound: usize,
    },
    /// A relation weight was negative or non-finite.
    InvalidRelationWeight {
        /// Dense relation index.
        index: usize,
        /// Invalid value.
        value: S,
    },
    /// An incidence weight was negative or non-finite.
    InvalidIncidenceWeight {
        /// Dense incidence index.
        index: usize,
        /// Invalid value.
        value: S,
    },
    /// Power iteration reached the maximum iteration count before convergence.
    NonConverged {
        /// Iterations executed.
        iterations: usize,
        /// Final L1 delta.
        delta: S,
    },
}

impl<S: fmt::Debug> fmt::Display for PageRankError<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyState => formatter.write_str("pagerank state set is empty"),
            Self::InvalidDamping { damping } => {
                write!(formatter, "invalid pagerank damping {damping:?}")
            }
            Self::InvalidTolerance { tolerance } => {
                write!(formatter, "invalid pagerank tolerance {tolerance:?}")
            }
            Self::InvalidMaxIterations => {
                formatter.write_str("pagerank max_iterations must be positive")
            }
            Self::OutputTooShort { required, actual } => write!(
                formatter,
                "pagerank output too short: required {required}, got {actual}"
            ),
            Self::ScratchTooShort {
                name,
                required,
                actual,
            } => write!(
                formatter,
                "pagerank scratch '{name}' too short: required {required}, got {actual}"
            ),
            Self::PersonalizationTooShort { required, actual } => write!(
                formatter,
                "pagerank personalization too short: required {required}, got {actual}"
            ),
            Self::InvalidPersonalization { index, value } => write!(
                formatter,
                "invalid pagerank personalization at {index}: {value:?}"
            ),
            Self::ZeroPersonalization => {
                formatter.write_str("pagerank personalization sum is zero")
            }
            Self::ElementIndexOutOfBounds { index, bound } => {
                write!(formatter, "element index {index} is outside bound {bound}")
            }
            Self::DuplicateElement { index } => {
                write!(formatter, "duplicate pagerank element index {index}")
            }
            Self::DuplicateRelation { index } => {
                write!(formatter, "duplicate pagerank relation index {index}")
            }
            Self::RelationIndexOutOfBounds { index, bound } => {
                write!(formatter, "relation index {index} is outside bound {bound}")
            }
            Self::IncidenceIndexOutOfBounds { index, bound } => {
                write!(
                    formatter,
                    "incidence index {index} is outside bound {bound}"
                )
            }
            Self::InvalidRelationWeight { index, value } => {
                write!(formatter, "invalid relation weight at {index}: {value:?}")
            }
            Self::InvalidIncidenceWeight { index, value } => {
                write!(formatter, "invalid incidence weight at {index}: {value:?}")
            }
            Self::NonConverged { iterations, delta } => write!(
                formatter,
                "pagerank did not converge after {iterations} iterations; delta {delta:?}"
            ),
        }
    }
}

impl<S: fmt::Debug> Error for PageRankError<S> {}

/// Borrowed scratch storage for ordinary graph `PageRank`.
///
/// # Performance
///
/// Construction is `O(1)`. The slices must be at least `graph.element_bound()`
/// long for the graph passed to a scratch API.
#[derive(Debug)]
#[must_use]
pub struct PageRankScratch<'scratch, S> {
    /// Teleport/personalization scratch by element index.
    teleport: &'scratch mut [S],
    /// Next-rank scratch by element index.
    next: &'scratch mut [S],
    /// Visible element bitset by element index.
    visible_elements: &'scratch mut [u8],
}

impl<'scratch, S> PageRankScratch<'scratch, S> {
    /// Constructs borrowed graph `PageRank` scratch from caller-owned slices.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new(
        teleport: &'scratch mut [S],
        next: &'scratch mut [S],
        visible_elements: &'scratch mut [u8],
    ) -> Self {
        Self {
            teleport,
            next,
            visible_elements,
        }
    }

    /// Returns current teleport scratch capacity.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn teleport_capacity(&self) -> usize {
        self.teleport.len()
    }

    /// Returns current next-rank scratch capacity.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn next_capacity(&self) -> usize {
        self.next.len()
    }

    /// Returns current visible-element scratch capacity.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn visible_element_capacity(&self) -> usize {
        self.visible_elements.len()
    }
}

/// Borrowed scratch storage for incidence/bipartite hypergraph `PageRank`.
///
/// # Performance
///
/// Construction is `O(1)`. `teleport` must cover `element_bound + relation_bound`,
/// while `next_elements` and `next_relations` cover their respective bounds.
#[derive(Debug)]
#[must_use]
pub struct HypergraphPageRankScratch<'scratch, S> {
    /// Teleport/personalization scratch by combined element+relation state index.
    teleport: &'scratch mut [S],
    /// Next element ranks by element index.
    next_elements: &'scratch mut [S],
    /// Next relation ranks by relation index.
    next_relations: &'scratch mut [S],
    /// Visible element bitset by element index.
    visible_elements: &'scratch mut [u8],
    /// Visible relation bitset by relation index.
    visible_relations: &'scratch mut [u8],
}

impl<'scratch, S> HypergraphPageRankScratch<'scratch, S> {
    /// Constructs borrowed hypergraph `PageRank` scratch from caller-owned slices.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new(
        teleport: &'scratch mut [S],
        next_elements: &'scratch mut [S],
        next_relations: &'scratch mut [S],
        visible_elements: &'scratch mut [u8],
        visible_relations: &'scratch mut [u8],
    ) -> Self {
        Self {
            teleport,
            next_elements,
            next_relations,
            visible_elements,
            visible_relations,
        }
    }

    /// Returns current teleport scratch capacity.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn teleport_capacity(&self) -> usize {
        self.teleport.len()
    }
}

/// Owned reusable workspace for ordinary graph `PageRank`.
///
/// The `G` parameter brands the workspace to a view type, mirroring
/// [`crate::BfsWorkspace`]. The scalar `S` fixes the rank/storage scalar.
///
/// # Performance
///
/// Memory usage is `O(b)` for the largest element bound used with the workspace.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub struct PageRankWorkspace<G, S> {
    /// Teleport/personalization scratch.
    teleport: Vec<S>,
    /// Next-rank scratch.
    next: Vec<S>,
    /// Visible element bitset.
    visible_elements: Vec<u8>,
    /// Brands workspace storage to a topology view type without owning the view.
    _graph: PhantomData<fn() -> G>,
}

#[cfg(feature = "alloc")]
impl<G, S: PageRankScalar> Default for PageRankWorkspace<G, S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl<G, S: PageRankScalar> PageRankWorkspace<G, S> {
    /// Creates an empty reusable `PageRank` workspace.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` and performs no allocation.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            teleport: Vec::new(),
            next: Vec::new(),
            visible_elements: Vec::new(),
            _graph: PhantomData,
        }
    }

    /// Creates a workspace sized for `graph.element_bound()`.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(graph.element_bound())` storage.
    #[must_use]
    pub fn for_graph(graph: &G) -> Self
    where
        G: DenseElementIndex,
    {
        Self::with_element_bound(graph.element_bound())
    }

    /// Creates a workspace with capacity for `element_bound` element states.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(element_bound)` storage.
    #[must_use]
    pub fn with_element_bound(element_bound: usize) -> Self {
        Self {
            teleport: vec![S::ZERO; element_bound],
            next: vec![S::ZERO; element_bound],
            visible_elements: vec![0; element_bound],
            _graph: PhantomData,
        }
    }

    /// Returns the element-bound capacity currently available without growth.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn element_bound_capacity(&self) -> usize {
        self.teleport.len()
    }

    /// Ensures workspace storage covers `element_bound`.
    fn ensure_element_bound(&mut self, element_bound: usize) {
        if self.teleport.len() < element_bound {
            self.teleport.resize(element_bound, S::ZERO);
        }
        if self.next.len() < element_bound {
            self.next.resize(element_bound, S::ZERO);
        }
        if self.visible_elements.len() < element_bound {
            self.visible_elements.resize(element_bound, 0);
        }
    }

    /// Borrows this workspace as scratch.
    fn as_scratch(&mut self) -> PageRankScratch<'_, S> {
        PageRankScratch::new(
            &mut self.teleport,
            &mut self.next,
            &mut self.visible_elements,
        )
    }
}

/// Owned reusable workspace for incidence/bipartite hypergraph `PageRank`.
///
/// # Performance
///
/// Memory usage is `O(e + r)` for the largest element and relation bounds used.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub struct HypergraphPageRankWorkspace<H, S> {
    /// Combined element+relation teleport/personalization scratch.
    teleport: Vec<S>,
    /// Next element ranks.
    next_elements: Vec<S>,
    /// Next relation ranks.
    next_relations: Vec<S>,
    /// Visible element bitset.
    visible_elements: Vec<u8>,
    /// Visible relation bitset.
    visible_relations: Vec<u8>,
    /// Brands workspace storage to a hypergraph view type without owning the view.
    _hypergraph: PhantomData<fn() -> H>,
}

#[cfg(feature = "alloc")]
impl<H, S: PageRankScalar> Default for HypergraphPageRankWorkspace<H, S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl<H, S: PageRankScalar> HypergraphPageRankWorkspace<H, S> {
    /// Creates an empty reusable hypergraph `PageRank` workspace.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` and performs no allocation.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            teleport: Vec::new(),
            next_elements: Vec::new(),
            next_relations: Vec::new(),
            visible_elements: Vec::new(),
            visible_relations: Vec::new(),
            _hypergraph: PhantomData,
        }
    }

    /// Creates a workspace sized for a hypergraph's element/relation bounds.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(element_bound + relation_bound)` storage.
    #[must_use]
    pub fn for_hypergraph(hypergraph: &H) -> Self
    where
        H: DenseElementIndex + DenseRelationIndex,
    {
        Self::with_bounds(hypergraph.element_bound(), hypergraph.relation_bound())
    }

    /// Creates a workspace with capacity for element and relation bounds.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(element_bound + relation_bound)` storage.
    #[must_use]
    pub fn with_bounds(element_bound: usize, relation_bound: usize) -> Self {
        let state_bound = element_bound.saturating_add(relation_bound);
        Self {
            teleport: vec![S::ZERO; state_bound],
            next_elements: vec![S::ZERO; element_bound],
            next_relations: vec![S::ZERO; relation_bound],
            visible_elements: vec![0; element_bound],
            visible_relations: vec![0; relation_bound],
            _hypergraph: PhantomData,
        }
    }

    /// Returns current element-rank capacity.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn element_bound_capacity(&self) -> usize {
        self.next_elements.len()
    }

    /// Returns current relation-rank capacity.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn relation_bound_capacity(&self) -> usize {
        self.next_relations.len()
    }

    /// Ensures workspace storage covers the requested bounds.
    fn ensure_bounds(&mut self, element_bound: usize, relation_bound: usize, state_bound: usize) {
        if self.teleport.len() < state_bound {
            self.teleport.resize(state_bound, S::ZERO);
        }
        if self.next_elements.len() < element_bound {
            self.next_elements.resize(element_bound, S::ZERO);
        }
        if self.next_relations.len() < relation_bound {
            self.next_relations.resize(relation_bound, S::ZERO);
        }
        if self.visible_elements.len() < element_bound {
            self.visible_elements.resize(element_bound, 0);
        }
        if self.visible_relations.len() < relation_bound {
            self.visible_relations.resize(relation_bound, 0);
        }
    }

    /// Borrows this workspace as hypergraph scratch.
    fn as_scratch(&mut self) -> HypergraphPageRankScratch<'_, S> {
        HypergraphPageRankScratch::new(
            &mut self.teleport,
            &mut self.next_elements,
            &mut self.next_relations,
            &mut self.visible_elements,
            &mut self.visible_relations,
        )
    }
}

/// Computes ordinary directed graph `PageRank` with the supplied outgoing
/// distribution policy, allocating temporary scratch.
///
/// `distribution` selects the per-edge rank-transfer rule:
/// [`Uniform`] reproduces the textbook unweighted `PageRank` (every parallel
/// outgoing edge carries a unit transition weight), while [`Weighted`]
/// reads a caller-supplied edge weight. Any [`OutgoingDistribution`] impl
/// is accepted. `elements` defines the visible state iteration order.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Each iteration is `O(n + m · cost(D))` for `n` visible elements, `m`
/// outgoing edge entries yielded from those elements, and `cost(D)` the
/// per-edge cost of [`OutgoingDistribution::distribute_outgoing`]
/// (`O(1)` for the built-in [`Uniform`] and [`Weighted`] impls).
/// Scratch allocation is `O(b)` where `b` is `graph.element_bound()`.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "PageRank entry point keeps graph, distribution, elements, config, personalization, and output explicit"
)]
pub fn pagerank_graph<G, D, I, S>(
    graph: &G,
    distribution: &D,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + DenseElementIndex,
    D: OutgoingDistribution<G, S>,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    let bound = graph.element_bound();
    let mut teleport = vec![S::ZERO; bound];
    let mut next = vec![S::ZERO; bound];
    let mut visible_elements = vec![0; bound];
    pagerank_graph_with_scratch(
        graph,
        distribution,
        elements,
        config,
        personalization,
        ranks,
        PageRankScratch::new(&mut teleport, &mut next, &mut visible_elements),
    )
}

/// Computes graph `PageRank` under the supplied outgoing distribution
/// policy with caller-provided borrowed scratch.
///
/// See [`pagerank_graph`] for the role of `distribution`.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Performs no heap allocation after caller scratch has been provided. Each
/// iteration is `O(n + m · cost(D))`.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "PageRank scratch API consumes a scratch handle and keeps policy inputs explicit"
)]
pub fn pagerank_graph_with_scratch<G, D, I, S>(
    graph: &G,
    distribution: &D,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
    scratch: PageRankScratch<'_, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + DenseElementIndex,
    D: OutgoingDistribution<G, S>,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    validate_config(config)?;
    let bound = graph.element_bound();
    ensure_output_len(ranks.len(), bound)?;
    ensure_scratch_len("teleport", scratch.teleport.len(), bound)?;
    ensure_scratch_len("next", scratch.next.len(), bound)?;
    ensure_scratch_len("visible_elements", scratch.visible_elements.len(), bound)?;
    build_personalization_into(
        elements.clone(),
        bound,
        personalization,
        |element| graph.element_index(element),
        scratch.teleport,
        scratch.visible_elements,
    )?;
    initialize_ranks(elements.clone(), graph, scratch.teleport, ranks)?;
    iterate_graph(
        graph,
        distribution,
        elements,
        config,
        scratch.teleport,
        scratch.visible_elements,
        ranks,
        scratch.next,
    )
}

/// Computes graph `PageRank` under the supplied outgoing distribution
/// policy with a reusable owned workspace.
///
/// See [`pagerank_graph`] for the role of `distribution`.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, or non-convergence.
///
/// # Performance
///
/// Grows workspace storage to `graph.element_bound()` if needed, then performs no
/// additional heap allocation. Each iteration is `O(n + m · cost(D))`.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "PageRank workspace API keeps policy and reusable storage inputs explicit"
)]
pub fn pagerank_graph_with_workspace<G, D, I, S>(
    graph: &G,
    distribution: &D,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
    workspace: &mut PageRankWorkspace<G, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + DenseElementIndex,
    D: OutgoingDistribution<G, S>,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    workspace.ensure_element_bound(graph.element_bound());
    pagerank_graph_with_scratch(
        graph,
        distribution,
        elements,
        config,
        personalization,
        ranks,
        workspace.as_scratch(),
    )
}

/// Computes directed hypergraph incidence/bipartite `PageRank` under the
/// supplied bipartite outgoing distribution policy, allocating temporary
/// scratch.
///
/// `distribution` selects the per-state rank-transfer rule:
/// [`Uniform`] divides each leg uniformly over visible out-degree, while
/// [`HyperWeighted`] reads caller-supplied relation and target-incidence
/// weights. Any [`HypergraphOutgoingDistribution`] impl is accepted.
/// `elements` and `relations` define the visible state iteration order
/// across the bipartite element + relation state space.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, scratch length, or
/// non-convergence.
///
/// # Performance
///
/// Each iteration is `O(e + r + p · cost(D))` for `e` visible elements, `r`
/// visible relations, `p` traversed source/target participant entries, and
/// `cost(D)` the per-entry cost of
/// [`HypergraphOutgoingDistribution::distribute_from_element`] and
/// [`HypergraphOutgoingDistribution::distribute_from_relation`] (`O(1)` for
/// the built-in [`Uniform`] and [`HyperWeighted`] impls). Scratch allocation
/// is `O(e + r)`.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph PageRank entry point keeps hypergraph, distribution, state families, and output explicit"
)]
pub fn pagerank_hypergraph<H, D, IE, IR, S>(
    hypergraph: &H,
    distribution: &D,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: PageRankHypergraph,
    D: HypergraphOutgoingDistribution<H, S>,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let e_bound = hypergraph.element_bound();
    let r_bound = hypergraph.relation_bound();
    let state_bound =
        checked_state_bound::<S>(e_bound, r_bound, element_ranks.len(), relation_ranks.len())?;
    let mut teleport = vec![S::ZERO; state_bound];
    let mut next_elements = vec![S::ZERO; e_bound];
    let mut next_relations = vec![S::ZERO; r_bound];
    let mut visible_elements = vec![0; e_bound];
    let mut visible_relations = vec![0; r_bound];
    pagerank_hypergraph_with_scratch(
        hypergraph,
        distribution,
        elements,
        relations,
        config,
        personalization,
        element_ranks,
        relation_ranks,
        HypergraphPageRankScratch::new(
            &mut teleport,
            &mut next_elements,
            &mut next_relations,
            &mut visible_elements,
            &mut visible_relations,
        ),
    )
}

/// Computes hypergraph `PageRank` under the supplied outgoing distribution
/// policy with caller-provided borrowed scratch.
///
/// See [`pagerank_hypergraph`] for the role of `distribution`.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, scratch length, or
/// non-convergence.
///
/// # Performance
///
/// Performs no heap allocation after caller scratch has been provided. Each
/// iteration is `O(e + r + p · cost(D))`.
#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph PageRank scratch entry point keeps policy and storage inputs explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "hypergraph PageRank scratch API consumes a scratch handle and keeps policy inputs explicit"
)]
pub fn pagerank_hypergraph_with_scratch<H, D, IE, IR, S>(
    hypergraph: &H,
    distribution: &D,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    scratch: HypergraphPageRankScratch<'_, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: PageRankHypergraph,
    D: HypergraphOutgoingDistribution<H, S>,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    validate_config(config)?;
    let e_bound = hypergraph.element_bound();
    let r_bound = hypergraph.relation_bound();
    let state_bound =
        checked_state_bound::<S>(e_bound, r_bound, element_ranks.len(), relation_ranks.len())?;
    ensure_scratch_len("teleport", scratch.teleport.len(), state_bound)?;
    ensure_scratch_len("next_elements", scratch.next_elements.len(), e_bound)?;
    ensure_scratch_len("next_relations", scratch.next_relations.len(), r_bound)?;
    ensure_scratch_len("visible_elements", scratch.visible_elements.len(), e_bound)?;
    ensure_scratch_len(
        "visible_relations",
        scratch.visible_relations.len(),
        r_bound,
    )?;
    build_hyper_personalization_into(
        hypergraph,
        elements.clone(),
        relations.clone(),
        state_bound,
        personalization,
        scratch.teleport,
        scratch.visible_elements,
        scratch.visible_relations,
    )?;
    initialize_hyper_ranks(
        hypergraph,
        elements.clone(),
        relations.clone(),
        scratch.teleport,
        element_ranks,
        relation_ranks,
    )?;
    iterate_hypergraph(
        hypergraph,
        distribution,
        elements,
        relations,
        config,
        scratch.teleport,
        scratch.visible_elements,
        scratch.visible_relations,
        element_ranks,
        relation_ranks,
        scratch.next_elements,
        scratch.next_relations,
    )
}

/// Computes hypergraph `PageRank` under the supplied outgoing distribution
/// policy with a reusable owned workspace.
///
/// See [`pagerank_hypergraph`] for the role of `distribution`.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, or non-convergence.
///
/// # Performance
///
/// Grows workspace storage to the visible bounds if needed, then performs no
/// additional heap allocation. Each iteration is `O(e + r + p · cost(D))`.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph PageRank workspace entry point keeps policy and storage inputs explicit"
)]
pub fn pagerank_hypergraph_with_workspace<H, D, IE, IR, S>(
    hypergraph: &H,
    distribution: &D,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    workspace: &mut HypergraphPageRankWorkspace<H, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: PageRankHypergraph,
    D: HypergraphOutgoingDistribution<H, S>,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let e_bound = hypergraph.element_bound();
    let r_bound = hypergraph.relation_bound();
    let state_bound =
        checked_state_bound::<S>(e_bound, r_bound, element_ranks.len(), relation_ranks.len())?;
    workspace.ensure_bounds(e_bound, r_bound, state_bound);
    pagerank_hypergraph_with_scratch(
        hypergraph,
        distribution,
        elements,
        relations,
        config,
        personalization,
        element_ranks,
        relation_ranks,
        workspace.as_scratch(),
    )
}

fn validate_config<S: PageRankScalar>(config: PageRankConfig<S>) -> Result<(), PageRankError<S>> {
    if !config.damping.is_finite() || config.damping < S::ZERO || config.damping > S::ONE {
        return Err(PageRankError::InvalidDamping {
            damping: config.damping,
        });
    }
    if !config.tolerance.is_finite() || config.tolerance < S::ZERO {
        return Err(PageRankError::InvalidTolerance {
            tolerance: config.tolerance,
        });
    }
    if config.max_iterations == 0 {
        return Err(PageRankError::InvalidMaxIterations);
    }
    Ok(())
}

const fn ensure_output_len<S>(actual: usize, required: usize) -> Result<(), PageRankError<S>> {
    if actual < required {
        Err(PageRankError::OutputTooShort { required, actual })
    } else {
        Ok(())
    }
}

const fn ensure_scratch_len<S>(
    name: &'static str,
    actual: usize,
    required: usize,
) -> Result<(), PageRankError<S>> {
    if actual < required {
        Err(PageRankError::ScratchTooShort {
            name,
            required,
            actual,
        })
    } else {
        Ok(())
    }
}

fn checked_state_bound<S>(
    e_bound: usize,
    r_bound: usize,
    element_output_len: usize,
    relation_output_len: usize,
) -> Result<usize, PageRankError<S>> {
    ensure_output_len(element_output_len, e_bound)?;
    ensure_output_len(relation_output_len, r_bound)?;
    e_bound
        .checked_add(r_bound)
        .ok_or_else(|| PageRankError::OutputTooShort {
            required: usize::MAX,
            actual: element_output_len.saturating_add(relation_output_len),
        })
}

fn clear<S: PageRankScalar>(values: &mut [S], len: usize) {
    for value in &mut values[..len] {
        *value = S::ZERO;
    }
}

fn clear_u8(values: &mut [u8], len: usize) {
    for value in &mut values[..len] {
        *value = 0;
    }
}

fn mark_visible_element<S>(visible: &mut [u8], index: usize) -> Result<(), PageRankError<S>> {
    if visible[index] != 0 {
        return Err(PageRankError::DuplicateElement { index });
    }
    visible[index] = 1;
    Ok(())
}

fn mark_visible_relation<S>(visible: &mut [u8], index: usize) -> Result<(), PageRankError<S>> {
    if visible[index] != 0 {
        return Err(PageRankError::DuplicateRelation { index });
    }
    visible[index] = 1;
    Ok(())
}

// `visible` slices are sized to the view's element/relation bound and every
// index is derived from `element_index`/`relation_index`, so direct indexing is
// in range — matching `mark_visible_element`/`mark_visible_relation`, which also
// index directly. This keeps a single "exactly-sized bitset" contract.
fn is_visible(visible: &[u8], index: usize) -> bool {
    visible[index] != 0
}

#[expect(
    clippy::too_many_arguments,
    reason = "personalization normalization keeps topology family bounds and caller buffers explicit"
)]
fn build_personalization_into<I, F, S>(
    elements: I,
    bound: usize,
    personalization: Option<&[S]>,
    index_of: F,
    out: &mut [S],
    visible: &mut [u8],
) -> Result<(), PageRankError<S>>
where
    I: IntoIterator,
    F: Fn(I::Item) -> usize,
    S: PageRankScalar,
{
    clear(out, bound);
    clear_u8(visible, bound);
    let mut count = 0_usize;
    let mut sum = S::ZERO;
    if let Some(input) = personalization {
        if input.len() < bound {
            return Err(PageRankError::PersonalizationTooShort {
                required: bound,
                actual: input.len(),
            });
        }
        for element in elements {
            let index = index_of(element);
            check_index(index, bound)?;
            mark_visible_element(visible, index)?;
            let value = input[index];
            check_personalization_value(index, value)?;
            out[index] = value;
            sum += value;
            count += 1;
        }
    } else {
        for element in elements {
            let index = index_of(element);
            check_index(index, bound)?;
            mark_visible_element(visible, index)?;
            out[index] = S::ONE;
            sum += S::ONE;
            count += 1;
        }
    }
    normalize_personalization(out, count, sum)
}

/// Personalization source used by [`PageRank`](crate) initialization.
///
/// Drives the per-visible-state initialization: either copy from a
/// caller-supplied vector ([`Self::FromInput`]) or fill every visible state
/// with `S::ONE` ([`Self::Uniform`]).
///
/// # Performance
///
/// `value_at` is `O(1)`. The fill methods walk visible elements and (for
/// hypergraphs) visible relations exactly once.
enum PersonalizationSource<'a, S> {
    /// Initialize every visible state slot to `S::ONE`.
    Uniform,
    /// Copy the value at the matching state index from the supplied slice.
    /// The slice must already be range-checked against the caller's state
    /// bound; [`Self::value_at`] only validates each value.
    FromInput(&'a [S]),
}

impl<S> PersonalizationSource<'_, S>
where
    S: PageRankScalar,
{
    /// Returns the value to place at `state_index`.
    ///
    /// `FromInput` reads `input[state_index]` and rejects non-finite or
    /// negative entries via [`PageRankError::InvalidPersonalization`].
    /// `Uniform` returns `S::ONE` unconditionally.
    ///
    /// # Performance
    ///
    /// `O(1)`.
    fn value_at(&self, state_index: usize) -> Result<S, PageRankError<S>> {
        match *self {
            Self::Uniform => Ok(S::ONE),
            Self::FromInput(input) => {
                let value = input[state_index];
                check_personalization_value(state_index, value)?;
                Ok(value)
            }
        }
    }

    /// Fills the hypergraph teleport vector at visible-element and
    /// visible-relation states.
    ///
    /// Returns `(count, sum)` for the normalization step. `state_bound` is
    /// the total length of `out` (`element_bound + relation_bound`); it is
    /// used to bounds-check `FromInput` slices before iteration.
    ///
    /// # Errors
    ///
    /// [`PageRankError::PersonalizationTooShort`] when a `FromInput` slice is
    /// shorter than `state_bound`;
    /// [`PageRankError::ElementIndexOutOfBounds`] /
    /// [`PageRankError::RelationIndexOutOfBounds`] when a visible element or
    /// relation maps outside its bound; [`PageRankError::DuplicateElement`] /
    /// [`PageRankError::DuplicateRelation`] when a state is marked visible
    /// twice; and [`PageRankError::InvalidPersonalization`] for a negative
    /// personalization entry.
    ///
    /// # Performance
    ///
    /// `O(|elements| + |relations|)` plus the index/visibility check cost
    /// per state slot.
    #[expect(
        clippy::too_many_arguments,
        reason = "method threads separate element/relation iterables, scratch slices, and the state bound; bundling them obscures the borrow shape that callers explicitly construct"
    )]
    fn fill_hypergraph<H, IE, IR>(
        &self,
        hypergraph: &H,
        elements: IE,
        relations: IR,
        state_bound: usize,
        out: &mut [S],
        visible_elements: &mut [u8],
        visible_relations: &mut [u8],
    ) -> Result<(usize, S), PageRankError<S>>
    where
        H: DenseElementIndex + DenseRelationIndex,
        IE: IntoIterator<Item = ElementId<H>>,
        IR: IntoIterator<Item = RelationId<H>>,
    {
        if let Self::FromInput(input) = *self
            && input.len() < state_bound
        {
            return Err(PageRankError::PersonalizationTooShort {
                required: state_bound,
                actual: input.len(),
            });
        }
        let e_bound = hypergraph.element_bound();
        let r_bound = hypergraph.relation_bound();
        let mut count = 0_usize;
        let mut sum = S::ZERO;
        for element in elements {
            let index = hypergraph.element_index(element);
            check_index(index, e_bound)?;
            mark_visible_element(visible_elements, index)?;
            let value = self.value_at(index)?;
            out[index] = value;
            sum += value;
            count += 1;
        }
        for relation in relations {
            let index = hypergraph.relation_index(relation);
            check_relation_index(index, r_bound)?;
            mark_visible_relation(visible_relations, index)?;
            let state = e_bound + index;
            let value = self.value_at(state)?;
            out[state] = value;
            sum += value;
            count += 1;
        }
        Ok((count, sum))
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "helper threads separate element/relation state and caller scratch explicitly"
)]
fn build_hyper_personalization_into<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    state_bound: usize,
    personalization: Option<&[S]>,
    out: &mut [S],
    visible_elements: &mut [u8],
    visible_relations: &mut [u8],
) -> Result<(), PageRankError<S>>
where
    H: DenseElementIndex + DenseRelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    clear(out, state_bound);
    clear_u8(visible_elements, hypergraph.element_bound());
    clear_u8(visible_relations, hypergraph.relation_bound());
    let source = personalization.map_or(
        PersonalizationSource::Uniform,
        PersonalizationSource::FromInput,
    );
    let (count, sum) = source.fill_hypergraph(
        hypergraph,
        elements,
        relations,
        state_bound,
        out,
        visible_elements,
        visible_relations,
    )?;
    normalize_personalization(out, count, sum)
}

fn normalize_personalization<S: PageRankScalar>(
    out: &mut [S],
    count: usize,
    sum: S,
) -> Result<(), PageRankError<S>> {
    if count == 0 {
        return Err(PageRankError::EmptyState);
    }
    if sum <= S::ZERO {
        return Err(PageRankError::ZeroPersonalization);
    }
    for value in out {
        *value = *value / sum;
    }
    Ok(())
}

fn check_personalization_value<S: PageRankScalar>(
    index: usize,
    value: S,
) -> Result<(), PageRankError<S>> {
    if !value.is_finite() || value < S::ZERO {
        Err(PageRankError::InvalidPersonalization { index, value })
    } else {
        Ok(())
    }
}

fn initialize_ranks<G, I, S>(
    elements: I,
    graph: &G,
    teleport: &[S],
    ranks: &mut [S],
) -> Result<(), PageRankError<S>>
where
    G: DenseElementIndex,
    I: IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    clear(ranks, graph.element_bound());
    for element in elements {
        let index = graph.element_index(element);
        check_index(index, graph.element_bound())?;
        ranks[index] = teleport[index];
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "initialization writes separate element and relation rank slices"
)]
fn initialize_hyper_ranks<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    teleport: &[S],
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
) -> Result<(), PageRankError<S>>
where
    H: DenseElementIndex + DenseRelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    clear(element_ranks, hypergraph.element_bound());
    clear(relation_ranks, hypergraph.relation_bound());
    for element in elements {
        let index = hypergraph.element_index(element);
        check_index(index, hypergraph.element_bound())?;
        element_ranks[index] = teleport[index];
    }
    for relation in relations {
        let index = hypergraph.relation_index(relation);
        check_relation_index(index, hypergraph.relation_bound())?;
        relation_ranks[index] = teleport[hypergraph.element_bound() + index];
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "graph iteration helper keeps distribution, scratch, and policy inputs explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "iteration helpers own cloneable iterator values and clone them each power iteration"
)]
fn iterate_graph<G, D, I, S>(
    graph: &G,
    distribution: &D,
    elements: I,
    config: PageRankConfig<S>,
    teleport: &[S],
    visible: &[u8],
    ranks: &mut [S],
    next: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + DenseElementIndex,
    D: OutgoingDistribution<G, S>,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    let mut last_delta = S::INFINITY;
    for iteration in 1..=config.max_iterations {
        clear(next, graph.element_bound());
        let mut dangling = S::ZERO;
        for element in elements.clone() {
            let index = checked_element_index(graph, element)?;
            let rank = ranks[index];
            dangling += distribution.distribute_outgoing(graph, element, rank, next, visible)?;
        }
        let delta = apply_graph_teleport(
            graph,
            elements.clone(),
            config,
            teleport,
            dangling,
            ranks,
            next,
        )?;
        last_delta = delta;
        if delta <= config.tolerance {
            return Ok(PageRankReport {
                iterations: iteration,
                delta,
            });
        }
    }
    Err(PageRankError::NonConverged {
        iterations: config.max_iterations,
        delta: last_delta,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph iteration helper keeps distribution, state families, scratch, and policy inputs explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "iteration helpers own cloneable iterator values and clone them each power iteration"
)]
fn iterate_hypergraph<H, D, IE, IR, S>(
    hypergraph: &H,
    distribution: &D,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    teleport: &[S],
    visible_elements: &[u8],
    visible_relations: &[u8],
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    next_elements: &mut [S],
    next_relations: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: PageRankHypergraph,
    D: HypergraphOutgoingDistribution<H, S>,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut last_delta = S::INFINITY;
    for iteration in 1..=config.max_iterations {
        clear(next_elements, hypergraph.element_bound());
        clear(next_relations, hypergraph.relation_bound());
        let mut dangling = S::ZERO;
        for element in elements.clone() {
            let index = checked_element_index(hypergraph, element)?;
            let rank = element_ranks[index];
            dangling += distribution.distribute_from_element(
                hypergraph,
                element,
                rank,
                next_relations,
                visible_relations,
            )?;
        }
        for relation in relations.clone() {
            let index = checked_relation_index_for(hypergraph, relation)?;
            let rank = relation_ranks[index];
            dangling += distribution.distribute_from_relation(
                hypergraph,
                relation,
                rank,
                next_elements,
                visible_elements,
            )?;
        }
        let delta = apply_hyper_teleport(
            hypergraph,
            elements.clone(),
            relations.clone(),
            config,
            teleport,
            dangling,
            element_ranks,
            relation_ranks,
            next_elements,
            next_relations,
        )?;
        last_delta = delta;
        if delta <= config.tolerance {
            return Ok(PageRankReport {
                iterations: iteration,
                delta,
            });
        }
    }
    Err(PageRankError::NonConverged {
        iterations: config.max_iterations,
        delta: last_delta,
    })
}

fn checked_element_index<G: DenseElementIndex, S>(
    graph: &G,
    element: ElementId<G>,
) -> Result<usize, PageRankError<S>> {
    let index = graph.element_index(element);
    check_index(index, graph.element_bound())?;
    Ok(index)
}

const fn check_index<S>(index: usize, bound: usize) -> Result<(), PageRankError<S>> {
    if index < bound {
        Ok(())
    } else {
        Err(PageRankError::ElementIndexOutOfBounds { index, bound })
    }
}

const fn check_relation_index<S>(index: usize, bound: usize) -> Result<(), PageRankError<S>> {
    if index < bound {
        Ok(())
    } else {
        Err(PageRankError::RelationIndexOutOfBounds { index, bound })
    }
}

fn checked_relation_weight<G, W, S>(
    graph: &G,
    weights: &W,
    relation: RelationId<G>,
) -> Result<S, PageRankError<S>>
where
    G: DenseRelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    let index = graph.relation_index(relation);
    check_relation_index(index, graph.relation_bound())?;
    let value = weights.relation_weight(relation).into_pagerank_scalar();
    if !value.is_finite() || value < S::ZERO {
        Err(PageRankError::InvalidRelationWeight { index, value })
    } else {
        Ok(value)
    }
}

fn checked_incidence_weight<H, W, S>(
    hypergraph: &H,
    weights: &W,
    incidence: H::IncidenceId,
) -> Result<S, PageRankError<S>>
where
    H: DenseIncidenceIndex,
    W: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    W::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    let index = hypergraph.incidence_index(incidence);
    let bound = hypergraph.incidence_bound();
    if index >= bound {
        return Err(PageRankError::IncidenceIndexOutOfBounds { index, bound });
    }
    let value = weights.incidence_weight(incidence).into_pagerank_scalar();
    if !value.is_finite() || value < S::ZERO {
        Err(PageRankError::InvalidIncidenceWeight { index, value })
    } else {
        Ok(value)
    }
}

fn outgoing_weight_total<G, W, S>(
    graph: &G,
    weights: &W,
    element: ElementId<G>,
    visible: &[u8],
) -> Result<S, PageRankError<S>>
where
    G: ForwardGraph + DenseElementIndex + DenseRelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    let mut total = S::ZERO;
    for edge in graph.outgoing_edges(element) {
        let target = graph.target(edge);
        let target_index = checked_element_index(graph, target)?;
        if is_visible(visible, target_index) {
            total += checked_relation_weight(graph, weights, edge)?;
        }
    }
    Ok(total)
}

/// Computes the teleport-blended rank for one state and its absolute delta.
///
/// Shared by the graph and both hypergraph state legs so the convergence
/// formula `damping * accumulated + teleport_scale * teleport` lives in exactly
/// one place. Returns `(new_value, |new_value - previous|)`; the caller writes
/// `new_value` into the rank slice (the `next`/accumulator slice is cleared at
/// the top of the following iteration, so it is intentionally not written back).
///
/// # Performance
///
/// This function is `O(1)`.
fn teleport_update<S: PageRankScalar>(
    damping: S,
    teleport_scale: S,
    accumulated: S,
    teleport: S,
    previous: S,
) -> (S, S) {
    let value = (damping * accumulated) + (teleport_scale * teleport);
    let delta = (value - previous).abs();
    (value, delta)
}

#[expect(
    clippy::too_many_arguments,
    reason = "teleport helper updates caller-provided rank and scratch slices"
)]
fn apply_graph_teleport<G, I, S>(
    graph: &G,
    elements: I,
    config: PageRankConfig<S>,
    teleport: &[S],
    dangling: S,
    ranks: &mut [S],
    next: &[S],
) -> Result<S, PageRankError<S>>
where
    G: DenseElementIndex,
    I: IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    let mut delta = S::ZERO;
    let teleport_scale = (S::ONE - config.damping) + (config.damping * dangling);
    for element in elements {
        let index = checked_element_index(graph, element)?;
        let (value, state_delta) = teleport_update(
            config.damping,
            teleport_scale,
            next[index],
            teleport[index],
            ranks[index],
        );
        delta += state_delta;
        ranks[index] = value;
    }
    Ok(delta)
}

fn checked_relation_index_for<H: DenseRelationIndex, S>(
    hypergraph: &H,
    relation: RelationId<H>,
) -> Result<usize, PageRankError<S>> {
    let index = hypergraph.relation_index(relation);
    check_relation_index(index, hypergraph.relation_bound())?;
    Ok(index)
}

fn hyper_outgoing_relation_weight<H, W, S>(
    hypergraph: &H,
    weights: &W,
    element: ElementId<H>,
    visible_relations: &[u8],
) -> Result<S, PageRankError<S>>
where
    H: DirectedVertexHyperedges + DenseRelationIndex,
    W: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    let mut total = S::ZERO;
    for relation in hypergraph.outgoing_hyperedges(element) {
        let relation_index = checked_relation_index_for(hypergraph, relation)?;
        if is_visible(visible_relations, relation_index) {
            total += checked_relation_weight(hypergraph, weights, relation)?;
        }
    }
    Ok(total)
}

fn hyper_target_incidence_weight<H, W, S>(
    hypergraph: &H,
    weights: &W,
    relation: RelationId<H>,
    visible_elements: &[u8],
) -> Result<S, PageRankError<S>>
where
    H: DirectedHyperedgeIncidences + IncidenceElement + DenseElementIndex + DenseIncidenceIndex,
    W: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    W::Weight: IntoPageRankScalar<S>,
    S: PageRankScalar,
{
    let mut total = S::ZERO;
    for incidence in hypergraph.target_incidences(relation) {
        let target = hypergraph.incidence_element(incidence);
        let target_index = checked_element_index(hypergraph, target)?;
        if is_visible(visible_elements, target_index) {
            total += checked_incidence_weight(hypergraph, weights, incidence)?;
        }
    }
    Ok(total)
}

#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph teleport updates separate element and relation slices"
)]
fn apply_hyper_teleport<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    teleport: &[S],
    dangling: S,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    next_elements: &[S],
    next_relations: &[S],
) -> Result<S, PageRankError<S>>
where
    H: DenseElementIndex + DenseRelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut delta = S::ZERO;
    let e_bound = hypergraph.element_bound();
    let teleport_scale = (S::ONE - config.damping) + (config.damping * dangling);
    for element in elements {
        let index = checked_element_index(hypergraph, element)?;
        let (value, state_delta) = teleport_update(
            config.damping,
            teleport_scale,
            next_elements[index],
            teleport[index],
            element_ranks[index],
        );
        delta += state_delta;
        element_ranks[index] = value;
    }
    for relation in relations {
        let index = checked_relation_index_for(hypergraph, relation)?;
        let state = e_bound + index;
        let (value, state_delta) = teleport_update(
            config.damping,
            teleport_scale,
            next_relations[index],
            teleport[state],
            relation_ranks[index],
        );
        delta += state_delta;
        relation_ranks[index] = value;
    }
    Ok(delta)
}

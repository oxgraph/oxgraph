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
use oxgraph_topology::{
    ElementId, ElementIndex, IncidenceElement, IncidenceIndex, IncidenceWeight, RelationId,
    RelationIndex, RelationWeight,
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
        G: ElementIndex,
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
        H: ElementIndex + RelationIndex,
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

/// Computes unweighted ordinary directed graph `PageRank`, allocating temporary scratch.
///
/// `elements` defines the visible state iteration order. Edge multiplicity is
/// preserved: parallel outgoing edges each receive a unit transition weight.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Each iteration is `O(n + m)` for `n` visible elements and `m` outgoing edge
/// entries yielded from those elements. Scratch allocation is `O(b)` where `b`
/// is `graph.element_bound()`.
#[cfg(feature = "alloc")]
pub fn pagerank<G, I, S>(
    graph: &G,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    let bound = graph.element_bound();
    let mut teleport = vec![S::ZERO; bound];
    let mut next = vec![S::ZERO; bound];
    let mut visible_elements = vec![0; bound];
    pagerank_with_scratch(
        graph,
        elements,
        config,
        personalization,
        ranks,
        PageRankScratch::new(&mut teleport, &mut next, &mut visible_elements),
    )
}

/// Computes unweighted graph `PageRank` with caller-provided borrowed scratch.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Performs no heap allocation after caller scratch has been provided. Each
/// iteration is `O(n + m)`.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "PageRank scratch API consumes a scratch handle and keeps policy inputs explicit"
)]
pub fn pagerank_with_scratch<G, I, S>(
    graph: &G,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
    scratch: PageRankScratch<'_, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex,
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
    iterate_graph_unweighted(
        graph,
        elements,
        config,
        scratch.teleport,
        scratch.visible_elements,
        ranks,
        scratch.next,
    )
}

/// Computes unweighted graph `PageRank` with reusable owned workspace.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, or non-convergence.
///
/// # Performance
///
/// Grows workspace storage to `graph.element_bound()` if needed, then performs no
/// additional heap allocation. Each iteration is `O(n + m)`.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "PageRank workspace API keeps policy and reusable storage inputs explicit"
)]
pub fn pagerank_with_workspace<G, I, S>(
    graph: &G,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
    workspace: &mut PageRankWorkspace<G, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    workspace.ensure_element_bound(graph.element_bound());
    pagerank_with_scratch(
        graph,
        elements,
        config,
        personalization,
        ranks,
        workspace.as_scratch(),
    )
}

/// Computes relation-weighted ordinary directed graph `PageRank`, allocating scratch.
///
/// Weights are row-normalized per source element. Weights must be finite and
/// non-negative; zero-total outgoing rows are dangling rows.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Each iteration is `O(n + m)` for `n` visible elements and `m` outgoing edge
/// entries, with two outgoing-edge passes for non-dangling weighted rows.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "weighted PageRank entry point keeps graph, weights, policy, personalization, and output explicit"
)]
pub fn pagerank_weighted<G, W, I, S>(
    graph: &G,
    weights: &W,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex + RelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    let bound = graph.element_bound();
    let mut teleport = vec![S::ZERO; bound];
    let mut next = vec![S::ZERO; bound];
    let mut visible_elements = vec![0; bound];
    pagerank_weighted_with_scratch(
        graph,
        weights,
        elements,
        config,
        personalization,
        ranks,
        PageRankScratch::new(&mut teleport, &mut next, &mut visible_elements),
    )
}

/// Computes weighted graph `PageRank` with caller-provided borrowed scratch.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Performs no heap allocation after caller scratch has been provided. Each
/// iteration is `O(n + m)` with two outgoing-edge passes for weighted rows.
#[expect(
    clippy::too_many_arguments,
    reason = "weighted PageRank scratch entry point keeps all policy and storage inputs explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "PageRank scratch API consumes a scratch handle and keeps policy inputs explicit"
)]
pub fn pagerank_weighted_with_scratch<G, W, I, S>(
    graph: &G,
    weights: &W,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
    scratch: PageRankScratch<'_, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex + RelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
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
    iterate_graph_weighted(
        graph,
        weights,
        elements,
        config,
        scratch.teleport,
        scratch.visible_elements,
        ranks,
        scratch.next,
    )
}

/// Computes weighted graph `PageRank` with reusable owned workspace.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, or non-convergence.
///
/// # Performance
///
/// Grows workspace storage to `graph.element_bound()` if needed, then performs no
/// additional heap allocation.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "weighted PageRank workspace entry point keeps all policy and storage inputs explicit"
)]
pub fn pagerank_weighted_with_workspace<G, W, I, S>(
    graph: &G,
    weights: &W,
    elements: I,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    ranks: &mut [S],
    workspace: &mut PageRankWorkspace<G, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex + RelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
    I: Clone + IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    workspace.ensure_element_bound(graph.element_bound());
    pagerank_weighted_with_scratch(
        graph,
        weights,
        elements,
        config,
        personalization,
        ranks,
        workspace.as_scratch(),
    )
}

/// Computes unweighted directed hypergraph incidence/bipartite `PageRank`.
///
/// The state space is elements plus relations. Element states choose outgoing
/// hyperedges uniformly; relation states choose target incidences uniformly.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Each iteration is `O(e + r + p)` for visible elements, visible relations,
/// and traversed source/target participant entries.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph PageRank entry point keeps state families and output slices explicit"
)]
pub fn hypergraph_pagerank<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex,
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
    hypergraph_pagerank_with_scratch(
        hypergraph,
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

/// Computes unweighted hypergraph `PageRank` with caller-provided borrowed scratch.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Performs no heap allocation after caller scratch has been provided.
#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph PageRank scratch entry point keeps state families and storage explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "hypergraph PageRank scratch API consumes a scratch handle and keeps policy inputs explicit"
)]
pub fn hypergraph_pagerank_with_scratch<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    scratch: HypergraphPageRankScratch<'_, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex,
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
    iterate_hyper_unweighted(
        hypergraph,
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

/// Computes unweighted hypergraph `PageRank` with reusable owned workspace.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, output length, or non-convergence.
///
/// # Performance
///
/// Grows workspace storage to the visible bounds if needed, then performs no
/// additional heap allocation.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph PageRank workspace entry point keeps state families and storage explicit"
)]
pub fn hypergraph_pagerank_with_workspace<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    workspace: &mut HypergraphPageRankWorkspace<H, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let e_bound = hypergraph.element_bound();
    let r_bound = hypergraph.relation_bound();
    let state_bound =
        checked_state_bound::<S>(e_bound, r_bound, element_ranks.len(), relation_ranks.len())?;
    workspace.ensure_bounds(e_bound, r_bound, state_bound);
    hypergraph_pagerank_with_scratch(
        hypergraph,
        elements,
        relations,
        config,
        personalization,
        element_ranks,
        relation_ranks,
        workspace.as_scratch(),
    )
}

/// Computes weighted directed hypergraph incidence/bipartite `PageRank`.
///
/// Relation weights choose source element → relation transitions. Target
/// incidence weights choose relation → target element transitions. Source
/// incidence weights are intentionally not used by this default policy.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid relation/incidence weights, output length, scratch
/// length, or non-convergence.
///
/// # Performance
///
/// Each iteration is `O(e + r + p)` with two passes over weighted non-dangling
/// rows.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "weighted hypergraph PageRank keeps relation and incidence policies explicit"
)]
pub fn hypergraph_pagerank_weighted<H, RW, IW, IE, IR, S>(
    hypergraph: &H,
    relation_weights: &RW,
    incidence_weights: &IW,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex
        + IncidenceIndex,
    RW: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    RW::Weight: IntoPageRankScalar<S>,
    IW: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    IW::Weight: IntoPageRankScalar<S>,
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
    hypergraph_pagerank_weighted_with_scratch(
        hypergraph,
        relation_weights,
        incidence_weights,
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

/// Computes weighted hypergraph `PageRank` with caller-provided borrowed scratch.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, scratch length, or non-convergence.
///
/// # Performance
///
/// Performs no heap allocation after caller scratch has been provided.
#[expect(
    clippy::too_many_arguments,
    reason = "weighted hypergraph PageRank scratch entry point keeps all policy and storage inputs explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "hypergraph PageRank scratch API consumes a scratch handle and keeps policy inputs explicit"
)]
pub fn hypergraph_pagerank_weighted_with_scratch<H, RW, IW, IE, IR, S>(
    hypergraph: &H,
    relation_weights: &RW,
    incidence_weights: &IW,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    scratch: HypergraphPageRankScratch<'_, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex
        + IncidenceIndex,
    RW: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    RW::Weight: IntoPageRankScalar<S>,
    IW: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    IW::Weight: IntoPageRankScalar<S>,
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
    iterate_hyper_weighted(
        hypergraph,
        relation_weights,
        incidence_weights,
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

/// Computes weighted hypergraph `PageRank` with reusable owned workspace.
///
/// # Errors
///
/// Returns [`PageRankError`] for invalid configuration, personalization,
/// topology indexes, invalid weights, output length, or non-convergence.
///
/// # Performance
///
/// Grows workspace storage to the visible bounds if needed, then performs no
/// additional heap allocation.
#[cfg(feature = "alloc")]
#[expect(
    clippy::too_many_arguments,
    reason = "weighted hypergraph PageRank workspace entry point keeps all policy and storage inputs explicit"
)]
pub fn hypergraph_pagerank_weighted_with_workspace<H, RW, IW, IE, IR, S>(
    hypergraph: &H,
    relation_weights: &RW,
    incidence_weights: &IW,
    elements: IE,
    relations: IR,
    config: PageRankConfig<S>,
    personalization: Option<&[S]>,
    element_ranks: &mut [S],
    relation_ranks: &mut [S],
    workspace: &mut HypergraphPageRankWorkspace<H, S>,
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex
        + IncidenceIndex,
    RW: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    RW::Weight: IntoPageRankScalar<S>,
    IW: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    IW::Weight: IntoPageRankScalar<S>,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let e_bound = hypergraph.element_bound();
    let r_bound = hypergraph.relation_bound();
    let state_bound =
        checked_state_bound::<S>(e_bound, r_bound, element_ranks.len(), relation_ranks.len())?;
    workspace.ensure_bounds(e_bound, r_bound, state_bound);
    hypergraph_pagerank_weighted_with_scratch(
        hypergraph,
        relation_weights,
        incidence_weights,
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

fn is_visible(visible: &[u8], index: usize) -> bool {
    visible.get(index).copied().unwrap_or(0) != 0
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
    H: ElementIndex + RelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    clear(out, state_bound);
    clear_u8(visible_elements, hypergraph.element_bound());
    clear_u8(visible_relations, hypergraph.relation_bound());
    let mut count = 0_usize;
    let mut sum = S::ZERO;
    if let Some(input) = personalization {
        if input.len() < state_bound {
            return Err(PageRankError::PersonalizationTooShort {
                required: state_bound,
                actual: input.len(),
            });
        }
        fill_hyper_personalization_from_input(
            hypergraph,
            elements,
            relations,
            input,
            out,
            visible_elements,
            visible_relations,
            &mut count,
            &mut sum,
        )?;
    } else {
        fill_hyper_personalization_uniform(
            hypergraph,
            elements,
            relations,
            out,
            visible_elements,
            visible_relations,
            &mut count,
            &mut sum,
        )?;
    }
    normalize_personalization(out, count, sum)
}

#[expect(
    clippy::too_many_arguments,
    reason = "helper threads separate element and relation state without allocation wrappers"
)]
fn fill_hyper_personalization_from_input<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    input: &[S],
    out: &mut [S],
    visible_elements: &mut [u8],
    visible_relations: &mut [u8],
    count: &mut usize,
    sum: &mut S,
) -> Result<(), PageRankError<S>>
where
    H: ElementIndex + RelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let e_bound = hypergraph.element_bound();
    for element in elements {
        let index = hypergraph.element_index(element);
        check_index(index, e_bound)?;
        mark_visible_element(visible_elements, index)?;
        let value = input[index];
        check_personalization_value(index, value)?;
        out[index] = value;
        *sum += value;
        *count += 1;
    }
    for relation in relations {
        let index = hypergraph.relation_index(relation);
        check_relation_index(index, hypergraph.relation_bound())?;
        mark_visible_relation(visible_relations, index)?;
        let state = e_bound + index;
        let value = input[state];
        check_personalization_value(state, value)?;
        out[state] = value;
        *sum += value;
        *count += 1;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "helper threads separate element and relation state without allocation wrappers"
)]
fn fill_hyper_personalization_uniform<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    out: &mut [S],
    visible_elements: &mut [u8],
    visible_relations: &mut [u8],
    count: &mut usize,
    sum: &mut S,
) -> Result<(), PageRankError<S>>
where
    H: ElementIndex + RelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let e_bound = hypergraph.element_bound();
    for element in elements {
        let index = hypergraph.element_index(element);
        check_index(index, e_bound)?;
        mark_visible_element(visible_elements, index)?;
        out[index] = S::ONE;
        *sum += S::ONE;
        *count += 1;
    }
    for relation in relations {
        let index = hypergraph.relation_index(relation);
        check_relation_index(index, hypergraph.relation_bound())?;
        mark_visible_relation(visible_relations, index)?;
        out[e_bound + index] = S::ONE;
        *sum += S::ONE;
        *count += 1;
    }
    Ok(())
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
    G: ElementIndex,
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
    H: ElementIndex + RelationIndex,
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
    reason = "iteration helper keeps scratch and policy inputs explicit"
)]
#[expect(
    clippy::excessive_nesting,
    reason = "power iteration has row, dangling, and edge-distribution branches"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "iteration helpers own cloneable iterator values and clone them each power iteration"
)]
fn iterate_graph_unweighted<G, I, S>(
    graph: &G,
    elements: I,
    config: PageRankConfig<S>,
    teleport: &[S],
    visible: &[u8],
    ranks: &mut [S],
    next: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex,
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
            let mut degree = 0_usize;
            for edge in graph.outgoing_edges(element) {
                let target = graph.target(edge);
                let target_index = checked_element_index(graph, target)?;
                if is_visible(visible, target_index) {
                    degree += 1;
                }
            }
            if degree == 0 {
                dangling += rank;
            } else {
                let share = rank / S::from_usize(degree);
                for edge in graph.outgoing_edges(element) {
                    let target = graph.target(edge);
                    let target_index = checked_element_index(graph, target)?;
                    if is_visible(visible, target_index) {
                        next[target_index] += share;
                    }
                }
            }
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
    reason = "weighted iteration helper keeps weights, scratch, and policy inputs explicit"
)]
#[expect(
    clippy::excessive_nesting,
    reason = "power iteration has row, dangling, and edge-distribution branches"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "iteration helpers own cloneable iterator values and clone them each power iteration"
)]
fn iterate_graph_weighted<G, W, I, S>(
    graph: &G,
    weights: &W,
    elements: I,
    config: PageRankConfig<S>,
    teleport: &[S],
    visible: &[u8],
    ranks: &mut [S],
    next: &mut [S],
) -> Result<PageRankReport<S>, PageRankError<S>>
where
    G: ForwardGraph + ElementIndex + RelationIndex,
    W: RelationWeight<ElementId = G::ElementId, RelationId = G::RelationId>,
    W::Weight: IntoPageRankScalar<S>,
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
            let total = outgoing_weight_total(graph, weights, element, visible)?;
            if total <= S::ZERO {
                dangling += rank;
            } else {
                for edge in graph.outgoing_edges(element) {
                    let target = graph.target(edge);
                    let target_index = checked_element_index(graph, target)?;
                    if is_visible(visible, target_index) {
                        let weight = checked_relation_weight(graph, weights, edge)?;
                        next[target_index] += rank * (weight / total);
                    }
                }
            }
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
    reason = "hypergraph iteration threads element and relation states explicitly"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "iteration helpers own cloneable iterator values and clone them each power iteration"
)]
fn iterate_hyper_unweighted<H, IE, IR, S>(
    hypergraph: &H,
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
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut last_delta = S::INFINITY;
    for iteration in 1..=config.max_iterations {
        clear(next_elements, hypergraph.element_bound());
        clear(next_relations, hypergraph.relation_bound());
        let dangling = push_hyper_unweighted(
            hypergraph,
            elements.clone(),
            relations.clone(),
            visible_elements,
            visible_relations,
            element_ranks,
            relation_ranks,
            next_elements,
            next_relations,
        )?;
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

#[expect(
    clippy::too_many_arguments,
    reason = "weighted hypergraph iteration keeps relation and incidence policies explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "iteration helpers own cloneable iterator values and clone them each power iteration"
)]
fn iterate_hyper_weighted<H, RW, IW, IE, IR, S>(
    hypergraph: &H,
    relation_weights: &RW,
    incidence_weights: &IW,
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
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex
        + IncidenceIndex,
    RW: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    RW::Weight: IntoPageRankScalar<S>,
    IW: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    IW::Weight: IntoPageRankScalar<S>,
    IE: Clone + IntoIterator<Item = ElementId<H>>,
    IR: Clone + IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut last_delta = S::INFINITY;
    for iteration in 1..=config.max_iterations {
        clear(next_elements, hypergraph.element_bound());
        clear(next_relations, hypergraph.relation_bound());
        let dangling = push_hyper_weighted(
            hypergraph,
            relation_weights,
            incidence_weights,
            elements.clone(),
            relations.clone(),
            visible_elements,
            visible_relations,
            element_ranks,
            relation_ranks,
            next_elements,
            next_relations,
        )?;
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

fn checked_element_index<G: ElementIndex, S>(
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
    G: RelationIndex,
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
    H: IncidenceIndex,
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
    G: ForwardGraph + ElementIndex + RelationIndex,
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
    next: &mut [S],
) -> Result<S, PageRankError<S>>
where
    G: ElementIndex,
    I: IntoIterator<Item = ElementId<G>>,
    S: PageRankScalar,
{
    let mut delta = S::ZERO;
    let teleport_scale = (S::ONE - config.damping) + (config.damping * dangling);
    for element in elements {
        let index = checked_element_index(graph, element)?;
        let value = (config.damping * next[index]) + (teleport_scale * teleport[index]);
        delta += (value - ranks[index]).abs();
        ranks[index] = value;
        next[index] = value;
    }
    Ok(delta)
}

#[expect(
    clippy::too_many_arguments,
    reason = "hypergraph push uses separate state and scratch families"
)]
fn push_hyper_unweighted<H, IE, IR, S>(
    hypergraph: &H,
    elements: IE,
    relations: IR,
    visible_elements: &[u8],
    visible_relations: &[u8],
    element_ranks: &[S],
    relation_ranks: &[S],
    next_elements: &mut [S],
    next_relations: &mut [S],
) -> Result<S, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut dangling = S::ZERO;
    for element in elements {
        let index = checked_element_index(hypergraph, element)?;
        let mut degree = 0_usize;
        for relation in hypergraph.outgoing_hyperedges(element) {
            let relation_index = checked_relation_index_for(hypergraph, relation)?;
            if is_visible(visible_relations, relation_index) {
                degree += 1;
            }
        }
        if degree == 0 {
            dangling += element_ranks[index];
            continue;
        }
        let share = element_ranks[index] / S::from_usize(degree);
        for relation in hypergraph.outgoing_hyperedges(element) {
            let relation_index = checked_relation_index_for(hypergraph, relation)?;
            if !is_visible(visible_relations, relation_index) {
                continue;
            }
            next_relations[relation_index] += share;
        }
    }
    for relation in relations {
        let relation_index = checked_relation_index_for(hypergraph, relation)?;
        let mut degree = 0_usize;
        for incidence in hypergraph.target_incidences(relation) {
            let target = hypergraph.incidence_element(incidence);
            let target_index = checked_element_index(hypergraph, target)?;
            if is_visible(visible_elements, target_index) {
                degree += 1;
            }
        }
        if degree == 0 {
            dangling += relation_ranks[relation_index];
            continue;
        }
        let share = relation_ranks[relation_index] / S::from_usize(degree);
        for incidence in hypergraph.target_incidences(relation) {
            let target = hypergraph.incidence_element(incidence);
            let target_index = checked_element_index(hypergraph, target)?;
            if !is_visible(visible_elements, target_index) {
                continue;
            }
            next_elements[target_index] += share;
        }
    }
    Ok(dangling)
}

#[expect(
    clippy::too_many_arguments,
    reason = "weighted hypergraph push keeps weights and state families explicit"
)]
fn push_hyper_weighted<H, RW, IW, IE, IR, S>(
    hypergraph: &H,
    relation_weights: &RW,
    incidence_weights: &IW,
    elements: IE,
    relations: IR,
    visible_elements: &[u8],
    visible_relations: &[u8],
    element_ranks: &[S],
    relation_ranks: &[S],
    next_elements: &mut [S],
    next_relations: &mut [S],
) -> Result<S, PageRankError<S>>
where
    H: DirectedVertexHyperedges
        + DirectedHyperedgeIncidences
        + IncidenceElement
        + ElementIndex
        + RelationIndex
        + IncidenceIndex,
    RW: RelationWeight<ElementId = H::ElementId, RelationId = H::RelationId>,
    RW::Weight: IntoPageRankScalar<S>,
    IW: IncidenceWeight<
            ElementId = H::ElementId,
            RelationId = H::RelationId,
            IncidenceId = H::IncidenceId,
        >,
    IW::Weight: IntoPageRankScalar<S>,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut dangling = S::ZERO;
    for element in elements {
        let index = checked_element_index(hypergraph, element)?;
        let total = hyper_outgoing_relation_weight(
            hypergraph,
            relation_weights,
            element,
            visible_relations,
        )?;
        if total <= S::ZERO {
            dangling += element_ranks[index];
            continue;
        }
        for relation in hypergraph.outgoing_hyperedges(element) {
            let relation_index = checked_relation_index_for(hypergraph, relation)?;
            if !is_visible(visible_relations, relation_index) {
                continue;
            }
            let weight = checked_relation_weight(hypergraph, relation_weights, relation)?;
            next_relations[relation_index] += element_ranks[index] * (weight / total);
        }
    }
    for relation in relations {
        let relation_index = checked_relation_index_for(hypergraph, relation)?;
        let total = hyper_target_incidence_weight(
            hypergraph,
            incidence_weights,
            relation,
            visible_elements,
        )?;
        if total <= S::ZERO {
            dangling += relation_ranks[relation_index];
            continue;
        }
        for incidence in hypergraph.target_incidences(relation) {
            let target = hypergraph.incidence_element(incidence);
            let target_index = checked_element_index(hypergraph, target)?;
            if !is_visible(visible_elements, target_index) {
                continue;
            }
            let weight = checked_incidence_weight(hypergraph, incidence_weights, incidence)?;
            next_elements[target_index] += relation_ranks[relation_index] * (weight / total);
        }
    }
    Ok(dangling)
}

fn checked_relation_index_for<H: RelationIndex, S>(
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
    H: DirectedVertexHyperedges + RelationIndex,
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
    H: DirectedHyperedgeIncidences + IncidenceElement + ElementIndex + IncidenceIndex,
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
    next_elements: &mut [S],
    next_relations: &mut [S],
) -> Result<S, PageRankError<S>>
where
    H: ElementIndex + RelationIndex,
    IE: IntoIterator<Item = ElementId<H>>,
    IR: IntoIterator<Item = RelationId<H>>,
    S: PageRankScalar,
{
    let mut delta = S::ZERO;
    let e_bound = hypergraph.element_bound();
    let teleport_scale = (S::ONE - config.damping) + (config.damping * dangling);
    for element in elements {
        let index = checked_element_index(hypergraph, element)?;
        let value = (config.damping * next_elements[index]) + (teleport_scale * teleport[index]);
        delta += (value - element_ranks[index]).abs();
        element_ranks[index] = value;
        next_elements[index] = value;
    }
    for relation in relations {
        let index = checked_relation_index_for(hypergraph, relation)?;
        let state = e_bound + index;
        let value = (config.damping * next_relations[index]) + (teleport_scale * teleport[state]);
        delta += (value - relation_ranks[index]).abs();
        relation_ranks[index] = value;
        next_relations[index] = value;
    }
    Ok(delta)
}

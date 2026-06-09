//! Append/freeze builders for directed hypergraph topology.
//!
//! Core builders are `no_std + alloc` and do not depend on Arrow, named
//! properties, snapshots, or Python. Snapshot and property export helpers live
//! behind explicit features.
//! kani-skip: builders allocate variable-sized topology and snapshot buffers; proptests cover
//! participant invariants and export roundtrips.

#![cfg_attr(
    not(test),
    expect(
        clippy::missing_docs_in_private_items,
        reason = "hypergraph builder internals are documented at the public type and trait capability boundaries"
    )
)]

use alloc::{boxed::Box, vec, vec::Vec};
use core::{error::Error, fmt, marker::PhantomData};

use oxgraph_hyper::{
    CanonicalElementIdentity, CanonicalIncidenceIdentity, CanonicalRelationIdentity,
    ContainsElement, ContainsIncidence, ContainsRelation, DirectedHyperedgeIncidences,
    DirectedHyperedgeParticipants, DirectedVertexHyperedges, ElementIncidenceCount,
    ElementIncidences, ElementIndex as ElementIndexTrait, ElementPredecessors, ElementSuccessors,
    ElementWeight, HyperedgeParticipants, HypergraphCounts, IncidenceBase, IncidenceCounts,
    IncidenceElement, IncidenceIndex as IncidenceIndexTrait, IncidenceRelation, IncidenceRole,
    IncidenceWeight, IncidentHyperedges, LocalElementIdentity, LocalIncidenceIdentity,
    LocalRelationIdentity, RelationIncidenceCount, RelationIncidences,
    RelationIndex as RelationIndexTrait, RelationWeight, TopologyBase, TopologyCounts,
};
use oxgraph_layout_util::{
    IdOutOfBounds, LayoutIndex, LayoutWord, OffsetOverflow, build_offset_index, id_to_slot,
    index_from_usize, slot_or_max,
};
#[cfg(feature = "build-property-arrow")]
use oxgraph_property::{
    EncodedPropertySnapshot, HyperPropertyLayers, IdFamily, IdentityModeRecord, PropertyError,
    PropertyLayer, PropertySnapshotMetaWord, encode_hyper_property_snapshot, rekey_layer_to_local,
};
use oxgraph_snapshot::{PlanError, SnapshotBuilder};

use crate::{
    BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
    BcsrElementIncidences, BcsrError, BcsrHyperedgeSlice, BcsrHypergraph, BcsrNativeHypergraph,
    BcsrParticipantSlice, BcsrPredecessorVertices, BcsrRole, BcsrSections, BcsrSnapshotIndex,
    BcsrSuccessorVertices, BcsrValidation, BcsrVertexSlice,
};

/// Result returned by unweighted hypergraph freeze operations.
type FrozenHypergraphResult<VertexIndex, RelationIndex, IncidenceIndex> = Result<
    FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>,
    HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
>;

/// Result returned by weighted hypergraph freeze operations.
type FrozenWeightedHypergraphResult<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW> =
    Result<
        FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>,
        HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
    >;

/// Built frozen topology plus incidence weights.
type BuiltTopology<VertexIndex, RelationIndex, IncidenceIndex, IW> = (
    FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    Vec<IW>,
);

/// Result returned by topology construction.
type BuiltTopologyResult<VertexIndex, RelationIndex, IncidenceIndex, IW> = Result<
    BuiltTopology<VertexIndex, RelationIndex, IncidenceIndex, IW>,
    HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
>;

/// Vertex-to-relation offset and relation arrays.
type VertexRelationIndex<RelationIndex, IncidenceIndex> = (Vec<IncidenceIndex>, Vec<RelationIndex>);

/// Result returned by vertex-to-relation index construction.
type VertexRelationIndexResult<VertexIndex, RelationIndex, IncidenceIndex> = Result<
    VertexRelationIndex<RelationIndex, IncidenceIndex>,
    HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
>;

/// Local/canonical vertex ID assigned by hypergraph builders.
///
/// This is the same alias as the borrowed-view
/// [`BcsrVertexId`](crate::BcsrVertexId), so a built hypergraph and its
/// frozen/snapshot view share one vertex-handle type and no build-vs-view ID
/// mismatch can occur.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
pub type HyperVertexId<VertexIndex> = crate::BcsrVertexId<VertexIndex>;

/// Local/canonical hyperedge ID assigned by hypergraph builders.
///
/// This is the same alias as the borrowed-view
/// [`BcsrHyperedgeId`](crate::BcsrHyperedgeId).
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
pub type HyperedgeId<RelationIndex> = crate::BcsrHyperedgeId<RelationIndex>;

/// Local/canonical participant ID assigned when a hypergraph is frozen.
///
/// This is the same alias as the borrowed-view
/// [`BcsrParticipantId`](crate::BcsrParticipantId), so build-path and view-path
/// incidence numbering share one handle type. The build path now assigns
/// participant IDs in the same head-block-then-tail-block order the view uses.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
pub type HyperParticipantId<IncidenceIndex> = crate::BcsrParticipantId<IncidenceIndex>;

/// Role of a participant in a directed hyperedge.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum HyperParticipantRole {
    /// Source-side participant.
    Source,
    /// Target-side participant.
    Target,
}

/// Errors raised by hypergraph building and export operations.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex> {
    /// A vertex ID was not visible in the builder or frozen view.
    InvalidVertex {
        /// Invalid vertex ID.
        vertex: HyperVertexId<VertexIndex>,
    },
    /// A hyperedge ID was not visible in the builder or frozen view.
    InvalidHyperedge {
        /// Invalid hyperedge ID.
        hyperedge: HyperedgeId<RelationIndex>,
    },
    /// A participant ID was not visible in the frozen view.
    InvalidParticipant {
        /// Invalid participant ID.
        participant: HyperParticipantId<IncidenceIndex>,
    },
    /// A participant position was outside a hyperedge participant set.
    ParticipantPositionOutOfBounds {
        /// Hyperedge whose participant was requested.
        hyperedge: HyperedgeId<RelationIndex>,
        /// Hyperedge-local participant position.
        position: usize,
    },
    /// A source or target participant list repeated the same vertex.
    DuplicateParticipant {
        /// Repeated vertex ID.
        vertex: HyperVertexId<VertexIndex>,
        /// Role whose participant set contained the duplicate.
        role: HyperParticipantRole,
    },
    /// A dense ID or offset overflowed the selected builder width.
    IdOverflow {
        /// Value that did not fit.
        value: usize,
    },
    /// Freeze-time validation of the built topology failed.
    ///
    /// `freeze` validates its own output exactly once through the borrowed
    /// view's [`BcsrValidation::Strict`] logic; this variant indicates a
    /// builder bug, not a caller error.
    InvalidTopology {
        /// Underlying section-validation error.
        source: BcsrError,
    },
    /// An attached property layer used an unknown future ID family.
    #[cfg(feature = "build-property-arrow")]
    UnsupportedPropertyFamily {
        /// Unsupported family.
        id_family: IdFamily,
    },
    /// A property layer was too short for the frozen topology.
    #[cfg(feature = "build-property-arrow")]
    PropertyLayerTooShort {
        /// Property layer ID family.
        id_family: IdFamily,
        /// Required logical length.
        required: usize,
        /// Actual logical length.
        actual: usize,
    },
    /// Property snapshot encoding failed.
    #[cfg(feature = "build-property-arrow")]
    Property {
        /// Underlying property-layer error.
        source: PropertyError,
    },
    /// Snapshot export failed.
    SnapshotPlan {
        /// Underlying snapshot planning error.
        source: PlanError,
    },
}

impl<VertexIndex, RelationIndex, IncidenceIndex> fmt::Display
    for HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: fmt::Debug,
    RelationIndex: fmt::Debug,
    IncidenceIndex: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVertex { vertex } => {
                write!(formatter, "invalid hypergraph vertex ID {vertex:?}")
            }
            Self::InvalidHyperedge { hyperedge } => {
                write!(formatter, "invalid hyperedge ID {hyperedge:?}")
            }
            Self::InvalidParticipant { participant } => {
                write!(formatter, "invalid participant ID {participant:?}")
            }
            Self::ParticipantPositionOutOfBounds {
                hyperedge,
                position,
            } => write!(
                formatter,
                "participant position {position} is outside {hyperedge:?}"
            ),
            Self::DuplicateParticipant { vertex, role } => write!(
                formatter,
                "duplicate {role:?} participant vertex {vertex:?} in hyperedge"
            ),
            Self::IdOverflow { value } => {
                write!(formatter, "hypergraph builder ID overflow at {value}")
            }
            Self::InvalidTopology { source } => {
                write!(
                    formatter,
                    "frozen topology failed strict validation: {source}"
                )
            }
            #[cfg(feature = "build-property-arrow")]
            Self::UnsupportedPropertyFamily { id_family } => write!(
                formatter,
                "unsupported hypergraph property family {id_family:?}"
            ),
            #[cfg(feature = "build-property-arrow")]
            Self::PropertyLayerTooShort {
                id_family,
                required,
                actual,
            } => write!(
                formatter,
                "hypergraph property layer for {id_family:?} too short: required {required}, got {actual}"
            ),
            #[cfg(feature = "build-property-arrow")]
            Self::Property { source } => {
                write!(formatter, "property snapshot export failed: {source}")
            }
            Self::SnapshotPlan { source } => write!(formatter, "snapshot export failed: {source}"),
        }
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex> Error
    for HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: fmt::Debug,
    RelationIndex: fmt::Debug,
    IncidenceIndex: fmt::Debug,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "build-property-arrow")]
            Self::Property { source } => Some(source),
            Self::SnapshotPlan { source } => Some(source),
            Self::InvalidTopology { source } => Some(source),
            Self::InvalidVertex { .. }
            | Self::InvalidHyperedge { .. }
            | Self::InvalidParticipant { .. }
            | Self::ParticipantPositionOutOfBounds { .. }
            | Self::DuplicateParticipant { .. }
            | Self::IdOverflow { .. } => None,
            #[cfg(feature = "build-property-arrow")]
            Self::UnsupportedPropertyFamily { .. } | Self::PropertyLayerTooShort { .. } => None,
        }
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex> From<PlanError>
    for HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>
{
    fn from(source: PlanError) -> Self {
        Self::SnapshotPlan { source }
    }
}

#[cfg(feature = "build-property-arrow")]
impl<VertexIndex, RelationIndex, IncidenceIndex> From<PropertyError>
    for HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>
{
    fn from(source: PropertyError) -> Self {
        Self::Property { source }
    }
}

/// Append-only unweighted directed hypergraph builder.
///
/// # Performance
///
/// Adding a vertex is `O(1)`. Adding a hyperedge is
/// `O(s log s + t log t)` for source/target duplicate checks.
#[derive(Clone, Debug)]
#[must_use]
pub struct HypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    vertex_count: usize,
    hyperedges: Vec<HyperedgeRecord<VertexIndex>>,
    index_width: PhantomData<(RelationIndex, IncidenceIndex)>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex>
    HypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    /// Constructs an empty unweighted hypergraph builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new() -> Self {
        Self {
            vertex_count: 0,
            hyperedges: Vec::new(),
            index_width: PhantomData,
        }
    }

    /// Adds one isolated vertex.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::IdOverflow`] if `VertexIndex` cannot
    /// represent the new ID.
    ///
    /// # Performance
    ///
    /// Amortized `O(1)`.
    pub fn add_vertex(
        &mut self,
    ) -> Result<
        HyperVertexId<VertexIndex>,
        HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
    > {
        let id = VertexIndex::from_usize(self.vertex_count).ok_or(HyperBuildError::IdOverflow {
            value: self.vertex_count,
        })?;
        self.vertex_count = self
            .vertex_count
            .checked_add(1)
            .ok_or(HyperBuildError::IdOverflow { value: usize::MAX })?;
        Ok(HyperVertexId::new(id))
    }

    /// Adds one directed hyperedge.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::InvalidVertex`] when a participant is not
    /// visible, [`HyperBuildError::DuplicateParticipant`] when either side
    /// repeats a vertex, or [`HyperBuildError::IdOverflow`] if
    /// `RelationIndex` cannot represent the new ID.
    ///
    /// # Performance
    ///
    /// `O(s log s + t log t)` for `s` sources and `t` targets.
    pub fn add_hyperedge(
        &mut self,
        sources: &[HyperVertexId<VertexIndex>],
        targets: &[HyperVertexId<VertexIndex>],
    ) -> Result<
        HyperedgeId<RelationIndex>,
        HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
    > {
        ensure_vertices(self.vertex_count, sources.iter().copied())?;
        ensure_vertices(self.vertex_count, targets.iter().copied())?;
        ensure_unique_participants(sources.iter().copied(), HyperParticipantRole::Source)?;
        ensure_unique_participants(targets.iter().copied(), HyperParticipantRole::Target)?;
        let hyperedge = RelationIndex::from_usize(self.hyperedges.len()).ok_or(
            HyperBuildError::IdOverflow {
                value: self.hyperedges.len(),
            },
        )?;
        self.hyperedges.push(HyperedgeRecord {
            sources: sources.iter().map(|vertex| vertex.get()).collect(),
            targets: targets.iter().map(|vertex| vertex.get()).collect(),
        });
        Ok(HyperedgeId::new(hyperedge))
    }

    /// Returns the number of vertices assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    /// Returns the number of hyperedges assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn hyperedge_count(&self) -> usize {
        self.hyperedges.len()
    }

    /// Freezes the current builder contents into an owned immutable view.
    ///
    /// The built arrays are validated exactly once at
    /// [`BcsrValidation::Strict`] through the borrowed view's checked logic,
    /// so [`FrozenHypergraph::as_view`] can skip re-validation.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::IdOverflow`] if any offset or dense ID cannot
    /// fit in the selected index widths, or
    /// [`HyperBuildError::InvalidTopology`] if the freeze-time validation pass
    /// rejects the built arrays (a builder bug).
    ///
    /// # Performance
    ///
    /// `O(v + h + p log p)` for vertices `v`, hyperedges `h`, and participants
    /// `p`, including the freeze-time Strict validation walk.
    pub fn freeze(&self) -> FrozenHypergraphResult<VertexIndex, RelationIndex, IncidenceIndex>
    where
        VertexIndex: LayoutWord<Index = VertexIndex>,
        RelationIndex: LayoutWord<Index = RelationIndex>,
        IncidenceIndex: LayoutWord<Index = IncidenceIndex>,
    {
        let normalized = self.normalized_hyperedges();
        let (topology, _weights) = build_topology(self.vertex_count, &normalized)?;
        validate_frozen_topology(&topology)?;
        Ok(FrozenHypergraph { topology })
    }

    fn normalized_hyperedges(&self) -> Vec<NormalizedHyperedgeRecord<VertexIndex, ()>> {
        self.hyperedges
            .iter()
            .map(|record| {
                let mut sources: Vec<(VertexIndex, ())> = record
                    .sources
                    .iter()
                    .copied()
                    .map(|vertex| (vertex, ()))
                    .collect();
                let mut targets: Vec<(VertexIndex, ())> = record
                    .targets
                    .iter()
                    .copied()
                    .map(|vertex| (vertex, ()))
                    .collect();
                sources.sort_by_key(|(vertex, _weight)| *vertex);
                targets.sort_by_key(|(vertex, _weight)| *vertex);
                NormalizedHyperedgeRecord { sources, targets }
            })
            .collect()
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex> Default
    for HypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Append-only weighted directed hypergraph builder.
///
/// # Performance
///
/// Adding a vertex is `O(1)`. Adding a hyperedge is
/// `O(s log s + t log t)` for source/target duplicate checks.
#[derive(Clone, Debug)]
#[must_use]
pub struct WeightedHypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    vertex_count: usize,
    element_weights: Vec<EW>,
    hyperedges: Vec<NormalizedHyperedgeRecord<VertexIndex, IW>>,
    relation_weights: Vec<RW>,
    index_width: PhantomData<(RelationIndex, IncidenceIndex)>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
    WeightedHypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    /// Constructs an empty weighted hypergraph builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new() -> Self {
        Self {
            vertex_count: 0,
            element_weights: Vec::new(),
            hyperedges: Vec::new(),
            relation_weights: Vec::new(),
            index_width: PhantomData,
        }
    }

    /// Adds one isolated vertex with an explicit element weight.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::IdOverflow`] if `VertexIndex` cannot
    /// represent the new ID.
    ///
    /// # Performance
    ///
    /// Amortized `O(1)`.
    pub fn add_vertex(
        &mut self,
        weight: EW,
    ) -> Result<
        HyperVertexId<VertexIndex>,
        HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
    > {
        let id = VertexIndex::from_usize(self.vertex_count).ok_or(HyperBuildError::IdOverflow {
            value: self.vertex_count,
        })?;
        self.vertex_count = self
            .vertex_count
            .checked_add(1)
            .ok_or(HyperBuildError::IdOverflow { value: usize::MAX })?;
        self.element_weights.push(weight);
        Ok(HyperVertexId::new(id))
    }

    /// Adds one directed hyperedge with explicit relation and incidence weights.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::InvalidVertex`] when a participant is not
    /// visible, [`HyperBuildError::DuplicateParticipant`] when either side
    /// repeats a vertex, or [`HyperBuildError::IdOverflow`] if
    /// `RelationIndex` cannot represent the new ID.
    ///
    /// # Performance
    ///
    /// `O(s log s + t log t)` for `s` sources and `t` targets.
    pub fn add_hyperedge(
        &mut self,
        sources: &[(HyperVertexId<VertexIndex>, IW)],
        targets: &[(HyperVertexId<VertexIndex>, IW)],
        relation_weight: RW,
    ) -> Result<
        HyperedgeId<RelationIndex>,
        HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
    >
    where
        IW: Clone,
    {
        ensure_vertices(self.vertex_count, sources.iter().map(|(v, _)| *v))?;
        ensure_vertices(self.vertex_count, targets.iter().map(|(v, _)| *v))?;
        ensure_unique_participants(
            sources.iter().map(|(v, _)| *v),
            HyperParticipantRole::Source,
        )?;
        ensure_unique_participants(
            targets.iter().map(|(v, _)| *v),
            HyperParticipantRole::Target,
        )?;
        let hyperedge = RelationIndex::from_usize(self.hyperedges.len()).ok_or(
            HyperBuildError::IdOverflow {
                value: self.hyperedges.len(),
            },
        )?;
        self.hyperedges.push(NormalizedHyperedgeRecord {
            sources: sources
                .iter()
                .map(|(vertex, weight)| (vertex.get(), weight.clone()))
                .collect(),
            targets: targets
                .iter()
                .map(|(vertex, weight)| (vertex.get(), weight.clone()))
                .collect(),
        });
        self.relation_weights.push(relation_weight);
        Ok(HyperedgeId::new(hyperedge))
    }

    /// Updates a vertex element weight.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::InvalidVertex`] when `vertex` is not visible.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn set_element_weight(
        &mut self,
        vertex: HyperVertexId<VertexIndex>,
        weight: EW,
    ) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>> {
        let index = vertex_index_checked(self.vertex_count, vertex)?;
        self.element_weights[index] = weight;
        Ok(())
    }

    /// Updates a hyperedge relation weight.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::InvalidHyperedge`] when `hyperedge` is not
    /// visible.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn set_relation_weight(
        &mut self,
        hyperedge: HyperedgeId<RelationIndex>,
        weight: RW,
    ) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>> {
        let index = hyperedge_index_checked(self.hyperedges.len(), hyperedge)?;
        self.relation_weights[index] = weight;
        Ok(())
    }

    /// Updates one source-side incidence weight by hyperedge-local position.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::InvalidHyperedge`] when `hyperedge` is not
    /// visible or [`HyperBuildError::ParticipantPositionOutOfBounds`] when the
    /// position does not exist on the source side.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn set_source_incidence_weight(
        &mut self,
        hyperedge: HyperedgeId<RelationIndex>,
        position: usize,
        weight: IW,
    ) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>> {
        let index = hyperedge_index_checked(self.hyperedges.len(), hyperedge)?;
        let Some((_vertex, slot)) = self.hyperedges[index].sources.get_mut(position) else {
            return Err(HyperBuildError::ParticipantPositionOutOfBounds {
                hyperedge,
                position,
            });
        };
        *slot = weight;
        Ok(())
    }

    /// Updates one target-side incidence weight by hyperedge-local position.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::InvalidHyperedge`] when `hyperedge` is not
    /// visible or [`HyperBuildError::ParticipantPositionOutOfBounds`] when the
    /// position does not exist on the target side.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn set_target_incidence_weight(
        &mut self,
        hyperedge: HyperedgeId<RelationIndex>,
        position: usize,
        weight: IW,
    ) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>> {
        let index = hyperedge_index_checked(self.hyperedges.len(), hyperedge)?;
        let Some((_vertex, slot)) = self.hyperedges[index].targets.get_mut(position) else {
            return Err(HyperBuildError::ParticipantPositionOutOfBounds {
                hyperedge,
                position,
            });
        };
        *slot = weight;
        Ok(())
    }

    /// Returns the number of vertices assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    /// Returns the number of hyperedges assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn hyperedge_count(&self) -> usize {
        self.hyperedges.len()
    }

    /// Freezes the current builder contents into an owned immutable view.
    ///
    /// The built arrays are validated exactly once at
    /// [`BcsrValidation::Strict`] through the borrowed view's checked logic,
    /// so [`FrozenWeightedHypergraph::as_view`] can skip re-validation.
    ///
    /// # Errors
    ///
    /// Returns [`HyperBuildError::IdOverflow`] if any offset or dense ID cannot
    /// fit in the selected index widths, or
    /// [`HyperBuildError::InvalidTopology`] if the freeze-time validation pass
    /// rejects the built arrays (a builder bug).
    ///
    /// # Performance
    ///
    /// `O(v + h + p log p)` for vertices `v`, hyperedges `h`, and participants
    /// `p`, including the freeze-time Strict validation walk.
    pub fn freeze(
        &self,
    ) -> FrozenWeightedHypergraphResult<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
    where
        VertexIndex: LayoutWord<Index = VertexIndex>,
        RelationIndex: LayoutWord<Index = RelationIndex>,
        IncidenceIndex: LayoutWord<Index = IncidenceIndex>,
        EW: Clone,
        RW: Clone,
        IW: Clone,
    {
        let normalized = self.normalized_hyperedges();
        let (topology, incidence_weights) = build_topology(self.vertex_count, &normalized)?;
        validate_frozen_topology(&topology)?;
        Ok(FrozenWeightedHypergraph {
            topology,
            element_weights: self.element_weights.clone().into_boxed_slice(),
            relation_weights: self.relation_weights.clone().into_boxed_slice(),
            incidence_weights: incidence_weights.into_boxed_slice(),
        })
    }

    fn normalized_hyperedges(&self) -> Vec<NormalizedHyperedgeRecord<VertexIndex, IW>>
    where
        IW: Clone,
    {
        self.hyperedges
            .iter()
            .map(|record| {
                let mut sources = record.sources.clone();
                let mut targets = record.targets.clone();
                sources.sort_by_key(|(vertex, _weight)| *vertex);
                targets.sort_by_key(|(vertex, _weight)| *vertex);
                NormalizedHyperedgeRecord { sources, targets }
            })
            .collect()
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW> Default
    for WeightedHypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Owned immutable unweighted directed hypergraph topology.
///
/// Incidence and adjacency queries delegate to the borrowed
/// [`BcsrNativeHypergraph`] view over the eight canonical arrays (see
/// [`Self::as_view`]), so build path and view path share one traversal
/// implementation and one `IncidenceId` numbering.
///
/// # Performance
///
/// Count, containment, index, and endpoint methods are `O(1)`, except
/// incidence-to-hyperedge resolution
/// ([`IncidenceRelation::incidence_relation`]), which is `O(log h)` for `h`
/// hyperedges. Adjacency iteration is `O(degree)`; per-vertex incidence
/// iteration ([`ElementIncidences::element_incidences`]) is
/// `O(degree · log d_h)` for maximum hyperedge participant-set size `d_h`.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    topology: FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex>
    FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    /// Borrows this frozen topology as a zero-copy [`BcsrNativeHypergraph`].
    ///
    /// The build path and the borrowed view share all three ID spaces: vertex
    /// and hyperedge IDs are dense insertion-order in both, and participant
    /// (incidence) IDs are assigned in the view's head-block-then-tail-block
    /// order at freeze time. The view reports incidence roles as
    /// [`BcsrRole::Head`] / [`BcsrRole::Tail`], corresponding to this graph's
    /// [`HyperParticipantRole::Source`] / [`HyperParticipantRole::Target`].
    /// Construction skips re-validation because `freeze` already validated
    /// these arrays once at [`BcsrValidation::Strict`].
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn as_view(&self) -> BcsrNativeHypergraph<'_, VertexIndex, RelationIndex, IncidenceIndex>
    where
        VertexIndex: LayoutWord<Index = VertexIndex>,
        RelationIndex: LayoutWord<Index = RelationIndex>,
        IncidenceIndex: LayoutWord<Index = IncidenceIndex>,
    {
        self.topology.as_view()
    }
}

/// Owned immutable weighted directed hypergraph topology.
///
/// Topology queries share [`FrozenHypergraph`]'s delegation to the borrowed
/// [`BcsrNativeHypergraph`] view (see [`Self::as_view`]) and its perf
/// contract, including the `O(log h)` incidence-to-hyperedge resolution.
///
/// # Performance
///
/// Weight lookups are `O(1)`.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    topology: FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    element_weights: Box<[EW]>,
    relation_weights: Box<[RW]>,
    incidence_weights: Box<[IW]>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
    FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    /// Borrows this frozen topology as a zero-copy [`BcsrNativeHypergraph`].
    ///
    /// The build path and the borrowed view share all three ID spaces: vertex
    /// and hyperedge IDs are dense insertion-order in both, and participant
    /// (incidence) IDs are assigned in the view's head-block-then-tail-block
    /// order at freeze time. The view reports incidence roles as
    /// [`BcsrRole::Head`] / [`BcsrRole::Tail`], corresponding to this graph's
    /// [`HyperParticipantRole::Source`] / [`HyperParticipantRole::Target`].
    /// Construction skips re-validation because `freeze` already validated
    /// these arrays once at [`BcsrValidation::Strict`]. Weights are not part
    /// of the view; they stay on this owned wrapper.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn as_view(&self) -> BcsrNativeHypergraph<'_, VertexIndex, RelationIndex, IncidenceIndex>
    where
        VertexIndex: LayoutWord<Index = VertexIndex>,
        RelationIndex: LayoutWord<Index = RelationIndex>,
        IncidenceIndex: LayoutWord<Index = IncidenceIndex>,
    {
        self.topology.as_view()
    }
}

/// Owned canonical bipartite-CSR arrays produced by `freeze`.
///
/// Holds exactly the eight arrays the borrowed view consumes (plus the vertex
/// count); every incidence- or adjacency-keyed query is answered by
/// delegating to a [`BcsrNativeHypergraph`] over these arrays via
/// [`Self::as_view`], so no derived index arrays are materialized.
#[derive(Clone, Debug)]
struct FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    vertex_count: usize,
    head_offsets: Box<[IncidenceIndex]>,
    head_participants: Box<[VertexIndex]>,
    tail_offsets: Box<[IncidenceIndex]>,
    tail_participants: Box<[VertexIndex]>,
    vertex_outgoing_offsets: Box<[IncidenceIndex]>,
    vertex_outgoing_hyperedges: Box<[RelationIndex]>,
    vertex_incoming_offsets: Box<[IncidenceIndex]>,
    vertex_incoming_hyperedges: Box<[RelationIndex]>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex>
    FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    fn relation_count(&self) -> usize {
        self.head_offsets.len().saturating_sub(1)
    }

    /// Returns the total incidence (participant) count.
    ///
    /// Head and tail participants are separately allocated slices, so the sum
    /// cannot overflow `usize`.
    fn total_participants(&self) -> usize {
        self.head_participants
            .len()
            .saturating_add(self.tail_participants.len())
    }

    /// Borrows the eight canonical arrays as view sections.
    fn sections(&self) -> BcsrSections<'_, IncidenceIndex, VertexIndex, RelationIndex>
    where
        VertexIndex: LayoutWord<Index = VertexIndex>,
        RelationIndex: LayoutWord<Index = RelationIndex>,
        IncidenceIndex: LayoutWord<Index = IncidenceIndex>,
    {
        BcsrSections {
            head_offsets: &self.head_offsets,
            head_participants: &self.head_participants,
            tail_offsets: &self.tail_offsets,
            tail_participants: &self.tail_participants,
            vertex_outgoing_offsets: &self.vertex_outgoing_offsets,
            vertex_outgoing_hyperedges: &self.vertex_outgoing_hyperedges,
            vertex_incoming_offsets: &self.vertex_incoming_offsets,
            vertex_incoming_hyperedges: &self.vertex_incoming_hyperedges,
        }
    }

    /// Borrows these arrays as a validated-input native view.
    ///
    /// `freeze` already validated the arrays once at
    /// [`BcsrValidation::Strict`], so this skips re-validation.
    fn as_view(&self) -> BcsrNativeHypergraph<'_, VertexIndex, RelationIndex, IncidenceIndex>
    where
        VertexIndex: LayoutWord<Index = VertexIndex>,
        RelationIndex: LayoutWord<Index = RelationIndex>,
        IncidenceIndex: LayoutWord<Index = IncidenceIndex>,
    {
        BcsrHypergraph::from_validated_sections(self.sections())
    }
}

/// Validates a freshly built topology once at [`BcsrValidation::Strict`].
///
/// Builder output is valid by construction; this single freeze-time pass runs
/// the borrowed view's checked validation logic over the eight canonical
/// arrays as defense in depth, replacing hand-maintained build assertions.
/// The frozen wrappers' `as_view` borrows then skip re-validation.
///
/// # Errors
///
/// Returns [`HyperBuildError::InvalidTopology`] when any Strict invariant
/// fails, which would indicate a builder bug.
///
/// # Performance
///
/// `O(p · log d)` over `p` participants with maximum vertex degree `d`, plus
/// a linear layout walk.
fn validate_frozen_topology<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
    RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
    IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
{
    BcsrNativeHypergraph::<VertexIndex, RelationIndex, IncidenceIndex>::open_with(
        topology.sections(),
        BcsrValidation::Strict,
    )
    .map(|_view| ())
    .map_err(|source| HyperBuildError::InvalidTopology { source })
}

fn vertex_slot<VertexIndex: LayoutIndex>(vertex: HyperVertexId<VertexIndex>) -> usize {
    slot_or_max::<VertexIndex>(vertex.get())
}

fn hyperedge_slot<RelationIndex: LayoutIndex>(hyperedge: HyperedgeId<RelationIndex>) -> usize {
    slot_or_max::<RelationIndex>(hyperedge.get())
}

fn participant_slot<IncidenceIndex: LayoutIndex>(
    participant: HyperParticipantId<IncidenceIndex>,
) -> usize {
    slot_or_max::<IncidenceIndex>(participant.get())
}

/// Maps an [`OffsetOverflow`] from `oxgraph_layout_util` into a typed
/// [`HyperBuildError`].
const fn map_offset_overflow<VertexIndex, RelationIndex, IncidenceIndex>(
    error: OffsetOverflow,
) -> HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex> {
    match error {
        OffsetOverflow::IndexOverflow { value } => HyperBuildError::IdOverflow { value },
        _ => HyperBuildError::IdOverflow { value: usize::MAX },
    }
}

/// Implements hypergraph topology traits for one frozen wrapper.
macro_rules! impl_topology_for {
    ($name:ident, [$($extra:ident),*], $topology:ident) => {
        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> TopologyBase
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type ElementId = HyperVertexId<VertexIndex>;
            type RelationId = HyperedgeId<RelationIndex>;
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceBase
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type IncidenceId = HyperParticipantId<IncidenceIndex>;
            type Role = HyperParticipantRole;
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> TopologyCounts
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn element_count(&self) -> usize {
                self.$topology.vertex_count
            }

            fn relation_count(&self) -> usize {
                self.$topology.relation_count()
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> HypergraphCounts
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceCounts
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn incidence_count(&self) -> usize {
                self.$topology.total_participants()
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementIndexTrait
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn element_bound(&self) -> usize {
                self.$topology.vertex_count
            }

            fn element_index(&self, element: HyperVertexId<VertexIndex>) -> usize {
                element.get().to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> RelationIndexTrait
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn relation_bound(&self) -> usize {
                self.$topology.relation_count()
            }

            fn relation_index(&self, relation: HyperedgeId<RelationIndex>) -> usize {
                relation.get().to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceIndexTrait
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn incidence_bound(&self) -> usize {
                self.$topology.total_participants()
            }

            fn incidence_index(&self, incidence: HyperParticipantId<IncidenceIndex>) -> usize {
                incidence.get().to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ContainsElement
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn contains_element(&self, element: HyperVertexId<VertexIndex>) -> bool {
                element
                    .get()
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.vertex_count)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ContainsRelation
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn contains_relation(&self, relation: HyperedgeId<RelationIndex>) -> bool {
                relation
                    .get()
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.relation_count())
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ContainsIncidence
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn contains_incidence(&self, incidence: HyperParticipantId<IncidenceIndex>) -> bool {
                incidence
                    .get()
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.total_participants())
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceElement
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn incidence_element(
                &self,
                incidence: HyperParticipantId<IncidenceIndex>,
            ) -> HyperVertexId<VertexIndex> {
                self.$topology.as_view().incidence_element(incidence)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceRelation
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            // Perf: `O(log h)` — the view resolves the owning hyperedge by
            // binary search over the offset arrays instead of the deleted
            // `participant_relations` lookup table.
            fn incidence_relation(
                &self,
                incidence: HyperParticipantId<IncidenceIndex>,
            ) -> HyperedgeId<RelationIndex> {
                self.$topology.as_view().incidence_relation(incidence)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceRole
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn incidence_role(&self, incidence: HyperParticipantId<IncidenceIndex>) -> HyperParticipantRole {
                match self.$topology.as_view().incidence_role(incidence) {
                    BcsrRole::Head => HyperParticipantRole::Source,
                    BcsrRole::Tail => HyperParticipantRole::Target,
                }
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> RelationIncidences
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type Incidences<'view>
                = BcsrChainedRelationIncidences<IncidenceIndex>
            where
                Self: 'view;

            fn relation_incidences(&self, relation: HyperedgeId<RelationIndex>) -> Self::Incidences<'_> {
                self.$topology.as_view().relation_incidences(relation)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementIncidences
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type Incidences<'view>
                = BcsrElementIncidences<'view, IncidenceIndex, VertexIndex, RelationIndex>
            where
                Self: 'view;

            // Perf: advancing is `O(log d_h)` per item — the view locates the
            // vertex inside each incident hyperedge's participant bucket
            // instead of reading the deleted per-vertex incidence-ID table.
            fn element_incidences(&self, element: HyperVertexId<VertexIndex>) -> Self::Incidences<'_> {
                self.$topology.as_view().detached_element_incidences(element)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> RelationIncidenceCount
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn relation_incidence_count(&self, relation: HyperedgeId<RelationIndex>) -> usize {
                self.$topology.as_view().relation_incidence_count(relation)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementIncidenceCount
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn element_incidence_count(&self, element: HyperVertexId<VertexIndex>) -> usize {
                self.$topology.as_view().element_incidence_count(element)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> HyperedgeParticipants
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type Participants<'view>
                = BcsrChainedParticipants<'view, VertexIndex>
            where
                Self: 'view;

            fn hyperedge_participants(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::Participants<'_> {
                self.$topology.as_view().detached_hyperedge_participants(hyperedge)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidentHyperedges
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type IncidentHyperedges<'view>
                = BcsrChainedHyperedges<'view, RelationIndex>
            where
                Self: 'view;

            fn incident_hyperedges(&self, vertex: HyperVertexId<VertexIndex>) -> Self::IncidentHyperedges<'_> {
                self.$topology.as_view().detached_incident_hyperedges(vertex)
            }
        }

        // `HyperedgeParticipantCount` / `IncidentHyperedgeCount` come for free
        // from the blanket impls in `oxgraph-hyper` over the
        // `RelationIncidenceCount` / `ElementIncidenceCount` impls above.

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> DirectedHyperedgeParticipants
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type SourceParticipants<'view>
                = BcsrVertexSlice<'view, VertexIndex>
            where
                Self: 'view;
            type TargetParticipants<'view>
                = BcsrVertexSlice<'view, VertexIndex>
            where
                Self: 'view;

            fn source_participants(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::SourceParticipants<'_> {
                self.$topology.as_view().detached_source_participants(hyperedge)
            }

            fn target_participants(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::TargetParticipants<'_> {
                self.$topology.as_view().detached_target_participants(hyperedge)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> DirectedHyperedgeIncidences
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type SourceIncidences<'view>
                = BcsrParticipantSlice<IncidenceIndex>
            where
                Self: 'view;
            type TargetIncidences<'view>
                = BcsrParticipantSlice<IncidenceIndex>
            where
                Self: 'view;

            fn source_incidences(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::SourceIncidences<'_> {
                self.$topology.as_view().source_incidences(hyperedge)
            }

            fn target_incidences(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::TargetIncidences<'_> {
                self.$topology.as_view().target_incidences(hyperedge)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> DirectedVertexHyperedges
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type OutgoingHyperedges<'view>
                = BcsrHyperedgeSlice<'view, RelationIndex>
            where
                Self: 'view;
            type IncomingHyperedges<'view>
                = BcsrHyperedgeSlice<'view, RelationIndex>
            where
                Self: 'view;

            fn outgoing_hyperedges(&self, vertex: HyperVertexId<VertexIndex>) -> Self::OutgoingHyperedges<'_> {
                self.$topology.as_view().detached_outgoing_hyperedges(vertex)
            }

            fn incoming_hyperedges(&self, vertex: HyperVertexId<VertexIndex>) -> Self::IncomingHyperedges<'_> {
                self.$topology.as_view().detached_incoming_hyperedges(vertex)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementSuccessors
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type Successors<'view>
                = BcsrSuccessorVertices<'view, RelationIndex, IncidenceIndex, VertexIndex>
            where
                Self: 'view;

            fn element_successors(&self, element: HyperVertexId<VertexIndex>) -> Self::Successors<'_> {
                self.$topology.as_view().detached_element_successors(element)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementPredecessors
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type Predecessors<'view>
                = BcsrPredecessorVertices<'view, RelationIndex, IncidenceIndex, VertexIndex>
            where
                Self: 'view;

            fn element_predecessors(&self, element: HyperVertexId<VertexIndex>) -> Self::Predecessors<'_> {
                self.$topology.as_view().detached_element_predecessors(element)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> CanonicalElementIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type CanonicalElementId = HyperVertexId<VertexIndex>;

            fn canonical_element_id(&self, element: HyperVertexId<VertexIndex>) -> Self::CanonicalElementId {
                element
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> LocalElementIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId> {
                self.contains_element(canonical).then_some(canonical)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> CanonicalRelationIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type CanonicalRelationId = HyperedgeId<RelationIndex>;

            fn canonical_relation_id(&self, relation: HyperedgeId<RelationIndex>) -> Self::CanonicalRelationId {
                relation
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> LocalRelationIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId> {
                self.contains_relation(canonical).then_some(canonical)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> CanonicalIncidenceIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            type CanonicalIncidenceId = HyperParticipantId<IncidenceIndex>;

            fn canonical_incidence_id(
                &self,
                incidence: HyperParticipantId<IncidenceIndex>,
            ) -> Self::CanonicalIncidenceId {
                incidence
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> LocalIncidenceIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
            RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
            IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
        {
            fn local_incidence_id(
                &self,
                canonical: Self::CanonicalIncidenceId,
            ) -> Option<Self::IncidenceId> {
                self.contains_incidence(canonical).then_some(canonical)
            }
        }
    };
}

impl_topology_for!(FrozenHypergraph, [], topology);
impl_topology_for!(FrozenWeightedHypergraph, [EW, RW, IW], topology);

impl<VertexIndex, RelationIndex, IncidenceIndex, EW: Copy, RW, IW> ElementWeight
    for FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
    RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
    IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
{
    type Weight = EW;

    fn element_weight(&self, element: HyperVertexId<VertexIndex>) -> Self::Weight {
        self.element_weights[vertex_slot(element)]
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW: Copy, IW> RelationWeight
    for FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
    RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
    IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
{
    type Weight = RW;

    fn relation_weight(&self, relation: HyperedgeId<RelationIndex>) -> Self::Weight {
        self.relation_weights[hyperedge_slot(relation)]
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW: Copy> IncidenceWeight
    for FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
    RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
    IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
{
    type Weight = IW;

    fn incidence_weight(&self, incidence: HyperParticipantId<IncidenceIndex>) -> Self::Weight {
        self.incidence_weights[participant_slot(incidence)]
    }
}

/// Converts an identity-map payload slice into explicit little-endian words.
///
/// # Performance
///
/// This function is `O(values.len())`.
#[cfg(feature = "build-property-arrow")]
pub fn identity_slice_to_le<I>(
    values: &[I],
) -> Vec<<I as oxgraph_property::PropertyIndex>::LittleEndianWord>
where
    I: PropertySnapshotMetaWord,
{
    values
        .iter()
        .copied()
        .map(oxgraph_property::PropertyIndex::to_le_word)
        .collect()
}

/// Exports a frozen hypergraph topology as a BCSR snapshot.
///
/// # Errors
///
/// Returns [`HyperBuildError`] if snapshot planning fails.
///
/// # Performance
///
/// This function is `O(v + h + p)`.
pub fn export_bcsr_snapshot<VertexIndex, RelationIndex, IncidenceIndex>(
    graph: &FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex,
{
    export_topology_snapshot(&graph.topology)
}

/// Exports a frozen weighted hypergraph topology as a BCSR snapshot.
///
/// Weights are generic Rust values and are not serialized by the topology-only
/// snapshot feature; attach Arrow property layers through `property-arrow` when
/// weights need a wire representation.
///
/// # Errors
///
/// Returns [`HyperBuildError`] if snapshot planning fails.
///
/// # Performance
///
/// This function is `O(v + h + p)`.
pub fn export_weighted_bcsr_snapshot<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>(
    graph: &FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex,
{
    export_topology_snapshot(&graph.topology)
}

/// Exports a frozen hypergraph with external property layers.
///
/// # Errors
///
/// Returns [`HyperBuildError`] if property validation, rekeying, encoding, or
/// snapshot planning fails.
///
/// # Performance
///
/// This function is `O(v + h + p + property bytes)`.
#[cfg(feature = "build-property-arrow")]
pub fn export_bcsr_snapshot_with_properties<VertexIndex, RelationIndex, IncidenceIndex, Id>(
    graph: &FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>,
    layers: HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
    Id: Clone + Copy + Into<u64> + Ord + TryInto<IncidenceIndex>,
{
    let property = encode_hyper_properties(&graph.topology, layers)?;
    export_topology_property_snapshot(&graph.topology, property)
}

/// Exports a frozen weighted hypergraph with external property layers.
///
/// # Errors
///
/// Returns [`HyperBuildError`] if property validation, rekeying, encoding, or
/// snapshot planning fails.
///
/// # Performance
///
/// This function is `O(v + h + p + property bytes)`.
#[cfg(feature = "build-property-arrow")]
pub fn export_weighted_bcsr_snapshot_with_properties<
    VertexIndex,
    RelationIndex,
    IncidenceIndex,
    EW,
    RW,
    IW,
    Id,
>(
    graph: &FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>,
    layers: HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
    Id: Clone + Copy + Into<u64> + Ord + TryInto<IncidenceIndex>,
{
    let property = encode_hyper_properties(&graph.topology, layers)?;
    export_topology_property_snapshot(&graph.topology, property)
}

#[derive(Clone, Debug, Default)]
struct HyperedgeRecord<VertexIndex> {
    sources: Vec<VertexIndex>,
    targets: Vec<VertexIndex>,
}

#[derive(Clone, Debug, Default)]
struct NormalizedHyperedgeRecord<VertexIndex, IW> {
    sources: Vec<(VertexIndex, IW)>,
    targets: Vec<(VertexIndex, IW)>,
}

fn build_topology<VertexIndex, RelationIndex, IncidenceIndex, IW>(
    vertex_count: usize,
    records: &[NormalizedHyperedgeRecord<VertexIndex, IW>],
) -> BuiltTopologyResult<VertexIndex, RelationIndex, IncidenceIndex, IW>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
    IW: Clone,
{
    let participant_count = records
        .iter()
        .map(|record| record.sources.len() + record.targets.len())
        .sum();
    let mut head_offsets = Vec::with_capacity(records.len() + 1);
    let mut tail_offsets = Vec::with_capacity(records.len() + 1);
    let mut head_participants = Vec::new();
    let mut tail_participants = Vec::new();
    let mut incidence_weights = Vec::with_capacity(participant_count);

    // Participant IDs are assigned in head-block-then-tail-block order so that
    // the build path's `IncidenceId` numbering matches the borrowed view: the
    // view numbers all head incidences `[0, P_head)` (indexing `head_participants`)
    // before all tail incidences `[P_head, P_head + P_tail)`. The first pass
    // emits every hyperedge's source (head) incidences in hyperedge order; the
    // second pass emits every hyperedge's target (tail) incidences. The
    // per-hyperedge head/tail offset arrays are derived in the same pass and so
    // remain canonical (`head_offsets[h]` is the head-block start of hyperedge
    // `h`, and head incidence IDs equal that head-block position directly).
    // Incidence weights follow the same numbering, so `incidence_weights[i]`
    // belongs to incidence ID `i`.
    head_offsets.push(index_from_usize(0).map_err(map_offset_overflow)?);
    tail_offsets.push(index_from_usize(0).map_err(map_offset_overflow)?);
    // Head block: all source incidences, hyperedge-major.
    for record in records {
        for (vertex, weight) in &record.sources {
            head_participants.push(*vertex);
            incidence_weights.push(weight.clone());
        }
        head_offsets.push(index_from_usize(head_participants.len()).map_err(map_offset_overflow)?);
    }
    // Tail block: all target incidences, hyperedge-major, after the head block.
    for record in records {
        for (vertex, weight) in &record.targets {
            tail_participants.push(*vertex);
            incidence_weights.push(weight.clone());
        }
        tail_offsets.push(index_from_usize(tail_participants.len()).map_err(map_offset_overflow)?);
    }

    let (vertex_outgoing_offsets, vertex_outgoing_hyperedges) =
        build_vertex_relation_index(vertex_count, records, HyperParticipantRole::Source)?;
    let (vertex_incoming_offsets, vertex_incoming_hyperedges) =
        build_vertex_relation_index(vertex_count, records, HyperParticipantRole::Target)?;

    Ok((
        FrozenTopology {
            vertex_count,
            head_offsets: head_offsets.into_boxed_slice(),
            head_participants: head_participants.into_boxed_slice(),
            tail_offsets: tail_offsets.into_boxed_slice(),
            tail_participants: tail_participants.into_boxed_slice(),
            vertex_outgoing_offsets: vertex_outgoing_offsets.into_boxed_slice(),
            vertex_outgoing_hyperedges: vertex_outgoing_hyperedges.into_boxed_slice(),
            vertex_incoming_offsets: vertex_incoming_offsets.into_boxed_slice(),
            vertex_incoming_hyperedges: vertex_incoming_hyperedges.into_boxed_slice(),
        },
        incidence_weights,
    ))
}

fn build_vertex_relation_index<VertexIndex, RelationIndex, IncidenceIndex, IW>(
    vertex_count: usize,
    records: &[NormalizedHyperedgeRecord<VertexIndex, IW>],
    role: HyperParticipantRole,
) -> VertexRelationIndexResult<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    let mut buckets = vec![Vec::<RelationIndex>::new(); vertex_count];
    for (relation, record) in records.iter().enumerate() {
        let relation_id = index_from_usize(relation).map_err(map_offset_overflow)?;
        let participants = match role {
            HyperParticipantRole::Source => &record.sources,
            HyperParticipantRole::Target => &record.targets,
        };
        for (vertex, _weight) in participants {
            let slot = vertex
                .to_usize()
                .ok_or(HyperBuildError::IdOverflow { value: usize::MAX })?;
            buckets[slot].push(relation_id);
        }
    }
    build_offset_index::<IncidenceIndex, RelationIndex>(buckets).map_err(map_offset_overflow)
}

fn ensure_vertices<VertexIndex, RelationIndex, IncidenceIndex, I>(
    vertex_count: usize,
    participants: I,
) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
    I: IntoIterator<Item = HyperVertexId<VertexIndex>>,
{
    for vertex in participants {
        vertex_index_checked(vertex_count, vertex)?;
    }
    Ok(())
}

fn ensure_unique_participants<VertexIndex, RelationIndex, IncidenceIndex, I>(
    participants: I,
    role: HyperParticipantRole,
) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
    I: IntoIterator<Item = HyperVertexId<VertexIndex>>,
{
    let mut sorted: Vec<VertexIndex> = participants.into_iter().map(HyperVertexId::get).collect();
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        if pair[0] == pair[1] {
            return Err(HyperBuildError::DuplicateParticipant {
                vertex: HyperVertexId::new(pair[0]),
                role,
            });
        }
    }
    Ok(())
}

fn vertex_index_checked<VertexIndex, RelationIndex, IncidenceIndex>(
    vertex_count: usize,
    vertex: HyperVertexId<VertexIndex>,
) -> Result<usize, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    id_to_slot::<VertexIndex>(vertex.get(), vertex_count)
        .map_err(|_: IdOutOfBounds| HyperBuildError::InvalidVertex { vertex })
}

fn hyperedge_index_checked<VertexIndex, RelationIndex, IncidenceIndex>(
    hyperedge_count: usize,
    hyperedge: HyperedgeId<RelationIndex>,
) -> Result<usize, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    id_to_slot::<RelationIndex>(hyperedge.get(), hyperedge_count)
        .map_err(|_: IdOutOfBounds| HyperBuildError::InvalidHyperedge { hyperedge })
}

fn export_topology_snapshot<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex,
{
    let mut builder = SnapshotBuilder::new();
    add_topology_sections::<VertexIndex, RelationIndex, IncidenceIndex>(&mut builder, topology)?;
    builder.finish().map_err(HyperBuildError::from)
}

/// Appends the eight little-endian bipartite-CSR topology sections to `builder`.
///
/// Each native width slice is lowered to its little-endian storage words by
/// [`SnapshotBuilder::add_section_widths`].
fn add_topology_sections<VertexIndex, RelationIndex, IncidenceIndex>(
    builder: &mut SnapshotBuilder,
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex,
{
    builder.add_section_widths(
        IncidenceIndex::HEAD_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
        &topology.head_offsets,
    )?;
    builder.add_section_widths(
        VertexIndex::HEAD_PARTICIPANTS_KIND,
        VertexIndex::SECTION_VERSION,
        &topology.head_participants,
    )?;
    builder.add_section_widths(
        IncidenceIndex::TAIL_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
        &topology.tail_offsets,
    )?;
    builder.add_section_widths(
        VertexIndex::TAIL_PARTICIPANTS_KIND,
        VertexIndex::SECTION_VERSION,
        &topology.tail_participants,
    )?;
    builder.add_section_widths(
        IncidenceIndex::VERTEX_OUTGOING_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
        &topology.vertex_outgoing_offsets,
    )?;
    builder.add_section_widths(
        RelationIndex::VERTEX_OUTGOING_HYPEREDGES_KIND,
        RelationIndex::SECTION_VERSION,
        &topology.vertex_outgoing_hyperedges,
    )?;
    builder.add_section_widths(
        IncidenceIndex::VERTEX_INCOMING_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
        &topology.vertex_incoming_offsets,
    )?;
    builder.add_section_widths(
        RelationIndex::VERTEX_INCOMING_HYPEREDGES_KIND,
        RelationIndex::SECTION_VERSION,
        &topology.vertex_incoming_hyperedges,
    )?;
    Ok(())
}

#[cfg(feature = "build-property-arrow")]
fn encode_hyper_properties<VertexIndex, RelationIndex, IncidenceIndex, Id>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    layers: HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<EncodedPropertySnapshot, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
    Id: Clone + Copy + Into<u64> + Ord + TryInto<IncidenceIndex>,
{
    validate_property_lengths(topology, layers.element, IdFamily::Element)?;
    validate_property_lengths(topology, layers.relation, IdFamily::Relation)?;
    validate_property_lengths(topology, layers.incidence, IdFamily::Incidence)?;
    let incidence_map = bcsr_local_incidence_to_canonical(topology)?;
    let incidence = layers
        .incidence
        .iter()
        .map(|layer| rekey_layer_to_local(layer, &incidence_map).map_err(HyperBuildError::from))
        .collect::<Result<Vec<PropertyLayer<Id, IncidenceIndex>>, _>>()?;
    Ok(encode_hyper_property_snapshot::<
        IncidenceIndex,
        Id,
        VertexIndex,
        RelationIndex,
        IncidenceIndex,
    >(HyperPropertyLayers {
        element: layers.element,
        relation: layers.relation,
        incidence: &incidence,
    })?)
}

#[cfg(feature = "build-property-arrow")]
fn validate_property_lengths<Id, I, VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    layers: &[PropertyLayer<Id, I>],
    id_family: IdFamily,
) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    I: oxgraph_property::PropertyIndex,
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    let required = match id_family {
        IdFamily::Element => topology.vertex_count,
        IdFamily::Relation => topology.relation_count(),
        IdFamily::Incidence => topology.total_participants(),
        unsupported => {
            return Err(HyperBuildError::UnsupportedPropertyFamily {
                id_family: unsupported,
            });
        }
    };
    oxgraph_property::export::validate_layer_lengths(layers, id_family, required).map_err(|error| {
        HyperBuildError::PropertyLayerTooShort {
            id_family: error.id_family,
            required: error.required,
            actual: error.actual,
        }
    })
}

#[cfg(feature = "build-property-arrow")]
fn export_topology_property_snapshot<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    property: EncodedPropertySnapshot,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex + BcsrSnapshotIndex,
    RelationIndex: LayoutIndex + BcsrSnapshotIndex,
    IncidenceIndex: LayoutIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
{
    let incidence_map = bcsr_local_incidence_to_canonical(topology)?;
    let identity_modes = [
        IdentityModeRecord::<IncidenceIndex>::local_equals_canonical(
            IdFamily::Element,
            topology.vertex_count,
        )?,
        IdentityModeRecord::<IncidenceIndex>::local_equals_canonical(
            IdFamily::Relation,
            topology.relation_count(),
        )?,
        IdentityModeRecord::<IncidenceIndex>::explicit_map(
            IdFamily::Incidence,
            incidence_map.len(),
        )?,
    ];
    let mut builder = SnapshotBuilder::new();
    add_topology_sections::<VertexIndex, RelationIndex, IncidenceIndex>(&mut builder, topology)?;
    oxgraph_property::export::append_identity_and_property_sections(
        &mut builder,
        &identity_modes,
        IncidenceIndex::INCIDENCE_IDENTITY_MAP_KIND,
        &incidence_map,
        property,
    )?;
    builder.finish().map_err(HyperBuildError::from)
}

/// Returns the build-path local-incidence-to-canonical map.
///
/// The build path now numbers participant IDs in the same head-block-then-tail-block
/// order the borrowed view uses (see `build_topology`), so the local
/// participant slot equals the canonical incidence ID directly: this map is the
/// identity `[0, total_incidences)`. It is still emitted explicitly so the
/// property layer's incidence identity mode records an explicit map and the
/// reader can rekey incidence-keyed properties without assuming identity.
#[cfg(feature = "build-property-arrow")]
fn bcsr_local_incidence_to_canonical<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<IncidenceIndex>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: LayoutIndex,
    RelationIndex: LayoutIndex,
    IncidenceIndex: LayoutIndex,
{
    let total = topology.total_participants();
    let mut map = Vec::with_capacity(total);
    for incidence in 0..total {
        map.push(index_from_usize(incidence).map_err(map_offset_overflow)?);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    //! Tests for hypergraph builder freeze and export semantics.

    #[cfg(feature = "build-property-arrow")]
    use alloc::sync::Arc;
    use alloc::vec;
    #[cfg(feature = "build-property-arrow")]
    use core::error::Error;

    #[cfg(feature = "build-property-arrow")]
    use arrow_array::Int32Array;
    #[cfg(feature = "build-property-arrow")]
    use arrow_schema::{DataType, Field};
    #[cfg(feature = "build-property-arrow")]
    use oxgraph_property::{
        HyperPropertyLayers, IdFamily, LayerId, LayerRole, PropertyLayer, PropertyLayerDescriptor,
        StorageMode, validate_identity_snapshot, validate_property_snapshot,
    };
    #[cfg(feature = "build-property-arrow")]
    use oxgraph_snapshot::Snapshot;

    use super::*;
    #[cfg(feature = "build-property-arrow")]
    use crate::{BcsrSnapshotHypergraph, BcsrValidation};

    /// Weighted builders require explicit weights and preserve updates.
    #[test]
    fn freeze_preserves_generic_weights() -> Result<(), HyperBuildError<u32, u32, u32>> {
        let mut builder = WeightedHypergraphBuilder::<u32, u32, u32, i32, u16, i8>::new();
        let a = builder.add_vertex(1_i32)?;
        let b = builder.add_vertex(2_i32)?;
        let edge = builder.add_hyperedge(&[(a, 3_i8)], &[(b, 4_i8)], 2_u16)?;
        builder.set_element_weight(a, 9_i32)?;
        builder.set_relation_weight(edge, 7_u16)?;
        builder.set_source_incidence_weight(edge, 0, 5_i8)?;
        let frozen = builder.freeze()?;
        assert_eq!(frozen.element_weight(a), 9_i32);
        assert_eq!(frozen.relation_weight(edge), 7_u16);
        assert_eq!(frozen.incidence_weight(HyperParticipantId::new(0)), 5_i8);
        assert_eq!(frozen.element_successors(a).collect::<Vec<_>>(), vec![b]);
        Ok(())
    }

    /// Mixed vertex/relation/incidence widths compile and freeze.
    #[test]
    fn mixed_width_builder_freezes() -> Result<(), HyperBuildError<u32, u32, u64>> {
        let mut builder = HypergraphBuilder::<u32, u32, u64>::new();
        let a = builder.add_vertex()?;
        let b = builder.add_vertex()?;
        let edge = builder.add_hyperedge(&[a], &[b])?;
        let frozen = builder.freeze()?;
        assert_eq!(frozen.relation_index(edge), 0);
        assert_eq!(frozen.element_successors(a).collect::<Vec<_>>(), vec![b]);
        Ok(())
    }

    /// Snapshot export writes BCSR topology, identity, and property sections.
    #[cfg(feature = "build-property-arrow")]
    #[test]
    fn snapshot_includes_identity_and_property_sections() -> Result<(), Box<dyn Error>> {
        let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
        let a = builder.add_vertex()?;
        let b = builder.add_vertex()?;
        builder.add_hyperedge(&[a], &[b])?;
        let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(1_u32),
            "vertex_marker",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("vertex_marker", DataType::Int32, false),
        )?;
        let element_layers = [PropertyLayer::try_new_dense(
            descriptor,
            Arc::new(Int32Array::from(vec![1_i32, 2_i32])),
        )?];
        let frozen = builder.freeze()?;
        let bytes = export_bcsr_snapshot_with_properties(
            &frozen,
            HyperPropertyLayers {
                element: &element_layers,
                relation: &[],
                incidence: &[],
            },
        )?;
        let snapshot = Snapshot::open(&bytes)?;
        let _opened = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot_with(
            &snapshot,
            BcsrValidation::Strict,
        )?;
        assert_eq!(
            validate_identity_snapshot::<u32>(&snapshot)?.records.len(),
            3
        );
        assert_eq!(validate_property_snapshot::<u32>(&snapshot)?.layer_count, 1);
        Ok(())
    }

    /// Regression for C1: the build path and the borrowed view must assign the
    /// SAME `IncidenceId` numbering. With more than one hyperedge the old
    /// interleaved (hyperedge-major source-then-target) numbering diverged from
    /// the view's head-block-then-tail-block numbering; this asserts they now
    /// agree for every hyperedge and every incidence.
    #[test]
    fn build_and_view_incidence_numbering_match()
    -> Result<(), alloc::boxed::Box<dyn core::error::Error>> {
        use oxgraph_hyper::{IncidenceElement, IncidenceRelation, RelationIncidences};
        use oxgraph_snapshot::Snapshot;

        use crate::{BcsrParticipantId, BcsrSnapshotHypergraph};

        // Two hyperedges so head/tail blocks interleave under the old scheme.
        // e0: {a,b} -> {c}; e1: {a} -> {b,c}.
        let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
        let a = builder.add_vertex()?;
        let b = builder.add_vertex()?;
        let c = builder.add_vertex()?;
        let e0 = builder.add_hyperedge(&[a, b], &[c])?;
        let e1 = builder.add_hyperedge(&[a], &[b, c])?;
        let frozen = builder.freeze()?;

        let bytes = export_bcsr_snapshot(&frozen)?;
        let snapshot = Snapshot::open(&bytes)?;
        let view = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot)?;

        for hyperedge in [e0, e1] {
            let build_ids: Vec<BcsrParticipantId<u32>> =
                RelationIncidences::relation_incidences(&frozen, hyperedge).collect();
            let view_ids: Vec<BcsrParticipantId<u32>> =
                RelationIncidences::relation_incidences(&view, hyperedge).collect();
            assert_eq!(
                build_ids, view_ids,
                "incidence ID sequence diverged for {hyperedge:?}"
            );
            // Each incidence must resolve to the same element and relation in
            // both the build path and the view.
            for incidence in build_ids {
                assert_eq!(
                    IncidenceElement::incidence_element(&frozen, incidence),
                    IncidenceElement::incidence_element(&view, incidence)
                );
                assert_eq!(
                    IncidenceRelation::incidence_relation(&frozen, incidence),
                    IncidenceRelation::incidence_relation(&view, incidence)
                );
            }
        }
        Ok(())
    }
}

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
pub use oxgraph_layout_util::BuildIndex;
use oxgraph_layout_util::{
    IdOutOfBounds, OffsetOverflow, build_offset_index, id_to_slot, index_from_usize, slot_or_max,
};
#[cfg(feature = "build-property-arrow")]
use oxgraph_property::{
    EncodedPropertySnapshot, HyperPropertyLayers, IdFamily, IdentityModeRecord, PropertyError,
    PropertyLayer, PropertySnapshotMetaWord, SNAPSHOT_PROPERTY_VERSION,
    encode_hyper_property_snapshot, rekey_layer_to_local,
};
use oxgraph_snapshot::{PlanError, SnapshotBuilder};

use crate::BcsrSnapshotIndex;

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

/// Element-to-incidence offset and incidence arrays.
type ElementIncidenceIndex<IncidenceIndex> = (Vec<IncidenceIndex>, Vec<IncidenceIndex>);

/// Result returned by element-to-incidence index construction.
type ElementIncidenceIndexResult<VertexIndex, RelationIndex, IncidenceIndex> = Result<
    ElementIncidenceIndex<IncidenceIndex>,
    HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>,
>;

/// Local/canonical vertex ID assigned by hypergraph builders.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperVertexId<VertexIndex>(pub VertexIndex);

/// Local/canonical hyperedge ID assigned by hypergraph builders.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperedgeId<RelationIndex>(pub RelationIndex);

/// Local/canonical participant ID assigned when a hypergraph is frozen.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperParticipantId<IncidenceIndex>(pub IncidenceIndex);

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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    vertex_count: usize,
    hyperedges: Vec<HyperedgeRecord<VertexIndex>>,
    index_width: PhantomData<(RelationIndex, IncidenceIndex)>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex>
    HypergraphBuilder<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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
        Ok(HyperVertexId(id))
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
            sources: sources.iter().map(|vertex| vertex.0).collect(),
            targets: targets.iter().map(|vertex| vertex.0).collect(),
        });
        Ok(HyperedgeId(hyperedge))
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
    /// # Errors
    ///
    /// Returns [`HyperBuildError::IdOverflow`] if any offset or dense ID cannot
    /// fit in the selected index widths.
    ///
    /// # Performance
    ///
    /// `O(v + h + p log p)` for vertices `v`, hyperedges `h`, and participants
    /// `p`.
    pub fn freeze(&self) -> FrozenHypergraphResult<VertexIndex, RelationIndex, IncidenceIndex> {
        let normalized = self.normalized_hyperedges();
        let (topology, _weights) = build_topology(self.vertex_count, &normalized)?;
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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
        Ok(HyperVertexId(id))
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
                .map(|(vertex, weight)| (vertex.0, weight.clone()))
                .collect(),
            targets: targets
                .iter()
                .map(|(vertex, weight)| (vertex.0, weight.clone()))
                .collect(),
        });
        self.relation_weights.push(relation_weight);
        Ok(HyperedgeId(hyperedge))
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
    /// # Errors
    ///
    /// Returns [`HyperBuildError::IdOverflow`] if any offset or dense ID cannot
    /// fit in the selected index widths.
    ///
    /// # Performance
    ///
    /// `O(v + h + p log p)` for vertices `v`, hyperedges `h`, and participants
    /// `p`.
    pub fn freeze(
        &self,
    ) -> FrozenWeightedHypergraphResult<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
    where
        EW: Clone,
        RW: Clone,
        IW: Clone,
    {
        let normalized = self.normalized_hyperedges();
        let (topology, incidence_weights) = build_topology(self.vertex_count, &normalized)?;
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Owned immutable unweighted directed hypergraph topology.
///
/// # Performance
///
/// Count, containment, index, endpoint, and traversal methods are `O(1)` or
/// `O(degree)` for incidence and adjacency iteration.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenHypergraph<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    topology: FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
}

/// Owned immutable weighted directed hypergraph topology.
///
/// # Performance
///
/// Weight lookups are `O(1)`.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    topology: FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    element_weights: Box<[EW]>,
    relation_weights: Box<[RW]>,
    incidence_weights: Box<[IW]>,
}

#[derive(Clone, Debug)]
struct FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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
    relation_offsets: Box<[IncidenceIndex]>,
    participant_elements: Box<[VertexIndex]>,
    participant_relations: Box<[RelationIndex]>,
    participant_roles: Box<[HyperParticipantRole]>,
    element_incidence_offsets: Box<[IncidenceIndex]>,
    element_incidence_ids: Box<[IncidenceIndex]>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex>
    FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    fn relation_count(&self) -> usize {
        self.head_offsets.len().saturating_sub(1)
    }

    fn vertex_slot(vertex: HyperVertexId<VertexIndex>) -> usize {
        vertex.0.to_usize().unwrap_or(usize::MAX)
    }

    fn hyperedge_slot(hyperedge: HyperedgeId<RelationIndex>) -> usize {
        hyperedge.0.to_usize().unwrap_or(usize::MAX)
    }

    fn offset(value: IncidenceIndex, fallback: usize) -> usize {
        value.to_usize().unwrap_or(fallback)
    }

    fn head_range(&self, hyperedge: HyperedgeId<RelationIndex>) -> core::ops::Range<usize> {
        let slot = Self::hyperedge_slot(hyperedge);
        let fallback = self.head_participants.len();
        Self::offset(self.head_offsets[slot], fallback)
            ..Self::offset(self.head_offsets[slot + 1], fallback)
    }

    fn tail_range(&self, hyperedge: HyperedgeId<RelationIndex>) -> core::ops::Range<usize> {
        let slot = Self::hyperedge_slot(hyperedge);
        let fallback = self.tail_participants.len();
        Self::offset(self.tail_offsets[slot], fallback)
            ..Self::offset(self.tail_offsets[slot + 1], fallback)
    }

    fn relation_incidence_range(
        &self,
        hyperedge: HyperedgeId<RelationIndex>,
    ) -> core::ops::Range<usize> {
        let slot = Self::hyperedge_slot(hyperedge);
        let fallback = self.participant_elements.len();
        Self::offset(self.relation_offsets[slot], fallback)
            ..Self::offset(self.relation_offsets[slot + 1], fallback)
    }

    fn element_incidence_range(
        &self,
        vertex: HyperVertexId<VertexIndex>,
    ) -> core::ops::Range<usize> {
        let slot = Self::vertex_slot(vertex);
        let fallback = self.element_incidence_ids.len();
        Self::offset(self.element_incidence_offsets[slot], fallback)
            ..Self::offset(self.element_incidence_offsets[slot + 1], fallback)
    }

    fn outgoing_hyperedge_range(
        &self,
        vertex: HyperVertexId<VertexIndex>,
    ) -> core::ops::Range<usize> {
        let slot = Self::vertex_slot(vertex);
        let fallback = self.vertex_outgoing_hyperedges.len();
        Self::offset(self.vertex_outgoing_offsets[slot], fallback)
            ..Self::offset(self.vertex_outgoing_offsets[slot + 1], fallback)
    }

    fn incoming_hyperedge_range(
        &self,
        vertex: HyperVertexId<VertexIndex>,
    ) -> core::ops::Range<usize> {
        let slot = Self::vertex_slot(vertex);
        let fallback = self.vertex_incoming_hyperedges.len();
        Self::offset(self.vertex_incoming_offsets[slot], fallback)
            ..Self::offset(self.vertex_incoming_offsets[slot + 1], fallback)
    }

    fn source_participants(
        &self,
        hyperedge: HyperedgeId<RelationIndex>,
    ) -> VertexSliceIter<'_, VertexIndex> {
        VertexSliceIter {
            inner: self.head_participants[self.head_range(hyperedge)].iter(),
        }
    }

    fn target_participants(
        &self,
        hyperedge: HyperedgeId<RelationIndex>,
    ) -> VertexSliceIter<'_, VertexIndex> {
        VertexSliceIter {
            inner: self.tail_participants[self.tail_range(hyperedge)].iter(),
        }
    }

    fn outgoing_hyperedges(
        &self,
        vertex: HyperVertexId<VertexIndex>,
    ) -> HyperedgeSliceIter<'_, RelationIndex> {
        HyperedgeSliceIter {
            inner: self.vertex_outgoing_hyperedges[self.outgoing_hyperedge_range(vertex)].iter(),
        }
    }

    fn incoming_hyperedges(
        &self,
        vertex: HyperVertexId<VertexIndex>,
    ) -> HyperedgeSliceIter<'_, RelationIndex> {
        HyperedgeSliceIter {
            inner: self.vertex_incoming_hyperedges[self.incoming_hyperedge_range(vertex)].iter(),
        }
    }
}

fn vertex_slot<VertexIndex: BuildIndex>(vertex: HyperVertexId<VertexIndex>) -> usize {
    slot_or_max::<VertexIndex>(vertex.0)
}

fn hyperedge_slot<RelationIndex: BuildIndex>(hyperedge: HyperedgeId<RelationIndex>) -> usize {
    slot_or_max::<RelationIndex>(hyperedge.0)
}

fn participant_slot<IncidenceIndex: BuildIndex>(
    participant: HyperParticipantId<IncidenceIndex>,
) -> usize {
    slot_or_max::<IncidenceIndex>(participant.0)
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
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type ElementId = HyperVertexId<VertexIndex>;
            type RelationId = HyperedgeId<RelationIndex>;
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceBase
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type IncidenceId = HyperParticipantId<IncidenceIndex>;
            type Role = HyperParticipantRole;
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> TopologyCounts
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
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
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceCounts
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn incidence_count(&self) -> usize {
                self.$topology.participant_elements.len()
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementIndexTrait
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn element_bound(&self) -> usize {
                self.$topology.vertex_count
            }

            fn element_index(&self, element: HyperVertexId<VertexIndex>) -> usize {
                element.0.to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> RelationIndexTrait
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn relation_bound(&self) -> usize {
                self.$topology.relation_count()
            }

            fn relation_index(&self, relation: HyperedgeId<RelationIndex>) -> usize {
                relation.0.to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceIndexTrait
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn incidence_bound(&self) -> usize {
                self.$topology.participant_elements.len()
            }

            fn incidence_index(&self, incidence: HyperParticipantId<IncidenceIndex>) -> usize {
                incidence.0.to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ContainsElement
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn contains_element(&self, element: HyperVertexId<VertexIndex>) -> bool {
                element
                    .0
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.vertex_count)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ContainsRelation
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn contains_relation(&self, relation: HyperedgeId<RelationIndex>) -> bool {
                relation
                    .0
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.relation_count())
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ContainsIncidence
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn contains_incidence(&self, incidence: HyperParticipantId<IncidenceIndex>) -> bool {
                incidence
                    .0
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.participant_elements.len())
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceElement
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn incidence_element(
                &self,
                incidence: HyperParticipantId<IncidenceIndex>,
            ) -> HyperVertexId<VertexIndex> {
                HyperVertexId(self.$topology.participant_elements[participant_slot(incidence)])
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceRelation
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn incidence_relation(
                &self,
                incidence: HyperParticipantId<IncidenceIndex>,
            ) -> HyperedgeId<RelationIndex> {
                HyperedgeId(self.$topology.participant_relations[participant_slot(incidence)])
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidenceRole
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn incidence_role(&self, incidence: HyperParticipantId<IncidenceIndex>) -> HyperParticipantRole {
                self.$topology.participant_roles[participant_slot(incidence)]
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> RelationIncidences
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type Incidences<'view>
                = ParticipantRangeIter<IncidenceIndex>
            where
                Self: 'view;

            fn relation_incidences(&self, relation: HyperedgeId<RelationIndex>) -> Self::Incidences<'_> {
                ParticipantRangeIter::new(self.$topology.relation_incidence_range(relation))
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementIncidences
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type Incidences<'view>
                = ParticipantSliceIter<'view, IncidenceIndex>
            where
                Self: 'view;

            fn element_incidences(&self, element: HyperVertexId<VertexIndex>) -> Self::Incidences<'_> {
                ParticipantSliceIter {
                    inner: self.$topology.element_incidence_ids[self.$topology.element_incidence_range(element)].iter(),
                }
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> RelationIncidenceCount
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn relation_incidence_count(&self, relation: HyperedgeId<RelationIndex>) -> usize {
                self.$topology.relation_incidence_range(relation).len()
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementIncidenceCount
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn element_incidence_count(&self, element: HyperVertexId<VertexIndex>) -> usize {
                self.$topology.element_incidence_range(element).len()
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> HyperedgeParticipants
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type Participants<'view>
                = core::iter::Chain<VertexSliceIter<'view, VertexIndex>, VertexSliceIter<'view, VertexIndex>>
            where
                Self: 'view;

            fn hyperedge_participants(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::Participants<'_> {
                self.source_participants(hyperedge)
                    .chain(self.target_participants(hyperedge))
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> IncidentHyperedges
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type IncidentHyperedges<'view>
                = IncidentHyperedgeIter<'view, IncidenceIndex, RelationIndex>
            where
                Self: 'view;

            fn incident_hyperedges(&self, vertex: HyperVertexId<VertexIndex>) -> Self::IncidentHyperedges<'_> {
                IncidentHyperedgeIter {
                    incidences: self.element_incidences(vertex),
                    participant_relations: &self.$topology.participant_relations,
                }
            }
        }

        // `HyperedgeParticipantCount` / `IncidentHyperedgeCount` come for free
        // from the blanket impls in `oxgraph-hyper` over the
        // `RelationIncidenceCount` / `ElementIncidenceCount` impls above.

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> DirectedHyperedgeParticipants
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type SourceParticipants<'view>
                = VertexSliceIter<'view, VertexIndex>
            where
                Self: 'view;
            type TargetParticipants<'view>
                = VertexSliceIter<'view, VertexIndex>
            where
                Self: 'view;

            fn source_participants(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::SourceParticipants<'_> {
                self.$topology.source_participants(hyperedge)
            }

            fn target_participants(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::TargetParticipants<'_> {
                self.$topology.target_participants(hyperedge)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> DirectedHyperedgeIncidences
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type SourceIncidences<'view>
                = ParticipantRangeIter<IncidenceIndex>
            where
                Self: 'view;
            type TargetIncidences<'view>
                = ParticipantRangeIter<IncidenceIndex>
            where
                Self: 'view;

            fn source_incidences(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::SourceIncidences<'_> {
                let start = self.$topology.relation_incidence_range(hyperedge).start;
                let len = self.$topology.head_range(hyperedge).len();
                ParticipantRangeIter::new(start..start + len)
            }

            fn target_incidences(&self, hyperedge: HyperedgeId<RelationIndex>) -> Self::TargetIncidences<'_> {
                let start = self.$topology.relation_incidence_range(hyperedge).start
                    + self.$topology.head_range(hyperedge).len();
                let len = self.$topology.tail_range(hyperedge).len();
                ParticipantRangeIter::new(start..start + len)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> DirectedVertexHyperedges
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type OutgoingHyperedges<'view>
                = HyperedgeSliceIter<'view, RelationIndex>
            where
                Self: 'view;
            type IncomingHyperedges<'view>
                = HyperedgeSliceIter<'view, RelationIndex>
            where
                Self: 'view;

            fn outgoing_hyperedges(&self, vertex: HyperVertexId<VertexIndex>) -> Self::OutgoingHyperedges<'_> {
                self.$topology.outgoing_hyperedges(vertex)
            }

            fn incoming_hyperedges(&self, vertex: HyperVertexId<VertexIndex>) -> Self::IncomingHyperedges<'_> {
                self.$topology.incoming_hyperedges(vertex)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementSuccessors
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type Successors<'view>
                = SuccessorIter<'view, VertexIndex, RelationIndex, IncidenceIndex>
            where
                Self: 'view;

            fn element_successors(&self, element: HyperVertexId<VertexIndex>) -> Self::Successors<'_> {
                SuccessorIter {
                    topology: &self.$topology,
                    hyperedges: self.outgoing_hyperedges(element),
                    current: None,
                }
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> ElementPredecessors
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type Predecessors<'view>
                = PredecessorIter<'view, VertexIndex, RelationIndex, IncidenceIndex>
            where
                Self: 'view;

            fn element_predecessors(&self, element: HyperVertexId<VertexIndex>) -> Self::Predecessors<'_> {
                PredecessorIter {
                    topology: &self.$topology,
                    hyperedges: self.incoming_hyperedges(element),
                    current: None,
                }
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> CanonicalElementIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type CanonicalElementId = HyperVertexId<VertexIndex>;

            fn canonical_element_id(&self, element: HyperVertexId<VertexIndex>) -> Self::CanonicalElementId {
                element
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> LocalElementIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId> {
                self.contains_element(canonical).then_some(canonical)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> CanonicalRelationIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            type CanonicalRelationId = HyperedgeId<RelationIndex>;

            fn canonical_relation_id(&self, relation: HyperedgeId<RelationIndex>) -> Self::CanonicalRelationId {
                relation
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> LocalRelationIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
        {
            fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId> {
                self.contains_relation(canonical).then_some(canonical)
            }
        }

        impl<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*> CanonicalIncidenceIdentity
            for $name<VertexIndex, RelationIndex, IncidenceIndex$(, $extra)*>
        where
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
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
            VertexIndex: BuildIndex,
            RelationIndex: BuildIndex,
            IncidenceIndex: BuildIndex,
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    type Weight = EW;

    fn element_weight(&self, element: HyperVertexId<VertexIndex>) -> Self::Weight {
        self.element_weights[vertex_slot(element)]
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW: Copy, IW> RelationWeight
    for FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    type Weight = RW;

    fn relation_weight(&self, relation: HyperedgeId<RelationIndex>) -> Self::Weight {
        self.relation_weights[hyperedge_slot(relation)]
    }
}

impl<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW: Copy> IncidenceWeight
    for FrozenWeightedHypergraph<VertexIndex, RelationIndex, IncidenceIndex, EW, RW, IW>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    type Weight = IW;

    fn incidence_weight(&self, incidence: HyperParticipantId<IncidenceIndex>) -> Self::Weight {
        self.incidence_weights[participant_slot(incidence)]
    }
}

/// Iterator over vertex IDs stored in frozen hypergraph slices.
///
/// # Performance
///
/// `next` is `O(1)`.
pub struct VertexSliceIter<'view, VertexIndex> {
    inner: core::slice::Iter<'view, VertexIndex>,
}

impl<VertexIndex: Copy> Iterator for VertexSliceIter<'_, VertexIndex> {
    type Item = HyperVertexId<VertexIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(HyperVertexId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<VertexIndex: Copy> ExactSizeIterator for VertexSliceIter<'_, VertexIndex> {}

/// Iterator over hyperedge IDs stored in frozen hypergraph slices.
///
/// # Performance
///
/// `next` is `O(1)`.
pub struct HyperedgeSliceIter<'view, RelationIndex> {
    inner: core::slice::Iter<'view, RelationIndex>,
}

impl<RelationIndex: Copy> Iterator for HyperedgeSliceIter<'_, RelationIndex> {
    type Item = HyperedgeId<RelationIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(HyperedgeId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<RelationIndex: Copy> ExactSizeIterator for HyperedgeSliceIter<'_, RelationIndex> {}

/// Iterator over participant IDs stored in frozen hypergraph slices.
///
/// # Performance
///
/// `next` is `O(1)`.
pub struct ParticipantSliceIter<'view, IncidenceIndex> {
    inner: core::slice::Iter<'view, IncidenceIndex>,
}

impl<IncidenceIndex: Copy> Iterator for ParticipantSliceIter<'_, IncidenceIndex> {
    type Item = HyperParticipantId<IncidenceIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(HyperParticipantId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<IncidenceIndex: Copy> ExactSizeIterator for ParticipantSliceIter<'_, IncidenceIndex> {}

/// Iterator over contiguous participant IDs.
///
/// # Performance
///
/// `next` is `O(1)`.
pub struct ParticipantRangeIter<IncidenceIndex> {
    next: usize,
    end: usize,
    index_width: PhantomData<IncidenceIndex>,
}

impl<IncidenceIndex> ParticipantRangeIter<IncidenceIndex> {
    const fn new(range: core::ops::Range<usize>) -> Self {
        Self {
            next: range.start,
            end: range.end,
            index_width: PhantomData,
        }
    }
}

impl<IncidenceIndex: BuildIndex> Iterator for ParticipantRangeIter<IncidenceIndex> {
    type Item = HyperParticipantId<IncidenceIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let value = self.next;
        self.next += 1;
        IncidenceIndex::from_usize(value).map(HyperParticipantId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.end - self.next;
        (len, Some(len))
    }
}

impl<IncidenceIndex: BuildIndex> ExactSizeIterator for ParticipantRangeIter<IncidenceIndex> {}

/// Iterator that maps vertex incidences to hyperedges.
///
/// # Performance
///
/// `next` is `O(1)`.
pub struct IncidentHyperedgeIter<'view, IncidenceIndex, RelationIndex> {
    incidences: ParticipantSliceIter<'view, IncidenceIndex>,
    participant_relations: &'view [RelationIndex],
}

impl<IncidenceIndex, RelationIndex> Iterator
    for IncidentHyperedgeIter<'_, IncidenceIndex, RelationIndex>
where
    IncidenceIndex: BuildIndex,
    RelationIndex: Copy,
{
    type Item = HyperedgeId<RelationIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        let incidence = self.incidences.next()?;
        let slot = incidence.0.to_usize()?;
        self.participant_relations
            .get(slot)
            .copied()
            .map(HyperedgeId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.incidences.size_hint()
    }
}

impl<IncidenceIndex, RelationIndex> ExactSizeIterator
    for IncidentHyperedgeIter<'_, IncidenceIndex, RelationIndex>
where
    IncidenceIndex: BuildIndex,
    RelationIndex: Copy,
{
}

/// Iterator over successor vertices reached via outgoing hyperedges.
///
/// # Performance
///
/// Each yielded successor is `O(1)` after the containing hyperedge is loaded.
pub struct SuccessorIter<'view, VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    topology: &'view FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    hyperedges: HyperedgeSliceIter<'view, RelationIndex>,
    current: Option<VertexSliceIter<'view, VertexIndex>>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex> Iterator
    for SuccessorIter<'_, VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    type Item = HyperVertexId<VertexIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = &mut self.current
                && let Some(vertex) = current.next()
            {
                return Some(vertex);
            }
            let hyperedge = self.hyperedges.next()?;
            self.current = Some(self.topology.target_participants(hyperedge));
        }
    }
}

/// Iterator over predecessor vertices reached via incoming hyperedges.
///
/// # Performance
///
/// Each yielded predecessor is `O(1)` after the containing hyperedge is loaded.
pub struct PredecessorIter<'view, VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    topology: &'view FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    hyperedges: HyperedgeSliceIter<'view, RelationIndex>,
    current: Option<VertexSliceIter<'view, VertexIndex>>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex> Iterator
    for PredecessorIter<'_, VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    type Item = HyperVertexId<VertexIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = &mut self.current
                && let Some(vertex) = current.next()
            {
                return Some(vertex);
            }
            let hyperedge = self.hyperedges.next()?;
            self.current = Some(self.topology.source_participants(hyperedge));
        }
    }
}

/// Converts a BCSR payload slice into explicit little-endian words.
///
/// # Performance
///
/// This function is `O(values.len())`.
pub fn bcsr_slice_to_le<I>(values: &[I]) -> Vec<I::LittleEndianWord>
where
    I: BcsrSnapshotIndex,
{
    values
        .iter()
        .copied()
        .map(BcsrSnapshotIndex::to_le_word)
        .collect()
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
    VertexIndex: BuildIndex + BcsrSnapshotIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex,
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
    VertexIndex: BuildIndex + BcsrSnapshotIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex,
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
    VertexIndex: BuildIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
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
    VertexIndex: BuildIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
    IW: Clone,
{
    let participant_count = records
        .iter()
        .map(|record| record.sources.len() + record.targets.len())
        .sum();
    let mut head_offsets = Vec::with_capacity(records.len() + 1);
    let mut tail_offsets = Vec::with_capacity(records.len() + 1);
    let mut relation_offsets = Vec::with_capacity(records.len() + 1);
    let mut head_participants = Vec::new();
    let mut tail_participants = Vec::new();
    let mut participant_elements = Vec::with_capacity(participant_count);
    let mut participant_relations = Vec::with_capacity(participant_count);
    let mut participant_roles = Vec::with_capacity(participant_count);
    let mut incidence_weights = Vec::with_capacity(participant_count);

    head_offsets.push(index_from_usize(0).map_err(map_offset_overflow)?);
    tail_offsets.push(index_from_usize(0).map_err(map_offset_overflow)?);
    relation_offsets.push(index_from_usize(0).map_err(map_offset_overflow)?);
    for (relation, record) in records.iter().enumerate() {
        let relation_id = index_from_usize(relation).map_err(map_offset_overflow)?;
        for (vertex, weight) in &record.sources {
            head_participants.push(*vertex);
            participant_elements.push(*vertex);
            participant_relations.push(relation_id);
            participant_roles.push(HyperParticipantRole::Source);
            incidence_weights.push(weight.clone());
        }
        for (vertex, weight) in &record.targets {
            tail_participants.push(*vertex);
            participant_elements.push(*vertex);
            participant_relations.push(relation_id);
            participant_roles.push(HyperParticipantRole::Target);
            incidence_weights.push(weight.clone());
        }
        head_offsets.push(index_from_usize(head_participants.len()).map_err(map_offset_overflow)?);
        tail_offsets.push(index_from_usize(tail_participants.len()).map_err(map_offset_overflow)?);
        relation_offsets
            .push(index_from_usize(participant_elements.len()).map_err(map_offset_overflow)?);
    }

    let (vertex_outgoing_offsets, vertex_outgoing_hyperedges) =
        build_vertex_relation_index(vertex_count, records, HyperParticipantRole::Source)?;
    let (vertex_incoming_offsets, vertex_incoming_hyperedges) =
        build_vertex_relation_index(vertex_count, records, HyperParticipantRole::Target)?;
    let (element_incidence_offsets, element_incidence_ids) =
        build_element_incidence_index(vertex_count, &participant_elements)?;

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
            relation_offsets: relation_offsets.into_boxed_slice(),
            participant_elements: participant_elements.into_boxed_slice(),
            participant_relations: participant_relations.into_boxed_slice(),
            participant_roles: participant_roles.into_boxed_slice(),
            element_incidence_offsets: element_incidence_offsets.into_boxed_slice(),
            element_incidence_ids: element_incidence_ids.into_boxed_slice(),
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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

fn build_element_incidence_index<VertexIndex, RelationIndex, IncidenceIndex>(
    vertex_count: usize,
    participant_elements: &[VertexIndex],
) -> ElementIncidenceIndexResult<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    let mut buckets = vec![Vec::<IncidenceIndex>::new(); vertex_count];
    for (participant, vertex) in participant_elements.iter().copied().enumerate() {
        let slot = vertex
            .to_usize()
            .ok_or(HyperBuildError::IdOverflow { value: usize::MAX })?;
        buckets[slot].push(index_from_usize(participant).map_err(map_offset_overflow)?);
    }
    build_offset_index::<IncidenceIndex, IncidenceIndex>(buckets).map_err(map_offset_overflow)
}

fn ensure_vertices<VertexIndex, RelationIndex, IncidenceIndex, I>(
    vertex_count: usize,
    participants: I,
) -> Result<(), HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
    I: IntoIterator<Item = HyperVertexId<VertexIndex>>,
{
    let mut sorted: Vec<VertexIndex> = participants.into_iter().map(|vertex| vertex.0).collect();
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        if pair[0] == pair[1] {
            return Err(HyperBuildError::DuplicateParticipant {
                vertex: HyperVertexId(pair[0]),
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    id_to_slot::<VertexIndex>(vertex.0, vertex_count)
        .map_err(|_: IdOutOfBounds| HyperBuildError::InvalidVertex { vertex })
}

fn hyperedge_index_checked<VertexIndex, RelationIndex, IncidenceIndex>(
    hyperedge_count: usize,
    hyperedge: HyperedgeId<RelationIndex>,
) -> Result<usize, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    id_to_slot::<RelationIndex>(hyperedge.0, hyperedge_count)
        .map_err(|_: IdOutOfBounds| HyperBuildError::InvalidHyperedge { hyperedge })
}

fn export_topology_snapshot<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: BuildIndex + BcsrSnapshotIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex,
{
    let head_offsets = bcsr_slice_to_le(&topology.head_offsets);
    let head_participants = bcsr_slice_to_le(&topology.head_participants);
    let tail_offsets = bcsr_slice_to_le(&topology.tail_offsets);
    let tail_participants = bcsr_slice_to_le(&topology.tail_participants);
    let vertex_outgoing_offsets = bcsr_slice_to_le(&topology.vertex_outgoing_offsets);
    let vertex_outgoing_hyperedges = bcsr_slice_to_le(&topology.vertex_outgoing_hyperedges);
    let vertex_incoming_offsets = bcsr_slice_to_le(&topology.vertex_incoming_offsets);
    let vertex_incoming_hyperedges = bcsr_slice_to_le(&topology.vertex_incoming_hyperedges);
    let mut builder = SnapshotBuilder::new();
    builder.add_section_little_endian(IncidenceIndex::HEAD_OFFSETS_KIND, 1, &head_offsets)?;
    builder.add_section_little_endian(
        VertexIndex::HEAD_PARTICIPANTS_KIND,
        1,
        &head_participants,
    )?;
    builder.add_section_little_endian(IncidenceIndex::TAIL_OFFSETS_KIND, 1, &tail_offsets)?;
    builder.add_section_little_endian(
        VertexIndex::TAIL_PARTICIPANTS_KIND,
        1,
        &tail_participants,
    )?;
    builder.add_section_little_endian(
        IncidenceIndex::VERTEX_OUTGOING_OFFSETS_KIND,
        1,
        &vertex_outgoing_offsets,
    )?;
    builder.add_section_little_endian(
        RelationIndex::VERTEX_OUTGOING_HYPEREDGES_KIND,
        1,
        &vertex_outgoing_hyperedges,
    )?;
    builder.add_section_little_endian(
        IncidenceIndex::VERTEX_INCOMING_OFFSETS_KIND,
        1,
        &vertex_incoming_offsets,
    )?;
    builder.add_section_little_endian(
        RelationIndex::VERTEX_INCOMING_HYPEREDGES_KIND,
        1,
        &vertex_incoming_hyperedges,
    )?;
    builder.finish().map_err(HyperBuildError::from)
}

#[cfg(feature = "build-property-arrow")]
fn encode_hyper_properties<VertexIndex, RelationIndex, IncidenceIndex, Id>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    layers: HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<EncodedPropertySnapshot, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: BuildIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex + oxgraph_property::PropertyIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
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
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    let required = match id_family {
        IdFamily::Element => topology.vertex_count,
        IdFamily::Relation => topology.relation_count(),
        IdFamily::Incidence => topology.participant_elements.len(),
        unsupported => {
            return Err(HyperBuildError::UnsupportedPropertyFamily {
                id_family: unsupported,
            });
        }
    };
    for layer in layers {
        if layer.len() < required {
            return Err(HyperBuildError::PropertyLayerTooShort {
                id_family,
                required,
                actual: layer.len(),
            });
        }
    }
    Ok(())
}

#[cfg(feature = "build-property-arrow")]
fn export_topology_property_snapshot<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
    property: EncodedPropertySnapshot,
) -> Result<Vec<u8>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: BuildIndex + BcsrSnapshotIndex,
    RelationIndex: BuildIndex + BcsrSnapshotIndex,
    IncidenceIndex: BuildIndex + BcsrSnapshotIndex + PropertySnapshotMetaWord,
{
    let head_offsets = bcsr_slice_to_le(&topology.head_offsets);
    let head_participants = bcsr_slice_to_le(&topology.head_participants);
    let tail_offsets = bcsr_slice_to_le(&topology.tail_offsets);
    let tail_participants = bcsr_slice_to_le(&topology.tail_participants);
    let vertex_outgoing_offsets = bcsr_slice_to_le(&topology.vertex_outgoing_offsets);
    let vertex_outgoing_hyperedges = bcsr_slice_to_le(&topology.vertex_outgoing_hyperedges);
    let vertex_incoming_offsets = bcsr_slice_to_le(&topology.vertex_incoming_offsets);
    let vertex_incoming_hyperedges = bcsr_slice_to_le(&topology.vertex_incoming_hyperedges);
    let incidence_map = bcsr_local_incidence_to_canonical(topology)?;
    let incidence_map_le = identity_slice_to_le::<IncidenceIndex>(&incidence_map);
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
    builder.add_section_little_endian(IncidenceIndex::HEAD_OFFSETS_KIND, 1, &head_offsets)?;
    builder.add_section_little_endian(
        VertexIndex::HEAD_PARTICIPANTS_KIND,
        1,
        &head_participants,
    )?;
    builder.add_section_little_endian(IncidenceIndex::TAIL_OFFSETS_KIND, 1, &tail_offsets)?;
    builder.add_section_little_endian(
        VertexIndex::TAIL_PARTICIPANTS_KIND,
        1,
        &tail_participants,
    )?;
    builder.add_section_little_endian(
        IncidenceIndex::VERTEX_OUTGOING_OFFSETS_KIND,
        1,
        &vertex_outgoing_offsets,
    )?;
    builder.add_section_little_endian(
        RelationIndex::VERTEX_OUTGOING_HYPEREDGES_KIND,
        1,
        &vertex_outgoing_hyperedges,
    )?;
    builder.add_section_little_endian(
        IncidenceIndex::VERTEX_INCOMING_OFFSETS_KIND,
        1,
        &vertex_incoming_offsets,
    )?;
    builder.add_section_little_endian(
        RelationIndex::VERTEX_INCOMING_HYPEREDGES_KIND,
        1,
        &vertex_incoming_hyperedges,
    )?;
    builder.add_section_little_endian(
        IncidenceIndex::IDENTITY_MODES_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        &identity_modes,
    )?;
    builder.add_section_little_endian(
        IncidenceIndex::INCIDENCE_IDENTITY_MAP_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        &incidence_map_le,
    )?;
    builder.add_section(
        IncidenceIndex::PROPERTY_DESCRIPTORS_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        property.descriptors,
    )?;
    builder.add_section(
        IncidenceIndex::PROPERTY_DATA_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        property.data,
    )?;
    builder.finish().map_err(HyperBuildError::from)
}

#[cfg(feature = "build-property-arrow")]
fn bcsr_local_incidence_to_canonical<VertexIndex, RelationIndex, IncidenceIndex>(
    topology: &FrozenTopology<VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<Vec<IncidenceIndex>, HyperBuildError<VertexIndex, RelationIndex, IncidenceIndex>>
where
    VertexIndex: BuildIndex,
    RelationIndex: BuildIndex,
    IncidenceIndex: BuildIndex,
{
    let mut map = Vec::with_capacity(topology.participant_elements.len());
    for relation in 0..topology.relation_count() {
        let hyperedge = HyperedgeId(index_from_usize(relation).map_err(map_offset_overflow)?);
        let relation_start = topology.relation_incidence_range(hyperedge).start;
        let head_len = topology.head_range(hyperedge).len();
        for local in 0..head_len {
            map.push(index_from_usize(relation_start + local).map_err(map_offset_overflow)?);
        }
    }
    for relation in 0..topology.relation_count() {
        let hyperedge = HyperedgeId(index_from_usize(relation).map_err(map_offset_overflow)?);
        let relation_start = topology.relation_incidence_range(hyperedge).start;
        let head_len = topology.head_range(hyperedge).len();
        let tail_len = topology.tail_range(hyperedge).len();
        for local in 0..tail_len {
            map.push(
                index_from_usize(relation_start + head_len + local).map_err(map_offset_overflow)?,
            );
        }
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
        assert_eq!(frozen.incidence_weight(HyperParticipantId(0)), 5_i8);
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
}

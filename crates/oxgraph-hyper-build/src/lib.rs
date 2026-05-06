//! Append/update-only builders for directed hypergraph topology.
//!
//! The builder assigns dense append-only canonical vertex, hyperedge, and
//! participant IDs, supports directed source/target participant sets, stores
//! generic topology weights, attaches named Arrow property layers, and freezes to
//! an owned immutable view. Deletion, tombstones, ID reuse, compaction, and
//! overlay mutation are out of scope.
// kani-skip: builder freeze/export allocates variable-sized BCSR/property snapshots; randomized
// proptests cover set and strict-validation invariants here.
#![cfg_attr(
    not(test),
    expect(
        clippy::missing_docs_in_private_items,
        reason = "hypergraph builder internals are documented at the public type and trait capability boundaries"
    )
)]
#![expect(
    clippy::missing_errors_doc,
    reason = "hypergraph builder errors are enumerated by HyperBuildError variants"
)]
#![expect(
    clippy::missing_const_for_fn,
    reason = "builder constructors stay regular functions while generic owned fields evolve"
)]

use std::{error::Error, fmt, vec, vec::Vec};

use oxgraph_hyper::{
    CanonicalElementIdentity, CanonicalIncidenceIdentity, CanonicalRelationIdentity,
    ContainsElement, ContainsIncidence, ContainsRelation, DirectedHyperedgeIncidences,
    DirectedHyperedgeParticipants, DirectedVertexHyperedges, ElementIncidenceCount,
    ElementIncidences, ElementIndex, ElementPredecessors, ElementSuccessors, ElementWeight,
    HyperedgeParticipantCount, HyperedgeParticipants, HypergraphCounts, IncidenceBase,
    IncidenceCounts, IncidenceElement, IncidenceIndex, IncidenceRelation, IncidenceRole,
    IncidenceWeight, IncidentHyperedgeCount, IncidentHyperedges, LocalElementIdentity,
    LocalIncidenceIdentity, LocalRelationIdentity, RelationIncidenceCount, RelationIncidences,
    RelationIndex, RelationWeight, TopologyBase, TopologyCounts,
};
use oxgraph_hyper_bcsr::{
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
};
use oxgraph_property::{
    EncodedPropertySnapshot, IdFamily, IdentityModeRecord, PropertyError, PropertyLayer,
    SNAPSHOT_KIND_IDENTITY_MODES, SNAPSHOT_KIND_PROPERTY_DATA, SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
    SNAPSHOT_PROPERTY_VERSION, encode_property_snapshot,
};
use oxgraph_snapshot::{PlanError, SnapshotBuilder};

/// Local/canonical vertex ID assigned by [`HypergraphBuilder`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperVertexId(pub u32);

/// Local/canonical hyperedge ID assigned by [`HypergraphBuilder`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperedgeId(pub u32);

/// Local/canonical participant ID assigned when a hypergraph is frozen.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperParticipantId(pub u32);

/// Role of a participant in a directed hyperedge.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum HyperParticipantRole {
    /// Source-side participant.
    Source,
    /// Target-side participant.
    Target,
}

/// Errors raised by hypergraph building and freeze/export operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum HyperBuildError {
    /// A vertex ID was not visible.
    InvalidVertex {
        /// Invalid vertex ID.
        vertex: HyperVertexId,
    },
    /// A hyperedge ID was not visible.
    InvalidHyperedge {
        /// Invalid hyperedge ID.
        hyperedge: HyperedgeId,
    },
    /// A participant ID was not visible.
    InvalidParticipant {
        /// Invalid participant ID.
        participant: HyperParticipantId,
    },
    /// A participant position was outside a hyperedge participant set.
    ParticipantPositionOutOfBounds {
        /// Hyperedge whose participant was requested.
        hyperedge: HyperedgeId,
        /// Hyperedge-local participant position.
        position: usize,
    },
    /// A source or target participant list repeated the same vertex.
    DuplicateParticipant {
        /// Repeated vertex ID.
        vertex: HyperVertexId,
        /// Role whose participant set contained the duplicate.
        role: HyperParticipantRole,
    },
    /// A first-generation `u32` ID or offset overflowed.
    IdOverflow {
        /// Value that did not fit in `u32`.
        value: usize,
    },
    /// An attached property layer used an unknown future ID family.
    UnsupportedPropertyFamily {
        /// Unsupported family.
        id_family: IdFamily,
    },
    /// An attached property layer was too short for the frozen topology.
    PropertyLayerTooShort {
        /// Property layer ID family.
        id_family: IdFamily,
        /// Required logical length.
        required: usize,
        /// Actual logical length.
        actual: usize,
    },
    /// Property snapshot encoding failed.
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

impl fmt::Display for HyperBuildError {
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
            Self::UnsupportedPropertyFamily { id_family } => write!(
                formatter,
                "unsupported hypergraph property family {id_family:?}"
            ),
            Self::PropertyLayerTooShort {
                id_family,
                required,
                actual,
            } => write!(
                formatter,
                "hypergraph property layer for {id_family:?} too short: required {required}, got {actual}"
            ),
            Self::Property { source } => {
                write!(formatter, "property snapshot export failed: {source}")
            }
            Self::SnapshotPlan { source } => write!(formatter, "snapshot export failed: {source}"),
        }
    }
}

impl Error for HyperBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Property { source } => Some(source),
            Self::SnapshotPlan { source } => Some(source),
            Self::InvalidVertex { .. }
            | Self::InvalidHyperedge { .. }
            | Self::InvalidParticipant { .. }
            | Self::ParticipantPositionOutOfBounds { .. }
            | Self::DuplicateParticipant { .. }
            | Self::IdOverflow { .. }
            | Self::UnsupportedPropertyFamily { .. }
            | Self::PropertyLayerTooShort { .. } => None,
        }
    }
}

impl From<PlanError> for HyperBuildError {
    fn from(source: PlanError) -> Self {
        Self::SnapshotPlan { source }
    }
}

impl From<PropertyError> for HyperBuildError {
    fn from(source: PropertyError) -> Self {
        Self::Property { source }
    }
}

#[derive(Clone, Debug, Default)]
struct HyperedgeRecord<IW> {
    sources: Vec<u32>,
    targets: Vec<u32>,
    source_weights: Vec<IW>,
    target_weights: Vec<IW>,
}

#[derive(Clone, Debug, Default)]
struct NormalizedHyperedgeRecord<IW> {
    sources: Vec<(u32, IW)>,
    targets: Vec<(u32, IW)>,
}

/// Append/update-only directed hypergraph builder.
///
/// `EW`, `RW`, and `IW` are element, relation, and incidence weight types.
/// Callers provide default weights at construction; the builder does not assume
/// any scalar type or default value.
#[derive(Clone, Debug)]
#[must_use]
pub struct HypergraphBuilder<EW, RW, IW> {
    vertex_count: u32,
    default_element_weight: EW,
    default_relation_weight: RW,
    default_incidence_weight: IW,
    element_weights: Vec<EW>,
    hyperedges: Vec<HyperedgeRecord<IW>>,
    relation_weights: Vec<RW>,
    property_layers: Vec<PropertyLayer>,
    generation: u64,
}

impl<EW: Clone, RW: Clone, IW: Clone> HypergraphBuilder<EW, RW, IW> {
    /// Constructs an empty hypergraph builder with caller-provided default weights.
    pub fn new(
        default_element_weight: EW,
        default_relation_weight: RW,
        default_incidence_weight: IW,
    ) -> Self {
        Self {
            vertex_count: 0,
            default_element_weight,
            default_relation_weight,
            default_incidence_weight,
            element_weights: Vec::new(),
            hyperedges: Vec::new(),
            relation_weights: Vec::new(),
            property_layers: Vec::new(),
            generation: 0,
        }
    }

    /// Returns the current edit generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Adds one isolated vertex.
    pub fn add_vertex(&mut self) -> Result<HyperVertexId, HyperBuildError> {
        let id = self.vertex_count;
        self.vertex_count = self
            .vertex_count
            .checked_add(1)
            .ok_or(HyperBuildError::IdOverflow { value: usize::MAX })?;
        self.element_weights
            .push(self.default_element_weight.clone());
        self.generation = self.generation.wrapping_add(1);
        Ok(HyperVertexId(id))
    }

    /// Adds one directed hyperedge.
    pub fn add_hyperedge(
        &mut self,
        sources: &[HyperVertexId],
        targets: &[HyperVertexId],
    ) -> Result<HyperedgeId, HyperBuildError> {
        for &source in sources {
            self.ensure_vertex(source)?;
        }
        for &target in targets {
            self.ensure_vertex(target)?;
        }
        ensure_unique_participants(sources, HyperParticipantRole::Source)?;
        ensure_unique_participants(targets, HyperParticipantRole::Target)?;
        let hyperedge = usize_to_u32(self.hyperedges.len())?;
        self.hyperedges.push(HyperedgeRecord {
            sources: sources.iter().map(|vertex| vertex.0).collect(),
            targets: targets.iter().map(|vertex| vertex.0).collect(),
            source_weights: vec![self.default_incidence_weight.clone(); sources.len()],
            target_weights: vec![self.default_incidence_weight.clone(); targets.len()],
        });
        self.relation_weights
            .push(self.default_relation_weight.clone());
        self.generation = self.generation.wrapping_add(1);
        Ok(HyperedgeId(hyperedge))
    }

    /// Updates a vertex element weight.
    pub fn set_element_weight(
        &mut self,
        vertex: HyperVertexId,
        weight: EW,
    ) -> Result<(), HyperBuildError> {
        self.ensure_vertex(vertex)?;
        self.element_weights[vertex.0 as usize] = weight;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Updates a hyperedge relation weight.
    pub fn set_relation_weight(
        &mut self,
        hyperedge: HyperedgeId,
        weight: RW,
    ) -> Result<(), HyperBuildError> {
        let index = self.hyperedge_index_checked(hyperedge)?;
        self.relation_weights[index] = weight;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Updates one source-side incidence weight by hyperedge-local position.
    pub fn set_source_incidence_weight(
        &mut self,
        hyperedge: HyperedgeId,
        position: usize,
        weight: IW,
    ) -> Result<(), HyperBuildError> {
        let index = self.hyperedge_index_checked(hyperedge)?;
        let Some(slot) = self.hyperedges[index].source_weights.get_mut(position) else {
            return Err(HyperBuildError::ParticipantPositionOutOfBounds {
                hyperedge,
                position,
            });
        };
        *slot = weight;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Updates one target-side incidence weight by hyperedge-local position.
    pub fn set_target_incidence_weight(
        &mut self,
        hyperedge: HyperedgeId,
        position: usize,
        weight: IW,
    ) -> Result<(), HyperBuildError> {
        let index = self.hyperedge_index_checked(hyperedge)?;
        let Some(slot) = self.hyperedges[index].target_weights.get_mut(position) else {
            return Err(HyperBuildError::ParticipantPositionOutOfBounds {
                hyperedge,
                position,
            });
        };
        *slot = weight;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Attaches a named Arrow property layer for snapshot export.
    pub fn add_property_layer(&mut self, layer: PropertyLayer) {
        self.property_layers.push(layer);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Returns the number of vertices assigned so far.
    #[must_use]
    pub const fn vertex_count(&self) -> usize {
        self.vertex_count as usize
    }

    /// Returns the number of hyperedges assigned so far.
    #[must_use]
    pub const fn hyperedge_count(&self) -> usize {
        self.hyperedges.len()
    }

    /// Freezes the current builder contents into an owned immutable view.
    pub fn freeze(&self) -> Result<FrozenHypergraph<EW, RW, IW>, HyperBuildError> {
        let normalized = self.normalized_hyperedges();
        let participant_count = normalized
            .iter()
            .map(|record| record.sources.len() + record.targets.len())
            .sum();
        self.validate_property_layers(participant_count)?;

        let mut head_offsets = Vec::with_capacity(self.hyperedges.len() + 1);
        let mut tail_offsets = Vec::with_capacity(self.hyperedges.len() + 1);
        let mut relation_offsets = Vec::with_capacity(self.hyperedges.len() + 1);
        let mut head_participants = Vec::new();
        let mut tail_participants = Vec::new();
        let mut participant_elements = Vec::with_capacity(participant_count);
        let mut participant_relations = Vec::with_capacity(participant_count);
        let mut participant_roles = Vec::with_capacity(participant_count);
        let mut incidence_weights = Vec::with_capacity(participant_count);

        head_offsets.push(0);
        tail_offsets.push(0);
        relation_offsets.push(0);
        for (relation, record) in normalized.iter().enumerate() {
            let relation_u32 = usize_to_u32(relation)?;
            for &(vertex, ref weight) in &record.sources {
                head_participants.push(vertex);
                participant_elements.push(vertex);
                participant_relations.push(relation_u32);
                participant_roles.push(HyperParticipantRole::Source);
                incidence_weights.push(weight.clone());
            }
            for &(vertex, ref weight) in &record.targets {
                tail_participants.push(vertex);
                participant_elements.push(vertex);
                participant_relations.push(relation_u32);
                participant_roles.push(HyperParticipantRole::Target);
                incidence_weights.push(weight.clone());
            }
            head_offsets.push(usize_to_u32(head_participants.len())?);
            tail_offsets.push(usize_to_u32(tail_participants.len())?);
            relation_offsets.push(usize_to_u32(participant_elements.len())?);
        }

        let (vertex_outgoing_offsets, vertex_outgoing_hyperedges) = build_vertex_relation_index(
            self.vertex_count(),
            &normalized,
            HyperParticipantRole::Source,
        )?;
        let (vertex_incoming_offsets, vertex_incoming_hyperedges) = build_vertex_relation_index(
            self.vertex_count(),
            &normalized,
            HyperParticipantRole::Target,
        )?;
        let (element_incidence_offsets, element_incidence_ids) =
            build_element_incidence_index(self.vertex_count(), &participant_elements)?;

        Ok(FrozenHypergraph {
            vertex_count: self.vertex_count,
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
            element_weights: self.element_weights.clone().into_boxed_slice(),
            relation_weights: self.relation_weights.clone().into_boxed_slice(),
            incidence_weights: incidence_weights.into_boxed_slice(),
            property_layers: self.property_layers.clone().into_boxed_slice(),
        })
    }

    fn normalized_hyperedges(&self) -> Vec<NormalizedHyperedgeRecord<IW>> {
        self.hyperedges
            .iter()
            .map(|record| {
                let mut sources: Vec<(u32, IW)> = record
                    .sources
                    .iter()
                    .copied()
                    .zip(record.source_weights.iter().cloned())
                    .collect();
                let mut targets: Vec<(u32, IW)> = record
                    .targets
                    .iter()
                    .copied()
                    .zip(record.target_weights.iter().cloned())
                    .collect();
                sources.sort_by_key(|(vertex, _weight)| *vertex);
                targets.sort_by_key(|(vertex, _weight)| *vertex);
                NormalizedHyperedgeRecord { sources, targets }
            })
            .collect()
    }

    fn validate_property_layers(&self, participant_count: usize) -> Result<(), HyperBuildError> {
        for layer in &self.property_layers {
            let required = match layer.descriptor().id_family {
                IdFamily::Element => self.vertex_count(),
                IdFamily::Relation => self.hyperedge_count(),
                IdFamily::Incidence => participant_count,
                unsupported => {
                    return Err(HyperBuildError::UnsupportedPropertyFamily {
                        id_family: unsupported,
                    });
                }
            };
            if layer.len() < required {
                return Err(HyperBuildError::PropertyLayerTooShort {
                    id_family: layer.descriptor().id_family,
                    required,
                    actual: layer.len(),
                });
            }
        }
        Ok(())
    }

    const fn ensure_vertex(&self, vertex: HyperVertexId) -> Result<(), HyperBuildError> {
        if vertex.0 < self.vertex_count {
            Ok(())
        } else {
            Err(HyperBuildError::InvalidVertex { vertex })
        }
    }

    const fn hyperedge_index_checked(
        &self,
        hyperedge: HyperedgeId,
    ) -> Result<usize, HyperBuildError> {
        let index = hyperedge.0 as usize;
        if index < self.hyperedges.len() {
            Ok(index)
        } else {
            Err(HyperBuildError::InvalidHyperedge { hyperedge })
        }
    }
}

/// Owned immutable directed hypergraph produced by [`HypergraphBuilder::freeze`].
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenHypergraph<EW, RW, IW> {
    vertex_count: u32,
    head_offsets: Box<[u32]>,
    head_participants: Box<[u32]>,
    tail_offsets: Box<[u32]>,
    tail_participants: Box<[u32]>,
    vertex_outgoing_offsets: Box<[u32]>,
    vertex_outgoing_hyperedges: Box<[u32]>,
    vertex_incoming_offsets: Box<[u32]>,
    vertex_incoming_hyperedges: Box<[u32]>,
    relation_offsets: Box<[u32]>,
    participant_elements: Box<[u32]>,
    participant_relations: Box<[u32]>,
    participant_roles: Box<[HyperParticipantRole]>,
    element_incidence_offsets: Box<[u32]>,
    element_incidence_ids: Box<[u32]>,
    element_weights: Box<[EW]>,
    relation_weights: Box<[RW]>,
    incidence_weights: Box<[IW]>,
    property_layers: Box<[PropertyLayer]>,
}

impl<EW, RW, IW> FrozenHypergraph<EW, RW, IW> {
    /// Returns attached property layers.
    pub fn property_layers(&self) -> &[PropertyLayer] {
        &self.property_layers
    }

    /// Exports BCSR topology, identity, and attached property sections.
    pub fn to_bcsr_snapshot(&self) -> Result<Vec<u8>, HyperBuildError> {
        let property = self.encode_property_snapshot()?;
        let identity_modes = [
            IdentityModeRecord::local_equals_canonical(IdFamily::Element, self.vertex_count()),
            IdentityModeRecord::local_equals_canonical(IdFamily::Relation, self.hyperedge_count()),
            IdentityModeRecord::local_equals_canonical(
                IdFamily::Incidence,
                self.participant_elements.len(),
            ),
        ];
        let mut builder = SnapshotBuilder::new();
        builder.add_section_typed(SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, 1, &self.head_offsets)?;
        builder.add_section_typed(
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
            1,
            &self.head_participants,
        )?;
        builder.add_section_typed(SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, 1, &self.tail_offsets)?;
        builder.add_section_typed(
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
            1,
            &self.tail_participants,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
            1,
            &self.vertex_outgoing_offsets,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
            1,
            &self.vertex_outgoing_hyperedges,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
            1,
            &self.vertex_incoming_offsets,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
            1,
            &self.vertex_incoming_hyperedges,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_IDENTITY_MODES,
            SNAPSHOT_PROPERTY_VERSION,
            &identity_modes,
        )?;
        if let Some(property) = property {
            builder.add_section(
                SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
                SNAPSHOT_PROPERTY_VERSION,
                0,
                property.descriptors,
            )?;
            builder.add_section(
                SNAPSHOT_KIND_PROPERTY_DATA,
                SNAPSHOT_PROPERTY_VERSION,
                0,
                property.data,
            )?;
        }
        builder.finish().map_err(HyperBuildError::from)
    }

    fn encode_property_snapshot(&self) -> Result<Option<EncodedPropertySnapshot>, HyperBuildError> {
        if self.property_layers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(encode_property_snapshot(&self.property_layers)?))
        }
    }

    const fn vertex_slot(vertex: HyperVertexId) -> usize {
        vertex.0 as usize
    }
    const fn hyperedge_slot(hyperedge: HyperedgeId) -> usize {
        hyperedge.0 as usize
    }
    const fn participant_slot(participant: HyperParticipantId) -> usize {
        participant.0 as usize
    }

    fn head_range(&self, hyperedge: HyperedgeId) -> core::ops::Range<usize> {
        let slot = Self::hyperedge_slot(hyperedge);
        self.head_offsets[slot] as usize..self.head_offsets[slot + 1] as usize
    }

    fn tail_range(&self, hyperedge: HyperedgeId) -> core::ops::Range<usize> {
        let slot = Self::hyperedge_slot(hyperedge);
        self.tail_offsets[slot] as usize..self.tail_offsets[slot + 1] as usize
    }

    fn relation_incidence_range(&self, hyperedge: HyperedgeId) -> core::ops::Range<usize> {
        let slot = Self::hyperedge_slot(hyperedge);
        self.relation_offsets[slot] as usize..self.relation_offsets[slot + 1] as usize
    }

    fn element_incidence_range(&self, vertex: HyperVertexId) -> core::ops::Range<usize> {
        let slot = Self::vertex_slot(vertex);
        self.element_incidence_offsets[slot] as usize
            ..self.element_incidence_offsets[slot + 1] as usize
    }

    fn outgoing_hyperedge_range(&self, vertex: HyperVertexId) -> core::ops::Range<usize> {
        let slot = Self::vertex_slot(vertex);
        self.vertex_outgoing_offsets[slot] as usize..self.vertex_outgoing_offsets[slot + 1] as usize
    }

    fn incoming_hyperedge_range(&self, vertex: HyperVertexId) -> core::ops::Range<usize> {
        let slot = Self::vertex_slot(vertex);
        self.vertex_incoming_offsets[slot] as usize..self.vertex_incoming_offsets[slot + 1] as usize
    }
}

impl<EW, RW, IW> TopologyBase for FrozenHypergraph<EW, RW, IW> {
    type ElementId = HyperVertexId;
    type RelationId = HyperedgeId;
}

impl<EW, RW, IW> IncidenceBase for FrozenHypergraph<EW, RW, IW> {
    type IncidenceId = HyperParticipantId;
    type Role = HyperParticipantRole;
}

impl<EW, RW, IW> TopologyCounts for FrozenHypergraph<EW, RW, IW> {
    fn element_count(&self) -> usize {
        self.vertex_count as usize
    }
    fn relation_count(&self) -> usize {
        self.relation_weights.len()
    }
}

impl<EW, RW, IW> HypergraphCounts for FrozenHypergraph<EW, RW, IW> {}

impl<EW, RW, IW> IncidenceCounts for FrozenHypergraph<EW, RW, IW> {
    fn incidence_count(&self) -> usize {
        self.participant_elements.len()
    }
}

impl<EW, RW, IW> ElementIndex for FrozenHypergraph<EW, RW, IW> {
    fn element_bound(&self) -> usize {
        self.vertex_count as usize
    }
    fn element_index(&self, element: HyperVertexId) -> usize {
        element.0 as usize
    }
}

impl<EW, RW, IW> RelationIndex for FrozenHypergraph<EW, RW, IW> {
    fn relation_bound(&self) -> usize {
        self.relation_weights.len()
    }
    fn relation_index(&self, relation: HyperedgeId) -> usize {
        relation.0 as usize
    }
}

impl<EW, RW, IW> IncidenceIndex for FrozenHypergraph<EW, RW, IW> {
    fn incidence_bound(&self) -> usize {
        self.participant_elements.len()
    }
    fn incidence_index(&self, incidence: HyperParticipantId) -> usize {
        incidence.0 as usize
    }
}

impl<EW, RW, IW> ContainsElement for FrozenHypergraph<EW, RW, IW> {
    fn contains_element(&self, element: HyperVertexId) -> bool {
        element.0 < self.vertex_count
    }
}

impl<EW, RW, IW> ContainsRelation for FrozenHypergraph<EW, RW, IW> {
    fn contains_relation(&self, relation: HyperedgeId) -> bool {
        (relation.0 as usize) < self.relation_weights.len()
    }
}

impl<EW, RW, IW> ContainsIncidence for FrozenHypergraph<EW, RW, IW> {
    fn contains_incidence(&self, incidence: HyperParticipantId) -> bool {
        (incidence.0 as usize) < self.participant_elements.len()
    }
}

impl<EW, RW, IW> IncidenceElement for FrozenHypergraph<EW, RW, IW> {
    fn incidence_element(&self, incidence: HyperParticipantId) -> HyperVertexId {
        HyperVertexId(self.participant_elements[Self::participant_slot(incidence)])
    }
}

impl<EW, RW, IW> IncidenceRelation for FrozenHypergraph<EW, RW, IW> {
    fn incidence_relation(&self, incidence: HyperParticipantId) -> HyperedgeId {
        HyperedgeId(self.participant_relations[Self::participant_slot(incidence)])
    }
}

impl<EW, RW, IW> IncidenceRole for FrozenHypergraph<EW, RW, IW> {
    fn incidence_role(&self, incidence: HyperParticipantId) -> HyperParticipantRole {
        self.participant_roles[Self::participant_slot(incidence)]
    }
}

impl<EW, RW, IW> RelationIncidences for FrozenHypergraph<EW, RW, IW> {
    type Incidences<'view>
        = ParticipantRangeIter
    where
        Self: 'view;
    fn relation_incidences(&self, relation: HyperedgeId) -> Self::Incidences<'_> {
        ParticipantRangeIter::new(self.relation_incidence_range(relation))
    }
}

impl<EW, RW, IW> ElementIncidences for FrozenHypergraph<EW, RW, IW> {
    type Incidences<'view>
        = ParticipantSliceIter<'view>
    where
        Self: 'view;
    fn element_incidences(&self, element: HyperVertexId) -> Self::Incidences<'_> {
        ParticipantSliceIter {
            inner: self.element_incidence_ids[self.element_incidence_range(element)].iter(),
        }
    }
}

impl<EW, RW, IW> RelationIncidenceCount for FrozenHypergraph<EW, RW, IW> {
    fn relation_incidence_count(&self, relation: HyperedgeId) -> usize {
        self.relation_incidence_range(relation).len()
    }
}

impl<EW, RW, IW> ElementIncidenceCount for FrozenHypergraph<EW, RW, IW> {
    fn element_incidence_count(&self, element: HyperVertexId) -> usize {
        self.element_incidence_range(element).len()
    }
}

impl<EW, RW, IW> HyperedgeParticipants for FrozenHypergraph<EW, RW, IW> {
    type Participants<'view>
        = std::iter::Chain<VertexSliceIter<'view>, VertexSliceIter<'view>>
    where
        Self: 'view;
    fn hyperedge_participants(&self, hyperedge: HyperedgeId) -> Self::Participants<'_> {
        self.source_participants(hyperedge)
            .chain(self.target_participants(hyperedge))
    }
}

impl<EW, RW, IW> IncidentHyperedges for FrozenHypergraph<EW, RW, IW> {
    type IncidentHyperedges<'view>
        = IncidentHyperedgeIter<'view>
    where
        Self: 'view;
    fn incident_hyperedges(&self, vertex: HyperVertexId) -> Self::IncidentHyperedges<'_> {
        IncidentHyperedgeIter {
            incidences: self.element_incidences(vertex),
            participant_relations: &self.participant_relations,
        }
    }
}

impl<EW, RW, IW> HyperedgeParticipantCount for FrozenHypergraph<EW, RW, IW> {
    fn hyperedge_participant_count(&self, hyperedge: HyperedgeId) -> usize {
        self.relation_incidence_count(hyperedge)
    }
}

impl<EW, RW, IW> IncidentHyperedgeCount for FrozenHypergraph<EW, RW, IW> {
    fn incident_hyperedge_count(&self, vertex: HyperVertexId) -> usize {
        self.element_incidence_count(vertex)
    }
}

impl<EW, RW, IW> DirectedHyperedgeParticipants for FrozenHypergraph<EW, RW, IW> {
    type SourceParticipants<'view>
        = VertexSliceIter<'view>
    where
        Self: 'view;
    type TargetParticipants<'view>
        = VertexSliceIter<'view>
    where
        Self: 'view;
    fn source_participants(&self, hyperedge: HyperedgeId) -> Self::SourceParticipants<'_> {
        VertexSliceIter {
            inner: self.head_participants[self.head_range(hyperedge)].iter(),
        }
    }
    fn target_participants(&self, hyperedge: HyperedgeId) -> Self::TargetParticipants<'_> {
        VertexSliceIter {
            inner: self.tail_participants[self.tail_range(hyperedge)].iter(),
        }
    }
}

impl<EW, RW, IW> DirectedHyperedgeIncidences for FrozenHypergraph<EW, RW, IW> {
    type SourceIncidences<'view>
        = ParticipantRangeIter
    where
        Self: 'view;
    type TargetIncidences<'view>
        = ParticipantRangeIter
    where
        Self: 'view;
    fn source_incidences(&self, hyperedge: HyperedgeId) -> Self::SourceIncidences<'_> {
        let start = self.relation_offsets[Self::hyperedge_slot(hyperedge)] as usize;
        let len = self.head_range(hyperedge).len();
        ParticipantRangeIter::new(start..start + len)
    }
    fn target_incidences(&self, hyperedge: HyperedgeId) -> Self::TargetIncidences<'_> {
        let start = self.relation_offsets[Self::hyperedge_slot(hyperedge)] as usize
            + self.head_range(hyperedge).len();
        let len = self.tail_range(hyperedge).len();
        ParticipantRangeIter::new(start..start + len)
    }
}

impl<EW, RW, IW> DirectedVertexHyperedges for FrozenHypergraph<EW, RW, IW> {
    type OutgoingHyperedges<'view>
        = HyperedgeSliceIter<'view>
    where
        Self: 'view;
    type IncomingHyperedges<'view>
        = HyperedgeSliceIter<'view>
    where
        Self: 'view;
    fn outgoing_hyperedges(&self, vertex: HyperVertexId) -> Self::OutgoingHyperedges<'_> {
        HyperedgeSliceIter {
            inner: self.vertex_outgoing_hyperedges[self.outgoing_hyperedge_range(vertex)].iter(),
        }
    }
    fn incoming_hyperedges(&self, vertex: HyperVertexId) -> Self::IncomingHyperedges<'_> {
        HyperedgeSliceIter {
            inner: self.vertex_incoming_hyperedges[self.incoming_hyperedge_range(vertex)].iter(),
        }
    }
}

impl<EW, RW, IW> ElementSuccessors for FrozenHypergraph<EW, RW, IW> {
    type Successors<'view>
        = SuccessorIter<'view, EW, RW, IW>
    where
        Self: 'view;
    fn element_successors(&self, element: HyperVertexId) -> Self::Successors<'_> {
        SuccessorIter {
            graph: self,
            hyperedges: self.outgoing_hyperedges(element),
            current: None,
        }
    }
}

impl<EW, RW, IW> ElementPredecessors for FrozenHypergraph<EW, RW, IW> {
    type Predecessors<'view>
        = PredecessorIter<'view, EW, RW, IW>
    where
        Self: 'view;
    fn element_predecessors(&self, element: HyperVertexId) -> Self::Predecessors<'_> {
        PredecessorIter {
            graph: self,
            hyperedges: self.incoming_hyperedges(element),
            current: None,
        }
    }
}

impl<EW: Copy, RW, IW> ElementWeight for FrozenHypergraph<EW, RW, IW> {
    type Weight = EW;
    fn element_weight(&self, element: HyperVertexId) -> Self::Weight {
        self.element_weights[Self::vertex_slot(element)]
    }
}

impl<EW, RW: Copy, IW> RelationWeight for FrozenHypergraph<EW, RW, IW> {
    type Weight = RW;
    fn relation_weight(&self, relation: HyperedgeId) -> Self::Weight {
        self.relation_weights[Self::hyperedge_slot(relation)]
    }
}

impl<EW, RW, IW: Copy> IncidenceWeight for FrozenHypergraph<EW, RW, IW> {
    type Weight = IW;
    fn incidence_weight(&self, incidence: HyperParticipantId) -> Self::Weight {
        self.incidence_weights[Self::participant_slot(incidence)]
    }
}

impl<EW, RW, IW> CanonicalElementIdentity for FrozenHypergraph<EW, RW, IW> {
    type CanonicalElementId = HyperVertexId;
    fn canonical_element_id(&self, element: HyperVertexId) -> Self::CanonicalElementId {
        element
    }
}

impl<EW, RW, IW> LocalElementIdentity for FrozenHypergraph<EW, RW, IW> {
    fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId> {
        self.contains_element(canonical).then_some(canonical)
    }
}

impl<EW, RW, IW> CanonicalRelationIdentity for FrozenHypergraph<EW, RW, IW> {
    type CanonicalRelationId = HyperedgeId;
    fn canonical_relation_id(&self, relation: HyperedgeId) -> Self::CanonicalRelationId {
        relation
    }
}

impl<EW, RW, IW> LocalRelationIdentity for FrozenHypergraph<EW, RW, IW> {
    fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId> {
        self.contains_relation(canonical).then_some(canonical)
    }
}

impl<EW, RW, IW> CanonicalIncidenceIdentity for FrozenHypergraph<EW, RW, IW> {
    type CanonicalIncidenceId = HyperParticipantId;
    fn canonical_incidence_id(&self, incidence: HyperParticipantId) -> Self::CanonicalIncidenceId {
        incidence
    }
}

impl<EW, RW, IW> LocalIncidenceIdentity for FrozenHypergraph<EW, RW, IW> {
    fn local_incidence_id(
        &self,
        canonical: Self::CanonicalIncidenceId,
    ) -> Option<Self::IncidenceId> {
        self.contains_incidence(canonical).then_some(canonical)
    }
}

/// Iterator over vertex IDs stored as `u32` words.
pub struct VertexSliceIter<'view> {
    inner: core::slice::Iter<'view, u32>,
}
impl Iterator for VertexSliceIter<'_> {
    type Item = HyperVertexId;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(HyperVertexId)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl ExactSizeIterator for VertexSliceIter<'_> {}

/// Iterator over hyperedge IDs stored as `u32` words.
pub struct HyperedgeSliceIter<'view> {
    inner: core::slice::Iter<'view, u32>,
}
impl Iterator for HyperedgeSliceIter<'_> {
    type Item = HyperedgeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(HyperedgeId)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl ExactSizeIterator for HyperedgeSliceIter<'_> {}

/// Iterator over participant IDs stored as `u32` words.
pub struct ParticipantSliceIter<'view> {
    inner: core::slice::Iter<'view, u32>,
}
impl Iterator for ParticipantSliceIter<'_> {
    type Item = HyperParticipantId;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(HyperParticipantId)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl ExactSizeIterator for ParticipantSliceIter<'_> {}

/// Iterator over contiguous participant IDs.
pub struct ParticipantRangeIter {
    next: usize,
    end: usize,
}
impl ParticipantRangeIter {
    fn new(range: core::ops::Range<usize>) -> Self {
        Self {
            next: range.start,
            end: range.end,
        }
    }
}
impl Iterator for ParticipantRangeIter {
    type Item = HyperParticipantId;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            None
        } else {
            let value = self.next;
            self.next += 1;
            Some(HyperParticipantId(u32::try_from(value).ok()?))
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.end - self.next;
        (len, Some(len))
    }
}
impl ExactSizeIterator for ParticipantRangeIter {}

/// Iterator that maps vertex incidences to hyperedges.
pub struct IncidentHyperedgeIter<'view> {
    incidences: ParticipantSliceIter<'view>,
    participant_relations: &'view [u32],
}
impl Iterator for IncidentHyperedgeIter<'_> {
    type Item = HyperedgeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.incidences
            .next()
            .map(|id| HyperedgeId(self.participant_relations[id.0 as usize]))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.incidences.size_hint()
    }
}
impl ExactSizeIterator for IncidentHyperedgeIter<'_> {}

/// Iterator over successor vertices reached via outgoing hyperedges.
pub struct SuccessorIter<'view, EW, RW, IW> {
    graph: &'view FrozenHypergraph<EW, RW, IW>,
    hyperedges: HyperedgeSliceIter<'view>,
    current: Option<VertexSliceIter<'view>>,
}
impl<EW, RW, IW> Iterator for SuccessorIter<'_, EW, RW, IW> {
    type Item = HyperVertexId;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = &mut self.current
                && let Some(vertex) = current.next()
            {
                return Some(vertex);
            }
            let hyperedge = self.hyperedges.next()?;
            self.current = Some(self.graph.target_participants(hyperedge));
        }
    }
}

/// Iterator over predecessor vertices reached via incoming hyperedges.
pub struct PredecessorIter<'view, EW, RW, IW> {
    graph: &'view FrozenHypergraph<EW, RW, IW>,
    hyperedges: HyperedgeSliceIter<'view>,
    current: Option<VertexSliceIter<'view>>,
}
impl<EW, RW, IW> Iterator for PredecessorIter<'_, EW, RW, IW> {
    type Item = HyperVertexId;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = &mut self.current
                && let Some(vertex) = current.next()
            {
                return Some(vertex);
            }
            let hyperedge = self.hyperedges.next()?;
            self.current = Some(self.graph.source_participants(hyperedge));
        }
    }
}

fn ensure_unique_participants(
    participants: &[HyperVertexId],
    role: HyperParticipantRole,
) -> Result<(), HyperBuildError> {
    let mut sorted: Vec<u32> = participants.iter().map(|vertex| vertex.0).collect();
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

fn build_vertex_relation_index<IW: Clone>(
    vertex_count: usize,
    records: &[NormalizedHyperedgeRecord<IW>],
    role: HyperParticipantRole,
) -> Result<(Vec<u32>, Vec<u32>), HyperBuildError> {
    let mut buckets = vec![Vec::<u32>::new(); vertex_count];
    for (relation, record) in records.iter().enumerate() {
        let relation_id = usize_to_u32(relation)?;
        let participants = match role {
            HyperParticipantRole::Source => &record.sources,
            HyperParticipantRole::Target => &record.targets,
        };
        for &(vertex, _) in participants {
            buckets[vertex as usize].push(relation_id);
        }
    }
    let mut offsets = Vec::with_capacity(vertex_count + 1);
    let mut hyperedges = Vec::new();
    offsets.push(0);
    for bucket in buckets {
        hyperedges.extend(bucket);
        offsets.push(usize_to_u32(hyperedges.len())?);
    }
    Ok((offsets, hyperedges))
}

fn build_element_incidence_index(
    vertex_count: usize,
    participant_elements: &[u32],
) -> Result<(Vec<u32>, Vec<u32>), HyperBuildError> {
    let mut buckets = vec![Vec::<u32>::new(); vertex_count];
    for (participant, &vertex) in participant_elements.iter().enumerate() {
        buckets[vertex as usize].push(usize_to_u32(participant)?);
    }
    let mut offsets = Vec::with_capacity(vertex_count + 1);
    let mut incidences = Vec::new();
    offsets.push(0);
    for bucket in buckets {
        incidences.extend(bucket);
        offsets.push(usize_to_u32(incidences.len())?);
    }
    Ok((offsets, incidences))
}

fn usize_to_u32(value: usize) -> Result<u32, HyperBuildError> {
    u32::try_from(value).map_err(|_error| HyperBuildError::IdOverflow { value })
}

#[cfg(test)]
mod tests {
    use arrow_array::Int32Array;
    use arrow_schema::{DataType, Field};
    use oxgraph_property::{
        LayerId, LayerRole, PropertyLayerDescriptor, StorageMode, validate_identity_snapshot,
        validate_property_snapshot,
    };
    use oxgraph_snapshot::Snapshot;

    use super::*;

    #[test]
    fn freeze_preserves_generic_weights() -> Result<(), HyperBuildError> {
        let mut builder = HypergraphBuilder::new(1_i32, 2_u16, 3_i8);
        let a = builder.add_vertex()?;
        let b = builder.add_vertex()?;
        let edge = builder.add_hyperedge(&[a], &[b])?;
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

    #[test]
    fn snapshot_includes_identity_and_property_sections() -> Result<(), Box<dyn Error>> {
        let mut builder = HypergraphBuilder::new((), (), ());
        let a = builder.add_vertex()?;
        let b = builder.add_vertex()?;
        builder.add_hyperedge(&[a], &[b])?;
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(1),
            "vertex_marker",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("vertex_marker", DataType::Int32, false),
        )?;
        builder.add_property_layer(PropertyLayer::try_new_dense(
            descriptor,
            std::sync::Arc::new(Int32Array::from(vec![1_i32, 2_i32])),
        )?);
        let frozen = builder.freeze()?;
        let bytes = frozen.to_bcsr_snapshot()?;
        let snapshot = Snapshot::open(&bytes)?;
        assert_eq!(validate_identity_snapshot(&snapshot)?.records.len(), 3);
        assert_eq!(validate_property_snapshot(&snapshot)?.layer_count, 1);
        Ok(())
    }
}

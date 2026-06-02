//! Canonical record value types and the id-allocator watermark.
//!
//! After the base+overlay+WAL integration this module no longer owns a
//! whole-database state object. It owns only the durable record VALUE types the
//! base and overlay both materialize ([`ElementRecord`], [`RelationRecord`],
//! [`IncidenceRecord`], [`PropertySubject`]) plus the nine-value monotonic id
//! allocator watermark ([`NextIds`]). The live read surface is the merged
//! overlay-over-base view in [`crate::overlay`]; the owned whole-DB model is
//! gone.
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines value types and `O(1)` watermark
//! arithmetic.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    ElementId, IncidenceId, IndexId, LabelId, ProjectionId, PropertyKeyId, RelationId,
    RelationTypeId, RoleId, backing::DbHeader, catalog::PropertyFamily,
};

/// One visible canonical element.
///
/// # Performance
///
/// Cloning is `O(label count)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ElementRecord {
    /// Stable element identifier.
    pub id: ElementId,
    /// Labels assigned to this element.
    pub labels: BTreeSet<LabelId>,
}

/// One visible canonical relation.
///
/// # Performance
///
/// Cloning is `O(label count)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationRecord {
    /// Stable relation identifier.
    pub id: RelationId,
    /// Optional relation type.
    pub relation_type: Option<RelationTypeId>,
    /// Labels assigned to this relation.
    pub labels: BTreeSet<LabelId>,
}

/// One visible incidence in canonical database coordinates.
///
/// # Performance
///
/// Copying and comparing this record are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IncidenceRecord {
    /// Stable incidence id.
    pub id: IncidenceId,
    /// Stable relation id containing the incidence.
    pub relation: RelationId,
    /// Stable element id participating in the relation.
    pub element: ElementId,
    /// Structural role of the incidence.
    pub role: RoleId,
}

/// Subject that can own properties.
///
/// # Invariant
///
/// The variant declaration order (`Element`, `Relation`, `Incidence`) is
/// load-bearing: the derived `Ord` ranks them in that order, and that ranking
/// MUST match the wire kind tags `0`/`1`/`2` produced by the wire
/// `encode_subject` codec. The frozen base writes property records in
/// `BTreeMap<PropertySubject, …>` order — sorted by `(subject_kind, subject_id,
/// key)` — and the read path materializes them back into that same keyed order;
/// reordering these variants would silently desynchronize the two. See the wire
/// `encode_subject` docs for the full contract and the open-time assertion in
/// `backing` that guards it.
///
/// # Performance
///
/// Copying, comparing, ordering, and hashing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertySubject {
    /// Element property subject.
    Element(ElementId),
    /// Relation property subject.
    Relation(RelationId),
    /// Incidence property subject.
    Incidence(IncidenceId),
}

impl PropertySubject {
    /// Returns the property family for this subject.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn family(self) -> PropertyFamily {
        match self {
            Self::Element(_id) => PropertyFamily::Element,
            Self::Relation(_id) => PropertyFamily::Relation,
            Self::Incidence(_id) => PropertyFamily::Incidence,
        }
    }
}

/// The nine monotonic id allocators, captured for the store header and every
/// dirty commit's watermark op in a fixed order. Recovery folds the elementwise
/// maximum of the base header and every replayed frame's watermark so a
/// canonical id is never reissued across a crash.
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NextIds {
    /// Next element id candidate.
    pub(crate) element: ElementId,
    /// Next relation id candidate.
    pub(crate) relation: RelationId,
    /// Next incidence id candidate.
    pub(crate) incidence: IncidenceId,
    /// Next role id candidate.
    pub(crate) role: RoleId,
    /// Next label id candidate.
    pub(crate) label: LabelId,
    /// Next relation-type id candidate.
    pub(crate) relation_type: RelationTypeId,
    /// Next property-key id candidate.
    pub(crate) property_key: PropertyKeyId,
    /// Next projection id candidate.
    pub(crate) projection: ProjectionId,
    /// Next index id candidate.
    pub(crate) index: IndexId,
}

impl NextIds {
    /// The watermark of an empty store: every allocator starts at `1`, so the
    /// `0` sentinel ([`crate::wire::RELATION_TYPE_NONE`]) can never collide with
    /// a real id.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; this is a compile-time constant.
    pub(crate) const INITIAL: Self = Self {
        element: ElementId::new(1),
        relation: RelationId::new(1),
        incidence: IncidenceId::new(1),
        role: RoleId::new(1),
        label: LabelId::new(1),
        relation_type: RelationTypeId::new(1),
        property_key: PropertyKeyId::new(1),
        projection: ProjectionId::new(1),
        index: IndexId::new(1),
    };

    /// Reads the watermark out of a base header's nine `next_*` allocator
    /// snapshots.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn from_header(header: &DbHeader) -> Self {
        Self {
            element: ElementId::new(header.next_element),
            relation: RelationId::new(header.next_relation),
            incidence: IncidenceId::new(header.next_incidence),
            role: RoleId::new(header.next_role),
            label: LabelId::new(header.next_label),
            relation_type: RelationTypeId::new(header.next_relation_type),
            property_key: PropertyKeyId::new(header.next_property_key),
            projection: ProjectionId::new(header.next_projection),
            index: IndexId::new(header.next_index),
        }
    }

    /// Returns the elementwise maximum of two watermarks, allocator by
    /// allocator. Recovery folds this over the base header and every replayed
    /// frame so the recovered watermark sits at or above every id ever issued —
    /// canonical ids are never reused.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn elementwise_max(self, other: Self) -> Self {
        Self {
            element: self.element.max(other.element),
            relation: self.relation.max(other.relation),
            incidence: self.incidence.max(other.incidence),
            role: self.role.max(other.role),
            label: self.label.max(other.label),
            relation_type: self.relation_type.max(other.relation_type),
            property_key: self.property_key.max(other.property_key),
            projection: self.projection.max(other.projection),
            index: self.index.max(other.index),
        }
    }
}

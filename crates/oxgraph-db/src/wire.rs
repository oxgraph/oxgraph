//! OXGDB v2 on-disk wire vocabulary.
//!
//! This module owns the byte-level contract for the embedded database store:
//! the section-kind allocation inside the snapshot container's reserved
//! [`DATABASE_BAND`](oxgraph_snapshot::kinds::DATABASE_BAND) and the fixed-width
//! `zerocopy` records that hold the durable canonical state. It replaces the
//! previous "serialize the whole `DatabaseState` to one opaque JSON section"
//! scheme: topology, catalog, identity, and properties each persist as their
//! own typed section, so reads borrow directly from the mapped bytes instead of
//! parsing an intermediate document.
//!
//! Records here are deliberately small and `#[repr(C)]` with little-endian
//! [`zerocopy`] words, mirroring the snapshot container's own header/entry
//! layout. Variable-length data (label sets, names, projection/index definition
//! bodies) lives in companion run/blob sections referenced by `(offset, len)`
//! pairs; property columns and adjacency live in nested
//! [`oxgraph_property`]/[`oxgraph_csr`] snapshots under their own kinds. Only
//! the fixed records and the kind constants live in this module; the encode and
//! decode orchestration lives in `freeze`/`storage`.
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines data layout and `O(1)` field
//! mappings only.

use oxgraph_snapshot::kinds;
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32, U64},
};

use crate::{
    ElementId, IncidenceId, PropertyFamily, PropertySubject, PropertyType, RelationId,
    RelationTypeId,
};

/// Format version of the OXGDB store payload as a whole, recorded in
/// [`DbHeaderRecord::format_version`]. A reader that does not recognize the
/// value rejects the store rather than guessing the layout.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OXGDB_FORMAT_VERSION: u32 = 1;

/// Section version recorded on every OXGDB section entry. Bumped independently
/// of [`OXGDB_FORMAT_VERSION`] when a single section's record layout changes.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OXGDB_SECTION_VERSION: u32 = 1;

/// Sentinel `u64` standing in for "no relation type" inside a [`RelationWire`].
/// Valid canonical ids are allocated from `1`, so `0` can never collide with a
/// real [`RelationTypeId`] and is a safe absence marker.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const RELATION_TYPE_NONE: u64 = 0;

/// Fixed header section: one [`DbHeaderRecord`] carrying the format version,
/// commit/transaction/generation stamps, and the nine id allocators.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_DB_HEADER: u32 = 0x0300;
/// Concatenated UTF-8 string table; every catalog name is a `(offset, len)`
/// slice into this byte section.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_STRING_TABLE: u32 = 0x0301;
/// Catalog role records ([`NamedWire`] array).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_ROLES: u32 = 0x0302;
/// Catalog label records ([`NamedWire`] array).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_LABELS: u32 = 0x0303;
/// Catalog relation-type records ([`NamedWire`] array).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_RELATION_TYPES: u32 = 0x0304;
/// Catalog property-key records ([`PropertyKeyWire`] array).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_PROPERTY_KEYS: u32 = 0x0305;
/// Catalog projection records ([`DefWire`] array; bodies in
/// [`SECTION_CATALOG_DEFS`]).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_PROJECTIONS: u32 = 0x0306;
/// Catalog index records ([`DefWire`] array; bodies in
/// [`SECTION_CATALOG_DEFS`]).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_INDEXES: u32 = 0x0307;
/// Projection/index definition bodies: a flat `u64` run holding the id sets and
/// key vectors a [`DefWire`] slices with `(payload_off, payload_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CATALOG_DEFS: u32 = 0x0308;
/// Element records ([`ElementWire`] array; label runs in
/// [`SECTION_ELEMENT_LABELS`]).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_ELEMENT_RECORDS: u32 = 0x0310;
/// Element label run: a flat `u64` run of label ids sliced by each
/// [`ElementWire`]'s `(label_off, label_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_ELEMENT_LABELS: u32 = 0x0311;
/// Relation records ([`RelationWire`] array; label runs in
/// [`SECTION_RELATION_LABELS`]).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_RELATION_RECORDS: u32 = 0x0312;
/// Relation label run: a flat `u64` run of label ids sliced by each
/// [`RelationWire`]'s `(label_off, label_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_RELATION_LABELS: u32 = 0x0313;
/// Incidence records ([`IncidenceWire`] array).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_INCIDENCE_RECORDS: u32 = 0x0314;
/// Nested [`oxgraph_property`] snapshot bytes holding the typed property
/// columns keyed by snapshot-local id.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_PROPERTY_SNAPSHOT: u32 = 0x0320;
/// Nested local-to-canonical identity-map bytes, present only when a section's
/// snapshot-local id order differs from canonical id order.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_IDENTITY_MAP: u32 = 0x0321;
/// Typed property records ([`PropertyWire`] array); text values reference
/// [`SECTION_PROPERTY_TEXT`].
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_PROPERTY_RECORDS: u32 = 0x0322;
/// Concatenated UTF-8 property text values, sliced by each [`PropertyWire`]'s
/// `(text_off, text_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_PROPERTY_TEXT: u32 = 0x0323;
/// Nested forward (outbound) CSR adjacency snapshot bytes.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CSR_OUT: u32 = 0x0330;
/// Nested reverse (inbound) CSC adjacency snapshot bytes.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_CSC_IN: u32 = 0x0331;
/// Nested directed bipartite-CSR hypergraph snapshot bytes.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_HYPER_BCSR: u32 = 0x0332;
/// Physical equality/composite index postings.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_INDEX_EQUALITY: u32 = 0x0340;
/// Physical label and relation-type membership postings.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_INDEX_LABEL_POSTINGS: u32 = 0x0341;

/// Every section kind this store emits, used by the compile-time band check
/// below and available to tooling that wants to enumerate the layout.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const ALL_SECTION_KINDS: [u32; 23] = [
    SECTION_DB_HEADER,
    SECTION_STRING_TABLE,
    SECTION_CATALOG_ROLES,
    SECTION_CATALOG_LABELS,
    SECTION_CATALOG_RELATION_TYPES,
    SECTION_CATALOG_PROPERTY_KEYS,
    SECTION_CATALOG_PROJECTIONS,
    SECTION_CATALOG_INDEXES,
    SECTION_CATALOG_DEFS,
    SECTION_ELEMENT_RECORDS,
    SECTION_ELEMENT_LABELS,
    SECTION_RELATION_RECORDS,
    SECTION_RELATION_LABELS,
    SECTION_INCIDENCE_RECORDS,
    SECTION_PROPERTY_SNAPSHOT,
    SECTION_IDENTITY_MAP,
    SECTION_PROPERTY_RECORDS,
    SECTION_PROPERTY_TEXT,
    SECTION_CSR_OUT,
    SECTION_CSC_IN,
    SECTION_HYPER_BCSR,
    SECTION_INDEX_EQUALITY,
    SECTION_INDEX_LABEL_POSTINGS,
];

// Every OXGDB section kind must live inside the container's reserved
// `DATABASE_BAND`; a stray value would silently collide with another
// subsystem's band. Enforced at compile time so a typo in a constant above
// fails the build rather than corrupting a store.
const _: () = {
    let mut index = 0;
    while index < ALL_SECTION_KINDS.len() {
        assert!(
            kinds::in_band(ALL_SECTION_KINDS[index], kinds::DATABASE_BAND),
            "OXGDB section kind escaped DATABASE_BAND",
        );
        index += 1;
    }
};

/// Fixed store header: format version, durable commit/transaction/generation
/// stamps, and the nine monotonic id allocators. Exactly one record occupies
/// [`SECTION_DB_HEADER`].
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct DbHeaderRecord {
    /// OXGDB format version; must equal [`OXGDB_FORMAT_VERSION`] to open.
    pub(crate) format_version: U32<LE>,
    /// Reserved flag bits; must be zero in this format version.
    pub(crate) flags: U32<LE>,
    /// Last visible committed transaction sequence.
    pub(crate) commit_seq: U64<LE>,
    /// Last writer transaction id burned by the publishing handle.
    pub(crate) transaction_id: U64<LE>,
    /// Durable checkpoint/root generation stamp.
    pub(crate) checkpoint_generation: U64<LE>,
    /// Next element id candidate.
    pub(crate) next_element: U64<LE>,
    /// Next relation id candidate.
    pub(crate) next_relation: U64<LE>,
    /// Next incidence id candidate.
    pub(crate) next_incidence: U64<LE>,
    /// Next role id candidate.
    pub(crate) next_role: U64<LE>,
    /// Next label id candidate.
    pub(crate) next_label: U64<LE>,
    /// Next relation-type id candidate.
    pub(crate) next_relation_type: U64<LE>,
    /// Next property-key id candidate.
    pub(crate) next_property_key: U64<LE>,
    /// Next projection id candidate.
    pub(crate) next_projection: U64<LE>,
    /// Next index id candidate.
    pub(crate) next_index: U64<LE>,
}

/// One catalog entry that carries only an id and a name: roles, labels, and
/// relation types share this shape.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct NamedWire {
    /// Catalog id of the entry.
    pub(crate) id: U64<LE>,
    /// Byte offset of the name in [`SECTION_STRING_TABLE`].
    pub(crate) name_off: U32<LE>,
    /// Byte length of the name in [`SECTION_STRING_TABLE`].
    pub(crate) name_len: U32<LE>,
}

/// One catalog property-key entry: id, name, owning family, and value type.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct PropertyKeyWire {
    /// Property-key id.
    pub(crate) id: U64<LE>,
    /// Byte offset of the name in [`SECTION_STRING_TABLE`].
    pub(crate) name_off: U32<LE>,
    /// Byte length of the name in [`SECTION_STRING_TABLE`].
    pub(crate) name_len: U32<LE>,
    /// Owning property family tag (see [`property_family_tag`]).
    pub(crate) family: U32<LE>,
    /// Stored value-type tag (see [`property_type_tag`]).
    pub(crate) value_type: U32<LE>,
}

/// One projection or index catalog entry. The variable-length definition body
/// (role/relation-type id sets, property-key vectors) is a `u64` run sliced out
/// of [`SECTION_CATALOG_DEFS`] by `(payload_off, payload_len)`; `kind`
/// disambiguates how that body is interpreted.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct DefWire {
    /// Catalog id of the projection or index.
    pub(crate) id: U64<LE>,
    /// Byte offset of the name in [`SECTION_STRING_TABLE`].
    pub(crate) name_off: U32<LE>,
    /// Byte length of the name in [`SECTION_STRING_TABLE`].
    pub(crate) name_len: U32<LE>,
    /// Definition-kind discriminant interpreted by `freeze`/`storage`.
    pub(crate) kind: U32<LE>,
    /// Start index of the body in the [`SECTION_CATALOG_DEFS`] `u64` run.
    pub(crate) payload_off: U32<LE>,
    /// Number of `u64` words the body occupies in [`SECTION_CATALOG_DEFS`].
    pub(crate) payload_len: U32<LE>,
}

/// One element record: its canonical id plus the slice of
/// [`SECTION_ELEMENT_LABELS`] holding its label ids.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct ElementWire {
    /// Canonical element id.
    pub(crate) id: U64<LE>,
    /// Start index of this element's labels in [`SECTION_ELEMENT_LABELS`].
    pub(crate) label_off: U32<LE>,
    /// Number of labels this element carries.
    pub(crate) label_len: U32<LE>,
}

/// One relation record: its canonical id, optional relation type encoded with
/// the [`RELATION_TYPE_NONE`] sentinel, and the slice of
/// [`SECTION_RELATION_LABELS`] holding its label ids.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct RelationWire {
    /// Canonical relation id.
    pub(crate) id: U64<LE>,
    /// Relation-type id, or [`RELATION_TYPE_NONE`] when unset.
    pub(crate) relation_type: U64<LE>,
    /// Start index of this relation's labels in [`SECTION_RELATION_LABELS`].
    pub(crate) label_off: U32<LE>,
    /// Number of labels this relation carries.
    pub(crate) label_len: U32<LE>,
}

/// One incidence record in canonical coordinates: id, owning relation,
/// participating element, and structural role.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct IncidenceWire {
    /// Canonical incidence id.
    pub(crate) id: U64<LE>,
    /// Canonical id of the relation containing this incidence.
    pub(crate) relation: U64<LE>,
    /// Canonical id of the participating element.
    pub(crate) element: U64<LE>,
    /// Structural role id of the incidence.
    pub(crate) role: U64<LE>,
}

/// Encodes an optional relation type into a [`RelationWire::relation_type`]
/// word, using [`RELATION_TYPE_NONE`] for the absent case.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn encode_relation_type(value: Option<RelationTypeId>) -> u64 {
    match value {
        None => RELATION_TYPE_NONE,
        Some(id) => id.get(),
    }
}

/// Decodes a [`RelationWire::relation_type`] word back into an optional
/// relation type, treating [`RELATION_TYPE_NONE`] as absence.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn decode_relation_type(raw: u64) -> Option<RelationTypeId> {
    if raw == RELATION_TYPE_NONE {
        None
    } else {
        Some(RelationTypeId::new(raw))
    }
}

/// Maps a property family to its stored `u32` tag.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn property_family_tag(family: PropertyFamily) -> u32 {
    match family {
        PropertyFamily::Element => 0,
        PropertyFamily::Relation => 1,
        PropertyFamily::Incidence => 2,
    }
}

/// Maps a stored `u32` tag back to a property family, returning `None` for an
/// unrecognized tag so the read path can fail loudly instead of coercing.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn property_family_from_tag(tag: u32) -> Option<PropertyFamily> {
    match tag {
        0 => Some(PropertyFamily::Element),
        1 => Some(PropertyFamily::Relation),
        2 => Some(PropertyFamily::Incidence),
        _ => None,
    }
}

/// Maps a property value type to its stored `u32` tag.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn property_type_tag(value_type: PropertyType) -> u32 {
    match value_type {
        PropertyType::Boolean => 0,
        PropertyType::Integer => 1,
        PropertyType::Text => 2,
    }
}

/// Maps a stored `u32` tag back to a property value type, returning `None` for
/// an unrecognized tag so the read path can fail loudly instead of coercing.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn property_type_from_tag(tag: u32) -> Option<PropertyType> {
    match tag {
        0 => Some(PropertyType::Boolean),
        1 => Some(PropertyType::Integer),
        2 => Some(PropertyType::Text),
        _ => None,
    }
}

/// Encodes a property subject into its `(kind, id)` wire pair, where the kind
/// tag is `0` for elements, `1` for relations, and `2` for incidences.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn encode_subject(subject: PropertySubject) -> (u32, u64) {
    match subject {
        PropertySubject::Element(id) => (0, id.get()),
        PropertySubject::Relation(id) => (1, id.get()),
        PropertySubject::Incidence(id) => (2, id.get()),
    }
}

/// Decodes a `(kind, id)` property-subject wire pair, returning `None` for an
/// unrecognized kind so the read path fails loudly instead of coercing.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn decode_subject(kind: u32, id: u64) -> Option<PropertySubject> {
    match kind {
        0 => Some(PropertySubject::Element(ElementId::new(id))),
        1 => Some(PropertySubject::Relation(RelationId::new(id))),
        2 => Some(PropertySubject::Incidence(IncidenceId::new(id))),
        _ => None,
    }
}

/// One typed property value in canonical coordinates. The scalar word holds the
/// boolean (`0`/`1`) or the `i64` reinterpreted as `u64`; text values live in
/// [`SECTION_PROPERTY_TEXT`] and are referenced by `(text_off, text_len)`.
/// Records are written sorted by `(subject_kind, subject_id, key)`.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct PropertyWire {
    /// Subject kind tag (see [`encode_subject`]).
    pub(crate) subject_kind: U32<LE>,
    /// Stored value-type tag (see [`property_type_tag`]).
    pub(crate) value_tag: U32<LE>,
    /// Canonical subject id within its family.
    pub(crate) subject_id: U64<LE>,
    /// Property-key id.
    pub(crate) key: U64<LE>,
    /// Boolean (`0`/`1`) or `i64`-as-`u64` scalar; unused for text.
    pub(crate) scalar: U64<LE>,
    /// Byte offset of a text value in [`SECTION_PROPERTY_TEXT`]; unused
    /// otherwise.
    pub(crate) text_off: U32<LE>,
    /// Byte length of a text value in [`SECTION_PROPERTY_TEXT`]; unused
    /// otherwise.
    pub(crate) text_len: U32<LE>,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// A non-zero relation-type id round-trips through the sentinel
        /// encoding. Canonical ids are allocated from `1`, so the `1..` domain
        /// matches every value the encoder can actually receive.
        #[test]
        fn relation_type_some_roundtrips(raw in 1u64..=u64::MAX) {
            let id = RelationTypeId::new(raw);
            prop_assert_eq!(decode_relation_type(encode_relation_type(Some(id))), Some(id));
        }
    }

    #[test]
    fn relation_type_none_uses_sentinel() {
        assert_eq!(encode_relation_type(None), RELATION_TYPE_NONE);
        assert_eq!(decode_relation_type(RELATION_TYPE_NONE), None);
    }

    #[test]
    fn property_family_tags_roundtrip() {
        for family in [
            PropertyFamily::Element,
            PropertyFamily::Relation,
            PropertyFamily::Incidence,
        ] {
            assert_eq!(
                property_family_from_tag(property_family_tag(family)),
                Some(family)
            );
        }
        assert_eq!(property_family_from_tag(3), None);
    }

    #[test]
    fn property_type_tags_roundtrip() {
        for value_type in [
            PropertyType::Boolean,
            PropertyType::Integer,
            PropertyType::Text,
        ] {
            assert_eq!(
                property_type_from_tag(property_type_tag(value_type)),
                Some(value_type)
            );
        }
        assert_eq!(property_type_from_tag(3), None);
    }
}

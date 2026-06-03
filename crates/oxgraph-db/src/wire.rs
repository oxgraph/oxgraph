//! OXGDB v1 on-disk wire vocabulary.
//!
//! This module owns the byte-level contract for the embedded database store:
//! the section-kind allocation inside the snapshot container's reserved
//! [`DATABASE_BAND`](oxgraph_snapshot::kinds::DATABASE_BAND) and the fixed-width
//! `zerocopy` records that hold the durable canonical state. Topology, catalog,
//! identity, and properties each persist as their own typed section, so reads
//! borrow directly from the mapped bytes instead of parsing an intermediate
//! document.
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
/// Base content-integrity trailer: a single [`BaseTrailer`] written last in a
/// base file, holding the CRC-32C over every base byte preceding the trailer
/// section. Open recomputes it to fault truncation or in-place corruption into
/// [`crate::DbError::InvalidStore`] before any borrow.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SECTION_BASE_TRAILER: u32 = 0x0309;
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
pub(crate) const ALL_SECTION_KINDS: [u32; 24] = [
    SECTION_DB_HEADER,
    SECTION_STRING_TABLE,
    SECTION_CATALOG_ROLES,
    SECTION_CATALOG_LABELS,
    SECTION_CATALOG_RELATION_TYPES,
    SECTION_CATALOG_PROPERTY_KEYS,
    SECTION_CATALOG_PROJECTIONS,
    SECTION_CATALOG_INDEXES,
    SECTION_CATALOG_DEFS,
    SECTION_BASE_TRAILER,
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
/// # Invariant
///
/// These kind tags (`0`/`1`/`2`) are contractually tied to the
/// [`PropertySubject`] variant declaration order: the derived `Ord` on
/// [`PropertySubject`] ranks `Element < Relation < Incidence`, which MUST equal
/// the ascending tag order here. Property records are written in ascending
/// `PropertySubject` order and read back via a binary search keyed on
/// `(subject_kind, subject_id, key)` in
/// `backing::BaseView::property_by_key`. Reordering the variants or
/// renumbering these tags would silently desynchronize write order from search
/// order. `backing::attach_view` enforces this at open time with a debug
/// assertion that the property array is sorted by that triple.
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

/// Base content-integrity trailer record. Exactly one occupies
/// [`SECTION_BASE_TRAILER`], written last in a base file. Its `crc32c` is the
/// CRC-32C over every base byte preceding the trailer section, so open can
/// recompute the checksum and reject a truncated or in-place-corrupted base
/// before borrowing any section.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct BaseTrailer {
    /// CRC-32C over all base bytes preceding this trailer section.
    pub(crate) crc32c: U32<LE>,
    /// Reserved word; must be zero in this format version.
    pub(crate) reserved: U32<LE>,
}

/// Eight-byte magic identifying a [`SuperblockRecord`].
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SUPERBLOCK_MAGIC: [u8; 8] = *b"OXGSUPER";

/// Superblock / manifest record stored in `super.oxgdb`. It is the store's
/// single linearization point: a store's identity is whatever the superblock
/// names. It is a recovery FLOOR, not the live frontier — it names the base
/// generation, the last checkpoint LSN folded into that base, and the byte
/// offset in the delta-log where post-checkpoint records begin. The live
/// `commit_seq`/`transaction_id` are derived from the valid prefix of the
/// delta-log; the values here are a checkpoint-time snapshot.
///
/// `crc32c` covers all bytes of this record preceding the `crc32c` field; the
/// `pad` word follows it so the struct has no trailing padding for `IntoBytes`.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct SuperblockRecord {
    /// Format magic; must equal [`SUPERBLOCK_MAGIC`] to open.
    pub(crate) magic: [u8; 8],
    /// Base generation named by this superblock.
    pub(crate) base_generation: U64<LE>,
    /// Last checkpoint LSN folded into the named base.
    pub(crate) checkpoint_lsn: U64<LE>,
    /// Byte offset in the delta-log where post-checkpoint records begin.
    pub(crate) log_byte_offset: U64<LE>,
    /// Checkpoint-time commit sequence snapshot.
    pub(crate) commit_seq: U64<LE>,
    /// Checkpoint-time writer transaction id snapshot.
    pub(crate) transaction_id: U64<LE>,
    /// OXGDB format version; must equal [`OXGDB_FORMAT_VERSION`] to open.
    pub(crate) format_version: U32<LE>,
    /// Reserved flag bits; must be zero in this format version.
    pub(crate) flags: U32<LE>,
    /// CRC-32C over all preceding bytes of this record.
    pub(crate) crc32c: U32<LE>,
    /// Trailing pad word kept zero so the record has no implicit padding.
    pub(crate) pad: U32<LE>,
}

/// Byte length of a [`SuperblockRecord`] prefix the `crc32c` field covers: every
/// field before `crc32c` (the `pad` word that follows it is excluded along with
/// the checksum itself).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SUPERBLOCK_CRC_PREFIX_LEN: usize =
    size_of::<SuperblockRecord>() - size_of::<U32<LE>>() - size_of::<U32<LE>>();

/// Magic word stamped on every [`LogRecordHeader`]; misframing or a foreign
/// file is caught when this does not match.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OXGLOGR: u32 = 0x4F58_4C47;

/// Fixed delta-log record header. Each record is this header followed by
/// `op_count` [`MutationOp`] records and then an optional UTF-8 blob; `len` is
/// the total record size (header + ops + blob) so a reader skips to the next
/// record by `len` alone. Fields are grouped `u64`s-then-`u32`s so the
/// `#[repr(C)]` layout has no padding.
///
/// `crc32c` covers the ENTIRE record from the first header byte through the end
/// of the blob, EXCEPT the four bytes of the `crc32c` field itself; `len` IS
/// included so a torn `len` is caught by the checksum as well as the bounds
/// check. `crc32c` is the last field, so the covered prefix of the header is the
/// contiguous range `[0 .. size_of::<LogRecordHeader>() - 4]`.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct LogRecordHeader {
    /// Base generation this record applies over; must equal the superblock's.
    pub(crate) base_generation: U64<LE>,
    /// Log sequence number; strictly ascending across the log.
    pub(crate) lsn: U64<LE>,
    /// Writer transaction id that produced the record.
    pub(crate) txn_id: U64<LE>,
    /// Record magic; must equal [`OXGLOGR`].
    pub(crate) magic: U32<LE>,
    /// Total record byte length (header + ops + blob).
    pub(crate) len: U32<LE>,
    /// Number of [`MutationOp`] records following this header.
    pub(crate) op_count: U32<LE>,
    /// CRC-32C over the whole record except these four bytes.
    pub(crate) crc32c: U32<LE>,
}

/// Number of `U64<LE>` payload words in a [`MutationOp`], sized to the widest
/// op ([`OP_NEXT_ID_WATERMARK`] carries all nine id allocators).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const MUTATION_OP_PAYLOAD_WORDS: usize = 9;

/// One fixed-size mutation, discriminated by `op_kind`. `flags` packs small
/// secondary tags (e.g. a property subject kind and value tag, or a property
/// key's family and value type) without widening the record; `payload` carries
/// the id words, and variable-length text/name bytes are referenced as
/// `(offset, len)` words into the record's trailing blob. The op vocabulary
/// has one op per state mutator and reuses the same tag helpers
/// ([`encode_subject`], [`property_family_tag`], [`property_type_tag`]) as the
/// base format.
///
/// # Performance
///
/// Copying is `O(1)`; the record is a fixed-size value type.
#[derive(Clone, Copy, Debug, Eq, FromBytes, Immutable, IntoBytes, KnownLayout, PartialEq)]
#[repr(C)]
pub(crate) struct MutationOp {
    /// Op discriminant; one of the `OP_*` constants.
    pub(crate) op_kind: U32<LE>,
    /// Packed secondary tags (see the op packing helpers); zero when unused.
    pub(crate) flags: U32<LE>,
    /// Fixed id/offset payload words interpreted per `op_kind`.
    pub(crate) payload: [U64<LE>; MUTATION_OP_PAYLOAD_WORDS],
}

/// Creates one element; `payload[0]` is the allocated element id.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CREATE_ELEMENT: u32 = 1;
/// Tombstones one element; `payload[0]` is the element id.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_TOMBSTONE_ELEMENT: u32 = 2;
/// Creates one relation; `payload[0]` is the allocated relation id.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CREATE_RELATION: u32 = 3;
/// Tombstones one relation; `payload[0]` is the relation id.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_TOMBSTONE_RELATION: u32 = 4;
/// Creates one incidence; `payload` holds `(incidence, relation, element,
/// role)` in that order.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CREATE_INCIDENCE: u32 = 5;
/// Tombstones one incidence; `payload[0]` is the incidence id.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_TOMBSTONE_INCIDENCE: u32 = 6;
/// Sets a relation's type; `payload` holds `(relation, relation_type)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_SET_RELATION_TYPE: u32 = 7;
/// Adds a label to an element; `payload` holds `(element, label)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_ADD_ELEMENT_LABEL: u32 = 8;
/// Adds a label to a relation; `payload` holds `(relation, label)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_ADD_RELATION_LABEL: u32 = 9;
/// Sets a typed property; `flags` packs the subject kind and value tag, and
/// `payload` holds `(subject_id, key, scalar, text_off, text_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_SET_PROPERTY: u32 = 10;
/// Removes a property; `flags` packs the subject kind and `payload` holds
/// `(subject_id, key)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_REMOVE_PROPERTY: u32 = 11;
/// Registers a catalog role; `payload` holds `(id, name_off, name_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_REGISTER_ROLE: u32 = 12;
/// Registers a catalog label; `payload` holds `(id, name_off, name_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_REGISTER_LABEL: u32 = 13;
/// Registers a catalog relation type; `payload` holds `(id, name_off,
/// name_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_REGISTER_RELATION_TYPE: u32 = 14;
/// Registers a catalog property key; `flags` packs the family and value type
/// and `payload` holds `(id, name_off, name_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_REGISTER_PROPERTY_KEY: u32 = 15;
/// Registers a catalog projection; `payload` holds `(id, name_off, name_len,
/// def_off, def_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_REGISTER_PROJECTION: u32 = 16;
/// Registers a catalog index; `payload` holds `(id, name_off, name_len,
/// def_off, def_len)`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_REGISTER_INDEX: u32 = 17;
/// Carries the full nine-value next-id watermark; `payload` holds `(element,
/// relation, incidence, role, label, relation_type, property_key, projection,
/// index)`. Emitted as the last op of every dirty commit so recovery never
/// recomputes allocators from live records.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_NEXT_ID_WATERMARK: u32 = 18;
/// Reserved catalog-drop op kind. v1 is register-only; this value is reserved
/// so a future catalog-tombstone workstream owns it without renumbering.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_DROP: u32 = 19;
/// Reserved catalog-rename op kind. v1 is register-only; this value is reserved
/// for a future workstream.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const OP_CATALOG_RENAME: u32 = 20;

/// Op kinds reserved for a future catalog-tombstone workstream but never emitted
/// by v1 (catalog is register-only; any drift surfaces as a missing-catalog
/// lookup at read time). They are listed here so the reservation is referenced
/// (not dead) and the compile-time check below proves they sit strictly above
/// every emitted op kind.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const RESERVED_OP_KINDS: [u32; 2] = [OP_CATALOG_DROP, OP_CATALOG_RENAME];

// The reserved op kinds must sit strictly above the highest emitted op kind
// ([`OP_NEXT_ID_WATERMARK`] = 18) so a future workstream can claim them without
// renumbering any emitted op.
const _: () = {
    assert!(
        RESERVED_OP_KINDS[0] > OP_NEXT_ID_WATERMARK,
        "reserved op kind must not collide with an emitted op kind",
    );
    assert!(
        RESERVED_OP_KINDS[1] > RESERVED_OP_KINDS[0],
        "reserved op kinds must be distinct and ascending",
    );
};

/// Packs two `u16`-range tags into a single `flags` `u32`: the low 16 bits of
/// `low` in the low half and the low 16 bits of `high` in the high half. Used
/// for property subject-kind + value tag and property-key family + value type,
/// all of which are `0..=2`, so the high bits are always zero.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn pack_flags(low: u32, high: u32) -> u32 {
    (low & 0xFFFF) | ((high & 0xFFFF) << 16)
}

/// Unpacks a `flags` `u32` produced by [`pack_flags`] into its `(low, high)`
/// tags (each in the `0..=u16::MAX` range, returned widened to `u32`).
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn unpack_flags(flags: u32) -> (u32, u32) {
    (flags & 0xFFFF, flags >> 16)
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

    /// The new format records carry no implicit padding: every field is
    /// accounted for, which `IntoBytes` requires and recovery's byte-range CRC
    /// scopes depend on.
    #[test]
    fn new_records_have_no_padding() {
        assert_eq!(size_of::<BaseTrailer>(), 8);
        // 8 (magic) + 5 * 8 (u64 fields) + 4 * 4 (u32 fields incl. pad) = 64.
        assert_eq!(size_of::<SuperblockRecord>(), 64);
        // 3 * 8 (u64 fields) + 4 * 4 (u32 fields) = 40.
        assert_eq!(size_of::<LogRecordHeader>(), 40);
        assert_eq!(size_of::<MutationOp>(), 8 + MUTATION_OP_PAYLOAD_WORDS * 8);
    }

    /// The superblock CRC prefix covers every field before `crc32c` and excludes
    /// both the checksum and the trailing pad word.
    #[test]
    fn superblock_crc_prefix_excludes_crc_and_pad() {
        assert_eq!(SUPERBLOCK_CRC_PREFIX_LEN, size_of::<SuperblockRecord>() - 8);
    }

    proptest! {
        /// Two `u16`-range tags round-trip through the packed `flags` word in
        /// both positions independently (the packer keeps only the low 16 bits
        /// of each half).
        #[test]
        fn flags_pack_roundtrips(low in any::<u16>(), high in any::<u16>()) {
            let (low, high) = (u32::from(low), u32::from(high));
            prop_assert_eq!(unpack_flags(pack_flags(low, high)), (low, high));
        }
    }
}

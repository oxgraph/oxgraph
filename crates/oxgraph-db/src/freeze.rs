//! OXGDB v1 base freeze: encode a merged state view into typed zero-copy
//! `DATABASE_BAND` sections, and the borrowing decoders the base attach reuses.
//!
//! Each component — header, catalog, topology, properties — persists as its own
//! typed section described in [`crate::wire`]. [`freeze_view`] writes a complete
//! base file from any [`StateView`] (an empty overlay for `create`, a merged
//! base+overlay fold for `checkpoint`), finishing with the
//! [`wire::SECTION_BASE_TRAILER`] CRC. The decoders below ([`typed_records`],
//! [`raw_blob`], [`decode_catalog`]) borrow the fixed records straight from the
//! mapped bytes and are shared with [`crate::backing`].
//!
//! # Performance
//!
//! [`freeze_view`] is `O(catalog + topology + properties)` plus one
//! `O(base bytes)` trailer CRC scan.

use std::collections::{BTreeMap, BTreeSet};

use oxgraph_snapshot::{Snapshot, SnapshotBuilder};
use zerocopy::{
    FromBytes, IntoBytes,
    byteorder::{LE, U32, U64},
};

use crate::{
    Catalog, DbError, ElementId, IncidenceId, IndexId, LabelId, ProjectionId, PropertyKeyId,
    PropertySubject, PropertyValue, RelationId, RelationTypeId, RoleId,
    catalog::{
        GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
        ProjectionDefinition, PropertyKeyDefinition,
    },
    crc,
    index::OwnedBaseIndex,
    overlay::StateView,
    state::{ElementRecord, IncidenceRecord, RelationRecord},
    value::PropertyType,
    wire,
};

/// Projection definition-body discriminant for binary graph projections.
const DEF_PROJECTION_GRAPH: u32 = 0;
/// Projection definition-body discriminant for hypergraph projections.
const DEF_PROJECTION_HYPER: u32 = 1;
/// Index definition-body discriminant for label membership.
const DEF_INDEX_LABEL: u32 = 0;
/// Index definition-body discriminant for relation-type membership.
const DEF_INDEX_RELATION_TYPE: u32 = 1;
/// Index definition-body discriminant for single-key equality.
const DEF_INDEX_PROPERTY_EQUALITY: u32 = 2;
/// Index definition-body discriminant for single-key range.
const DEF_INDEX_PROPERTY_RANGE: u32 = 3;
/// Index definition-body discriminant for composite equality.
const DEF_INDEX_COMPOSITE_EQUALITY: u32 = 4;
/// Index definition-body discriminant for projection materialization.
const DEF_INDEX_PROJECTION: u32 = 5;

/// The durable header stamps a base file records: the checkpoint-time commit
/// sequence, transaction id, and generation folded into the base. Per the
/// reconciled design these are a checkpoint snapshot of the folded state, never
/// consulted as the live frontier (which is derived from the delta-log).
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FreezeStamps {
    /// Checkpoint-time committed transaction sequence.
    pub(crate) commit_seq: u64,
    /// Checkpoint-time writer transaction id.
    pub(crate) transaction_id: u64,
    /// Checkpoint/root generation stamp folded into this base.
    pub(crate) generation: u64,
}

/// Converts a `usize` length/offset to `u32`, failing the store rather than
/// silently truncating an implausibly large value.
///
/// # Performance
///
/// This function is `O(1)`.
fn checked_u32(value: usize) -> Result<u32, DbError> {
    u32::try_from(value).map_err(|_error| DbError::invalid_store("store offset exceeds u32"))
}

/// Converts a wire `u32` offset/length to `usize` for slicing.
///
/// # Performance
///
/// This function is `O(1)`.
const fn usize_of(value: u32) -> usize {
    value as usize
}

/// Accumulates UTF-8 names into a contiguous table, returning `(offset, len)`
/// byte slices for each interned string.
///
/// # Performance
///
/// `perf: unspecified`; see [`Self::intern`].
#[derive(Debug, Default)]
struct StringTable {
    /// Concatenated UTF-8 bytes.
    bytes: Vec<u8>,
}

impl StringTable {
    /// Interns `name`, returning its `(offset, len)` in the table.
    ///
    /// # Performance
    ///
    /// This method is `O(name.len())`.
    fn intern(&mut self, name: &str) -> Result<(u32, u32), DbError> {
        let offset = checked_u32(self.bytes.len())?;
        self.bytes.extend_from_slice(name.as_bytes());
        let len = checked_u32(name.len())?;
        Ok((offset, len))
    }
}

/// Reads a `(offset, len)` UTF-8 slice out of a string/text table.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the slice is out of bounds or not
/// UTF-8.
///
/// # Performance
///
/// This function is `O(len)`.
fn read_str(table: &[u8], offset: u32, len: u32) -> Result<String, DbError> {
    let start = usize_of(offset);
    let end = start
        .checked_add(usize_of(len))
        .ok_or_else(|| DbError::invalid_store("string slice overflow"))?;
    let bytes = table
        .get(start..end)
        .ok_or_else(|| DbError::invalid_store("string slice out of bounds"))?;
    core::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_error| DbError::invalid_store("non-UTF-8 string in store"))
}

/// Borrows a typed record slice from a section, or an empty slice when the
/// section is absent (the store omits empty sections).
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the section bytes cannot be borrowed
/// as a `T` slice.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) fn typed_records<'view, T>(
    snapshot: &Snapshot<'view>,
    kind: u32,
) -> Result<&'view [T], DbError>
where
    T: zerocopy::FromBytes + zerocopy::Immutable + zerocopy::KnownLayout,
{
    snapshot.section(kind).map_or(Ok(&[]), |section| {
        section
            .try_as_slice::<T>()
            .map_err(|error| DbError::invalid_store(error.to_string()))
    })
}

/// Borrows a raw byte blob from a section, or an empty slice when absent.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) fn raw_blob<'view>(snapshot: &Snapshot<'view>, kind: u32) -> &'view [u8] {
    snapshot
        .section(kind)
        .map_or(&[][..], |section| section.bytes())
}

/// Adds a typed record section, skipping it when empty so the open path can
/// treat absence as an empty collection.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when section planning fails.
///
/// # Performance
///
/// This function is `O(records.len() * size_of::<T>())`.
fn add_typed<T>(builder: &mut SnapshotBuilder, kind: u32, records: &[T]) -> Result<(), DbError>
where
    T: zerocopy::IntoBytes + zerocopy::Immutable,
{
    if records.is_empty() {
        return Ok(());
    }
    builder
        .add_section_typed(kind, wire::OXGDB_SECTION_VERSION, records)
        .map_err(|error| DbError::invalid_store(error.to_string()))?;
    Ok(())
}

/// Adds a raw byte-blob section, skipping it when empty.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when section planning fails.
///
/// # Performance
///
/// This function is `O(blob.len())`.
fn add_blob(builder: &mut SnapshotBuilder, kind: u32, blob: Vec<u8>) -> Result<(), DbError> {
    if blob.is_empty() {
        return Ok(());
    }
    builder
        .add_section(kind, wire::OXGDB_SECTION_VERSION, 0, blob)
        .map_err(|error| DbError::invalid_store(error.to_string()))?;
    Ok(())
}

/// Encodes a merged state `view` (plus durable `stamps`) into deterministic
/// OXGDB v1 base bytes, finishing with a [`wire::SECTION_BASE_TRAILER`] whose
/// CRC-32C covers every preceding base byte (see [`append_base_trailer`]).
///
/// `create` freezes an empty overlay over an empty base; `checkpoint` freezes a
/// merged base+overlay fold. Either way the input is one [`StateView`], so a
/// single encoder serves both.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a length exceeds the `u32` wire
/// bounds, section planning fails, or the trailer cannot be located after
/// encoding.
///
/// # Performance
///
/// This function is `O(catalog + topology + properties)`; the trailer pass adds
/// one extra `O(base bytes)` CRC scan.
pub(crate) fn freeze_view(view: &impl StateView, stamps: FreezeStamps) -> Result<Vec<u8>, DbError> {
    let mut builder = SnapshotBuilder::new();
    let mut strings = StringTable::default();
    encode_header(&mut builder, view, stamps)?;
    encode_catalog(&mut builder, view.catalog(), &mut strings)?;
    encode_topology(&mut builder, view)?;
    encode_properties(&mut builder, view)?;
    encode_index(&mut builder, view)?;
    add_blob(&mut builder, wire::SECTION_STRING_TABLE, strings.bytes)?;
    // The trailer is added LAST so its payload is the final region in the byte
    // stream; its CRC then covers the entire prefix before it. The CRC field is
    // a placeholder zero here and is patched after `finish` lays the bytes out.
    add_typed(
        &mut builder,
        wire::SECTION_BASE_TRAILER,
        &[wire::BaseTrailer {
            crc32c: U32::new(0),
            reserved: U32::new(0),
        }],
    )?;
    let mut bytes = builder
        .finish()
        .map_err(|error| DbError::invalid_store(error.to_string()))?;
    append_base_trailer(&mut bytes)?;
    Ok(bytes)
}

/// Patches the [`wire::SECTION_BASE_TRAILER`] record in already-encoded base
/// `bytes` with the CRC-32C over every byte preceding the trailer's payload.
///
/// The covered range is `bytes[..trailer_payload_offset]`: the container header,
/// the full section table (including the trailer's own entry, whose
/// `reserved_checksum` is zero and therefore stable), and every section payload
/// except the trailer's. The trailer's own payload — the `crc32c` word being
/// written and its reserved word — is the only excluded region, so the checksum
/// is self-consistent. The payload offset is located by the address delta
/// between the trailer section's borrowed payload and the buffer base (no
/// pointer reinterpretation, `unsafe_code = forbid` preserved).
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the encoded bytes cannot be reopened,
/// the trailer section is missing, or its payload is shorter than a
/// [`wire::BaseTrailer`].
///
/// # Performance
///
/// This function is `O(base bytes)` for the single CRC scan over the prefix.
fn append_base_trailer(bytes: &mut [u8]) -> Result<(), DbError> {
    let payload_offset = {
        let snapshot =
            Snapshot::open(bytes).map_err(|error| DbError::invalid_store(error.to_string()))?;
        let trailer = snapshot
            .section(wire::SECTION_BASE_TRAILER)
            .ok_or_else(|| DbError::invalid_store("encoded base is missing its trailer"))?;
        if trailer.bytes().len() < size_of::<wire::BaseTrailer>() {
            return Err(DbError::invalid_store("base trailer payload is truncated"));
        }
        trailer.bytes().as_ptr().addr() - bytes.as_ptr().addr()
    };
    let crc = crc::checksum(
        bytes
            .get(..payload_offset)
            .ok_or_else(|| DbError::invalid_store("base trailer offset out of bounds"))?,
    );
    let crc_field = bytes
        .get_mut(payload_offset..payload_offset + size_of::<U32<LE>>())
        .ok_or_else(|| DbError::invalid_store("base trailer crc field out of bounds"))?;
    crc_field.copy_from_slice(U32::<LE>::new(crc).as_bytes());
    Ok(())
}

/// Encodes the fixed header record (version, stamps, id allocators) from the
/// view's watermark.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when section planning fails.
///
/// # Performance
///
/// This function is `O(1)`.
fn encode_header(
    builder: &mut SnapshotBuilder,
    view: &impl StateView,
    stamps: FreezeStamps,
) -> Result<(), DbError> {
    let next = view.next_ids();
    let header = wire::DbHeaderRecord {
        format_version: U32::new(wire::OXGDB_FORMAT_VERSION),
        flags: U32::new(0),
        commit_seq: U64::new(stamps.commit_seq),
        transaction_id: U64::new(stamps.transaction_id),
        checkpoint_generation: U64::new(stamps.generation),
        next_element: U64::new(next.element.get()),
        next_relation: U64::new(next.relation.get()),
        next_incidence: U64::new(next.incidence.get()),
        next_role: U64::new(next.role.get()),
        next_label: U64::new(next.label.get()),
        next_relation_type: U64::new(next.relation_type.get()),
        next_property_key: U64::new(next.property_key.get()),
        next_projection: U64::new(next.projection.get()),
        next_index: U64::new(next.index.get()),
    };
    add_typed(builder, wire::SECTION_DB_HEADER, &[header])
}

/// Encodes the catalog as record sections plus the definition-body run, interning
/// every name into `strings`.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a name length exceeds the `u32` bounds
/// or section planning fails.
///
/// # Performance
///
/// This function is `O(catalog entries + name bytes)`.
fn encode_catalog(
    builder: &mut SnapshotBuilder,
    catalog: &Catalog,
    strings: &mut StringTable,
) -> Result<(), DbError> {
    let mut roles = Vec::new();
    let mut labels = Vec::new();
    let mut relation_types = Vec::new();
    for definition in catalog.roles() {
        roles.push(named_wire(definition.id.get(), &definition.name, strings)?);
    }
    for definition in catalog.labels() {
        labels.push(named_wire(definition.id.get(), &definition.name, strings)?);
    }
    for definition in catalog.relation_types() {
        relation_types.push(named_wire(definition.id.get(), &definition.name, strings)?);
    }
    add_typed(builder, wire::SECTION_CATALOG_ROLES, &roles)?;
    add_typed(builder, wire::SECTION_CATALOG_LABELS, &labels)?;
    add_typed(
        builder,
        wire::SECTION_CATALOG_RELATION_TYPES,
        &relation_types,
    )?;

    let mut property_keys = Vec::new();
    for definition in catalog.property_keys() {
        let (name_off, name_len) = strings.intern(&definition.name)?;
        property_keys.push(wire::PropertyKeyWire {
            id: U64::new(definition.id.get()),
            name_off: U32::new(name_off),
            name_len: U32::new(name_len),
            family: U32::new(wire::property_family_tag(definition.family)),
            value_type: U32::new(wire::property_type_tag(definition.value_type)),
        });
    }
    add_typed(builder, wire::SECTION_CATALOG_PROPERTY_KEYS, &property_keys)?;

    let mut defs: Vec<U64<LE>> = Vec::new();
    let mut projections = Vec::new();
    for entry in catalog.projections() {
        let body = encode_projection_def(&entry.definition, &mut defs)?;
        projections.push(def_wire(
            entry.id.get(),
            entry.definition.name(),
            body,
            strings,
        )?);
    }
    let mut indexes = Vec::new();
    for entry in catalog.indexes() {
        let body = encode_index_def(&entry.definition, &mut defs)?;
        indexes.push(def_wire(entry.id.get(), &entry.name, body, strings)?);
    }
    add_typed(builder, wire::SECTION_CATALOG_PROJECTIONS, &projections)?;
    add_typed(builder, wire::SECTION_CATALOG_INDEXES, &indexes)?;
    add_typed(builder, wire::SECTION_CATALOG_DEFS, &defs)
}

/// Builds a [`wire::NamedWire`] interning `name`.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the name length exceeds `u32`.
///
/// # Performance
///
/// This function is `O(name.len())`.
fn named_wire(id: u64, name: &str, strings: &mut StringTable) -> Result<wire::NamedWire, DbError> {
    let (name_off, name_len) = strings.intern(name)?;
    Ok(wire::NamedWire {
        id: U64::new(id),
        name_off: U32::new(name_off),
        name_len: U32::new(name_len),
    })
}

/// Builds a [`wire::DefWire`] interning `name`.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the name length exceeds `u32`.
///
/// # Performance
///
/// This function is `O(name.len())`.
fn def_wire(
    id: u64,
    name: &str,
    body: (u32, u32, u32),
    strings: &mut StringTable,
) -> Result<wire::DefWire, DbError> {
    let (kind, payload_off, payload_len) = body;
    let (name_off, name_len) = strings.intern(name)?;
    Ok(wire::DefWire {
        id: U64::new(id),
        name_off: U32::new(name_off),
        name_len: U32::new(name_len),
        kind: U32::new(kind),
        payload_off: U32::new(payload_off),
        payload_len: U32::new(payload_len),
    })
}

/// Encodes the merged view's elements, relations, and incidences with their side
/// label runs. The view yields records in ascending canonical id order, so the
/// emitted arrays are canonically sorted (the base attach binary-searches them).
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a label run length or offset exceeds
/// the `u32` bounds or section planning fails.
///
/// # Performance
///
/// This function is `O(topology size + labels)`.
fn encode_topology(builder: &mut SnapshotBuilder, view: &impl StateView) -> Result<(), DbError> {
    let mut element_records = Vec::new();
    let mut element_labels: Vec<U64<LE>> = Vec::new();
    for record in view.elements() {
        let label_off = checked_u32(element_labels.len())?;
        element_labels.extend(record.labels.iter().map(|label| U64::new(label.get())));
        element_records.push(wire::ElementWire {
            id: U64::new(record.id.get()),
            label_off: U32::new(label_off),
            label_len: U32::new(checked_u32(record.labels.len())?),
        });
    }
    add_typed(builder, wire::SECTION_ELEMENT_RECORDS, &element_records)?;
    add_typed(builder, wire::SECTION_ELEMENT_LABELS, &element_labels)?;

    let mut relation_records = Vec::new();
    let mut relation_labels: Vec<U64<LE>> = Vec::new();
    for record in view.relations() {
        let label_off = checked_u32(relation_labels.len())?;
        relation_labels.extend(record.labels.iter().map(|label| U64::new(label.get())));
        relation_records.push(wire::RelationWire {
            id: U64::new(record.id.get()),
            relation_type: U64::new(wire::encode_relation_type(record.relation_type)),
            label_off: U32::new(label_off),
            label_len: U32::new(checked_u32(record.labels.len())?),
        });
    }
    add_typed(builder, wire::SECTION_RELATION_RECORDS, &relation_records)?;
    add_typed(builder, wire::SECTION_RELATION_LABELS, &relation_labels)?;

    let mut incidence_records = Vec::new();
    for record in view.incidences() {
        incidence_records.push(wire::IncidenceWire {
            id: U64::new(record.id.get()),
            relation: U64::new(record.relation.get()),
            element: U64::new(record.element.get()),
            role: U64::new(record.role.get()),
        });
    }
    add_typed(builder, wire::SECTION_INCIDENCE_RECORDS, &incidence_records)
}

/// Encodes the merged view's typed property records and their side text blob.
/// The view yields triples in ascending `(subject, key)` order, so the emitted
/// array is sorted by `(subject_kind, subject_id, key)` (the order the base
/// attach binary-searches and validates).
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a text offset/length exceeds `u32` or
/// section planning fails.
///
/// # Performance
///
/// This function is `O(properties + text bytes)`.
fn encode_properties(builder: &mut SnapshotBuilder, view: &impl StateView) -> Result<(), DbError> {
    let mut records = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    for (subject, key, value) in view.properties() {
        let (subject_kind, subject_id) = wire::encode_subject(subject);
        let (scalar, text_off, text_len) = match value.as_ref() {
            PropertyValue::Boolean(flag) => (u64::from(*flag), 0, 0),
            PropertyValue::Integer(number) => ((*number).cast_unsigned(), 0, 0),
            PropertyValue::Text(string) => {
                let off = checked_u32(text.len())?;
                text.extend_from_slice(string.as_bytes());
                (0, off, checked_u32(string.len())?)
            }
        };
        records.push(wire::PropertyWire {
            subject_kind: U32::new(subject_kind),
            value_tag: U32::new(wire::property_type_tag(value.value_type())),
            subject_id: U64::new(subject_id),
            key: U64::new(key.get()),
            scalar: U64::new(scalar),
            text_off: U32::new(text_off),
            text_len: U32::new(text_len),
        });
    }
    add_typed(builder, wire::SECTION_PROPERTY_RECORDS, &records)?;
    add_blob(builder, wire::SECTION_PROPERTY_TEXT, text)
}

/// Builds the derived [`OwnedBaseIndex`] for this generation from the view's
/// records and serializes each of its five postings into a directory + value-pool
/// section, so the OPEN path borrows the postings instead of rebuilding them.
///
/// The index is a pure function of the records, so building it here and persisting
/// it keeps open `O(1)`-per-posting (binary search + page faults) rather than
/// `O(base)` (`from_records`).
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a posting offset/length exceeds the
/// `u32`/`u64` wire bounds or section planning fails.
///
/// # Performance
///
/// This function is `O(base records + labels + properties)`: one index build plus
/// one serialization pass.
fn encode_index(builder: &mut SnapshotBuilder, view: &impl StateView) -> Result<(), DbError> {
    let mut elements: BTreeMap<ElementId, ElementRecord> = BTreeMap::new();
    for record in view.elements() {
        elements.insert(record.id, record.into_owned());
    }
    let mut relations: BTreeMap<RelationId, RelationRecord> = BTreeMap::new();
    for record in view.relations() {
        relations.insert(record.id, record.into_owned());
    }
    let mut incidences: BTreeMap<IncidenceId, IncidenceRecord> = BTreeMap::new();
    for record in view.incidences() {
        incidences.insert(record.id, record.into_owned());
    }
    let mut properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> =
        BTreeMap::new();
    for (subject, key, value) in view.properties() {
        properties
            .entry(subject)
            .or_default()
            .insert(key, value.into_owned());
    }
    let index = OwnedBaseIndex::from_records(&elements, &relations, &incidences, &properties);
    let (
        label_members,
        relation_type_members,
        property_equality,
        element_incidences,
        relation_incidences,
    ) = index.maps();

    encode_simple_posting::<LabelId, ElementId>(
        builder,
        wire::SECTION_INDEX_LABEL_POSTINGS,
        label_members,
    )?;
    encode_simple_posting::<RelationTypeId, RelationId>(
        builder,
        wire::SECTION_INDEX_RELATION_TYPE_POSTINGS,
        relation_type_members,
    )?;
    encode_simple_posting::<ElementId, IncidenceId>(
        builder,
        wire::SECTION_INDEX_ELEMENT_INCIDENCES,
        element_incidences,
    )?;
    encode_simple_posting::<RelationId, IncidenceId>(
        builder,
        wire::SECTION_INDEX_RELATION_INCIDENCES,
        relation_incidences,
    )?;
    encode_equality_posting(builder, property_equality)
}

/// A canonical id (key or member) whose raw `u64` value the index posting
/// serializer reads. Mirrors the read-side decode in [`crate::index`].
///
/// # Performance
///
/// [`Self::raw`] is `O(1)`.
trait RawId: Copy {
    /// Returns the raw `u64` value of this id.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn raw(self) -> u64;
}

impl RawId for LabelId {
    fn raw(self) -> u64 {
        self.get()
    }
}

impl RawId for RelationTypeId {
    fn raw(self) -> u64 {
        self.get()
    }
}

impl RawId for ElementId {
    fn raw(self) -> u64 {
        self.get()
    }
}

impl RawId for RelationId {
    fn raw(self) -> u64 {
        self.get()
    }
}

impl RawId for IncidenceId {
    fn raw(self) -> u64 {
        self.get()
    }
}

/// Serializes one simple posting map (`key -> ascending member set`) into a
/// directory of [`wire::PostingDirEntry`] records (ascending by key, matching the
/// `BTreeMap` order) plus a trailing flat `[U64<LE>]` member value pool. The
/// directory and pool share one section: the directory comes first, the pool
/// follows, and the entry `(members_off, members_len)` index INTO THE POOL (in
/// `u64` words, relative to the pool start).
///
/// To keep both arrays in one section under the `add_section_typed` API, the
/// directory is emitted as its own typed sub-array and the pool as a typed
/// `[U64<LE>]` sub-array in the SAME kind via two record groups concatenated —
/// here both are encoded into a single `Vec<U64<LE>>` framed by a leading
/// directory-entry count. See [`split_posting_section`] for the read split.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when an offset/length exceeds the wire bounds
/// or section planning fails.
///
/// # Performance
///
/// This function is `O(postings + members)`.
fn encode_simple_posting<K, M>(
    builder: &mut SnapshotBuilder,
    kind: u32,
    map: &BTreeMap<K, BTreeSet<M>>,
) -> Result<(), DbError>
where
    K: RawId,
    M: RawId,
{
    if map.is_empty() {
        return Ok(());
    }
    let mut dir: Vec<wire::PostingDirEntry> = Vec::with_capacity(map.len());
    let mut pool: Vec<U64<LE>> = Vec::new();
    for (key, members) in map {
        let members_off = pool.len() as u64;
        pool.extend(members.iter().map(|member| U64::new(member.raw())));
        let members_len = members.len() as u64;
        dir.push(wire::PostingDirEntry {
            key: U64::new(key.raw()),
            members_off: U64::new(members_off),
            members_len: U64::new(members_len),
        });
    }
    add_framed_section(builder, kind, &dir, &pool)
}

/// Serializes the equality posting map (`(key, value) -> ascending subject set`)
/// into a directory of [`wire::EqualityDirEntry`] records (in `(key_id,
/// PropertyValue)` order, matching the `BTreeMap` order) plus a trailing flat
/// `[U64<LE>]` subject value pool (two words per subject from
/// [`wire::encode_subject`]) and a side text pool ([`wire::SECTION_INDEX_EQUALITY_TEXT`]).
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when an offset/length exceeds the wire bounds
/// or section planning fails.
///
/// # Performance
///
/// This function is `O(postings + subjects + text bytes)`.
fn encode_equality_posting(
    builder: &mut SnapshotBuilder,
    map: &BTreeMap<(PropertyKeyId, PropertyValue), BTreeSet<PropertySubject>>,
) -> Result<(), DbError> {
    if map.is_empty() {
        return Ok(());
    }
    let mut dir: Vec<wire::EqualityDirEntry> = Vec::with_capacity(map.len());
    let mut pool: Vec<U64<LE>> = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    for ((key, value), subjects) in map {
        let (value_tag, value_scalar, text_off, text_len) = match value {
            PropertyValue::Boolean(flag) => (
                wire::property_type_tag(PropertyType::Boolean),
                u64::from(*flag),
                0,
                0,
            ),
            PropertyValue::Integer(number) => (
                wire::property_type_tag(PropertyType::Integer),
                (*number).cast_unsigned(),
                0,
                0,
            ),
            PropertyValue::Text(string) => {
                let off = text.len() as u64;
                text.extend_from_slice(string.as_bytes());
                (
                    wire::property_type_tag(PropertyType::Text),
                    0,
                    off,
                    string.len() as u64,
                )
            }
        };
        let members_off = pool.len() as u64;
        for subject in subjects {
            let (subject_kind, subject_id) = wire::encode_subject(*subject);
            pool.push(U64::new(u64::from(subject_kind)));
            pool.push(U64::new(subject_id));
        }
        // `members_len` counts POOL WORDS (two per subject), so the read side
        // slices the whole run and `chunks_exact(2)` recovers the subjects.
        let members_len = (pool.len() as u64) - members_off;
        dir.push(wire::EqualityDirEntry {
            key_id: U64::new(key.get()),
            value_tag: U32::new(value_tag),
            reserved: U32::new(0),
            value_scalar: U64::new(value_scalar),
            text_off: U64::new(text_off),
            text_len: U64::new(text_len),
            members_off: U64::new(members_off),
            members_len: U64::new(members_len),
        });
    }
    add_framed_section(builder, wire::SECTION_INDEX_EQUALITY, &dir, &pool)?;
    add_blob(builder, wire::SECTION_INDEX_EQUALITY_TEXT, text)
}

/// Rebuilds the catalog from its record sections and the definition-body run.
/// Shared with the base-attach path in [`crate::backing`], which borrows the same
/// sections.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a record is malformed, a name slice is
/// out of bounds, or a definition kind is unknown.
///
/// # Performance
///
/// This function is `O(catalog entries + name bytes)`.
pub(crate) fn decode_catalog(
    snapshot: &Snapshot<'_>,
    table: &[u8],
    defs: &[U64<LE>],
) -> Result<Catalog, DbError> {
    let mut catalog = Catalog::empty();
    for record in typed_records::<wire::NamedWire>(snapshot, wire::SECTION_CATALOG_ROLES)? {
        let name = read_str(table, record.name_off.get(), record.name_len.get())?;
        catalog.insert_role(RoleId::new(record.id.get()), name)?;
    }
    for record in typed_records::<wire::NamedWire>(snapshot, wire::SECTION_CATALOG_LABELS)? {
        let name = read_str(table, record.name_off.get(), record.name_len.get())?;
        catalog.insert_label(LabelId::new(record.id.get()), name)?;
    }
    for record in typed_records::<wire::NamedWire>(snapshot, wire::SECTION_CATALOG_RELATION_TYPES)?
    {
        let name = read_str(table, record.name_off.get(), record.name_len.get())?;
        catalog.insert_relation_type(RelationTypeId::new(record.id.get()), name)?;
    }
    for record in
        typed_records::<wire::PropertyKeyWire>(snapshot, wire::SECTION_CATALOG_PROPERTY_KEYS)?
    {
        let name = read_str(table, record.name_off.get(), record.name_len.get())?;
        let family = wire::property_family_from_tag(record.family.get())
            .ok_or_else(|| DbError::invalid_store("unknown property family tag"))?;
        let value_type = wire::property_type_from_tag(record.value_type.get())
            .ok_or_else(|| DbError::invalid_store("unknown property type tag"))?;
        catalog.insert_property_key(PropertyKeyDefinition {
            id: PropertyKeyId::new(record.id.get()),
            name,
            family,
            value_type,
        })?;
    }
    for record in typed_records::<wire::DefWire>(snapshot, wire::SECTION_CATALOG_PROJECTIONS)? {
        let name = read_str(table, record.name_off.get(), record.name_len.get())?;
        catalog.insert_projection(
            ProjectionId::new(record.id.get()),
            decode_projection_def(record, name, defs)?,
        )?;
    }
    for record in typed_records::<wire::DefWire>(snapshot, wire::SECTION_CATALOG_INDEXES)? {
        let name = read_str(table, record.name_off.get(), record.name_len.get())?;
        catalog.insert_index(
            IndexId::new(record.id.get()),
            name,
            decode_index_def(record, defs)?,
        )?;
    }
    Ok(catalog)
}

/// Byte length of the framing prefix on a posting section: two `u64` words
/// giving the directory byte length and the pool byte length, so the read split
/// reinterprets each sub-slice as its typed array.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const POSTING_FRAME_PREFIX_LEN: usize = 2 * size_of::<U64<LE>>();

/// Writes a framed posting section: a two-`u64` prefix `(directory byte length,
/// pool byte length)` followed by the directory bytes and then the value-pool
/// bytes, all in one section. The read split ([`split_posting_section`])
/// reinterprets each region as its typed array.
///
/// Both `T` (a directory entry) and [`U64<LE>`] have alignment 1 (byteorder
/// wrappers store raw bytes), so the concatenated regions reinterpret at any byte
/// offset; the prefix is a whole number of `u64` words, keeping every region
/// 8-byte aligned anyway.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a region length exceeds the `u64`
/// bounds or section planning fails.
///
/// # Performance
///
/// This function is `O(directory bytes + pool bytes)`.
fn add_framed_section<T>(
    builder: &mut SnapshotBuilder,
    kind: u32,
    dir: &[T],
    pool: &[U64<LE>],
) -> Result<(), DbError>
where
    T: zerocopy::IntoBytes + zerocopy::Immutable,
{
    let dir_bytes = dir.as_bytes();
    let pool_bytes = pool.as_bytes();
    let mut payload =
        Vec::with_capacity(POSTING_FRAME_PREFIX_LEN + dir_bytes.len() + pool_bytes.len());
    payload.extend_from_slice(U64::<LE>::new(dir_bytes.len() as u64).as_bytes());
    payload.extend_from_slice(U64::<LE>::new(pool_bytes.len() as u64).as_bytes());
    payload.extend_from_slice(dir_bytes);
    payload.extend_from_slice(pool_bytes);
    builder
        .add_section(kind, wire::OXGDB_SECTION_VERSION, 3, payload)
        .map_err(|error| DbError::invalid_store(error.to_string()))?;
    Ok(())
}

/// Splits a framed posting section's raw bytes into its directory byte slice and
/// its value-pool byte slice, using the two-`u64` length prefix. Shared with the
/// borrowing open path in [`crate::backing`].
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the prefix is missing or the region
/// lengths run past the section bytes.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) fn split_posting_section(bytes: &[u8]) -> Result<(&[u8], &[u8]), DbError> {
    if bytes.is_empty() {
        return Ok((&[], &[]));
    }
    let prefix = bytes
        .get(..POSTING_FRAME_PREFIX_LEN)
        .ok_or_else(|| DbError::invalid_store("posting section is missing its frame prefix"))?;
    let dir_len_word =
        U64::<LE>::ref_from_bytes(&prefix[..size_of::<U64<LE>>()]).map_err(|_error| {
            DbError::invalid_store("posting section directory length is truncated")
        })?;
    let pool_len_word = U64::<LE>::ref_from_bytes(&prefix[size_of::<U64<LE>>()..])
        .map_err(|_error| DbError::invalid_store("posting section pool length is truncated"))?;
    let dir_len = usize::try_from(dir_len_word.get())
        .map_err(|_overflow| DbError::invalid_store("posting section directory length overflow"))?;
    let pool_len = usize::try_from(pool_len_word.get())
        .map_err(|_overflow| DbError::invalid_store("posting section pool length overflow"))?;
    let dir_start = POSTING_FRAME_PREFIX_LEN;
    let dir_end = dir_start
        .checked_add(dir_len)
        .ok_or_else(|| DbError::invalid_store("posting section directory overflow"))?;
    let pool_end = dir_end
        .checked_add(pool_len)
        .ok_or_else(|| DbError::invalid_store("posting section pool overflow"))?;
    let dir = bytes
        .get(dir_start..dir_end)
        .ok_or_else(|| DbError::invalid_store("posting section directory out of bounds"))?;
    let pool = bytes
        .get(dir_end..pool_end)
        .ok_or_else(|| DbError::invalid_store("posting section pool out of bounds"))?;
    Ok((dir, pool))
}

/// Encodes a projection definition body into the shared `u64` run, returning
/// its `(kind, offset, len)` in run words.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the body length exceeds `u32`.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn encode_projection_def(
    definition: &ProjectionDefinition,
    defs: &mut Vec<U64<LE>>,
) -> Result<(u32, u32, u32), DbError> {
    let offset = checked_u32(defs.len())?;
    let kind = match definition {
        ProjectionDefinition::Graph(graph) => {
            defs.push(U64::new(graph.source_role.get()));
            defs.push(U64::new(graph.target_role.get()));
            push_id_set(defs, graph.relation_types.iter().map(|id| id.get()))?;
            DEF_PROJECTION_GRAPH
        }
        ProjectionDefinition::Hypergraph(hyper) => {
            push_id_set(defs, hyper.source_roles.iter().map(|id| id.get()))?;
            push_id_set(defs, hyper.target_roles.iter().map(|id| id.get()))?;
            push_id_set(defs, hyper.relation_types.iter().map(|id| id.get()))?;
            DEF_PROJECTION_HYPER
        }
    };
    let len = checked_u32(defs.len() - usize_of(offset))?;
    Ok((kind, offset, len))
}

/// Encodes an index definition body into the shared `u64` run, returning its
/// `(kind, offset, len)` in run words.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the body length exceeds `u32`.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn encode_index_def(
    definition: &IndexDefinition,
    defs: &mut Vec<U64<LE>>,
) -> Result<(u32, u32, u32), DbError> {
    let offset = checked_u32(defs.len())?;
    let kind = match definition {
        IndexDefinition::Label { label } => {
            defs.push(U64::new(label.get()));
            DEF_INDEX_LABEL
        }
        IndexDefinition::RelationType { relation_type } => {
            defs.push(U64::new(relation_type.get()));
            DEF_INDEX_RELATION_TYPE
        }
        IndexDefinition::PropertyEquality { key } => {
            defs.push(U64::new(key.get()));
            DEF_INDEX_PROPERTY_EQUALITY
        }
        IndexDefinition::PropertyRange { key } => {
            defs.push(U64::new(key.get()));
            DEF_INDEX_PROPERTY_RANGE
        }
        IndexDefinition::CompositeEquality { keys } => {
            for key in keys {
                defs.push(U64::new(key.get()));
            }
            DEF_INDEX_COMPOSITE_EQUALITY
        }
        IndexDefinition::Projection { projection } => {
            defs.push(U64::new(projection.get()));
            DEF_INDEX_PROJECTION
        }
    };
    let len = checked_u32(defs.len() - usize_of(offset))?;
    Ok((kind, offset, len))
}

/// Pushes a length-prefixed id set into the definition run.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the set size exceeds `u32`.
///
/// # Performance
///
/// This function is `O(set size)`.
fn push_id_set(
    defs: &mut Vec<U64<LE>>,
    ids: impl ExactSizeIterator<Item = u64>,
) -> Result<(), DbError> {
    let count = checked_u32(ids.len())?;
    defs.push(U64::new(u64::from(count)));
    for id in ids {
        defs.push(U64::new(id));
    }
    Ok(())
}

/// Borrows a definition body slice out of the shared run.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the body slice is out of bounds.
///
/// # Performance
///
/// This function is `O(1)`.
fn def_body<'run>(
    record: &wire::DefWire,
    defs: &'run [U64<LE>],
) -> Result<&'run [U64<LE>], DbError> {
    let start = usize_of(record.payload_off.get());
    let end = start
        .checked_add(usize_of(record.payload_len.get()))
        .ok_or_else(|| DbError::invalid_store("definition body overflow"))?;
    defs.get(start..end)
        .ok_or_else(|| DbError::invalid_store("definition body out of bounds"))
}

/// Reads a length-prefixed id set starting at `cursor`, advancing it past the
/// set.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the length or slice is out of bounds.
///
/// # Performance
///
/// This function is `O(set size)`.
fn read_id_set(body: &[U64<LE>], cursor: &mut usize) -> Result<Vec<u64>, DbError> {
    let count = usize::try_from(
        body.get(*cursor)
            .ok_or_else(|| DbError::invalid_store("missing id-set length"))?
            .get(),
    )
    .map_err(|_error| DbError::invalid_store("id-set length exceeds usize"))?;
    *cursor += 1;
    let end = cursor
        .checked_add(count)
        .ok_or_else(|| DbError::invalid_store("id-set overflow"))?;
    let slice = body
        .get(*cursor..end)
        .ok_or_else(|| DbError::invalid_store("id-set out of bounds"))?;
    let ids = slice.iter().map(|word| word.get()).collect();
    *cursor = end;
    Ok(ids)
}

/// Decodes a projection definition from its record and the shared run.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the body is malformed or the kind is
/// unknown.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn decode_projection_def(
    record: &wire::DefWire,
    name: String,
    defs: &[U64<LE>],
) -> Result<ProjectionDefinition, DbError> {
    let body = def_body(record, defs)?;
    match record.kind.get() {
        DEF_PROJECTION_GRAPH => {
            let source_role = body
                .first()
                .ok_or_else(|| DbError::invalid_store("graph projection missing source role"))?
                .get();
            let target_role = body
                .get(1)
                .ok_or_else(|| DbError::invalid_store("graph projection missing target role"))?
                .get();
            let mut cursor = 2;
            let relation_types = read_id_set(body, &mut cursor)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Graph(GraphProjectionDefinition {
                name,
                relation_types,
                source_role: RoleId::new(source_role),
                target_role: RoleId::new(target_role),
            }))
        }
        DEF_PROJECTION_HYPER => {
            let mut cursor = 0;
            let source_roles = read_id_set(body, &mut cursor)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let target_roles = read_id_set(body, &mut cursor)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let relation_types = read_id_set(body, &mut cursor)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Hypergraph(
                HypergraphProjectionDefinition {
                    name,
                    relation_types,
                    source_roles,
                    target_roles,
                },
            ))
        }
        _other => Err(DbError::invalid_store("unknown projection definition kind")),
    }
}

/// Decodes an index definition from its record and the shared run.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the body is malformed or the kind is
/// unknown.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn decode_index_def(record: &wire::DefWire, defs: &[U64<LE>]) -> Result<IndexDefinition, DbError> {
    let body = def_body(record, defs)?;
    let first = || {
        body.first()
            .ok_or_else(|| DbError::invalid_store("index definition missing id"))
            .map(|word| word.get())
    };
    match record.kind.get() {
        DEF_INDEX_LABEL => Ok(IndexDefinition::Label {
            label: LabelId::new(first()?),
        }),
        DEF_INDEX_RELATION_TYPE => Ok(IndexDefinition::RelationType {
            relation_type: RelationTypeId::new(first()?),
        }),
        DEF_INDEX_PROPERTY_EQUALITY => Ok(IndexDefinition::PropertyEquality {
            key: PropertyKeyId::new(first()?),
        }),
        DEF_INDEX_PROPERTY_RANGE => Ok(IndexDefinition::PropertyRange {
            key: PropertyKeyId::new(first()?),
        }),
        DEF_INDEX_COMPOSITE_EQUALITY => Ok(IndexDefinition::CompositeEquality {
            keys: body
                .iter()
                .map(|word| PropertyKeyId::new(word.get()))
                .collect(),
        }),
        DEF_INDEX_PROJECTION => Ok(IndexDefinition::Projection {
            projection: ProjectionId::new(first()?),
        }),
        _other => Err(DbError::invalid_store("unknown index definition kind")),
    }
}

#[cfg(test)]
mod tests {
    use zerocopy::FromBytes;

    use super::*;
    use crate::overlay::test_support::base_view_from_ops;

    /// Frozen base bytes carry a [`wire::SECTION_BASE_TRAILER`] whose recorded
    /// CRC equals a fresh CRC-32C recomputed over every byte preceding the
    /// trailer payload — the exact check [`crate::backing::Base::open`] performs.
    #[test]
    fn freeze_emits_base_trailer_with_validating_crc() {
        let (base, overlay) = base_view_from_ops();
        let view = crate::overlay::MergedState::new(&base, &overlay);
        let bytes = freeze_view(
            &view,
            FreezeStamps {
                commit_seq: 42,
                transaction_id: 43,
                generation: 44,
            },
        )
        .expect("freeze view");

        let snapshot = Snapshot::open(&bytes).expect("reopen frozen base");
        let trailer_section = snapshot
            .section(wire::SECTION_BASE_TRAILER)
            .expect("frozen base has a trailer section");
        let payload_offset = trailer_section.bytes().as_ptr().addr() - bytes.as_ptr().addr();

        let trailer = wire::BaseTrailer::ref_from_bytes(trailer_section.bytes())
            .expect("trailer payload is a BaseTrailer");
        assert_eq!(
            trailer.reserved.get(),
            0,
            "trailer reserved word must be zero"
        );

        let recomputed = crc::checksum(&bytes[..payload_offset]);
        assert_eq!(
            trailer.crc32c.get(),
            recomputed,
            "stored trailer CRC must cover the whole prefix before its payload",
        );
        assert_ne!(recomputed, 0, "non-empty base prefix has a non-zero CRC");
    }
}

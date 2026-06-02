//! OXGDB v1 freeze/open: encode a [`StoredDatabase`] into typed zero-copy
//! `DATABASE_BAND` sections and decode it back.
//!
//! Each component — header, catalog, topology, properties — persists as
//! its own typed section described in [`crate::wire`]. Reading borrows the
//! fixed records directly from the mapped bytes; only variable-length data
//! (names, labels, property text, definition bodies) is copied out into the
//! owned [`StoredDatabase`].
//!
//! # Performance
//!
//! [`freeze`] is `O(catalog + topology + properties)`; [`open`] is the same,
//! dominated by rebuilding the owned in-memory collections.

use std::collections::{BTreeMap, BTreeSet};

use oxgraph_snapshot::{Snapshot, SnapshotBuilder};
use zerocopy::byteorder::{LE, U32, U64};

use crate::{
    Catalog, DbError, ElementId, ElementRecord, IncidenceId, IncidenceRecord, IndexId, LabelId,
    ProjectionId, PropertyKeyId, PropertySubject, PropertyValue, RelationId, RelationRecord,
    RelationTypeId, RoleId,
    catalog::{
        GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
        ProjectionDefinition, PropertyKeyDefinition,
    },
    id::{CheckpointGeneration, CommitSeq, TransactionId},
    state::{DatabaseParts, DatabaseState, NextIds},
    storage::StoredDatabase,
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
/// # Performance
///
/// This function is `O(1)`.
fn typed_records<'view, T>(snapshot: &Snapshot<'view>, kind: u32) -> Result<&'view [T], DbError>
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
fn raw_blob<'view>(snapshot: &Snapshot<'view>, kind: u32) -> &'view [u8] {
    snapshot
        .section(kind)
        .map_or(&[][..], |section| section.bytes())
}

/// Adds a typed record section, skipping it when empty so the open path can
/// treat absence as an empty collection.
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

/// Encodes a stored database into deterministic OXGDB v1 snapshot bytes.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a length exceeds the `u32` wire
/// bounds or section planning fails.
///
/// # Performance
///
/// This function is `O(catalog + topology + properties)`.
pub(crate) fn freeze(stored: &StoredDatabase) -> Result<Vec<u8>, DbError> {
    let state = &stored.state;
    let mut builder = SnapshotBuilder::new();
    let mut strings = StringTable::default();
    encode_header(&mut builder, stored)?;
    encode_catalog(&mut builder, state.catalog(), &mut strings)?;
    encode_topology(&mut builder, state)?;
    encode_properties(&mut builder, state)?;
    add_blob(&mut builder, wire::SECTION_STRING_TABLE, strings.bytes)?;
    builder
        .finish()
        .map_err(|error| DbError::invalid_store(error.to_string()))
}

/// Encodes the fixed header record (version, stamps, id allocators).
///
/// # Performance
///
/// This function is `O(1)`.
fn encode_header(builder: &mut SnapshotBuilder, stored: &StoredDatabase) -> Result<(), DbError> {
    let next = stored.state.next_ids();
    let header = wire::DbHeaderRecord {
        format_version: U32::new(wire::OXGDB_FORMAT_VERSION),
        flags: U32::new(0),
        commit_seq: U64::new(stored.commit_seq.get()),
        transaction_id: U64::new(stored.transaction_id.get()),
        checkpoint_generation: U64::new(stored.generation.get()),
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

/// Encodes elements, relations, and incidences with their side label runs.
///
/// # Performance
///
/// This function is `O(topology size + labels)`.
fn encode_topology(builder: &mut SnapshotBuilder, state: &DatabaseState) -> Result<(), DbError> {
    let mut element_records = Vec::new();
    let mut element_labels: Vec<U64<LE>> = Vec::new();
    for record in state.elements() {
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
    for record in state.relations() {
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
    for record in state.incidences() {
        incidence_records.push(wire::IncidenceWire {
            id: U64::new(record.id.get()),
            relation: U64::new(record.relation.get()),
            element: U64::new(record.element.get()),
            role: U64::new(record.role.get()),
        });
    }
    add_typed(builder, wire::SECTION_INCIDENCE_RECORDS, &incidence_records)
}

/// Encodes typed property records and their side text blob.
///
/// # Performance
///
/// This function is `O(properties + text bytes)`.
fn encode_properties(builder: &mut SnapshotBuilder, state: &DatabaseState) -> Result<(), DbError> {
    let mut records = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    for (subject, key, value) in state.property_iter() {
        let (subject_kind, subject_id) = wire::encode_subject(subject);
        let (scalar, text_off, text_len) = match value {
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

/// Decodes OXGDB v1 snapshot bytes back into a stored database.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the bytes are malformed, carry an
/// unknown format version, or fail semantic validation.
///
/// # Performance
///
/// This function is `O(catalog + topology + properties)`.
pub(crate) fn open(bytes: &[u8]) -> Result<StoredDatabase, DbError> {
    let snapshot =
        Snapshot::open(bytes).map_err(|error| DbError::invalid_store(error.to_string()))?;
    check_exact_length(&snapshot, bytes)?;

    let headers = typed_records::<wire::DbHeaderRecord>(&snapshot, wire::SECTION_DB_HEADER)?;
    let header = headers
        .first()
        .ok_or_else(|| DbError::invalid_store("store is missing the header section"))?;
    if header.format_version.get() != wire::OXGDB_FORMAT_VERSION {
        return Err(DbError::invalid_store("unsupported OXGDB format version"));
    }

    let table = raw_blob(&snapshot, wire::SECTION_STRING_TABLE);
    let defs = typed_records::<U64<LE>>(&snapshot, wire::SECTION_CATALOG_DEFS)?;
    let catalog = decode_catalog(&snapshot, table, defs)?;
    let (elements, relations, incidences) = decode_topology(&snapshot)?;
    let properties = decode_properties(&snapshot)?;

    let state = DatabaseState::from_parts(DatabaseParts {
        elements,
        relations,
        incidences,
        properties,
        catalog,
        next: read_next_ids(header),
    });
    state.validate()?;

    Ok(StoredDatabase {
        commit_seq: CommitSeq::new(header.commit_seq.get()),
        transaction_id: TransactionId::new(header.transaction_id.get()),
        generation: CheckpointGeneration::new(header.checkpoint_generation.get()),
        state,
    })
}

/// Rejects trailing or truncated bytes. Every OXGDB section is byte-aligned, so
/// the canonical length is the header plus the section table plus the
/// contiguous payloads; the container otherwise tolerates trailing bytes.
///
/// # Performance
///
/// This function is `O(section count)`.
fn check_exact_length(snapshot: &Snapshot<'_>, bytes: &[u8]) -> Result<(), DbError> {
    let payload_len: usize = snapshot
        .sections()
        .map(|section| section.bytes().len())
        .sum();
    let expected_len = oxgraph_snapshot::HEADER_SIZE
        + snapshot.section_count() * oxgraph_snapshot::SECTION_ENTRY_SIZE
        + payload_len;
    if bytes.len() == expected_len {
        Ok(())
    } else {
        Err(DbError::invalid_store(
            "store has trailing or truncated bytes",
        ))
    }
}

/// Reads the nine id allocators from the header record.
///
/// # Performance
///
/// This function is `O(1)`.
const fn read_next_ids(header: &wire::DbHeaderRecord) -> NextIds {
    NextIds {
        element: ElementId::new(header.next_element.get()),
        relation: RelationId::new(header.next_relation.get()),
        incidence: IncidenceId::new(header.next_incidence.get()),
        role: RoleId::new(header.next_role.get()),
        label: LabelId::new(header.next_label.get()),
        relation_type: RelationTypeId::new(header.next_relation_type.get()),
        property_key: PropertyKeyId::new(header.next_property_key.get()),
        projection: ProjectionId::new(header.next_projection.get()),
        index: IndexId::new(header.next_index.get()),
    }
}

/// Rebuilds the catalog from its record sections and the definition-body run.
///
/// # Performance
///
/// This function is `O(catalog entries + name bytes)`.
fn decode_catalog(
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

/// The decoded topology maps returned by [`decode_topology`].
type DecodedTopology = (
    BTreeMap<ElementId, ElementRecord>,
    BTreeMap<RelationId, RelationRecord>,
    BTreeMap<IncidenceId, IncidenceRecord>,
);

/// Rebuilds the topology maps from their record and label-run sections.
///
/// # Performance
///
/// This function is `O(topology size + labels)`.
fn decode_topology(snapshot: &Snapshot<'_>) -> Result<DecodedTopology, DbError> {
    let element_labels = typed_records::<U64<LE>>(snapshot, wire::SECTION_ELEMENT_LABELS)?;
    let mut elements = BTreeMap::new();
    for record in typed_records::<wire::ElementWire>(snapshot, wire::SECTION_ELEMENT_RECORDS)? {
        let labels = read_label_set(
            element_labels,
            record.label_off.get(),
            record.label_len.get(),
        )?;
        let id = ElementId::new(record.id.get());
        elements.insert(id, ElementRecord { id, labels });
    }

    let relation_labels = typed_records::<U64<LE>>(snapshot, wire::SECTION_RELATION_LABELS)?;
    let mut relations = BTreeMap::new();
    for record in typed_records::<wire::RelationWire>(snapshot, wire::SECTION_RELATION_RECORDS)? {
        let labels = read_label_set(
            relation_labels,
            record.label_off.get(),
            record.label_len.get(),
        )?;
        let id = RelationId::new(record.id.get());
        relations.insert(
            id,
            RelationRecord {
                id,
                relation_type: wire::decode_relation_type(record.relation_type.get()),
                labels,
            },
        );
    }

    let mut incidences = BTreeMap::new();
    for record in typed_records::<wire::IncidenceWire>(snapshot, wire::SECTION_INCIDENCE_RECORDS)? {
        let id = IncidenceId::new(record.id.get());
        incidences.insert(
            id,
            IncidenceRecord {
                id,
                relation: RelationId::new(record.relation.get()),
                element: ElementId::new(record.element.get()),
                role: RoleId::new(record.role.get()),
            },
        );
    }
    Ok((elements, relations, incidences))
}

/// Rebuilds the property map from its record section and text blob.
///
/// # Performance
///
/// This function is `O(properties + text bytes)`.
fn decode_properties(
    snapshot: &Snapshot<'_>,
) -> Result<BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>, DbError> {
    let text = raw_blob(snapshot, wire::SECTION_PROPERTY_TEXT);
    let mut properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> =
        BTreeMap::new();
    for record in typed_records::<wire::PropertyWire>(snapshot, wire::SECTION_PROPERTY_RECORDS)? {
        let subject = wire::decode_subject(record.subject_kind.get(), record.subject_id.get())
            .ok_or_else(|| DbError::invalid_store("unknown property subject kind"))?;
        let value = decode_property_value(record, text)?;
        properties
            .entry(subject)
            .or_default()
            .insert(PropertyKeyId::new(record.key.get()), value);
    }
    Ok(properties)
}

/// Reads a label set out of a label run by `(offset, len)` element indices.
///
/// # Performance
///
/// This function is `O(len)`.
fn read_label_set(run: &[U64<LE>], offset: u32, len: u32) -> Result<BTreeSet<LabelId>, DbError> {
    let start = usize_of(offset);
    let end = start
        .checked_add(usize_of(len))
        .ok_or_else(|| DbError::invalid_store("label slice overflow"))?;
    let slice = run
        .get(start..end)
        .ok_or_else(|| DbError::invalid_store("label slice out of bounds"))?;
    Ok(slice.iter().map(|word| LabelId::new(word.get())).collect())
}

/// Decodes a property value from its record and the shared text blob.
///
/// # Performance
///
/// This function is `O(value length)` for text and `O(1)` otherwise.
fn decode_property_value(
    record: &wire::PropertyWire,
    text: &[u8],
) -> Result<PropertyValue, DbError> {
    let value_type = wire::property_type_from_tag(record.value_tag.get())
        .ok_or_else(|| DbError::invalid_store("unknown property value tag"))?;
    Ok(match value_type {
        crate::PropertyType::Boolean => PropertyValue::Boolean(record.scalar.get() != 0),
        crate::PropertyType::Integer => PropertyValue::Integer(record.scalar.get().cast_signed()),
        crate::PropertyType::Text => PropertyValue::Text(read_str(
            text,
            record.text_off.get(),
            record.text_len.get(),
        )?),
    })
}

/// Encodes a projection definition body into the shared `u64` run, returning
/// its `(kind, offset, len)` in run words.
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

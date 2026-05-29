//! Property and identity snapshot encoding, validation, and decoding.
//!
//! Owns the wire record/header types, the [`PropertySnapshotEncoder`], the
//! `encode_*` / `validate_*` entry points, the [`DecodedPropertyLayer`]
//! decoder, and the snapshot-only byte/Arrow-IPC helpers.

use std::{
    collections::BTreeSet,
    io::Cursor,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use arrow_array::{Array, ArrayRef, PrimitiveArray, RecordBatch};
use arrow_ipc::{reader::StreamReader, writer::StreamWriter};
use arrow_schema::{DataType, Field, Schema};
use oxgraph_snapshot::Snapshot;
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U64},
};

use crate::{
    model::{
        IdFamily, IdentityMapMode, IdentityModeRecord, IdentityModeSummary,
        IdentitySnapshotSummary, LayerName, LayerRole, MissingPolicy, PropertyError, PropertyLayer,
        PropertyLayerData, StorageMode, id_family_from_tag, id_family_tag, layer_role_from_tag,
        layer_role_tag, map_arrow_error, missing_policy_tag, storage_from_tags, storage_tag,
        validate_sparse_indices,
    },
    weights::{GraphPropertyLayers, HyperPropertyLayers},
    width::{
        PropertyIndex, PropertySnapshotMetaWord, SNAPSHOT_PROPERTY_VERSION, le_word,
        le_word_to_u32, le_word_to_u64, le_word_to_usize,
    },
};

/// Validates identity mode and explicit map sections in a snapshot.
///
/// # Errors
///
/// Returns [`PropertyError`] if mode records are malformed, duplicated, or if
/// an explicit map is missing or length-inconsistent.
///
/// # Performance
///
/// This function is `O(s + f)` for snapshot section count `s` and identity
/// family count `f`.
pub fn validate_identity_snapshot<W>(
    snapshot: &Snapshot<'_>,
) -> Result<IdentitySnapshotSummary, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let section =
        snapshot
            .section(W::IDENTITY_MODES_KIND)
            .ok_or(PropertyError::MissingSnapshotSection {
                kind: W::IDENTITY_MODES_KIND,
            })?;
    if section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind: W::IDENTITY_MODES_KIND,
            version: section.version(),
        });
    }
    let records: &[IdentityModeRecord<W>] =
        section
            .try_as_slice()
            .map_err(|error| PropertyError::SnapshotSectionView {
                kind: W::IDENTITY_MODES_KIND,
                error,
            })?;
    let records = validate_identity_records::<W>(snapshot, records)?;
    Ok(IdentitySnapshotSummary { records })
}

/// Encoded property descriptor and Arrow IPC data payloads.
///
/// # Performance
///
/// Cloning is `O(descriptor bytes + data bytes)`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct EncodedPropertySnapshot {
    /// Payload for the selected property descriptor section kind.
    pub descriptors: Vec<u8>,
    /// Payload for the selected property data section kind.
    pub data: Vec<u8>,
}

/// Summary returned after property snapshot validation.
///
/// # Performance
///
/// Cloning is `O(layer_count)`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct PropertySnapshotSummary {
    /// Number of validated property layers.
    pub layer_count: usize,
    /// Total logical values across layers.
    pub total_logical_values: usize,
}

/// Arrow payload of a property layer decoded from snapshot bytes.
///
/// Dense layers expose a single value array indexed by logical position. Sparse
/// layers expose the explicit `(indices, values)` pair plus an optional
/// non-null default array; the index array's [`arrow_schema::DataType`] matches
/// the encoded sparse index width.
///
/// # Performance
///
/// Cloning is `O(1)` (each variant holds [`ArrayRef`] handles).
#[derive(Clone, Debug)]
#[must_use]
#[non_exhaustive]
pub enum DecodedPropertyData {
    /// Dense Arrow values; `values.len()` equals the descriptor's logical length.
    Dense {
        /// Decoded value Arrow array.
        values: ArrayRef,
    },
    /// Sparse Arrow values plus optional default scalar.
    Sparse {
        /// Sparse index Arrow array; `indices.len() == values.len()`.
        indices: ArrayRef,
        /// Sparse value Arrow array; `values.len() == indices.len()`.
        values: ArrayRef,
        /// Length-one non-null default array when [`MissingPolicy::Default`] is in effect.
        default: Option<ArrayRef>,
    },
}

/// One property layer decoded from snapshot bytes.
///
/// Returned by [`DecodedPropertyLayer::decode_all`] and
/// [`DecodedPropertyLayer::decode_sections`].
/// Field types mirror the descriptor record without exposing the wire word
/// width, so callers can introspect the layer without referencing
/// [`PropertySnapshotMetaWord`] directly.
///
/// # Performance
///
/// Cloning is `O(name bytes)` (the Arrow payload clones in `O(1)`).
#[derive(Clone, Debug)]
#[must_use]
pub struct DecodedPropertyLayer {
    /// Stable layer ID as decoded from the descriptor record.
    pub layer_id: u64,
    /// Layer name decoded from the descriptor string table.
    pub name: String,
    /// ID family the layer is keyed by.
    pub id_family: IdFamily,
    /// Layer role tag.
    pub role: LayerRole,
    /// Storage mode (carrying the sparse missing policy when applicable).
    pub storage: StorageMode,
    /// Logical layer length declared by the descriptor record.
    pub logical_len: usize,
    /// Arrow payload decoded from the layer's IPC value (and optional default) stream.
    pub data: DecodedPropertyData,
}

/// Wire header for the property descriptor section.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(crate) struct PropertySnapshotHeader {
    /// Number of descriptor records.
    record_count: U64<LE>,
    /// Byte length occupied by descriptor records after this header.
    record_bytes: U64<LE>,
}

/// Wire descriptor record for one property layer.
#[derive(Clone, Copy, Debug, Eq, FromBytes, Immutable, IntoBytes, KnownLayout, PartialEq)]
#[repr(C)]
pub struct PropertySnapshotRecord<W>
where
    W: PropertySnapshotMetaWord,
{
    /// Stable layer ID.
    layer_id: W::LittleEndianWord,
    /// Offset of layer name in descriptor string table.
    name_offset: W::LittleEndianWord,
    /// Length of layer name in descriptor string table.
    name_len: W::LittleEndianWord,
    /// ID-family tag.
    id_family: W::LittleEndianWord,
    /// Layer-role tag.
    role: W::LittleEndianWord,
    /// Storage tag.
    storage: W::LittleEndianWord,
    /// Missing-policy tag.
    missing_policy: W::LittleEndianWord,
    /// Logical layer length.
    logical_len: W::LittleEndianWord,
    /// Explicit sparse value count, or dense value count.
    value_count: W::LittleEndianWord,
    /// Offset of the value Arrow IPC stream in the property data section.
    value_data_offset: W::LittleEndianWord,
    /// Byte length of the value Arrow IPC stream in the property data section.
    value_data_len: W::LittleEndianWord,
    /// Offset of the sparse-default Arrow IPC stream in the property data section.
    default_data_offset: W::LittleEndianWord,
    /// Byte length of the sparse-default Arrow IPC stream in the property data section.
    default_data_len: W::LittleEndianWord,
    /// Reserved for future descriptor flags.
    reserved: W::LittleEndianWord,
}

/// Encodes property descriptor and Arrow IPC data sections.
///
/// # Errors
///
/// Returns [`PropertyError`] for duplicate layer IDs/names or inconsistent
/// descriptor/storage combinations.
///
/// # Performance
///
/// This function is `O(l + total values + total name bytes)` for `l` layers.
pub fn encode_property_snapshot<W, Id, I>(
    layers: &[PropertyLayer<Id, I>],
) -> Result<EncodedPropertySnapshot, PropertyError>
where
    W: PropertySnapshotMetaWord,
    Id: Copy + Into<u64> + TryInto<W>,
    I: PropertyIndex,
{
    let mut encoder = PropertySnapshotEncoder::<W>::with_capacity(layers.len());
    for layer in layers {
        encoder.append::<Id, I>(layer)?;
    }
    encoder.finish()
}

/// Encodes graph property layers into descriptor/data payloads.
///
/// # Errors
///
/// Returns [`PropertyError`] if any layer metadata or Arrow payload is invalid.
///
/// # Performance
///
/// This function is `O(l + total values + total name bytes)`.
pub fn encode_graph_property_snapshot<W, Id, NodeIndex, EdgeIndex>(
    layers: GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>,
) -> Result<EncodedPropertySnapshot, PropertyError>
where
    W: PropertySnapshotMetaWord,
    Id: Copy + Into<u64> + TryInto<W>,
    NodeIndex: PropertyIndex,
    EdgeIndex: PropertyIndex,
{
    let mut encoder = PropertySnapshotEncoder::<W>::with_capacity(
        layers.element.len().saturating_add(layers.relation.len()),
    );
    for layer in layers.element {
        encoder.append::<Id, NodeIndex>(layer)?;
    }
    for layer in layers.relation {
        encoder.append::<Id, EdgeIndex>(layer)?;
    }
    encoder.finish()
}

/// Encodes hypergraph property layers into descriptor/data payloads.
///
/// # Errors
///
/// Returns [`PropertyError`] if any layer metadata or Arrow payload is invalid.
///
/// # Performance
///
/// This function is `O(l + total values + total name bytes)`.
pub fn encode_hyper_property_snapshot<W, Id, VertexIndex, RelationIndex, IncidenceIndex>(
    layers: HyperPropertyLayers<'_, Id, VertexIndex, RelationIndex, IncidenceIndex>,
) -> Result<EncodedPropertySnapshot, PropertyError>
where
    W: PropertySnapshotMetaWord,
    Id: Copy + Into<u64> + TryInto<W>,
    VertexIndex: PropertyIndex,
    RelationIndex: PropertyIndex,
    IncidenceIndex: PropertyIndex,
{
    let mut encoder = PropertySnapshotEncoder::<W>::with_capacity(
        layers
            .element
            .len()
            .saturating_add(layers.relation.len())
            .saturating_add(layers.incidence.len()),
    );
    for layer in layers.element {
        encoder.append::<Id, VertexIndex>(layer)?;
    }
    for layer in layers.relation {
        encoder.append::<Id, RelationIndex>(layer)?;
    }
    for layer in layers.incidence {
        encoder.append::<Id, IncidenceIndex>(layer)?;
    }
    encoder.finish()
}

/// Mutable accumulator for one in-progress property snapshot encoding pass.
///
/// Owns the descriptor record table, string table, and data payload between
/// calls to [`PropertySnapshotEncoder::append`], and finalizes them into an
/// [`EncodedPropertySnapshot`] via [`PropertySnapshotEncoder::finish`].
///
/// # Performance
///
/// Construction is `O(1)`. `append` is `O(layer values + layer name length)`.
/// `finish` is `O(record bytes + string bytes)`.
struct PropertySnapshotEncoder<W>
where
    W: PropertySnapshotMetaWord,
{
    /// Concatenated Arrow IPC value/default payload bytes referenced by records.
    data: Vec<u8>,
    /// Concatenated layer name bytes referenced by descriptor records.
    strings: Vec<u8>,
    /// In-order descriptor records emitted during the encoding pass.
    records: Vec<PropertySnapshotRecord<W>>,
    /// Layer names seen so far, scoped by ID family, used to reject duplicates.
    names: BTreeSet<(IdFamily, LayerName)>,
    /// Layer IDs (as `u64`) seen so far, used to reject duplicates.
    ids: BTreeSet<u64>,
}

impl<W> PropertySnapshotEncoder<W>
where
    W: PropertySnapshotMetaWord,
{
    /// Constructs an encoder with descriptor capacity hint `capacity`.
    fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::new(),
            strings: Vec::new(),
            records: Vec::with_capacity(capacity),
            names: BTreeSet::new(),
            ids: BTreeSet::new(),
        }
    }

    /// Encodes `layer` into a descriptor record and appends its Arrow payloads.
    fn append<Id, I>(&mut self, layer: &PropertyLayer<Id, I>) -> Result<(), PropertyError>
    where
        Id: Copy + Into<u64> + TryInto<W>,
        I: PropertyIndex,
    {
        let descriptor = layer.descriptor();
        if !self
            .names
            .insert((descriptor.id_family, descriptor.name.clone()))
        {
            return Err(PropertyError::DuplicateName {
                id_family: descriptor.id_family,
                name: descriptor.name.clone(),
            });
        }
        let diagnostic_layer_id = descriptor.layer_id.0.into();
        if !self.ids.insert(diagnostic_layer_id) {
            return Err(PropertyError::DuplicateLayerId {
                layer_id: diagnostic_layer_id,
            });
        }
        let name_offset = append_string(&mut self.strings, descriptor.name.as_str());
        let value_data_offset = self.data.len();
        let layer_data = encode_layer_value_ipc(layer)?;
        let value_data_len = layer_data.len();
        self.data.extend_from_slice(&layer_data);
        let (default_data_offset, default_data_len) =
            encode_layer_default_ipc(layer)?.map_or((0, 0), |default_data| {
                let offset = self.data.len();
                let len = default_data.len();
                self.data.extend_from_slice(&default_data);
                (offset, len)
            });
        let layer_id = descriptor.layer_id.0.try_into().map_err(|_error| {
            PropertyError::SnapshotDescriptorMismatch {
                reason: "layer ID does not fit selected metadata width",
            }
        })?;
        self.records.push(PropertySnapshotRecord::<W> {
            layer_id: layer_id.to_le_word(),
            name_offset: le_word::<W>(name_offset)?,
            name_len: le_word::<W>(descriptor.name.as_str().len())?,
            id_family: le_word::<W>(id_family_tag(descriptor.id_family) as usize)?,
            role: le_word::<W>(layer_role_tag(descriptor.role) as usize)?,
            storage: le_word::<W>(storage_tag(descriptor.storage) as usize)?,
            missing_policy: le_word::<W>(missing_policy_tag(descriptor.storage) as usize)?,
            logical_len: le_word::<W>(layer.len())?,
            value_count: le_word::<W>(layer_value_count(layer))?,
            value_data_offset: le_word::<W>(value_data_offset)?,
            value_data_len: le_word::<W>(value_data_len)?,
            default_data_offset: le_word::<W>(default_data_offset)?,
            default_data_len: le_word::<W>(default_data_len)?,
            reserved: le_word::<W>(0)?,
        });
        Ok(())
    }

    /// Finalizes descriptor/data bytes after all records have been appended.
    fn finish(self) -> Result<EncodedPropertySnapshot, PropertyError> {
        let record_bytes = self
            .records
            .len()
            .checked_mul(core::mem::size_of::<PropertySnapshotRecord<W>>())
            .ok_or(PropertyError::SnapshotDescriptorMismatch {
                reason: "record byte length overflow",
            })?;
        let header = PropertySnapshotHeader {
            record_count: U64::new(usize_to_u64(self.records.len())?),
            record_bytes: U64::new(usize_to_u64(record_bytes)?),
        };
        let mut descriptor_bytes = Vec::with_capacity(
            core::mem::size_of::<PropertySnapshotHeader>() + record_bytes + self.strings.len(),
        );
        descriptor_bytes.extend_from_slice(header.as_bytes());
        descriptor_bytes.extend_from_slice(self.records.as_bytes());
        descriptor_bytes.extend_from_slice(&self.strings);
        Ok(EncodedPropertySnapshot {
            descriptors: descriptor_bytes,
            data: self.data,
        })
    }
}

/// Validates property descriptor/data sections in a snapshot.
///
/// # Errors
///
/// Returns [`PropertyError`] if required sections are missing, have unsupported
/// versions, or contain inconsistent descriptor/data records.
///
/// # Performance
///
/// This function is `O(s + l log l + total name bytes)` for snapshot section
/// count `s` and property layer count `l`.
pub fn validate_property_snapshot<W>(
    snapshot: &Snapshot<'_>,
) -> Result<PropertySnapshotSummary, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let descriptor_section = snapshot.section(W::PROPERTY_DESCRIPTORS_KIND).ok_or(
        PropertyError::MissingSnapshotSection {
            kind: W::PROPERTY_DESCRIPTORS_KIND,
        },
    )?;
    let data_section =
        snapshot
            .section(W::PROPERTY_DATA_KIND)
            .ok_or(PropertyError::MissingSnapshotSection {
                kind: W::PROPERTY_DATA_KIND,
            })?;
    if descriptor_section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind: W::PROPERTY_DESCRIPTORS_KIND,
            version: descriptor_section.version(),
        });
    }
    if data_section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind: W::PROPERTY_DATA_KIND,
            version: data_section.version(),
        });
    }
    validate_property_sections::<W>(descriptor_section.bytes(), data_section.bytes())
}

/// Validates raw property descriptor and data section payloads.
///
/// # Errors
///
/// Returns [`PropertyError`] if the encoded payloads are structurally invalid.
///
/// # Performance
///
/// This function is `O(l log l + total name bytes + Arrow IPC validation)`.
pub fn validate_property_sections<W>(
    descriptor_bytes: &[u8],
    data_bytes: &[u8],
) -> Result<PropertySnapshotSummary, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let header_len = core::mem::size_of::<PropertySnapshotHeader>();
    if descriptor_bytes.len() < header_len {
        return Err(PropertyError::SnapshotDataLength {
            reason: "descriptor header is truncated",
        });
    }
    let record_count = read_u64_le(&descriptor_bytes[0..8])?;
    let record_bytes = read_u64_le(&descriptor_bytes[8..16])?;
    let record_count_usize = u64_to_usize(record_count)?;
    let record_bytes_usize = u64_to_usize(record_bytes)?;
    let expected_record_bytes = record_count_usize
        .checked_mul(core::mem::size_of::<PropertySnapshotRecord<W>>())
        .ok_or(PropertyError::SnapshotDescriptorMismatch {
            reason: "record byte length overflow",
        })?;
    if record_bytes_usize != expected_record_bytes {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "record byte length does not match record count",
        });
    }
    let record_start = header_len;
    let string_start = record_start.checked_add(record_bytes_usize).ok_or(
        PropertyError::SnapshotDescriptorMismatch {
            reason: "descriptor section length overflow",
        },
    )?;
    if descriptor_bytes.len() < string_start {
        return Err(PropertyError::SnapshotDataLength {
            reason: "descriptor records are truncated",
        });
    }
    let record_bytes_slice = &descriptor_bytes[record_start..string_start];
    let string_bytes = &descriptor_bytes[string_start..];
    let mut names: BTreeSet<(IdFamily, &str)> = BTreeSet::new();
    let mut ids: BTreeSet<u64> = BTreeSet::new();
    let mut ranges = Vec::with_capacity(record_count_usize);
    let mut total_logical_values = 0_usize;
    for position in 0..record_count_usize {
        let start = position * core::mem::size_of::<PropertySnapshotRecord<W>>();
        let record = parse_property_record::<W>(&record_bytes_slice[start..])?;
        let id_family = id_family_from_tag(le_word_to_u32::<W>(record.id_family)?)?;
        let _role = layer_role_from_tag(le_word_to_u32::<W>(record.role)?)?;
        let storage = storage_from_tags(
            le_word_to_u32::<W>(record.storage)?,
            le_word_to_u32::<W>(record.missing_policy)?,
        )?;
        let name = read_snapshot_str(
            string_bytes,
            le_word_to_usize::<W>(record.name_offset)?,
            le_word_to_usize::<W>(record.name_len)?,
        )?;
        let layer_id = le_word_to_u64::<W>(record.layer_id);
        if !ids.insert(layer_id) {
            return Err(PropertyError::DuplicateLayerId { layer_id });
        }
        if !names.insert((id_family, name)) {
            return Err(PropertyError::DuplicateName {
                id_family,
                name: LayerName::try_new(name)?,
            });
        }
        let layer_ranges = validate_property_record_data::<W>(&record, storage, data_bytes)?;
        ranges.extend(layer_ranges);
        total_logical_values = total_logical_values
            .checked_add(le_word_to_usize::<W>(record.logical_len)?)
            .ok_or(PropertyError::SnapshotDescriptorMismatch {
                reason: "logical value total overflow",
            })?;
    }
    validate_data_coverage(&mut ranges, data_bytes.len())?;
    Ok(PropertySnapshotSummary {
        layer_count: record_count_usize,
        total_logical_values,
    })
}

impl DecodedPropertyLayer {
    /// Decodes every property layer carried by a snapshot.
    ///
    /// Mirrors [`BcsrSnapshotHypergraph::from_snapshot`] on the topology side:
    /// a single constructor on the decoded type that takes the wire snapshot
    /// and returns the materialized form. Each layer is returned in descriptor
    /// order with its Arrow payload restored via
    /// [`arrow_ipc::reader::StreamReader`].
    ///
    /// Calls [`validate_property_snapshot`] before decoding so the diagnostics
    /// match the validator exactly.
    ///
    /// [`BcsrSnapshotHypergraph::from_snapshot`]: https://docs.rs/oxgraph-hyper-bcsr/latest/oxgraph_hyper_bcsr/struct.BcsrSnapshotHypergraph.html#method.from_snapshot
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] if required sections are missing, have an
    /// unsupported version, or contain inconsistent descriptor/data records.
    ///
    /// # Performance
    ///
    /// `O(s + l + total Arrow IPC payload bytes)` for snapshot section count
    /// `s` and property layer count `l`.
    pub fn decode_all<W>(snapshot: &Snapshot<'_>) -> Result<Vec<Self>, PropertyError>
    where
        W: PropertySnapshotMetaWord,
    {
        let descriptor_section = snapshot.section(W::PROPERTY_DESCRIPTORS_KIND).ok_or(
            PropertyError::MissingSnapshotSection {
                kind: W::PROPERTY_DESCRIPTORS_KIND,
            },
        )?;
        let data_section = snapshot.section(W::PROPERTY_DATA_KIND).ok_or(
            PropertyError::MissingSnapshotSection {
                kind: W::PROPERTY_DATA_KIND,
            },
        )?;
        if descriptor_section.version() != SNAPSHOT_PROPERTY_VERSION {
            return Err(PropertyError::SnapshotSectionVersion {
                kind: W::PROPERTY_DESCRIPTORS_KIND,
                version: descriptor_section.version(),
            });
        }
        if data_section.version() != SNAPSHOT_PROPERTY_VERSION {
            return Err(PropertyError::SnapshotSectionVersion {
                kind: W::PROPERTY_DATA_KIND,
                version: data_section.version(),
            });
        }
        Self::decode_sections::<W>(descriptor_section.bytes(), data_section.bytes())
    }

    /// Decodes property layers from raw descriptor and data section payloads.
    ///
    /// Lower-level entry point for callers that already have the two section
    /// byte slices in hand (e.g. when reassembling property data from a custom
    /// container). Re-runs [`validate_property_sections`] so structural errors
    /// surface with identical diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] if the encoded payloads are structurally
    /// invalid.
    ///
    /// # Performance
    ///
    /// `O(l + total Arrow IPC payload bytes + total name bytes)` for layer
    /// count `l`.
    pub fn decode_sections<W>(
        descriptor_bytes: &[u8],
        data_bytes: &[u8],
    ) -> Result<Vec<Self>, PropertyError>
    where
        W: PropertySnapshotMetaWord,
    {
        let _summary = validate_property_sections::<W>(descriptor_bytes, data_bytes)?;
        let header_len = core::mem::size_of::<PropertySnapshotHeader>();
        let record_count_usize = u64_to_usize(read_u64_le(&descriptor_bytes[0..8])?)?;
        let record_bytes_usize = u64_to_usize(read_u64_le(&descriptor_bytes[8..16])?)?;
        let record_start = header_len;
        let string_start = record_start.checked_add(record_bytes_usize).ok_or(
            PropertyError::SnapshotDescriptorMismatch {
                reason: "descriptor section length overflow",
            },
        )?;
        let record_bytes_slice = &descriptor_bytes[record_start..string_start];
        let string_bytes = &descriptor_bytes[string_start..];
        let record_size = core::mem::size_of::<PropertySnapshotRecord<W>>();
        let mut out = Vec::with_capacity(record_count_usize);
        for position in 0..record_count_usize {
            let start = position.checked_mul(record_size).ok_or(
                PropertyError::SnapshotDescriptorMismatch {
                    reason: "record offset overflow",
                },
            )?;
            let record = parse_property_record::<W>(&record_bytes_slice[start..])?;
            let layer_id = le_word_to_u64::<W>(record.layer_id);
            let id_family = id_family_from_tag(le_word_to_u32::<W>(record.id_family)?)?;
            let role = layer_role_from_tag(le_word_to_u32::<W>(record.role)?)?;
            let storage = storage_from_tags(
                le_word_to_u32::<W>(record.storage)?,
                le_word_to_u32::<W>(record.missing_policy)?,
            )?;
            let name = read_snapshot_str(
                string_bytes,
                le_word_to_usize::<W>(record.name_offset)?,
                le_word_to_usize::<W>(record.name_len)?,
            )?
            .to_string();
            let logical_len = le_word_to_usize::<W>(record.logical_len)?;
            let value_offset = le_word_to_usize::<W>(record.value_data_offset)?;
            let value_len = le_word_to_usize::<W>(record.value_data_len)?;
            let value_end = checked_end(value_offset, value_len, data_bytes.len())?;
            let value_batch = read_one_ipc_batch(&data_bytes[value_offset..value_end])?;
            let default_offset = le_word_to_usize::<W>(record.default_data_offset)?;
            let default_len = le_word_to_usize::<W>(record.default_data_len)?;
            let default_batch = if default_len == 0 {
                None
            } else {
                let default_end = checked_end(default_offset, default_len, data_bytes.len())?;
                Some(read_one_ipc_batch(
                    &data_bytes[default_offset..default_end],
                )?)
            };
            let data = match storage {
                StorageMode::Dense => DecodedPropertyData::Dense {
                    values: Arc::clone(value_batch.column(0)),
                },
                StorageMode::Sparse { .. } => DecodedPropertyData::Sparse {
                    indices: Arc::clone(value_batch.column(0)),
                    values: Arc::clone(value_batch.column(1)),
                    default: default_batch
                        .as_ref()
                        .map(|batch| Arc::clone(batch.column(0))),
                },
            };
            out.push(Self {
                layer_id,
                name,
                id_family,
                role,
                storage,
                logical_len,
                data,
            });
        }
        Ok(out)
    }
}

/// Validates identity records and required map sections.
///
/// # Performance
///
/// This function is `O(f)` for `f` records.
fn validate_identity_records<W>(
    snapshot: &Snapshot<'_>,
    records: &[IdentityModeRecord<W>],
) -> Result<Vec<IdentityModeSummary>, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let mut seen = BTreeSet::new();
    let mut summaries = Vec::with_capacity(records.len());
    for record in records {
        let family = record.id_family()?;
        if !seen.insert(family) {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "duplicate identity family mode record",
            });
        }
        let mode = record.mode()?;
        let local_len = record.local_len();
        match mode {
            IdentityMapMode::LocalEqualsCanonical => {}
            IdentityMapMode::ExplicitMap => {
                validate_identity_map_section::<W>(snapshot, family, local_len)?;
            }
        }
        summaries.push(IdentityModeSummary {
            id_family: family,
            mode,
            local_len,
        });
    }
    Ok(summaries)
}

/// Validates one explicit identity-map section.
///
/// # Performance
///
/// This function is `O(s)` for snapshot section count `s`.
fn validate_identity_map_section<W>(
    snapshot: &Snapshot<'_>,
    id_family: IdFamily,
    required: usize,
) -> Result<(), PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let kind = identity_map_kind::<W>(id_family);
    let section = snapshot
        .section(kind)
        .ok_or(PropertyError::MissingIdentityMap { id_family })?;
    if section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind,
            version: section.version(),
        });
    }
    let map: &[W::LittleEndianWord] = section
        .try_as_slice()
        .map_err(|error| PropertyError::SnapshotSectionView { kind, error })?;
    if map.len() != required {
        return Err(PropertyError::IdentityMapLength {
            id_family,
            required,
            actual: map.len(),
        });
    }
    Ok(())
}

/// Returns the explicit identity-map section kind for a family.
///
/// # Performance
///
/// This function is `O(1)`.
const fn identity_map_kind<W>(id_family: IdFamily) -> u32
where
    W: PropertySnapshotMetaWord,
{
    match id_family {
        IdFamily::Element => W::ELEMENT_IDENTITY_MAP_KIND,
        IdFamily::Relation => W::RELATION_IDENTITY_MAP_KIND,
        IdFamily::Incidence => W::INCIDENCE_IDENTITY_MAP_KIND,
    }
}

/// Appends a string to a snapshot string table.
///
/// # Performance
///
/// This function is `O(value.len())`.
fn append_string(strings: &mut Vec<u8>, value: &str) -> usize {
    let offset = strings.len();
    strings.extend_from_slice(value.as_bytes());
    offset
}

/// Returns the number of value slots encoded for a layer.
///
/// # Performance
///
/// This function is `O(1)`.
fn layer_value_count<Id, I>(layer: &PropertyLayer<Id, I>) -> usize
where
    I: PropertyIndex,
{
    match layer.data() {
        PropertyLayerData::Dense { values } => values.len(),
        PropertyLayerData::Sparse { indices, .. } => indices.len(),
    }
}

/// Encodes one property layer's value stream as Arrow IPC.
///
/// # Performance
///
/// This function is `O(layer payload bytes)`.
fn encode_layer_value_ipc<Id, I>(layer: &PropertyLayer<Id, I>) -> Result<Vec<u8>, PropertyError>
where
    I: PropertyIndex,
{
    let (schema, columns) = match layer.data() {
        PropertyLayerData::Dense { values } => {
            let schema = Arc::new(Schema::new(vec![layer.descriptor().arrow_field.clone()]));
            (schema, vec![Arc::clone(values)])
        }
        PropertyLayerData::Sparse {
            indices,
            values,
            default: _,
        } => {
            let fields = vec![
                Field::new("index", index_data_type::<I>(), false),
                layer.descriptor().arrow_field.clone(),
            ];
            let columns: Vec<ArrayRef> = vec![Arc::clone(indices) as ArrayRef, Arc::clone(values)];
            (Arc::new(Schema::new(fields)), columns)
        }
    };
    write_one_ipc_batch(&schema, columns)
}

/// Encodes one sparse-default layer's default stream as Arrow IPC.
///
/// # Performance
///
/// This function is `O(default payload bytes)`.
fn encode_layer_default_ipc<Id, I>(
    layer: &PropertyLayer<Id, I>,
) -> Result<Option<Vec<u8>>, PropertyError>
where
    I: PropertyIndex,
{
    let PropertyLayerData::Sparse {
        default: Some(default),
        ..
    } = layer.data()
    else {
        return Ok(None);
    };
    let schema = Arc::new(Schema::new(vec![layer.descriptor().arrow_field.clone()]));
    write_one_ipc_batch(&schema, vec![Arc::clone(default)]).map(Some)
}

/// Writes one Arrow IPC stream with a single record batch.
///
/// # Performance
///
/// This function is `O(payload bytes)`.
fn write_one_ipc_batch(
    schema: &Arc<Schema>,
    columns: Vec<ArrayRef>,
) -> Result<Vec<u8>, PropertyError> {
    let batch = RecordBatch::try_new(Arc::clone(schema), columns).map_err(map_arrow_error)?;
    let mut out = Vec::new();
    {
        let mut writer =
            StreamWriter::try_new(&mut out, schema.as_ref()).map_err(map_arrow_error)?;
        writer.write(&batch).map_err(map_arrow_error)?;
        writer.finish().map_err(map_arrow_error)?;
    }
    Ok(out)
}

/// Parses one property snapshot record from the front of `bytes`.
///
/// # Performance
///
/// This function is `O(1)`.
fn parse_property_record<W>(bytes: &[u8]) -> Result<PropertySnapshotRecord<W>, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let need = core::mem::size_of::<PropertySnapshotRecord<W>>();
    if bytes.len() < need {
        return Err(PropertyError::SnapshotDataLength {
            reason: "property record is truncated",
        });
    }
    PropertySnapshotRecord::<W>::read_from_bytes(&bytes[..need]).map_err(|_error| {
        PropertyError::SnapshotDataLength {
            reason: "property record is truncated",
        }
    })
}

/// Validates a property data range declared by one record.
///
/// # Performance
///
/// This function is `O(Arrow IPC payload validation)`.
fn validate_property_record_data<W>(
    record: &PropertySnapshotRecord<W>,
    storage: StorageMode,
    data: &[u8],
) -> Result<Vec<core::ops::Range<usize>>, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    if le_word_to_u64::<W>(record.reserved) != 0 {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "property descriptor reserved word must be zero",
        });
    }
    let offset = le_word_to_usize::<W>(record.value_data_offset)?;
    let len = le_word_to_usize::<W>(record.value_data_len)?;
    let end = checked_end(offset, len, data.len())?;
    let value_batch = read_one_ipc_batch(&data[offset..end])?;
    let default_offset = le_word_to_usize::<W>(record.default_data_offset)?;
    let default_len = le_word_to_usize::<W>(record.default_data_len)?;
    let default_batch = if default_len == 0 {
        None
    } else {
        let default_end = checked_end(default_offset, default_len, data.len())?;
        Some(read_one_ipc_batch(&data[default_offset..default_end])?)
    };
    match storage {
        StorageMode::Dense => {
            if default_len != 0 {
                return Err(PropertyError::SnapshotDescriptorMismatch {
                    reason: "dense property must not declare a default stream",
                });
            }
            validate_dense_batch::<W>(record, &value_batch)?;
        }
        StorageMode::Sparse { missing } => {
            validate_sparse_batch::<W>(record, missing, &value_batch, default_batch.as_ref())?;
        }
    }
    let mut ranges = Vec::with_capacity(2);
    ranges.push(offset..end);
    if default_len != 0 {
        ranges.push(default_offset..default_offset + default_len);
    }
    Ok(ranges)
}

/// Reads exactly one Arrow IPC record batch.
///
/// # Performance
///
/// This function is `O(bytes.len())`.
fn read_one_ipc_batch(bytes: &[u8]) -> Result<RecordBatch, PropertyError> {
    let reader = StreamReader::try_new(Cursor::new(bytes), None).map_err(map_arrow_error)?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(map_arrow_error)?);
        if batches.len() > 1 {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "property IPC stream contains more than one batch",
            });
        }
    }
    let mut iter = batches.into_iter();
    iter.next()
        .ok_or(PropertyError::SnapshotDescriptorMismatch {
            reason: "property IPC stream contains no batches",
        })
}

/// Validates one dense Arrow IPC batch.
///
/// # Performance
///
/// This function is `O(1)`.
fn validate_dense_batch<W>(
    record: &PropertySnapshotRecord<W>,
    batch: &RecordBatch,
) -> Result<(), PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    if batch.num_columns() != 1 {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "dense property batch must contain one column",
        });
    }
    let values = batch.column(0);
    if values.len() != le_word_to_usize::<W>(record.logical_len)?
        || values.len() != le_word_to_usize::<W>(record.value_count)?
    {
        return Err(PropertyError::SnapshotDataLength {
            reason: "dense property Arrow length does not match descriptor",
        });
    }
    validate_value_column(values.as_ref())
}

/// Validates one sparse Arrow IPC batch.
///
/// # Performance
///
/// This function is `O(value_count)` for sparse index validation.
fn validate_sparse_batch<W>(
    record: &PropertySnapshotRecord<W>,
    missing: MissingPolicy,
    value_batch: &RecordBatch,
    default_batch: Option<&RecordBatch>,
) -> Result<(), PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    if value_batch.num_columns() != 2 {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "sparse property value stream must contain index and value columns",
        });
    }
    let indexes = value_batch.column(0);
    let values = value_batch.column(1);
    let value_count = le_word_to_usize::<W>(record.value_count)?;
    if indexes.len() != value_count || values.len() != value_count {
        return Err(PropertyError::SnapshotDataLength {
            reason: "sparse property Arrow value count does not match descriptor",
        });
    }
    validate_value_column(values.as_ref())?;
    validate_sparse_index_column(indexes.as_ref(), le_word_to_usize::<W>(record.logical_len)?)?;
    match (missing, default_batch) {
        (MissingPolicy::Null, None) => {}
        (MissingPolicy::Null, Some(_)) => {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "sparse-null property must not declare a default stream",
            });
        }
        (MissingPolicy::Default, Some(default_batch)) => {
            if default_batch.num_columns() != 1 {
                return Err(PropertyError::SnapshotDescriptorMismatch {
                    reason: "sparse default stream must contain one column",
                });
            }
            let default = default_batch.column(0);
            if default.len() != 1 || default.data_type() != values.data_type() || default.is_null(0)
            {
                return Err(PropertyError::SnapshotDescriptorMismatch {
                    reason: "sparse property default column is not a non-null matching scalar",
                });
            }
        }
        (MissingPolicy::Default, None) => {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "sparse-default property is missing its default stream",
            });
        }
    }
    Ok(())
}

/// Validates an Arrow value column against snapshot metadata.
///
/// # Performance
///
/// This function is `O(1)`.
fn validate_value_column(values: &dyn Array) -> Result<(), PropertyError> {
    if values.null_count() > values.len() {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "Arrow value column has invalid null accounting",
        });
    }
    Ok(())
}

/// Validates descriptor ranges cover data exactly without overlap or trailing bytes.
///
/// # Performance
///
/// This function is `O(n log n)` for `n` ranges.
fn validate_data_coverage(
    ranges: &mut [core::ops::Range<usize>],
    data_len: usize,
) -> Result<(), PropertyError> {
    ranges.sort_by_key(|range| range.start);
    let mut cursor = 0_usize;
    for range in ranges {
        if range.start != cursor {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "property data ranges leave a gap or overlap",
            });
        }
        cursor = range.end;
    }
    if cursor != data_len {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "property data section has trailing bytes",
        });
    }
    Ok(())
}

/// Reads a UTF-8 string from a snapshot string table.
///
/// # Performance
///
/// This function is `O(len)` for UTF-8 validation.
fn read_snapshot_str(bytes: &[u8], offset: usize, len: usize) -> Result<&str, PropertyError> {
    let end = checked_end(offset, len, bytes.len())?;
    core::str::from_utf8(&bytes[offset..end])
        .map_err(|_error| PropertyError::SnapshotInvalidUtf8 { offset })
}

/// Checks a byte range against an available length.
///
/// # Performance
///
/// This function is `O(1)`.
fn checked_end(offset: usize, len: usize, available: usize) -> Result<usize, PropertyError> {
    let end = offset
        .checked_add(len)
        .ok_or(PropertyError::SnapshotRangeOutOfBounds {
            offset,
            len,
            available,
        })?;
    if end > available {
        Err(PropertyError::SnapshotRangeOutOfBounds {
            offset,
            len,
            available,
        })
    } else {
        Ok(end)
    }
}

/// Reads a little-endian `u64` from an eight-byte slice.
///
/// # Performance
///
/// This function is `O(1)`.
fn read_u64_le(bytes: &[u8]) -> Result<u64, PropertyError> {
    if bytes.len() < core::mem::size_of::<u64>() {
        return Err(PropertyError::SnapshotDataLength {
            reason: "u64 field is truncated",
        });
    }
    let mut array = [0_u8; 8];
    array.copy_from_slice(&bytes[..8]);
    Ok(u64::from_le_bytes(array))
}

/// Converts `u64` to `usize` for snapshot lengths.
///
/// # Performance
///
/// This function is `O(1)`.
fn u64_to_usize(value: u64) -> Result<usize, PropertyError> {
    usize::try_from(value).map_err(|_error| PropertyError::SnapshotDescriptorMismatch {
        reason: "snapshot length does not fit usize",
    })
}

/// Converts `usize` to `u64` for snapshot lengths.
///
/// # Performance
///
/// This function is `O(1)`.
fn usize_to_u64(value: usize) -> Result<u64, PropertyError> {
    u64::try_from(value).map_err(|_error| PropertyError::LengthDoesNotFitU64 { value })
}

/// Validates a decoded sparse index column against the layer's logical length.
///
/// The wire descriptor records the metadata word width `W`, not the sparse
/// index width `I`; `I` is only recoverable from the decoded Arrow column's
/// [`DataType`]. This selects the typed [`PropertyIndex`] width from that
/// runtime data type, downcasts once, and forwards to the typed
/// [`validate_sparse_indices`] so the ordering/bounds contract has a single
/// definition shared with [`PropertyLayer::try_new_sparse`].
///
/// # Performance
///
/// This function is `O(indices.len())`.
fn validate_sparse_index_column(indices: &dyn Array, len: usize) -> Result<(), PropertyError> {
    /// Downcasts `indices` to the `I`-typed primitive column and validates it.
    fn typed<I>(indices: &dyn Array, len: usize) -> Result<(), PropertyError>
    where
        I: PropertyIndex,
    {
        let typed = indices
            .as_any()
            .downcast_ref::<PrimitiveArray<I::ArrowType>>()
            .ok_or(PropertyError::SnapshotDescriptorMismatch {
                reason: "sparse property index column does not match its declared Arrow width",
            })?;
        validate_sparse_indices::<I>(typed, len)
    }

    match indices.data_type() {
        DataType::UInt16 => typed::<u16>(indices, len),
        DataType::UInt32 => typed::<u32>(indices, len),
        DataType::UInt64 => typed::<u64>(indices, len),
        _ => Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "sparse property index column is not UInt16, UInt32, or UInt64",
        }),
    }
}

/// Returns the Arrow data type for a property index width.
///
/// # Performance
///
/// This function is `O(1)`.
const fn index_data_type<I>() -> DataType
where
    I: PropertyIndex,
{
    if core::mem::size_of::<I>() == core::mem::size_of::<u16>() {
        DataType::UInt16
    } else if core::mem::size_of::<I>() == core::mem::size_of::<u32>() {
        DataType::UInt32
    } else {
        DataType::UInt64
    }
}

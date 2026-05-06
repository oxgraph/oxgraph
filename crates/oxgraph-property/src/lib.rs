//! Arrow-backed named property layers for `OxGraph` topology views.
//!
//! `oxgraph-property` is a higher layer than topology. It stores named typed
//! Arrow arrays keyed by topology ID family and adapts selected total primitive
//! layers into topology weight capabilities. Foundation crates do not depend on
//! this crate, Arrow, or named properties.
// kani-skip: property layers depend on Arrow heap arrays and snapshot byte streams outside Kani's
// bounded no-std proof scope.

use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    io::Cursor,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use arrow_array::{
    Array, ArrayRef, PrimitiveArray, RecordBatch, UInt64Array, types::ArrowPrimitiveType,
};
use arrow_ipc::{reader::StreamReader, writer::StreamWriter};
use arrow_schema::{DataType, Field, Schema};
use oxgraph_snapshot::{SectionViewError, Snapshot};
use oxgraph_topology::{
    ElementIndex, ElementWeight, IncidenceBase, IncidenceIndex, IncidenceWeight, RelationIndex,
    RelationWeight, TopologyBase,
};
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32, U64},
};

/// Snapshot section kind reserved for property-layer descriptors.
///
/// The payload format is owned by this crate and remains an OxGraph-internal
/// ABI candidate while snapshot v1 bytes are not stable.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_PROPERTY_DESCRIPTORS: u32 = 0x0100;

/// Snapshot section kind reserved for Arrow IPC property-layer payloads.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_PROPERTY_DATA: u32 = 0x0101;

/// Snapshot section kind for identity-mode metadata records.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_IDENTITY_MODES: u32 = 0x0102;

/// Snapshot section kind for element local-to-canonical `u32` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U32: u32 = 0x0103;

/// Snapshot section kind for relation local-to-canonical `u32` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32: u32 = 0x0104;

/// Snapshot section kind for incidence local-to-canonical `u32` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U32: u32 = 0x0105;

/// Internal property/identity snapshot section version.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_PROPERTY_VERSION: u32 = 1;

/// Stable numeric identifier for one property layer.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayerId(pub u64);

/// Human-facing property layer name.
///
/// # Performance
///
/// Cloning is `O(name.len())`; comparison and display are `O(name.len())`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayerName {
    /// Owned layer name.
    value: String,
}

impl LayerName {
    /// Builds a non-empty layer name.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError::EmptyLayerName`] when `value` is empty.
    ///
    /// # Performance
    ///
    /// This function is `O(value.len())`.
    pub fn try_new(value: &str) -> Result<Self, PropertyError> {
        if value.is_empty() {
            return Err(PropertyError::EmptyLayerName);
        }
        Ok(Self {
            value: String::from(value),
        })
    }

    /// Returns the layer name as a borrowed string.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

impl fmt::Display for LayerName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Topology ID family keyed by a property layer.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IdFamily {
    /// Element/node/vertex-keyed layer.
    Element,
    /// Relation/edge/hyperedge-keyed layer.
    Relation,
    /// Incidence/endpoint/participant-keyed layer.
    Incidence,
}

/// Declared role of a property layer.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum LayerRole {
    /// Layer is intended to be selected as a topology weight capability.
    Weight,
    /// Layer is a named property with no required weight interpretation.
    Property,
}

/// Coarse Arrow value family recorded by descriptors and snapshot metadata.
///
/// Full Arrow type fidelity lives in the Arrow schema encoded in the property
/// data section. This enum is review metadata and quick validation context.
///
/// # Performance
///
/// Copying, comparing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ArrowValueFamily {
    /// Arrow null type.
    Null,
    /// Boolean values.
    Boolean,
    /// Signed integer values.
    SignedInteger,
    /// Unsigned integer values.
    UnsignedInteger,
    /// Floating-point values.
    Floating,
    /// Decimal numeric values.
    Decimal,
    /// Date, time, duration, or interval values.
    Temporal,
    /// UTF-8 string values.
    Utf8,
    /// Binary values.
    Binary,
    /// Dictionary/categorical values.
    Dictionary,
    /// List-like values.
    List,
    /// Struct/grouped values.
    Struct,
    /// Map values.
    Map,
    /// Union values.
    Union,
    /// Run-end encoded values.
    RunEndEncoded,
}

impl ArrowValueFamily {
    /// Classifies an Arrow data type into an `OxGraph` property value family.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` except for following dictionary child metadata.
    #[must_use]
    pub const fn from_data_type(data_type: &DataType) -> Self {
        match data_type {
            DataType::Null => Self::Null,
            DataType::Boolean => Self::Boolean,
            DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
                Self::SignedInteger
            }
            DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
                Self::UnsignedInteger
            }
            DataType::Float16 | DataType::Float32 | DataType::Float64 => Self::Floating,
            DataType::Decimal32(_, _)
            | DataType::Decimal64(_, _)
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _) => Self::Decimal,
            DataType::Timestamp(_, _)
            | DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Duration(_)
            | DataType::Interval(_) => Self::Temporal,
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Self::Utf8,
            DataType::Binary
            | DataType::FixedSizeBinary(_)
            | DataType::LargeBinary
            | DataType::BinaryView => Self::Binary,
            DataType::Dictionary(_, _) => Self::Dictionary,
            DataType::List(_)
            | DataType::ListView(_)
            | DataType::FixedSizeList(_, _)
            | DataType::LargeList(_)
            | DataType::LargeListView(_) => Self::List,
            DataType::Struct(_) => Self::Struct,
            DataType::Map(_, _) => Self::Map,
            DataType::Union(_, _) => Self::Union,
            DataType::RunEndEncoded(_, _) => Self::RunEndEncoded,
        }
    }

    /// Returns the snapshot tag for this value family.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn tag(self) -> u32 {
        match self {
            Self::Null => 0,
            Self::Boolean => 1,
            Self::SignedInteger => 2,
            Self::UnsignedInteger => 3,
            Self::Floating => 4,
            Self::Decimal => 5,
            Self::Temporal => 6,
            Self::Utf8 => 7,
            Self::Binary => 8,
            Self::Dictionary => 9,
            Self::List => 10,
            Self::Struct => 11,
            Self::Map => 12,
            Self::Union => 13,
            Self::RunEndEncoded => 14,
        }
    }

    /// Decodes a snapshot value-family tag.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn from_tag(tag: u32) -> Option<Self> {
        match tag {
            0 => Some(Self::Null),
            1 => Some(Self::Boolean),
            2 => Some(Self::SignedInteger),
            3 => Some(Self::UnsignedInteger),
            4 => Some(Self::Floating),
            5 => Some(Self::Decimal),
            6 => Some(Self::Temporal),
            7 => Some(Self::Utf8),
            8 => Some(Self::Binary),
            9 => Some(Self::Dictionary),
            10 => Some(Self::List),
            11 => Some(Self::Struct),
            12 => Some(Self::Map),
            13 => Some(Self::Union),
            14 => Some(Self::RunEndEncoded),
            _ => None,
        }
    }
}

/// Missing-value policy for sparse property layers.
///
/// The actual default scalar, when present, is stored in Arrow data for the
/// sparse layer. This enum records whether a total default exists.
///
/// # Performance
///
/// Copying, comparing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum MissingPolicy {
    /// Missing positions are null and therefore not directly weight-total.
    Null,
    /// Missing positions read from an Arrow scalar default stored with the layer.
    Default,
}

/// Physical storage mode for a property layer.
///
/// # Performance
///
/// Copying, comparing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum StorageMode {
    /// Dense array with one slot per ID index.
    Dense,
    /// Sparse array keyed by explicit indexes plus a missing-value policy.
    Sparse {
        /// Policy used for indexes not present in the sparse index array.
        missing: MissingPolicy,
    },
}

/// Descriptor for one Arrow-backed property layer.
///
/// # Performance
///
/// Cloning is `O(name.len() + arrow field clone cost)`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PropertyLayerDescriptor {
    /// Stable layer identifier.
    pub layer_id: LayerId,
    /// Human-facing layer name.
    pub name: LayerName,
    /// Topology ID family keyed by this layer.
    pub id_family: IdFamily,
    /// Declared layer role.
    pub role: LayerRole,
    /// Physical storage mode.
    pub storage: StorageMode,
    /// Arrow schema field for stored values.
    pub arrow_field: Field,
}

impl PropertyLayerDescriptor {
    /// Constructs a descriptor and validates the layer name.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError::EmptyLayerName`] when `name` is empty.
    ///
    /// # Performance
    ///
    /// This function is `O(name.len())` plus Arrow field move cost.
    #[expect(
        clippy::too_many_arguments,
        reason = "descriptor constructor mirrors the six-field descriptor contract"
    )]
    pub fn try_new(
        layer_id: LayerId,
        name: &str,
        id_family: IdFamily,
        role: LayerRole,
        storage: StorageMode,
        arrow_field: Field,
    ) -> Result<Self, PropertyError> {
        Ok(Self {
            layer_id,
            name: LayerName::try_new(name)?,
            id_family,
            role,
            storage,
            arrow_field,
        })
    }

    /// Returns the coarse Arrow value family described by this layer.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` except for following dictionary child metadata.
    #[must_use]
    pub const fn value_family(&self) -> ArrowValueFamily {
        ArrowValueFamily::from_data_type(self.arrow_field.data_type())
    }
}

/// Errors raised while validating property descriptors, layers, or snapshots.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PropertyError {
    /// Layer names must not be empty.
    EmptyLayerName,
    /// Dense layers must use dense descriptors.
    ExpectedDenseStorage {
        /// Name of the offending layer.
        name: LayerName,
    },
    /// Sparse layers must use sparse descriptors.
    ExpectedSparseStorage {
        /// Name of the offending layer.
        name: LayerName,
    },
    /// A sparse descriptor and default value disagreed.
    DefaultPolicyMismatch {
        /// Name of the offending layer.
        name: LayerName,
    },
    /// A layer's Arrow data type did not match the descriptor field type.
    ArrowTypeMismatch {
        /// Name of the offending layer.
        name: LayerName,
    },
    /// A layer's ID family did not match the requested adapter family.
    IdFamilyMismatch {
        /// Expected ID family.
        expected: IdFamily,
        /// Actual ID family.
        actual: IdFamily,
    },
    /// A layer had too few values for the topology index bound.
    LayerTooShort {
        /// Required minimum length.
        required: usize,
        /// Actual layer length.
        actual: usize,
    },
    /// A non-nullable selected layer contained a null slot.
    UnexpectedNull {
        /// Index of the null slot.
        index: usize,
    },
    /// Sparse index and value arrays differed in length.
    SparseLengthMismatch {
        /// Sparse index count.
        indices: usize,
        /// Sparse value count.
        values: usize,
    },
    /// Sparse indexes must be strictly increasing.
    SparseIndexOrder {
        /// Sparse array position where order failed.
        position: usize,
    },
    /// Sparse index was outside the declared logical length.
    SparseIndexOutOfBounds {
        /// Invalid sparse index.
        index: u64,
        /// Logical layer length.
        len: usize,
    },
    /// A name was reused within an ID-family namespace.
    DuplicateName {
        /// ID family namespace.
        id_family: IdFamily,
        /// Duplicate layer name.
        name: LayerName,
    },
    /// Sparse null-missing policy cannot be selected as a total weight view.
    SparseNullMissingNotTotal {
        /// Name of the offending layer.
        name: LayerName,
    },
    /// A layer ID was reused within one descriptor set.
    DuplicateLayerId {
        /// Duplicate layer ID.
        layer_id: LayerId,
    },
    /// A snapshot section was missing.
    MissingSnapshotSection {
        /// Missing section kind.
        kind: u32,
    },
    /// A snapshot section had an unsupported version.
    SnapshotSectionVersion {
        /// Section kind.
        kind: u32,
        /// Actual section version.
        version: u32,
    },
    /// A snapshot section could not be borrowed as the expected record type.
    SnapshotSectionView {
        /// Section kind.
        kind: u32,
        /// Underlying typed-view error.
        error: SectionViewError,
    },
    /// Snapshot bytes ended before a declared range.
    SnapshotRangeOutOfBounds {
        /// Byte range start.
        offset: usize,
        /// Byte range length.
        len: usize,
        /// Available section byte length.
        available: usize,
    },
    /// Snapshot string table bytes were not valid UTF-8.
    SnapshotInvalidUtf8 {
        /// Byte offset of the invalid string.
        offset: usize,
    },
    /// Snapshot metadata used an unknown ID family tag.
    UnknownIdFamilyTag {
        /// Invalid tag.
        tag: u32,
    },
    /// Snapshot metadata used an unknown layer role tag.
    UnknownLayerRoleTag {
        /// Invalid tag.
        tag: u32,
    },
    /// Snapshot metadata used an unknown storage tag.
    UnknownStorageTag {
        /// Invalid tag.
        tag: u32,
    },
    /// Snapshot metadata used an unknown missing-policy tag.
    UnknownMissingPolicyTag {
        /// Invalid tag.
        tag: u32,
    },
    /// Snapshot metadata used an unknown Arrow value-family tag.
    UnknownArrowFamilyTag {
        /// Invalid tag.
        tag: u32,
    },
    /// Snapshot metadata used an unknown identity-map mode tag.
    UnknownIdentityModeTag {
        /// Invalid tag.
        tag: u32,
    },
    /// A property snapshot descriptor was structurally inconsistent.
    SnapshotDescriptorMismatch {
        /// Human-readable mismatch reason.
        reason: &'static str,
    },
    /// A property data payload had an invalid byte length.
    SnapshotDataLength {
        /// Human-readable mismatch reason.
        reason: &'static str,
    },
    /// Arrow IPC/schema validation failed.
    Arrow {
        /// Arrow error message.
        message: String,
    },
    /// An explicit identity map was required but missing.
    MissingIdentityMap {
        /// ID family whose map was missing.
        id_family: IdFamily,
    },
    /// An identity map length did not match its mode metadata.
    IdentityMapLength {
        /// ID family whose map had the wrong length.
        id_family: IdFamily,
        /// Required map length.
        required: usize,
        /// Actual map length.
        actual: usize,
    },
    /// A `usize` value could not be represented as `u64`.
    LengthDoesNotFitU64 {
        /// Value that did not fit.
        value: usize,
    },
}

impl fmt::Display for PropertyError {
    #[expect(
        clippy::too_many_lines,
        reason = "property validation has one display branch per concrete error variant"
    )]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLayerName => formatter.write_str("property layer name is empty"),
            Self::ExpectedDenseStorage { name } => {
                write!(formatter, "property layer '{name}' is not dense")
            }
            Self::ExpectedSparseStorage { name } => {
                write!(formatter, "property layer '{name}' is not sparse")
            }
            Self::DefaultPolicyMismatch { name } => {
                write!(formatter, "property layer '{name}' default policy mismatch")
            }
            Self::ArrowTypeMismatch { name } => {
                write!(formatter, "property layer '{name}' Arrow type mismatch")
            }
            Self::IdFamilyMismatch { expected, actual } => write!(
                formatter,
                "property ID family mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::LayerTooShort { required, actual } => write!(
                formatter,
                "property layer too short: required {required}, got {actual}"
            ),
            Self::UnexpectedNull { index } => write!(
                formatter,
                "property layer has unexpected null at index {index}"
            ),
            Self::SparseLengthMismatch { indices, values } => write!(
                formatter,
                "sparse property length mismatch: {indices} indexes for {values} values"
            ),
            Self::SparseIndexOrder { position } => write!(
                formatter,
                "sparse property indexes are not strictly increasing at position {position}"
            ),
            Self::SparseIndexOutOfBounds { index, len } => write!(
                formatter,
                "sparse property index {index} is outside logical length {len}"
            ),
            Self::DuplicateName { id_family, name } => write!(
                formatter,
                "duplicate property name '{name}' in {id_family:?} namespace"
            ),
            Self::SparseNullMissingNotTotal { name } => write!(
                formatter,
                "sparse property layer '{name}' has null missing policy and is not total"
            ),
            Self::DuplicateLayerId { layer_id } => {
                write!(formatter, "duplicate property layer ID {layer_id:?}")
            }
            Self::MissingSnapshotSection { kind } => {
                write!(formatter, "snapshot is missing section kind {kind:#x}")
            }
            Self::SnapshotSectionVersion { kind, version } => write!(
                formatter,
                "snapshot section {kind:#x} has unsupported version {version}"
            ),
            Self::SnapshotSectionView { kind, error } => write!(
                formatter,
                "snapshot section {kind:#x} cannot be borrowed as expected records: {error}"
            ),
            Self::SnapshotRangeOutOfBounds {
                offset,
                len,
                available,
            } => write!(
                formatter,
                "snapshot range {offset}..{} exceeds available {available} bytes",
                offset.saturating_add(*len)
            ),
            Self::SnapshotInvalidUtf8 { offset } => {
                write!(
                    formatter,
                    "snapshot string at byte offset {offset} is not UTF-8"
                )
            }
            Self::UnknownIdFamilyTag { tag } => {
                write!(formatter, "unknown property ID-family tag {tag}")
            }
            Self::UnknownLayerRoleTag { tag } => {
                write!(formatter, "unknown property layer-role tag {tag}")
            }
            Self::UnknownStorageTag { tag } => {
                write!(formatter, "unknown property storage tag {tag}")
            }
            Self::UnknownMissingPolicyTag { tag } => {
                write!(formatter, "unknown property missing-policy tag {tag}")
            }
            Self::UnknownArrowFamilyTag { tag } => {
                write!(formatter, "unknown Arrow value-family tag {tag}")
            }
            Self::UnknownIdentityModeTag { tag } => {
                write!(formatter, "unknown identity-map mode tag {tag}")
            }
            Self::SnapshotDescriptorMismatch { reason } => {
                write!(formatter, "property snapshot descriptor mismatch: {reason}")
            }
            Self::SnapshotDataLength { reason } => {
                write!(
                    formatter,
                    "property snapshot data length mismatch: {reason}"
                )
            }
            Self::Arrow { message } => write!(formatter, "Arrow property error: {message}"),
            Self::MissingIdentityMap { id_family } => {
                write!(formatter, "missing explicit identity map for {id_family:?}")
            }
            Self::IdentityMapLength {
                id_family,
                required,
                actual,
            } => write!(
                formatter,
                "identity map for {id_family:?} has length {actual}, required {required}"
            ),
            Self::LengthDoesNotFitU64 { value } => {
                write!(formatter, "length {value} does not fit u64")
            }
        }
    }
}

impl Error for PropertyError {}

/// Data backing one property layer.
///
/// # Performance
///
/// Cloning is `O(1)` because Arrow arrays are reference-counted.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PropertyLayerData {
    /// Dense Arrow array with one slot per ID index.
    Dense {
        /// Dense values.
        values: ArrayRef,
    },
    /// Sparse Arrow array keyed by explicit indexes.
    Sparse {
        /// Strictly ascending sparse indexes.
        indices: Arc<UInt64Array>,
        /// Values aligned with `indices`.
        values: ArrayRef,
        /// Optional Arrow scalar default encoded as a length-one array.
        default: Option<ArrayRef>,
    },
}

/// Arrow-backed property layer.
///
/// # Performance
///
/// Cloning is `O(1)` for Arrow buffers plus descriptor clone cost.
#[derive(Clone, Debug)]
#[must_use]
pub struct PropertyLayer {
    /// Layer descriptor.
    descriptor: PropertyLayerDescriptor,
    /// Logical layer length.
    len: usize,
    /// Layer data.
    data: PropertyLayerData,
}

impl PropertyLayer {
    /// Builds a dense Arrow-backed property layer.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when storage, Arrow type, or nullability is invalid.
    ///
    /// # Performance
    ///
    /// Validation is `O(values.len())` only when nullability must be checked.
    pub fn try_new_dense(
        descriptor: PropertyLayerDescriptor,
        values: ArrayRef,
    ) -> Result<Self, PropertyError> {
        if descriptor.storage != StorageMode::Dense {
            return Err(PropertyError::ExpectedDenseStorage {
                name: descriptor.name,
            });
        }
        ensure_arrow_type(&descriptor, values.as_ref())?;
        if !descriptor.arrow_field.is_nullable() {
            ensure_no_nulls(values.as_ref())?;
        }
        let len = values.len();
        Ok(Self {
            descriptor,
            len,
            data: PropertyLayerData::Dense { values },
        })
    }

    /// Builds a sparse Arrow-backed property layer.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when storage, Arrow type, default policy,
    /// sparse index ordering, or nullability is invalid.
    ///
    /// # Performance
    ///
    /// Validation is `O(indices.len() + default length)`.
    pub fn try_new_sparse(
        descriptor: PropertyLayerDescriptor,
        len: usize,
        indices: Arc<UInt64Array>,
        values: ArrayRef,
        default: Option<ArrayRef>,
    ) -> Result<Self, PropertyError> {
        let StorageMode::Sparse { missing } = descriptor.storage else {
            return Err(PropertyError::ExpectedSparseStorage {
                name: descriptor.name,
            });
        };
        validate_default_policy(&descriptor, missing, default.as_ref())?;
        ensure_arrow_type(&descriptor, values.as_ref())?;
        if indices.len() != values.len() {
            return Err(PropertyError::SparseLengthMismatch {
                indices: indices.len(),
                values: values.len(),
            });
        }
        ensure_no_nulls(indices.as_ref())?;
        if !descriptor.arrow_field.is_nullable() {
            ensure_no_nulls(values.as_ref())?;
        }
        validate_sparse_indices(indices.as_ref(), len)?;
        Ok(Self {
            descriptor,
            len,
            data: PropertyLayerData::Sparse {
                indices,
                values,
                default,
            },
        })
    }

    /// Returns this layer's descriptor.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn descriptor(&self) -> &PropertyLayerDescriptor {
        &self.descriptor
    }

    /// Returns this layer's data.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn data(&self) -> &PropertyLayerData {
        &self.data
    }

    /// Returns the logical layer length.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the logical layer is empty.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Selected dense primitive element weights for a topology view.
///
/// # Performance
///
/// Weight lookup is `O(1)`.
pub struct DenseElementWeights<'view, T: ElementIndex, P: ArrowPrimitiveType> {
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Primitive values.
    values: &'view PrimitiveArray<P>,
}

impl<'view, T: ElementIndex, P: ArrowPrimitiveType> DenseElementWeights<'view, T, P> {
    /// Selects a dense primitive layer as element weights for `topology`.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] if the layer is not element-keyed, dense,
    /// primitive type `P`, non-null, or long enough.
    ///
    /// # Performance
    ///
    /// Validation is `O(layer.len())` for the null check.
    pub fn new(topology: &'view T, layer: &'view PropertyLayer) -> Result<Self, PropertyError> {
        let values = validate_dense_primitive_selection::<P>(
            layer,
            IdFamily::Element,
            topology.element_bound(),
        )?;
        Ok(Self { topology, values })
    }
}

impl<T: ElementIndex, P: ArrowPrimitiveType> TopologyBase for DenseElementWeights<'_, T, P> {
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T: ElementIndex, P: ArrowPrimitiveType> ElementWeight for DenseElementWeights<'_, T, P>
where
    P::Native: Copy,
{
    type Weight = P::Native;

    fn element_weight(&self, element: Self::ElementId) -> Self::Weight {
        self.values.value(self.topology.element_index(element))
    }
}

/// Selected dense primitive relation weights for a topology view.
///
/// # Performance
///
/// Weight lookup is `O(1)`.
pub struct DenseRelationWeights<'view, T: RelationIndex, P: ArrowPrimitiveType> {
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Primitive values.
    values: &'view PrimitiveArray<P>,
}

impl<'view, T: RelationIndex, P: ArrowPrimitiveType> DenseRelationWeights<'view, T, P> {
    /// Selects a dense primitive layer as relation weights for `topology`.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] if layer validation fails.
    ///
    /// # Performance
    ///
    /// Validation is `O(layer.len())` for the null check.
    pub fn new(topology: &'view T, layer: &'view PropertyLayer) -> Result<Self, PropertyError> {
        let values = validate_dense_primitive_selection::<P>(
            layer,
            IdFamily::Relation,
            topology.relation_bound(),
        )?;
        Ok(Self { topology, values })
    }
}

impl<T: RelationIndex, P: ArrowPrimitiveType> TopologyBase for DenseRelationWeights<'_, T, P> {
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T: RelationIndex, P: ArrowPrimitiveType> RelationWeight for DenseRelationWeights<'_, T, P>
where
    P::Native: Copy,
{
    type Weight = P::Native;

    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight {
        self.values.value(self.topology.relation_index(relation))
    }
}

/// Selected dense primitive incidence weights for an incidence topology view.
///
/// # Performance
///
/// Weight lookup is `O(1)`.
pub struct DenseIncidenceWeights<'view, T: IncidenceIndex, P: ArrowPrimitiveType> {
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Primitive values.
    values: &'view PrimitiveArray<P>,
}

impl<'view, T: IncidenceIndex, P: ArrowPrimitiveType> DenseIncidenceWeights<'view, T, P> {
    /// Selects a dense primitive layer as incidence weights for `topology`.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] if layer validation fails.
    ///
    /// # Performance
    ///
    /// Validation is `O(layer.len())` for the null check.
    pub fn new(topology: &'view T, layer: &'view PropertyLayer) -> Result<Self, PropertyError> {
        let values = validate_dense_primitive_selection::<P>(
            layer,
            IdFamily::Incidence,
            topology.incidence_bound(),
        )?;
        Ok(Self { topology, values })
    }
}

impl<T: IncidenceIndex, P: ArrowPrimitiveType> TopologyBase for DenseIncidenceWeights<'_, T, P> {
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T: IncidenceIndex, P: ArrowPrimitiveType> IncidenceBase for DenseIncidenceWeights<'_, T, P> {
    type IncidenceId = T::IncidenceId;
    type Role = T::Role;
}

impl<T: IncidenceIndex, P: ArrowPrimitiveType> IncidenceWeight for DenseIncidenceWeights<'_, T, P>
where
    P::Native: Copy,
{
    type Weight = P::Native;

    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight {
        self.values.value(self.topology.incidence_index(incidence))
    }
}

/// Selected sparse primitive element weights for a topology view.
///
/// # Performance
///
/// Weight lookup is `O(log k)` for `k` explicitly stored values.
pub struct SparseElementWeights<'view, T: ElementIndex, P: ArrowPrimitiveType> {
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Sparse indexes.
    indices: &'view UInt64Array,
    /// Sparse values.
    values: &'view PrimitiveArray<P>,
    /// Totalizing default value.
    default: P::Native,
}

impl<'view, T: ElementIndex, P: ArrowPrimitiveType> SparseElementWeights<'view, T, P>
where
    P::Native: Copy,
{
    /// Selects a sparse primitive layer as total element weights.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when the sparse layer is not total or type-compatible.
    ///
    /// # Performance
    ///
    /// Validation is `O(1)` plus default downcast.
    pub fn new(topology: &'view T, layer: &'view PropertyLayer) -> Result<Self, PropertyError> {
        let (indices, values, default) = validate_sparse_primitive_selection::<P>(
            layer,
            IdFamily::Element,
            topology.element_bound(),
        )?;
        Ok(Self {
            topology,
            indices,
            values,
            default,
        })
    }
}

impl<T: ElementIndex, P: ArrowPrimitiveType> TopologyBase for SparseElementWeights<'_, T, P> {
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T: ElementIndex, P: ArrowPrimitiveType> ElementWeight for SparseElementWeights<'_, T, P>
where
    P::Native: Copy,
{
    type Weight = P::Native;

    fn element_weight(&self, element: Self::ElementId) -> Self::Weight {
        sparse_value(
            self.indices,
            self.values,
            self.default,
            self.topology.element_index(element),
        )
    }
}

/// Selected sparse primitive relation weights for a topology view.
///
/// # Performance
///
/// Weight lookup is `O(log k)` for `k` explicitly stored values.
pub struct SparseRelationWeights<'view, T: RelationIndex, P: ArrowPrimitiveType> {
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Sparse indexes.
    indices: &'view UInt64Array,
    /// Sparse values.
    values: &'view PrimitiveArray<P>,
    /// Totalizing default value.
    default: P::Native,
}

impl<'view, T: RelationIndex, P: ArrowPrimitiveType> SparseRelationWeights<'view, T, P>
where
    P::Native: Copy,
{
    /// Selects a sparse primitive layer as total relation weights.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when the sparse layer is not total or type-compatible.
    ///
    /// # Performance
    ///
    /// Validation is `O(1)` plus default downcast.
    pub fn new(topology: &'view T, layer: &'view PropertyLayer) -> Result<Self, PropertyError> {
        let (indices, values, default) = validate_sparse_primitive_selection::<P>(
            layer,
            IdFamily::Relation,
            topology.relation_bound(),
        )?;
        Ok(Self {
            topology,
            indices,
            values,
            default,
        })
    }
}

impl<T: RelationIndex, P: ArrowPrimitiveType> TopologyBase for SparseRelationWeights<'_, T, P> {
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T: RelationIndex, P: ArrowPrimitiveType> RelationWeight for SparseRelationWeights<'_, T, P>
where
    P::Native: Copy,
{
    type Weight = P::Native;

    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight {
        sparse_value(
            self.indices,
            self.values,
            self.default,
            self.topology.relation_index(relation),
        )
    }
}

/// Selected sparse primitive incidence weights for an incidence topology view.
///
/// # Performance
///
/// Weight lookup is `O(log k)` for `k` explicitly stored values.
pub struct SparseIncidenceWeights<'view, T: IncidenceIndex, P: ArrowPrimitiveType> {
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Sparse indexes.
    indices: &'view UInt64Array,
    /// Sparse values.
    values: &'view PrimitiveArray<P>,
    /// Totalizing default value.
    default: P::Native,
}

impl<'view, T: IncidenceIndex, P: ArrowPrimitiveType> SparseIncidenceWeights<'view, T, P>
where
    P::Native: Copy,
{
    /// Selects a sparse primitive layer as total incidence weights.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when the sparse layer is not total or type-compatible.
    ///
    /// # Performance
    ///
    /// Validation is `O(1)` plus default downcast.
    pub fn new(topology: &'view T, layer: &'view PropertyLayer) -> Result<Self, PropertyError> {
        let (indices, values, default) = validate_sparse_primitive_selection::<P>(
            layer,
            IdFamily::Incidence,
            topology.incidence_bound(),
        )?;
        Ok(Self {
            topology,
            indices,
            values,
            default,
        })
    }
}

impl<T: IncidenceIndex, P: ArrowPrimitiveType> TopologyBase for SparseIncidenceWeights<'_, T, P> {
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T: IncidenceIndex, P: ArrowPrimitiveType> IncidenceBase for SparseIncidenceWeights<'_, T, P> {
    type IncidenceId = T::IncidenceId;
    type Role = T::Role;
}

impl<T: IncidenceIndex, P: ArrowPrimitiveType> IncidenceWeight for SparseIncidenceWeights<'_, T, P>
where
    P::Native: Copy,
{
    type Weight = P::Native;

    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight {
        sparse_value(
            self.indices,
            self.values,
            self.default,
            self.topology.incidence_index(incidence),
        )
    }
}

/// Validates that layer names are unique within each ID-family namespace.
///
/// # Errors
///
/// Returns [`PropertyError::DuplicateName`] for the first duplicate name.
///
/// # Performance
///
/// This function is `O(n log n + total name length)` for `n` descriptors.
pub fn validate_unique_names<'descriptor, I>(descriptors: I) -> Result<(), PropertyError>
where
    I: IntoIterator<Item = &'descriptor PropertyLayerDescriptor>,
{
    let mut seen: BTreeSet<(IdFamily, &str)> = BTreeSet::new();
    for descriptor in descriptors {
        let key = (descriptor.id_family, descriptor.name.as_str());
        if !seen.insert(key) {
            return Err(PropertyError::DuplicateName {
                id_family: descriptor.id_family,
                name: descriptor.name.clone(),
            });
        }
    }
    Ok(())
}

/// Validates that layer IDs are unique within one descriptor set.
///
/// # Errors
///
/// Returns [`PropertyError::DuplicateLayerId`] for the first duplicate ID.
///
/// # Performance
///
/// This function is `O(n log n)` for `n` descriptors.
pub fn validate_unique_layer_ids<'descriptor, I>(descriptors: I) -> Result<(), PropertyError>
where
    I: IntoIterator<Item = &'descriptor PropertyLayerDescriptor>,
{
    let mut seen: BTreeSet<LayerId> = BTreeSet::new();
    for descriptor in descriptors {
        if !seen.insert(descriptor.layer_id) {
            return Err(PropertyError::DuplicateLayerId {
                layer_id: descriptor.layer_id,
            });
        }
    }
    Ok(())
}

/// Identity snapshot map mode.
///
/// # Performance
///
/// Copying, comparing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IdentityMapMode {
    /// Local IDs are identical to canonical IDs for this family.
    LocalEqualsCanonical,
    /// The snapshot stores an explicit `u32` local-to-canonical map section.
    ExplicitU32Map,
}

impl IdentityMapMode {
    /// Returns the snapshot tag for this mode.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn tag(self) -> u32 {
        match self {
            Self::LocalEqualsCanonical => 0,
            Self::ExplicitU32Map => 1,
        }
    }

    /// Decodes a snapshot mode tag.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn from_tag(tag: u32) -> Option<Self> {
        match tag {
            0 => Some(Self::LocalEqualsCanonical),
            1 => Some(Self::ExplicitU32Map),
            _ => None,
        }
    }
}

/// Wire record declaring one identity family map mode.
///
/// # Performance
///
/// Copying and reading fields are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, FromBytes, Immutable, IntoBytes, KnownLayout, PartialEq)]
#[repr(C)]
pub struct IdentityModeRecord {
    /// ID-family tag.
    id_family: U32<LE>,
    /// Map-mode tag.
    mode: U32<LE>,
    /// Number of local IDs covered by the mode.
    local_len: U64<LE>,
}

impl IdentityModeRecord {
    /// Builds a local-equals-canonical identity mode record.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn local_equals_canonical(id_family: IdFamily, local_len: usize) -> Self {
        Self::new(id_family, IdentityMapMode::LocalEqualsCanonical, local_len)
    }

    /// Builds an explicit-`u32`-map identity mode record.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn explicit_u32_map(id_family: IdFamily, local_len: usize) -> Self {
        Self::new(id_family, IdentityMapMode::ExplicitU32Map, local_len)
    }

    /// Builds an identity mode record.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(id_family: IdFamily, mode: IdentityMapMode, local_len: usize) -> Self {
        Self {
            id_family: U32::new(id_family_tag(id_family)),
            mode: U32::new(mode.tag()),
            local_len: U64::new(local_len as u64),
        }
    }

    /// Returns this record's ID family.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError::UnknownIdFamilyTag`] if the record tag is unknown.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn id_family(&self) -> Result<IdFamily, PropertyError> {
        id_family_from_tag(self.id_family.get())
    }

    /// Returns this record's identity map mode.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError::UnknownIdentityModeTag`] if the record tag is unknown.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn mode(&self) -> Result<IdentityMapMode, PropertyError> {
        IdentityMapMode::from_tag(self.mode.get()).ok_or_else(|| {
            PropertyError::UnknownIdentityModeTag {
                tag: self.mode.get(),
            }
        })
    }

    /// Returns the local ID count covered by this mode.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` on targets where `u64` to `usize` fits; values
    /// above `usize::MAX` saturate to `usize::MAX` for validation errors.
    #[must_use]
    pub fn local_len(&self) -> usize {
        match usize::try_from(self.local_len.get()) {
            Ok(value) => value,
            Err(_error) => usize::MAX,
        }
    }
}

/// Summary returned after identity snapshot validation.
///
/// # Performance
///
/// Cloning is `O(f)` for `f` identity-family records.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct IdentitySnapshotSummary {
    /// Validated identity records.
    pub records: Vec<IdentityModeRecord>,
}

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
pub fn validate_identity_snapshot(
    snapshot: &Snapshot<'_>,
) -> Result<IdentitySnapshotSummary, PropertyError> {
    let section = snapshot.section(SNAPSHOT_KIND_IDENTITY_MODES).ok_or(
        PropertyError::MissingSnapshotSection {
            kind: SNAPSHOT_KIND_IDENTITY_MODES,
        },
    )?;
    if section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind: SNAPSHOT_KIND_IDENTITY_MODES,
            version: section.version(),
        });
    }
    let records: &[IdentityModeRecord] =
        section
            .try_as_slice()
            .map_err(|error| PropertyError::SnapshotSectionView {
                kind: SNAPSHOT_KIND_IDENTITY_MODES,
                error,
            })?;
    validate_identity_records(snapshot, records)?;
    Ok(IdentitySnapshotSummary {
        records: records.to_vec(),
    })
}

/// Encoded property descriptor and Arrow IPC data payloads.
///
/// # Performance
///
/// Cloning is `O(descriptor bytes + data bytes)`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct EncodedPropertySnapshot {
    /// Payload for [`SNAPSHOT_KIND_PROPERTY_DESCRIPTORS`].
    pub descriptors: Vec<u8>,
    /// Payload for [`SNAPSHOT_KIND_PROPERTY_DATA`].
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

/// Wire header for the property descriptor section.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct PropertySnapshotHeader {
    /// Number of descriptor records.
    record_count: U64<LE>,
    /// Byte length occupied by descriptor records after this header.
    record_bytes: U64<LE>,
}

/// Wire descriptor record for one property layer.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct PropertySnapshotRecord {
    /// Stable layer ID.
    layer_id: U64<LE>,
    /// Offset of layer name in descriptor string table.
    name_offset: U32<LE>,
    /// Length of layer name in descriptor string table.
    name_len: U32<LE>,
    /// Offset of Arrow field name in descriptor string table.
    field_name_offset: U32<LE>,
    /// Length of Arrow field name in descriptor string table.
    field_name_len: U32<LE>,
    /// ID-family tag.
    id_family: U32<LE>,
    /// Layer-role tag.
    role: U32<LE>,
    /// Storage tag.
    storage: U32<LE>,
    /// Missing-policy tag.
    missing_policy: U32<LE>,
    /// Arrow value-family tag.
    arrow_family: U32<LE>,
    /// Arrow field nullable flag.
    nullable: U32<LE>,
    /// Logical layer length.
    logical_len: U64<LE>,
    /// Offset in property data section.
    data_offset: U64<LE>,
    /// Byte length in property data section.
    data_len: U64<LE>,
    /// Explicit sparse value count, or dense value count.
    value_count: U64<LE>,
    /// Reserved for future descriptor flags.
    reserved: U64<LE>,
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
pub fn encode_property_snapshot(
    layers: &[PropertyLayer],
) -> Result<EncodedPropertySnapshot, PropertyError> {
    validate_unique_names(layers.iter().map(PropertyLayer::descriptor))?;
    validate_unique_layer_ids(layers.iter().map(PropertyLayer::descriptor))?;

    let mut data = Vec::new();
    let mut strings = Vec::new();
    let mut records = Vec::with_capacity(layers.len());
    for layer in layers {
        let descriptor = layer.descriptor();
        let name_offset = append_string(&mut strings, descriptor.name.as_str())?;
        let field_name_offset = append_string(&mut strings, descriptor.arrow_field.name())?;
        let data_offset = data.len();
        let layer_data = encode_layer_ipc(layer)?;
        let data_len = layer_data.len();
        data.extend_from_slice(&layer_data);
        records.push(PropertySnapshotRecord {
            layer_id: U64::new(descriptor.layer_id.0),
            name_offset: U32::new(name_offset),
            name_len: U32::new(str_len_u32(descriptor.name.as_str())?),
            field_name_offset: U32::new(field_name_offset),
            field_name_len: U32::new(str_len_u32(descriptor.arrow_field.name())?),
            id_family: U32::new(id_family_tag(descriptor.id_family)),
            role: U32::new(layer_role_tag(descriptor.role)),
            storage: U32::new(storage_tag(descriptor.storage)),
            missing_policy: U32::new(missing_policy_tag(descriptor.storage)),
            arrow_family: U32::new(descriptor.value_family().tag()),
            nullable: U32::new(u32::from(descriptor.arrow_field.is_nullable())),
            logical_len: U64::new(usize_to_u64(layer.len())?),
            data_offset: U64::new(usize_to_u64(data_offset)?),
            data_len: U64::new(usize_to_u64(data_len)?),
            value_count: U64::new(usize_to_u64(layer_value_count(layer))?),
            reserved: U64::new(0),
        });
    }
    let record_bytes = records
        .len()
        .checked_mul(core::mem::size_of::<PropertySnapshotRecord>())
        .ok_or(PropertyError::SnapshotDescriptorMismatch {
            reason: "record byte length overflow",
        })?;
    let header = PropertySnapshotHeader {
        record_count: U64::new(usize_to_u64(records.len())?),
        record_bytes: U64::new(usize_to_u64(record_bytes)?),
    };
    let mut descriptor_bytes = Vec::with_capacity(
        core::mem::size_of::<PropertySnapshotHeader>() + record_bytes + strings.len(),
    );
    descriptor_bytes.extend_from_slice(header.as_bytes());
    descriptor_bytes.extend_from_slice(records.as_bytes());
    descriptor_bytes.extend_from_slice(&strings);
    Ok(EncodedPropertySnapshot {
        descriptors: descriptor_bytes,
        data,
    })
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
pub fn validate_property_snapshot(
    snapshot: &Snapshot<'_>,
) -> Result<PropertySnapshotSummary, PropertyError> {
    let descriptor_section = snapshot.section(SNAPSHOT_KIND_PROPERTY_DESCRIPTORS).ok_or(
        PropertyError::MissingSnapshotSection {
            kind: SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
        },
    )?;
    let data_section = snapshot.section(SNAPSHOT_KIND_PROPERTY_DATA).ok_or(
        PropertyError::MissingSnapshotSection {
            kind: SNAPSHOT_KIND_PROPERTY_DATA,
        },
    )?;
    if descriptor_section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind: SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
            version: descriptor_section.version(),
        });
    }
    if data_section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind: SNAPSHOT_KIND_PROPERTY_DATA,
            version: data_section.version(),
        });
    }
    validate_property_sections(descriptor_section.bytes(), data_section.bytes())
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
#[expect(
    clippy::too_many_lines,
    reason = "property descriptor/data validation keeps the binary section checks in one straight-line pass"
)]
pub fn validate_property_sections(
    descriptor_bytes: &[u8],
    data_bytes: &[u8],
) -> Result<PropertySnapshotSummary, PropertyError> {
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
        .checked_mul(core::mem::size_of::<PropertySnapshotRecord>())
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
    let mut ids: BTreeSet<LayerId> = BTreeSet::new();
    let mut ranges = Vec::with_capacity(record_count_usize);
    let mut total_logical_values = 0_usize;
    for position in 0..record_count_usize {
        let start = position * core::mem::size_of::<PropertySnapshotRecord>();
        let record = parse_property_record(&record_bytes_slice[start..])?;
        let id_family = id_family_from_tag(record.id_family.get())?;
        let _role = layer_role_from_tag(record.role.get())?;
        let storage = storage_from_tags(record.storage.get(), record.missing_policy.get())?;
        let family = ArrowValueFamily::from_tag(record.arrow_family.get()).ok_or_else(|| {
            PropertyError::UnknownArrowFamilyTag {
                tag: record.arrow_family.get(),
            }
        })?;
        let name = read_snapshot_str(
            string_bytes,
            record.name_offset.get(),
            record.name_len.get(),
        )?;
        let field_name = read_snapshot_str(
            string_bytes,
            record.field_name_offset.get(),
            record.field_name_len.get(),
        )?;
        if !ids.insert(LayerId(record.layer_id.get())) {
            return Err(PropertyError::DuplicateLayerId {
                layer_id: LayerId(record.layer_id.get()),
            });
        }
        if !names.insert((id_family, name)) {
            return Err(PropertyError::DuplicateName {
                id_family,
                name: LayerName::try_new(name)?,
            });
        }
        let range =
            validate_property_record_data(&record, storage, family, field_name, data_bytes)?;
        ranges.push(range);
        total_logical_values = total_logical_values
            .checked_add(u64_to_usize(record.logical_len.get())?)
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

/// Validates identity records and required map sections.
///
/// # Performance
///
/// This function is `O(f)` for `f` records.
fn validate_identity_records(
    snapshot: &Snapshot<'_>,
    records: &[IdentityModeRecord],
) -> Result<(), PropertyError> {
    let mut seen = BTreeSet::new();
    for record in records {
        let family = record.id_family()?;
        if !seen.insert(family) {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "duplicate identity family mode record",
            });
        }
        match record.mode()? {
            IdentityMapMode::LocalEqualsCanonical => {}
            IdentityMapMode::ExplicitU32Map => {
                validate_identity_map_section(snapshot, family, record.local_len())?;
            }
        }
    }
    Ok(())
}

/// Validates one explicit identity-map section.
///
/// # Performance
///
/// This function is `O(s)` for snapshot section count `s`.
fn validate_identity_map_section(
    snapshot: &Snapshot<'_>,
    id_family: IdFamily,
    required: usize,
) -> Result<(), PropertyError> {
    let kind = identity_map_kind(id_family);
    let section = snapshot
        .section(kind)
        .ok_or(PropertyError::MissingIdentityMap { id_family })?;
    if section.version() != SNAPSHOT_PROPERTY_VERSION {
        return Err(PropertyError::SnapshotSectionVersion {
            kind,
            version: section.version(),
        });
    }
    let map: &[U32<LE>] = section
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
const fn identity_map_kind(id_family: IdFamily) -> u32 {
    match id_family {
        IdFamily::Element => SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U32,
        IdFamily::Relation => SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32,
        IdFamily::Incidence => SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U32,
    }
}

/// Appends a string to a snapshot string table.
///
/// # Performance
///
/// This function is `O(value.len())`.
fn append_string(strings: &mut Vec<u8>, value: &str) -> Result<u32, PropertyError> {
    let offset = u32::try_from(strings.len()).map_err(|_error| {
        PropertyError::SnapshotDescriptorMismatch {
            reason: "string table offset overflow",
        }
    })?;
    strings.extend_from_slice(value.as_bytes());
    Ok(offset)
}

/// Returns a string length as `u32`.
///
/// # Performance
///
/// This function is `O(1)`.
fn str_len_u32(value: &str) -> Result<u32, PropertyError> {
    u32::try_from(value.len()).map_err(|_error| PropertyError::SnapshotDescriptorMismatch {
        reason: "string length overflow",
    })
}

/// Returns the number of value slots encoded for a layer.
///
/// # Performance
///
/// This function is `O(1)`.
fn layer_value_count(layer: &PropertyLayer) -> usize {
    match layer.data() {
        PropertyLayerData::Dense { values } => values.len(),
        PropertyLayerData::Sparse { indices, .. } => indices.len(),
    }
}

/// Encodes one property layer as an Arrow IPC stream.
///
/// # Performance
///
/// This function is `O(layer payload bytes)`.
fn encode_layer_ipc(layer: &PropertyLayer) -> Result<Vec<u8>, PropertyError> {
    let (schema, columns) = match layer.data() {
        PropertyLayerData::Dense { values } => {
            let schema = Arc::new(Schema::new(vec![layer.descriptor().arrow_field.clone()]));
            (schema, vec![Arc::clone(values)])
        }
        PropertyLayerData::Sparse {
            indices,
            values,
            default,
        } => {
            let mut fields = vec![Field::new("index", DataType::UInt64, false)];
            fields.push(layer.descriptor().arrow_field.clone());
            let mut columns: Vec<ArrayRef> =
                vec![Arc::clone(indices) as ArrayRef, Arc::clone(values)];
            if let Some(default_value) = default {
                fields.push(Field::new("default", values.data_type().clone(), false));
                columns.push(Arc::clone(default_value));
            }
            (Arc::new(Schema::new(fields)), columns)
        }
    };
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns).map_err(map_arrow_error)?;
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
fn parse_property_record(bytes: &[u8]) -> Result<PropertySnapshotRecord, PropertyError> {
    let need = core::mem::size_of::<PropertySnapshotRecord>();
    if bytes.len() < need {
        return Err(PropertyError::SnapshotDataLength {
            reason: "property record is truncated",
        });
    }
    let mut words = [0_u64; 11];
    for (slot, chunk) in bytes[..need].chunks_exact(8).enumerate() {
        words[slot] = read_u64_le(chunk)?;
    }
    Ok(PropertySnapshotRecord {
        layer_id: U64::new(words[0]),
        name_offset: U32::new(low_u32(words[1])?),
        name_len: U32::new(high_u32(words[1])?),
        field_name_offset: U32::new(low_u32(words[2])?),
        field_name_len: U32::new(high_u32(words[2])?),
        id_family: U32::new(low_u32(words[3])?),
        role: U32::new(high_u32(words[3])?),
        storage: U32::new(low_u32(words[4])?),
        missing_policy: U32::new(high_u32(words[4])?),
        arrow_family: U32::new(low_u32(words[5])?),
        nullable: U32::new(high_u32(words[5])?),
        logical_len: U64::new(words[6]),
        data_offset: U64::new(words[7]),
        data_len: U64::new(words[8]),
        value_count: U64::new(words[9]),
        reserved: U64::new(words[10]),
    })
}

/// Returns the low 32 bits of a packed u64 record word.
///
/// # Performance
///
/// This function is `O(1)`.
fn low_u32(word: u64) -> Result<u32, PropertyError> {
    u32::try_from(word & u64::from(u32::MAX)).map_err(|_error| {
        PropertyError::SnapshotDescriptorMismatch {
            reason: "packed low u32 did not fit",
        }
    })
}

/// Returns the high 32 bits of a packed u64 record word.
///
/// # Performance
///
/// This function is `O(1)`.
fn high_u32(word: u64) -> Result<u32, PropertyError> {
    u32::try_from(word >> 32).map_err(|_error| PropertyError::SnapshotDescriptorMismatch {
        reason: "packed high u32 did not fit",
    })
}

/// Validates a property data range declared by one record.
///
/// # Performance
///
/// This function is `O(Arrow IPC payload validation)`.
fn validate_property_record_data(
    record: &PropertySnapshotRecord,
    storage: StorageMode,
    family: ArrowValueFamily,
    field_name: &str,
    data: &[u8],
) -> Result<core::ops::Range<usize>, PropertyError> {
    if record.reserved.get() != 0 {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "property descriptor reserved word must be zero",
        });
    }
    let offset = u64_to_usize(record.data_offset.get())?;
    let len = u64_to_usize(record.data_len.get())?;
    let end = checked_end(offset, len, data.len())?;
    let batch = read_one_ipc_batch(&data[offset..end])?;
    match storage {
        StorageMode::Dense => validate_dense_batch(record, family, field_name, &batch)?,
        StorageMode::Sparse { missing } => {
            validate_sparse_batch(record, family, field_name, missing, &batch)?;
        }
    }
    Ok(offset..end)
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
fn validate_dense_batch(
    record: &PropertySnapshotRecord,
    family: ArrowValueFamily,
    field_name: &str,
    batch: &RecordBatch,
) -> Result<(), PropertyError> {
    if batch.num_columns() != 1 {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "dense property batch must contain one column",
        });
    }
    let values = batch.column(0);
    if values.len() != u64_to_usize(record.logical_len.get())?
        || values.len() != u64_to_usize(record.value_count.get())?
    {
        return Err(PropertyError::SnapshotDataLength {
            reason: "dense property Arrow length does not match descriptor",
        });
    }
    validate_value_column(record, family, field_name, values.as_ref())
}

/// Validates one sparse Arrow IPC batch.
///
/// # Performance
///
/// This function is `O(value_count)` for sparse index validation.
fn validate_sparse_batch(
    record: &PropertySnapshotRecord,
    family: ArrowValueFamily,
    field_name: &str,
    missing: MissingPolicy,
    batch: &RecordBatch,
) -> Result<(), PropertyError> {
    let expected_columns = match missing {
        MissingPolicy::Null => 2,
        MissingPolicy::Default => 3,
    };
    if batch.num_columns() != expected_columns {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "sparse property batch column count does not match missing policy",
        });
    }
    let indexes = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .ok_or(PropertyError::SnapshotDescriptorMismatch {
            reason: "sparse property index column is not UInt64",
        })?;
    let values = batch.column(1);
    let value_count = u64_to_usize(record.value_count.get())?;
    if indexes.len() != value_count || values.len() != value_count {
        return Err(PropertyError::SnapshotDataLength {
            reason: "sparse property Arrow value count does not match descriptor",
        });
    }
    validate_value_column(record, family, field_name, values.as_ref())?;
    validate_sparse_indices(indexes, u64_to_usize(record.logical_len.get())?)?;
    if missing == MissingPolicy::Default {
        let default = batch.column(2);
        if default.len() != 1 || default.data_type() != values.data_type() || default.is_null(0) {
            return Err(PropertyError::SnapshotDescriptorMismatch {
                reason: "sparse property default column is not a non-null matching scalar",
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
fn validate_value_column(
    record: &PropertySnapshotRecord,
    family: ArrowValueFamily,
    field_name: &str,
    values: &dyn Array,
) -> Result<(), PropertyError> {
    if ArrowValueFamily::from_data_type(values.data_type()) != family {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "Arrow value family does not match descriptor",
        });
    }
    let nullable = record.nullable.get() != 0;
    if !nullable && values.null_count() != 0 {
        return Err(PropertyError::UnexpectedNull { index: 0 });
    }
    if field_name.is_empty() {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "Arrow field name must not be empty",
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
fn read_snapshot_str(bytes: &[u8], offset: u32, len: u32) -> Result<&str, PropertyError> {
    let start = offset as usize;
    let len_usize = len as usize;
    let end = checked_end(start, len_usize, bytes.len())?;
    core::str::from_utf8(&bytes[start..end])
        .map_err(|_error| PropertyError::SnapshotInvalidUtf8 { offset: start })
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

/// Converts an ID family to its snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
const fn id_family_tag(id_family: IdFamily) -> u32 {
    match id_family {
        IdFamily::Element => 0,
        IdFamily::Relation => 1,
        IdFamily::Incidence => 2,
    }
}

/// Decodes an ID family snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
const fn id_family_from_tag(tag: u32) -> Result<IdFamily, PropertyError> {
    match tag {
        0 => Ok(IdFamily::Element),
        1 => Ok(IdFamily::Relation),
        2 => Ok(IdFamily::Incidence),
        _ => Err(PropertyError::UnknownIdFamilyTag { tag }),
    }
}

/// Converts a layer role to its snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
const fn layer_role_tag(role: LayerRole) -> u32 {
    match role {
        LayerRole::Weight => 0,
        LayerRole::Property => 1,
    }
}

/// Decodes a layer role snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
const fn layer_role_from_tag(tag: u32) -> Result<LayerRole, PropertyError> {
    match tag {
        0 => Ok(LayerRole::Weight),
        1 => Ok(LayerRole::Property),
        _ => Err(PropertyError::UnknownLayerRoleTag { tag }),
    }
}

/// Converts storage mode to its snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
const fn storage_tag(storage: StorageMode) -> u32 {
    match storage {
        StorageMode::Dense => 0,
        StorageMode::Sparse { .. } => 1,
    }
}

/// Converts missing policy to its snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
const fn missing_policy_tag(storage: StorageMode) -> u32 {
    match storage {
        StorageMode::Dense => 0,
        StorageMode::Sparse {
            missing: MissingPolicy::Null,
        } => 1,
        StorageMode::Sparse {
            missing: MissingPolicy::Default,
        } => 2,
    }
}

/// Decodes storage and missing policy tags.
///
/// # Performance
///
/// This function is `O(1)`.
const fn storage_from_tags(storage: u32, missing: u32) -> Result<StorageMode, PropertyError> {
    match (storage, missing) {
        (0, 0) => Ok(StorageMode::Dense),
        (1, 1) => Ok(StorageMode::Sparse {
            missing: MissingPolicy::Null,
        }),
        (1, 2) => Ok(StorageMode::Sparse {
            missing: MissingPolicy::Default,
        }),
        (0, _) => Err(PropertyError::UnknownMissingPolicyTag { tag: missing }),
        (_, _) => Err(PropertyError::UnknownStorageTag { tag: storage }),
    }
}

/// Ensures an Arrow array matches a descriptor field data type.
///
/// # Performance
///
/// This function is `O(1)`.
fn ensure_arrow_type(
    descriptor: &PropertyLayerDescriptor,
    values: &dyn Array,
) -> Result<(), PropertyError> {
    if descriptor.arrow_field.data_type() == values.data_type() {
        Ok(())
    } else {
        Err(PropertyError::ArrowTypeMismatch {
            name: descriptor.name.clone(),
        })
    }
}

/// Validates sparse default policy and Arrow type.
///
/// # Performance
///
/// This function is `O(1)`.
fn validate_default_policy(
    descriptor: &PropertyLayerDescriptor,
    missing: MissingPolicy,
    default: Option<&ArrayRef>,
) -> Result<(), PropertyError> {
    match (missing, default) {
        (MissingPolicy::Null, None) => Ok(()),
        (MissingPolicy::Default, Some(array)) => {
            ensure_arrow_type(descriptor, array.as_ref())?;
            if array.len() == 1 && !array.is_null(0) {
                Ok(())
            } else {
                Err(PropertyError::DefaultPolicyMismatch {
                    name: descriptor.name.clone(),
                })
            }
        }
        (MissingPolicy::Null | MissingPolicy::Default, _) => {
            Err(PropertyError::DefaultPolicyMismatch {
                name: descriptor.name.clone(),
            })
        }
    }
}

/// Ensures an Arrow array has no null slots.
///
/// # Performance
///
/// This function is `O(array.len())`.
fn ensure_no_nulls(array: &dyn Array) -> Result<(), PropertyError> {
    for index in 0..array.len() {
        if array.is_null(index) {
            return Err(PropertyError::UnexpectedNull { index });
        }
    }
    Ok(())
}

/// Validates sparse index ordering and bounds.
///
/// # Performance
///
/// This function is `O(indices.len())`.
fn validate_sparse_indices(indices: &UInt64Array, len: usize) -> Result<(), PropertyError> {
    let len_u64 = usize_to_u64(len)?;
    let mut previous = None;
    for position in 0..indices.len() {
        let index = indices.value(position);
        if index >= len_u64 {
            return Err(PropertyError::SparseIndexOutOfBounds { index, len });
        }
        if let Some(prior) = previous
            && index <= prior
        {
            return Err(PropertyError::SparseIndexOrder { position });
        }
        previous = Some(index);
    }
    Ok(())
}

/// Validates a dense primitive layer selection.
///
/// # Performance
///
/// This function is `O(layer.len())` for the null check.
fn validate_dense_primitive_selection<P: ArrowPrimitiveType>(
    layer: &PropertyLayer,
    expected: IdFamily,
    required: usize,
) -> Result<&PrimitiveArray<P>, PropertyError> {
    if layer.descriptor.id_family != expected {
        return Err(PropertyError::IdFamilyMismatch {
            expected,
            actual: layer.descriptor.id_family,
        });
    }
    if layer.len() < required {
        return Err(PropertyError::LayerTooShort {
            required,
            actual: layer.len(),
        });
    }
    let PropertyLayerData::Dense { values } = layer.data() else {
        return Err(PropertyError::ExpectedDenseStorage {
            name: layer.descriptor.name.clone(),
        });
    };
    let primitive = values
        .as_any()
        .downcast_ref::<PrimitiveArray<P>>()
        .ok_or_else(|| PropertyError::ArrowTypeMismatch {
            name: layer.descriptor.name.clone(),
        })?;
    ensure_no_nulls(primitive)?;
    Ok(primitive)
}

/// Borrowed sparse primitive selection parts.
type SparsePrimitiveSelection<'layer, P> = (
    &'layer UInt64Array,
    &'layer PrimitiveArray<P>,
    <P as ArrowPrimitiveType>::Native,
);

/// Validates a sparse primitive layer selection.
///
/// # Performance
///
/// This function is `O(1)` plus default downcast.
fn validate_sparse_primitive_selection<P: ArrowPrimitiveType>(
    layer: &PropertyLayer,
    expected: IdFamily,
    required: usize,
) -> Result<SparsePrimitiveSelection<'_, P>, PropertyError>
where
    P::Native: Copy,
{
    if layer.descriptor.id_family != expected {
        return Err(PropertyError::IdFamilyMismatch {
            expected,
            actual: layer.descriptor.id_family,
        });
    }
    if layer.len() < required {
        return Err(PropertyError::LayerTooShort {
            required,
            actual: layer.len(),
        });
    }
    let PropertyLayerData::Sparse {
        indices,
        values,
        default,
    } = layer.data()
    else {
        return Err(PropertyError::ExpectedSparseStorage {
            name: layer.descriptor.name.clone(),
        });
    };
    let Some(default_array) = default else {
        return Err(PropertyError::SparseNullMissingNotTotal {
            name: layer.descriptor.name.clone(),
        });
    };
    let primitive = values
        .as_any()
        .downcast_ref::<PrimitiveArray<P>>()
        .ok_or_else(|| PropertyError::ArrowTypeMismatch {
            name: layer.descriptor.name.clone(),
        })?;
    let default_primitive = default_array
        .as_any()
        .downcast_ref::<PrimitiveArray<P>>()
        .ok_or_else(|| PropertyError::ArrowTypeMismatch {
            name: layer.descriptor.name.clone(),
        })?;
    if default_primitive.len() != 1 || default_primitive.is_null(0) {
        return Err(PropertyError::DefaultPolicyMismatch {
            name: layer.descriptor.name.clone(),
        });
    }
    Ok((indices.as_ref(), primitive, default_primitive.value(0)))
}

/// Returns a sparse primitive value or the layer default.
///
/// # Performance
///
/// This function is `O(log k)` for `k` sparse indexes.
fn sparse_value<P: ArrowPrimitiveType>(
    indices: &UInt64Array,
    values: &PrimitiveArray<P>,
    default: P::Native,
    index: usize,
) -> P::Native
where
    P::Native: Copy,
{
    let Ok(target) = u64::try_from(index) else {
        return default;
    };
    let mut low = 0_usize;
    let mut high = indices.len();
    while low < high {
        let mid = low + ((high - low) / 2);
        let value = indices.value(mid);
        if value < target {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low < indices.len() && indices.value(low) == target {
        values.value(low)
    } else {
        default
    }
}

/// Converts an Arrow error into a property error.
///
/// # Performance
///
/// This function is `O(error message length)`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Arrow result adapters hand over owned errors and this helper consumes them into messages"
)]
fn map_arrow_error(error: arrow_schema::ArrowError) -> PropertyError {
    PropertyError::Arrow {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for generic property descriptor and selected weight validation.

    use arrow_array::{
        Float32Array, Int32Array, UInt32Array,
        types::{Float32Type, Int32Type},
    };
    use oxgraph_snapshot::{Snapshot, SnapshotBuilder};

    use super::*;

    /// Test topology with dense relation IDs.
    #[derive(Clone, Copy)]
    struct Topology;

    impl TopologyBase for Topology {
        type ElementId = u32;
        type RelationId = u32;
    }

    impl RelationIndex for Topology {
        fn relation_bound(&self) -> usize {
            2
        }

        fn relation_index(&self, relation: Self::RelationId) -> usize {
            relation as usize
        }
    }

    /// Builds an Arrow field for test descriptors.
    ///
    /// # Performance
    ///
    /// This function is `O(name.len())`.
    fn field(name: &str, data_type: DataType) -> Field {
        Field::new(name, data_type, false)
    }

    /// Dense relation layers can be selected by relation index without f64.
    #[test]
    fn dense_relation_layer_selects_i32_weight() -> Result<(), PropertyError> {
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(1),
            "count",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            field("count", DataType::Int32),
        )?;
        let layer =
            PropertyLayer::try_new_dense(descriptor, Arc::new(Int32Array::from(vec![2, 7])))?;
        let selected = DenseRelationWeights::<_, Int32Type>::new(&Topology, &layer)?;
        assert_eq!(selected.relation_weight(1), 7);
        Ok(())
    }

    /// Sparse relation layers can totalize with an Arrow default scalar.
    #[test]
    fn sparse_relation_layer_totalizes_with_default() -> Result<(), PropertyError> {
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(2),
            "capacity",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            },
            field("capacity", DataType::Float32),
        )?;
        let layer = PropertyLayer::try_new_sparse(
            descriptor,
            4,
            Arc::new(UInt64Array::from(vec![1_u64])),
            Arc::new(Float32Array::from(vec![3.5_f32])),
            Some(Arc::new(Float32Array::from(vec![1.25_f32]))),
        )?;
        let selected = SparseRelationWeights::<_, Float32Type>::new(&Topology, &layer)?;
        assert!((selected.relation_weight(0) - 1.25).abs() < f32::EPSILON);
        assert!((selected.relation_weight(1) - 3.5).abs() < f32::EPSILON);
        Ok(())
    }

    /// Duplicate names are rejected only within the same ID family.
    #[test]
    fn duplicate_names_are_family_scoped() -> Result<(), PropertyError> {
        let first = PropertyLayerDescriptor::try_new(
            LayerId(1),
            "weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            field("weight", DataType::UInt32),
        )?;
        let second = PropertyLayerDescriptor::try_new(
            LayerId(2),
            "weight",
            IdFamily::Element,
            LayerRole::Weight,
            StorageMode::Dense,
            field("weight", DataType::UInt32),
        )?;
        let duplicate = PropertyLayerDescriptor::try_new(
            LayerId(3),
            "weight",
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            field("weight", DataType::UInt32),
        )?;
        assert!(validate_unique_names([&first, &second]).is_ok());
        assert!(matches!(
            validate_unique_names([&first, &duplicate]),
            Err(PropertyError::DuplicateName { .. })
        ));
        Ok(())
    }

    /// Descriptor value-family classification covers non-floating Arrow properties.
    #[test]
    fn descriptor_classifies_generic_arrow_families() -> Result<(), PropertyError> {
        let boolean = PropertyLayerDescriptor::try_new(
            LayerId(1),
            "flag",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("flag", DataType::Boolean, false),
        )?;
        let utf8 = PropertyLayerDescriptor::try_new(
            LayerId(2),
            "label",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("label", DataType::Utf8, true),
        )?;
        assert_eq!(boolean.value_family(), ArrowValueFamily::Boolean);
        assert_eq!(utf8.value_family(), ArrowValueFamily::Utf8);
        Ok(())
    }

    /// Property descriptor/data sections roundtrip through Arrow IPC payloads.
    #[test]
    fn property_snapshot_sections_validate() -> Result<(), Box<dyn Error>> {
        let dense_descriptor = PropertyLayerDescriptor::try_new(
            LayerId(10),
            "count",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            field("count", DataType::UInt32),
        )?;
        let dense = PropertyLayer::try_new_dense(
            dense_descriptor,
            Arc::new(UInt32Array::from(vec![4_u32, 9_u32])),
        )?;
        let sparse_descriptor = PropertyLayerDescriptor::try_new(
            LayerId(11),
            "score",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            },
            field("score", DataType::Float32),
        )?;
        let sparse = PropertyLayer::try_new_sparse(
            sparse_descriptor,
            3,
            Arc::new(UInt64Array::from(vec![2_u64])),
            Arc::new(Float32Array::from(vec![8.0_f32])),
            Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
        )?;
        let encoded = encode_property_snapshot(&[dense, sparse])?;
        let mut builder = SnapshotBuilder::new();
        builder.add_section(
            SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
            SNAPSHOT_PROPERTY_VERSION,
            0,
            encoded.descriptors,
        )?;
        builder.add_section(
            SNAPSHOT_KIND_PROPERTY_DATA,
            SNAPSHOT_PROPERTY_VERSION,
            0,
            encoded.data,
        )?;
        let bytes = builder.finish()?;
        let snapshot = Snapshot::open(&bytes)?;
        let summary = validate_property_snapshot(&snapshot)?;
        assert_eq!(summary.layer_count, 2);
        assert_eq!(summary.total_logical_values, 5);
        Ok(())
    }

    /// Invalid property data sections are rejected structurally.
    #[test]
    fn property_snapshot_rejects_trailing_data() -> Result<(), PropertyError> {
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(1),
            "count",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            field("count", DataType::Int32),
        )?;
        let dense =
            PropertyLayer::try_new_dense(descriptor, Arc::new(Int32Array::from(vec![1_i32])))?;
        let mut encoded = encode_property_snapshot(&[dense])?;
        encoded.data.push(0);
        assert!(matches!(
            validate_property_sections(&encoded.descriptors, &encoded.data),
            Err(PropertyError::SnapshotDescriptorMismatch { .. })
        ));
        Ok(())
    }

    /// Identity mode sections validate local-equals and explicit map modes.
    #[test]
    fn identity_snapshot_sections_validate() -> Result<(), Box<dyn Error>> {
        let modes = [
            IdentityModeRecord::local_equals_canonical(IdFamily::Element, 3),
            IdentityModeRecord::explicit_u32_map(IdFamily::Relation, 2),
        ];
        let maps = [U32::<LE>::new(10), U32::<LE>::new(12)];
        let mut builder = SnapshotBuilder::new();
        builder.add_section_typed(
            SNAPSHOT_KIND_IDENTITY_MODES,
            SNAPSHOT_PROPERTY_VERSION,
            &modes,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32,
            SNAPSHOT_PROPERTY_VERSION,
            &maps,
        )?;
        let bytes = builder.finish()?;
        let snapshot = Snapshot::open(&bytes)?;
        let summary = validate_identity_snapshot(&snapshot)?;
        assert_eq!(summary.records.len(), 2);
        Ok(())
    }
}

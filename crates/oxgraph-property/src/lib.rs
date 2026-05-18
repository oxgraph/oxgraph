//! Arrow-backed named property layers for `OxGraph` topology views.
//!
//! `oxgraph-property` is a higher layer than topology. It stores named typed
//! Arrow arrays keyed by topology ID family and adapts selected total primitive
//! layers into topology weight capabilities. Foundation crates do not depend on
//! this crate, Arrow, or named properties.
//!
//! # Snapshot section kinds
//!
//! | Constant family | Description |
//! | --------------- | ----------- |
//! | `PROPERTY_DESCRIPTORS_*` | Per-layer descriptor records (header + records + string table) |
//! | `PROPERTY_DATA_*` | Concatenated Arrow IPC value and sparse-default streams |
//!
//! The `_U16` / `_U32` / `_U64` suffix selects the descriptor metadata word
//! width. The payload format is owned by this crate and remains an
//! OxGraph-internal ABI candidate while snapshot v1 bytes are not stable. All
//! section-kind constants are `perf: unspecified` — compile-time `u32` tags.
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

use arrow_array::{Array, ArrayRef, PrimitiveArray, RecordBatch, types::ArrowPrimitiveType};
use arrow_ipc::{reader::StreamReader, writer::StreamWriter};
use arrow_schema::{DataType, Field, Schema};
use arrow_select::take::take;
use oxgraph_snapshot::{SectionViewError, Snapshot};
use oxgraph_topology::{
    ElementIndex, ElementWeight, IncidenceBase, IncidenceIndex, IncidenceWeight, RelationIndex,
    RelationWeight, TopologyBase,
};
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned,
    byteorder::{LE, U16, U32, U64},
};

/// Snapshot section kind reserved for `u16` property-layer descriptors.
pub const SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U16: u32 = 0x0100;
/// Snapshot section kind reserved for `u16` Arrow IPC property-layer payloads.
pub const SNAPSHOT_KIND_PROPERTY_DATA_U16: u32 = 0x0101;
/// Snapshot section kind reserved for `u32` property-layer descriptors.
pub const SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U32: u32 = 0x0102;
/// Snapshot section kind reserved for `u32` Arrow IPC property-layer payloads.
pub const SNAPSHOT_KIND_PROPERTY_DATA_U32: u32 = 0x0103;
/// Snapshot section kind reserved for `u64` property-layer descriptors.
pub const SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U64: u32 = 0x0104;
/// Snapshot section kind reserved for `u64` Arrow IPC property-layer payloads.
pub const SNAPSHOT_KIND_PROPERTY_DATA_U64: u32 = 0x0105;

/// Snapshot section kind for `u16` identity-mode metadata records.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_IDENTITY_MODES_U16: u32 = 0x0110;

/// Snapshot section kind for `u32` identity-mode metadata records.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_IDENTITY_MODES_U32: u32 = 0x0111;

/// Snapshot section kind for `u64` identity-mode metadata records.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_IDENTITY_MODES_U64: u32 = 0x0112;

/// Snapshot section kind for element local-to-canonical `u16` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U16: u32 = 0x0113;

/// Snapshot section kind for element local-to-canonical `u32` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U32: u32 = 0x0114;

/// Snapshot section kind for element local-to-canonical `u64` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U64: u32 = 0x0115;

/// Snapshot section kind for relation local-to-canonical `u16` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U16: u32 = 0x0116;

/// Snapshot section kind for relation local-to-canonical `u32` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32: u32 = 0x0117;

/// Snapshot section kind for relation local-to-canonical `u64` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U64: u32 = 0x0118;

/// Snapshot section kind for incidence local-to-canonical `u16` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U16: u32 = 0x0119;

/// Snapshot section kind for incidence local-to-canonical `u32` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U32: u32 = 0x011A;

/// Snapshot section kind for incidence local-to-canonical `u64` maps.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U64: u32 = 0x011B;

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
pub struct LayerId<Id>(pub Id);

/// Sealed trait modules for property width contracts.
mod sealed {
    /// Seals [`super::PropertyIndex`] to supported unsigned sparse widths.
    pub trait PropertyIndex {}

    /// Seals [`super::PropertySnapshotMetaWord`] to supported metadata widths.
    pub trait PropertySnapshotMetaWord {}

    /// Seals [`super::PropertyAxis`] to the three built-in axis markers.
    pub trait PropertyAxis {}
}

/// Unsigned index width usable for sparse property indexes.
///
/// # Performance
///
/// Implementations perform checked conversions in `O(1)`.
pub trait PropertyIndex: sealed::PropertyIndex + Copy + Ord {
    /// Arrow unsigned primitive type for sparse index arrays.
    type ArrowType: ArrowPrimitiveType<Native = Self> + 'static;

    /// Little-endian word used when this width appears in snapshots.
    type LittleEndianWord: FromBytes + Immutable + IntoBytes + KnownLayout + Unaligned + Copy;

    /// Returns `self` as `usize`, or `None` if the target platform cannot hold it.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_usize(self) -> Option<usize>;

    /// Converts `value` into this index width if it fits.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_usize(value: usize) -> Option<Self>;

    /// Converts `value` into this index width if it fits.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_u64(value: u64) -> Option<Self>;

    /// Returns `self` as `u64` for diagnostics.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_u64(self) -> u64;

    /// Encodes `self` as a little-endian snapshot word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_le_word(self) -> Self::LittleEndianWord;

    /// Decodes a little-endian snapshot word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_le_word(word: Self::LittleEndianWord) -> Self;

    /// Builds an Arrow primitive array from native index values.
    ///
    /// # Performance
    ///
    /// This function is `O(values.len())`.
    fn primitive_array(values: Vec<Self>) -> PrimitiveArray<Self::ArrowType>;
}

/// Metadata/canonical-ID word width for property and identity snapshot sections.
///
/// # Performance
///
/// Implementations perform checked conversions in `O(1)`.
pub trait PropertySnapshotMetaWord: sealed::PropertySnapshotMetaWord + PropertyIndex {
    /// Property descriptor section kind for this metadata width.
    const PROPERTY_DESCRIPTORS_KIND: u32;

    /// Property data section kind for this metadata width.
    const PROPERTY_DATA_KIND: u32;

    /// Identity mode section kind for this metadata width.
    const IDENTITY_MODES_KIND: u32;

    /// Element identity map section kind for this metadata width.
    const ELEMENT_IDENTITY_MAP_KIND: u32;

    /// Relation identity map section kind for this metadata width.
    const RELATION_IDENTITY_MAP_KIND: u32;

    /// Incidence identity map section kind for this metadata width.
    const INCIDENCE_IDENTITY_MAP_KIND: u32;
}

/// Implements property width traits for one unsigned integer.
macro_rules! impl_property_width {
    (
        $index:ty,
        $arrow:ty,
        $word:ty,
        $descriptor_kind:expr,
        $data_kind:expr,
        $identity_kind:expr,
        $element_kind:expr,
        $relation_kind:expr,
        $incidence_kind:expr
    ) => {
        impl sealed::PropertyIndex for $index {}

        impl PropertyIndex for $index {
            type ArrowType = $arrow;
            type LittleEndianWord = $word;

            fn to_usize(self) -> Option<usize> {
                usize::try_from(self).ok()
            }

            fn from_usize(value: usize) -> Option<Self> {
                <$index>::try_from(value).ok()
            }

            fn from_u64(value: u64) -> Option<Self> {
                <$index>::try_from(value).ok()
            }

            fn to_u64(self) -> u64 {
                u64::from(self)
            }

            fn to_le_word(self) -> Self::LittleEndianWord {
                <$word>::new(self)
            }

            fn from_le_word(word: Self::LittleEndianWord) -> Self {
                word.get()
            }

            fn primitive_array(values: Vec<Self>) -> PrimitiveArray<Self::ArrowType> {
                PrimitiveArray::<$arrow>::from(values)
            }
        }

        impl sealed::PropertySnapshotMetaWord for $index {}

        impl PropertySnapshotMetaWord for $index {
            const PROPERTY_DESCRIPTORS_KIND: u32 = $descriptor_kind;
            const PROPERTY_DATA_KIND: u32 = $data_kind;
            const IDENTITY_MODES_KIND: u32 = $identity_kind;
            const ELEMENT_IDENTITY_MAP_KIND: u32 = $element_kind;
            const RELATION_IDENTITY_MAP_KIND: u32 = $relation_kind;
            const INCIDENCE_IDENTITY_MAP_KIND: u32 = $incidence_kind;
        }
    };
}

impl_property_width!(
    u16,
    arrow_array::types::UInt16Type,
    U16<LE>,
    SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U16,
    SNAPSHOT_KIND_PROPERTY_DATA_U16,
    SNAPSHOT_KIND_IDENTITY_MODES_U16,
    SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U16,
    SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U16,
    SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U16
);

impl_property_width!(
    u32,
    arrow_array::types::UInt32Type,
    U32<LE>,
    SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U32,
    SNAPSHOT_KIND_PROPERTY_DATA_U32,
    SNAPSHOT_KIND_IDENTITY_MODES_U32,
    SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U32,
    SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32,
    SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U32
);

impl_property_width!(
    u64,
    arrow_array::types::UInt64Type,
    U64<LE>,
    SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U64,
    SNAPSHOT_KIND_PROPERTY_DATA_U64,
    SNAPSHOT_KIND_IDENTITY_MODES_U64,
    SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U64,
    SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U64,
    SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U64
);

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
pub struct PropertyLayerDescriptor<Id, I>
where
    I: PropertyIndex,
{
    /// Stable layer identifier.
    pub layer_id: LayerId<Id>,
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
    /// Sparse/logical index width selected for this layer.
    index_width: core::marker::PhantomData<I>,
}

impl<Id, I> PropertyLayerDescriptor<Id, I>
where
    I: PropertyIndex,
{
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
        layer_id: LayerId<Id>,
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
            index_width: core::marker::PhantomData,
        })
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
        layer_id: u64,
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
#[non_exhaustive]
pub enum PropertyLayerData<I>
where
    I: PropertyIndex,
{
    /// Dense Arrow array with one slot per ID index.
    Dense {
        /// Dense values.
        values: ArrayRef,
    },
    /// Sparse Arrow array keyed by explicit indexes.
    Sparse {
        /// Strictly ascending sparse indexes.
        indices: Arc<PrimitiveArray<I::ArrowType>>,
        /// Values aligned with `indices`.
        values: ArrayRef,
        /// Optional Arrow scalar default encoded as a length-one array.
        default: Option<ArrayRef>,
    },
}

impl<I> Clone for PropertyLayerData<I>
where
    I: PropertyIndex,
{
    fn clone(&self) -> Self {
        match self {
            Self::Dense { values } => Self::Dense {
                values: Arc::clone(values),
            },
            Self::Sparse {
                indices,
                values,
                default,
            } => Self::Sparse {
                indices: Arc::clone(indices),
                values: Arc::clone(values),
                default: default.clone(),
            },
        }
    }
}

impl<I> fmt::Debug for PropertyLayerData<I>
where
    I: PropertyIndex,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dense { values } => formatter
                .debug_struct("Dense")
                .field("len", &values.len())
                .finish(),
            Self::Sparse {
                indices,
                values,
                default,
            } => formatter
                .debug_struct("Sparse")
                .field("indices", &indices.len())
                .field("values", &values.len())
                .field("has_default", &default.is_some())
                .finish(),
        }
    }
}

/// Arrow-backed property layer.
///
/// # Performance
///
/// Cloning is `O(1)` for Arrow buffers plus descriptor clone cost.
#[derive(Clone, Debug)]
#[must_use]
pub struct PropertyLayer<Id, I>
where
    I: PropertyIndex,
{
    /// Layer descriptor.
    descriptor: PropertyLayerDescriptor<Id, I>,
    /// Logical layer length.
    len: usize,
    /// Layer data.
    data: PropertyLayerData<I>,
}

impl<Id, I> PropertyLayer<Id, I>
where
    I: PropertyIndex,
{
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
        descriptor: PropertyLayerDescriptor<Id, I>,
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
        descriptor: PropertyLayerDescriptor<Id, I>,
        len: usize,
        indices: Arc<PrimitiveArray<I::ArrowType>>,
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
        validate_sparse_indices::<I>(indices.as_ref(), len)?;
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
    pub const fn descriptor(&self) -> &PropertyLayerDescriptor<Id, I> {
        &self.descriptor
    }

    /// Returns this layer's data.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn data(&self) -> &PropertyLayerData<I> {
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

/// Borrowed graph property layers partitioned by topology ID family.
///
/// # Performance
///
/// Copying this struct is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub struct GraphPropertyLayers<'view, Id, NodeIndex, EdgeIndex>
where
    NodeIndex: PropertyIndex,
    EdgeIndex: PropertyIndex,
{
    /// Element/node-keyed property layers.
    pub element: &'view [PropertyLayer<Id, NodeIndex>],
    /// Relation/edge-keyed property layers.
    pub relation: &'view [PropertyLayer<Id, EdgeIndex>],
}

/// Borrowed hypergraph property layers partitioned by topology ID family.
///
/// # Performance
///
/// Copying this struct is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub struct HyperPropertyLayers<'view, Id, VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: PropertyIndex,
    RelationIndex: PropertyIndex,
    IncidenceIndex: PropertyIndex,
{
    /// Element/vertex-keyed property layers.
    pub element: &'view [PropertyLayer<Id, VertexIndex>],
    /// Relation/hyperedge-keyed property layers.
    pub relation: &'view [PropertyLayer<Id, RelationIndex>],
    /// Incidence/participant-keyed property layers.
    pub incidence: &'view [PropertyLayer<Id, IncidenceIndex>],
}

/// Marker trait selecting which axis of a topology view a property layer
/// keys against (elements, relations, or incidences).
///
/// Built-in axis markers — [`ElementAxis`], [`RelationAxis`], [`IncidenceAxis`]
/// — opt into the corresponding [`*Index`] topology trait when paired with
/// [`DenseWeights`] or [`SparseWeights`] storage. The trait itself only
/// reports the layer's [`IdFamily`]; per-axis topology accessors live in
/// inherent impls on each storage type for each axis marker.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata trait.
pub trait PropertyAxis: sealed::PropertyAxis {
    /// Returns the [`IdFamily`] this axis selects from a property layer.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn id_family() -> IdFamily;
}

/// Element-keyed axis marker.
///
/// # Performance
///
/// Copying and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ElementAxis;

impl sealed::PropertyAxis for ElementAxis {}
impl PropertyAxis for ElementAxis {
    fn id_family() -> IdFamily {
        IdFamily::Element
    }
}

/// Relation-keyed axis marker.
///
/// # Performance
///
/// Copying and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RelationAxis;

impl sealed::PropertyAxis for RelationAxis {}
impl PropertyAxis for RelationAxis {
    fn id_family() -> IdFamily {
        IdFamily::Relation
    }
}

/// Incidence-keyed axis marker.
///
/// # Performance
///
/// Copying and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct IncidenceAxis;

impl sealed::PropertyAxis for IncidenceAxis {}
impl PropertyAxis for IncidenceAxis {
    fn id_family() -> IdFamily {
        IdFamily::Incidence
    }
}

/// Axis-aware topology bound accessor.
///
/// Implemented for every topology view that exposes the per-axis index trait
/// `ElementIndex` / `RelationIndex` / `IncidenceIndex`. Exists so that
/// generic constructors on [`DenseWeights`] and [`SparseWeights`] can dispatch
/// to the right `element_bound` / `relation_bound` / `incidence_bound` accessor
/// from a single body, without parallel per-axis impl blocks.
///
/// External code does not normally implement this trait; it is `pub` only
/// because it appears as a bound in `pub` constructor signatures.
///
/// # Performance
///
/// `axis_bound` is `O(1)` — it forwards to the topology's own
/// `*_bound` accessor.
pub trait AxisIndex<A: PropertyAxis>: TopologyBase {
    /// Returns the dense index bound for axis `A` on this topology view.
    ///
    /// # Performance
    ///
    /// `O(1)`.
    fn axis_bound(&self) -> usize;
}

impl<T> AxisIndex<ElementAxis> for T
where
    T: ElementIndex,
{
    fn axis_bound(&self) -> usize {
        self.element_bound()
    }
}

impl<T> AxisIndex<RelationAxis> for T
where
    T: RelationIndex,
{
    fn axis_bound(&self) -> usize {
        self.relation_bound()
    }
}

impl<T> AxisIndex<IncidenceAxis> for T
where
    T: IncidenceIndex,
{
    fn axis_bound(&self) -> usize {
        self.incidence_bound()
    }
}

/// Selected dense primitive weights bound to one axis of a topology view.
///
/// `A` is one of [`ElementAxis`], [`RelationAxis`], or [`IncidenceAxis`];
/// the per-axis `new` constructor selects the right topology bound.
///
/// # Performance
///
/// Weight lookup is `O(1)`.
pub struct DenseWeights<'view, A, T, Id, I, P>
where
    A: PropertyAxis,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Primitive values.
    values: &'view PrimitiveArray<P>,
    /// Property axis, ID, and index marker.
    property: core::marker::PhantomData<(A, Id, I)>,
}

impl<'view, A, T, Id, I, P> DenseWeights<'view, A, T, Id, I, P>
where
    A: PropertyAxis,
    T: AxisIndex<A>,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    /// Selects a dense primitive layer as weights for `topology` along axis
    /// `A` ([`ElementAxis`], [`RelationAxis`], or [`IncidenceAxis`]).
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] if the layer is not `A`-keyed, dense,
    /// primitive type `P`, non-null, or long enough.
    ///
    /// # Performance
    ///
    /// Validation is `O(layer.len())` for the null check.
    pub fn new(
        topology: &'view T,
        layer: &'view PropertyLayer<Id, I>,
    ) -> Result<Self, PropertyError> {
        let values = validate_dense_primitive_selection::<Id, I, P>(
            layer,
            A::id_family(),
            topology.axis_bound(),
        )?;
        Ok(Self {
            topology,
            values,
            property: core::marker::PhantomData,
        })
    }
}

impl<T, Id, I, P> TopologyBase for DenseWeights<'_, ElementAxis, T, Id, I, P>
where
    T: ElementIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T, Id, I, P> ElementWeight for DenseWeights<'_, ElementAxis, T, Id, I, P>
where
    T: ElementIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    type Weight = P::Native;

    fn element_weight(&self, element: Self::ElementId) -> Self::Weight {
        self.values.value(self.topology.element_index(element))
    }
}

impl<T, Id, I, P> TopologyBase for DenseWeights<'_, RelationAxis, T, Id, I, P>
where
    T: RelationIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T, Id, I, P> RelationWeight for DenseWeights<'_, RelationAxis, T, Id, I, P>
where
    T: RelationIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    type Weight = P::Native;

    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight {
        self.values.value(self.topology.relation_index(relation))
    }
}

impl<T, Id, I, P> TopologyBase for DenseWeights<'_, IncidenceAxis, T, Id, I, P>
where
    T: IncidenceIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T, Id, I, P> IncidenceBase for DenseWeights<'_, IncidenceAxis, T, Id, I, P>
where
    T: IncidenceIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type IncidenceId = T::IncidenceId;
    type Role = T::Role;
}

impl<T, Id, I, P> IncidenceWeight for DenseWeights<'_, IncidenceAxis, T, Id, I, P>
where
    T: IncidenceIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    type Weight = P::Native;

    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight {
        self.values.value(self.topology.incidence_index(incidence))
    }
}

/// Selected sparse primitive weights bound to one axis of a topology view.
///
/// `A` is one of [`ElementAxis`], [`RelationAxis`], or [`IncidenceAxis`];
/// the per-axis `new` constructor selects the right topology bound.
///
/// # Performance
///
/// Weight lookup is `O(log k)` for `k` explicitly stored values.
pub struct SparseWeights<'view, A, T, Id, I, P>
where
    A: PropertyAxis,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    /// Topology view that supplies ID-to-index mapping.
    topology: &'view T,
    /// Sparse indexes.
    indices: &'view PrimitiveArray<I::ArrowType>,
    /// Sparse values.
    values: &'view PrimitiveArray<P>,
    /// Totalizing default value.
    default: P::Native,
    /// Property axis and ID marker.
    property: core::marker::PhantomData<(A, Id)>,
}

impl<'view, A, T, Id, I, P> SparseWeights<'view, A, T, Id, I, P>
where
    A: PropertyAxis,
    T: AxisIndex<A>,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    /// Selects a sparse primitive layer as total weights for `topology`
    /// along axis `A` ([`ElementAxis`], [`RelationAxis`], or
    /// [`IncidenceAxis`]).
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when the sparse layer is not total or
    /// type-compatible.
    ///
    /// # Performance
    ///
    /// Validation is `O(1)` plus default downcast.
    pub fn new(
        topology: &'view T,
        layer: &'view PropertyLayer<Id, I>,
    ) -> Result<Self, PropertyError> {
        let (indices, values, default) = validate_sparse_primitive_selection::<I, P, Id>(
            layer,
            A::id_family(),
            topology.axis_bound(),
        )?;
        Ok(Self {
            topology,
            indices,
            values,
            default,
            property: core::marker::PhantomData,
        })
    }
}

impl<T, Id, I, P> TopologyBase for SparseWeights<'_, ElementAxis, T, Id, I, P>
where
    T: ElementIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T, Id, I, P> ElementWeight for SparseWeights<'_, ElementAxis, T, Id, I, P>
where
    T: ElementIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    type Weight = P::Native;

    fn element_weight(&self, element: Self::ElementId) -> Self::Weight {
        sparse_value::<I, P>(
            self.indices,
            self.values,
            self.default,
            self.topology.element_index(element),
        )
    }
}

impl<T, Id, I, P> TopologyBase for SparseWeights<'_, RelationAxis, T, Id, I, P>
where
    T: RelationIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T, Id, I, P> RelationWeight for SparseWeights<'_, RelationAxis, T, Id, I, P>
where
    T: RelationIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    type Weight = P::Native;

    fn relation_weight(&self, relation: Self::RelationId) -> Self::Weight {
        sparse_value::<I, P>(
            self.indices,
            self.values,
            self.default,
            self.topology.relation_index(relation),
        )
    }
}

impl<T, Id, I, P> TopologyBase for SparseWeights<'_, IncidenceAxis, T, Id, I, P>
where
    T: IncidenceIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type ElementId = T::ElementId;
    type RelationId = T::RelationId;
}

impl<T, Id, I, P> IncidenceBase for SparseWeights<'_, IncidenceAxis, T, Id, I, P>
where
    T: IncidenceIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
{
    type IncidenceId = T::IncidenceId;
    type Role = T::Role;
}

impl<T, Id, I, P> IncidenceWeight for SparseWeights<'_, IncidenceAxis, T, Id, I, P>
where
    T: IncidenceIndex,
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    type Weight = P::Native;

    fn incidence_weight(&self, incidence: Self::IncidenceId) -> Self::Weight {
        sparse_value::<I, P>(
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
pub fn validate_unique_names<'descriptor, Id, Index, Descriptors>(
    descriptors: Descriptors,
) -> Result<(), PropertyError>
where
    Id: 'descriptor,
    Index: PropertyIndex + 'descriptor,
    Descriptors: IntoIterator<Item = &'descriptor PropertyLayerDescriptor<Id, Index>>,
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
pub fn validate_unique_layer_ids<'descriptor, Id, Index, Descriptors>(
    descriptors: Descriptors,
) -> Result<(), PropertyError>
where
    Id: Copy + Into<u64> + Ord + 'descriptor,
    Index: PropertyIndex + 'descriptor,
    Descriptors: IntoIterator<Item = &'descriptor PropertyLayerDescriptor<Id, Index>>,
{
    let mut seen: BTreeSet<LayerId<Id>> = BTreeSet::new();
    for descriptor in descriptors {
        if !seen.insert(descriptor.layer_id) {
            return Err(PropertyError::DuplicateLayerId {
                layer_id: descriptor.layer_id.0.into(),
            });
        }
    }
    Ok(())
}

/// Rekeys a property layer from canonical order into snapshot-local order.
///
/// `local_to_canonical[local]` names the canonical index that should appear at
/// `local` in the returned layer.
///
/// # Errors
///
/// Returns [`PropertyError`] if a mapping index is out of bounds, a sparse
/// explicit index is not present in `local_to_canonical`, or Arrow take fails.
///
/// # Performance
///
/// Dense rekeying is `O(local_to_canonical.len())`; sparse rekeying is
/// `O(layer.len() + k log k)` for `k` explicit sparse values.
#[expect(
    clippy::too_many_lines,
    reason = "rekeying keeps dense and sparse Arrow remapping in one contract path"
)]
pub fn rekey_layer_to_local<Id, I>(
    layer: &PropertyLayer<Id, I>,
    local_to_canonical: &[I],
) -> Result<PropertyLayer<Id, I>, PropertyError>
where
    Id: Clone,
    I: PropertyIndex,
{
    let descriptor = layer.descriptor().clone();
    match layer.data() {
        PropertyLayerData::Dense { values } => {
            let take_indices = I::primitive_array(local_to_canonical.to_vec());
            let values = take(values.as_ref(), &take_indices, None).map_err(map_arrow_error)?;
            PropertyLayer::try_new_dense(descriptor, values)
        }
        PropertyLayerData::Sparse {
            indices,
            values,
            default,
        } => {
            let mut canonical_to_local = vec![None; layer.len()];
            for (local, canonical) in local_to_canonical.iter().copied().enumerate() {
                let Some(canonical) = canonical.to_usize() else {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.to_u64(),
                        len: layer.len(),
                    });
                };
                if canonical >= layer.len() {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical as u64,
                        len: layer.len(),
                    });
                }
                canonical_to_local[canonical] = Some(I::from_usize(local).ok_or(
                    PropertyError::SparseIndexOutOfBounds {
                        index: local as u64,
                        len: local_to_canonical.len(),
                    },
                )?);
            }
            let mut remapped = Vec::with_capacity(indices.len());
            for position in 0..indices.len() {
                let canonical = indices.value(position);
                let Some(canonical_usize) = canonical.to_usize() else {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.to_u64(),
                        len: layer.len(),
                    });
                };
                if canonical_usize >= canonical_to_local.len() {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.to_u64(),
                        len: layer.len(),
                    });
                }
                let Some(local) = canonical_to_local[canonical_usize] else {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.to_u64(),
                        len: layer.len(),
                    });
                };
                let take_position =
                    I::from_usize(position).ok_or(PropertyError::SparseIndexOutOfBounds {
                        index: position as u64,
                        len: indices.len(),
                    })?;
                remapped.push((local, take_position));
            }
            remapped.sort_by_key(|(local, _position)| *local);
            let new_indices = I::primitive_array(
                remapped
                    .iter()
                    .map(|(local, _position)| *local)
                    .collect::<Vec<_>>(),
            );
            let take_indices = I::primitive_array(
                remapped
                    .iter()
                    .map(|(_local, position)| *position)
                    .collect::<Vec<_>>(),
            );
            let values = take(values.as_ref(), &take_indices, None).map_err(map_arrow_error)?;
            if let Some(default) = default {
                ensure_arrow_type(&descriptor, default.as_ref())?;
            }
            PropertyLayer::try_new_sparse(
                descriptor,
                local_to_canonical.len(),
                Arc::new(new_indices),
                values,
                default.clone(),
            )
        }
    }
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
    /// The snapshot stores an explicit local-to-canonical map section.
    ExplicitMap,
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
            Self::ExplicitMap => 1,
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
            1 => Some(Self::ExplicitMap),
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
pub struct IdentityModeRecord<W>
where
    W: PropertySnapshotMetaWord,
{
    /// ID-family tag.
    id_family: W::LittleEndianWord,
    /// Map-mode tag.
    mode: W::LittleEndianWord,
    /// Number of local IDs covered by the mode.
    local_len: W::LittleEndianWord,
}

impl<W> IdentityModeRecord<W>
where
    W: PropertySnapshotMetaWord,
{
    /// Builds a local-equals-canonical identity mode record.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when `local_len` cannot be represented by the
    /// selected metadata width.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn local_equals_canonical(
        id_family: IdFamily,
        local_len: usize,
    ) -> Result<Self, PropertyError> {
        Self::new(id_family, IdentityMapMode::LocalEqualsCanonical, local_len)
    }

    /// Builds an explicit-map identity mode record.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when `local_len` cannot be represented by the
    /// selected metadata width.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn explicit_map(id_family: IdFamily, local_len: usize) -> Result<Self, PropertyError> {
        Self::new(id_family, IdentityMapMode::ExplicitMap, local_len)
    }

    /// Builds an identity mode record.
    ///
    /// # Errors
    ///
    /// Returns [`PropertyError`] when `local_len` cannot be represented by the
    /// selected metadata width.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn new(
        id_family: IdFamily,
        mode: IdentityMapMode,
        local_len: usize,
    ) -> Result<Self, PropertyError> {
        Ok(Self {
            id_family: le_word::<W>(id_family_tag(id_family) as usize)?,
            mode: le_word::<W>(mode.tag() as usize)?,
            local_len: le_word::<W>(local_len)?,
        })
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
    pub fn id_family(&self) -> Result<IdFamily, PropertyError> {
        id_family_from_tag(le_word_to_u32::<W>(self.id_family)?)
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
        let tag = le_word_to_u32::<W>(self.mode)?;
        IdentityMapMode::from_tag(tag).ok_or(PropertyError::UnknownIdentityModeTag { tag })
    }

    /// Returns the local ID count covered by this mode.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` on targets where `u64` to `usize` fits; values
    /// above `usize::MAX` saturate to `usize::MAX` for validation errors.
    #[must_use]
    pub fn local_len(&self) -> usize {
        le_word_to_usize::<W>(self.local_len).unwrap_or(usize::MAX)
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
    pub records: Vec<IdentityModeSummary>,
}

/// Decoded identity mode summary.
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityModeSummary {
    /// ID family covered by this record.
    pub id_family: IdFamily,
    /// Identity map mode.
    pub mode: IdentityMapMode,
    /// Number of local IDs covered.
    pub local_len: usize,
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
    Id: Copy + Into<u64> + Ord + TryInto<W>,
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
    Id: Copy + Into<u64> + Ord + TryInto<W>,
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
    Id: Copy + Into<u64> + Ord + TryInto<W>,
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
    validate_sparse_indices_dyn(indexes.as_ref(), le_word_to_usize::<W>(record.logical_len)?)?;
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

/// Converts `value` into a little-endian metadata word.
///
/// # Performance
///
/// This function is `O(1)`.
fn le_word<W>(value: usize) -> Result<W::LittleEndianWord, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let Some(value) = W::from_usize(value) else {
        return Err(PropertyError::SnapshotDescriptorMismatch {
            reason: "value does not fit selected metadata width",
        });
    };
    Ok(value.to_le_word())
}

/// Decodes a little-endian metadata word as `usize`.
///
/// # Performance
///
/// This function is `O(1)`.
fn le_word_to_usize<W>(word: W::LittleEndianWord) -> Result<usize, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    W::from_le_word(word)
        .to_usize()
        .ok_or(PropertyError::SnapshotDescriptorMismatch {
            reason: "metadata word does not fit usize",
        })
}

/// Decodes a little-endian metadata word as `u64`.
///
/// # Performance
///
/// This function is `O(1)`.
fn le_word_to_u64<W>(word: W::LittleEndianWord) -> u64
where
    W: PropertySnapshotMetaWord,
{
    W::from_le_word(word).to_u64()
}

/// Decodes a little-endian metadata word as `u32`.
///
/// # Performance
///
/// This function is `O(1)`.
fn le_word_to_u32<W>(word: W::LittleEndianWord) -> Result<u32, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let value = le_word_to_u64::<W>(word);
    u32::try_from(value).map_err(|_error| PropertyError::SnapshotDescriptorMismatch {
        reason: "metadata word does not fit u32 tag",
    })
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
fn ensure_arrow_type<Id, I>(
    descriptor: &PropertyLayerDescriptor<Id, I>,
    values: &dyn Array,
) -> Result<(), PropertyError>
where
    I: PropertyIndex,
{
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
fn validate_default_policy<Id, I>(
    descriptor: &PropertyLayerDescriptor<Id, I>,
    missing: MissingPolicy,
    default: Option<&ArrayRef>,
) -> Result<(), PropertyError>
where
    I: PropertyIndex,
{
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
fn validate_sparse_indices<I>(
    indices: &PrimitiveArray<I::ArrowType>,
    len: usize,
) -> Result<(), PropertyError>
where
    I: PropertyIndex,
{
    let mut previous = None;
    for position in 0..indices.len() {
        let index = indices.value(position);
        let Some(index_usize) = index.to_usize() else {
            return Err(PropertyError::SparseIndexOutOfBounds {
                index: index.to_u64(),
                len,
            });
        };
        if index_usize >= len {
            return Err(PropertyError::SparseIndexOutOfBounds {
                index: index.to_u64(),
                len,
            });
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

/// Validates sparse index ordering and bounds for a dynamic unsigned Arrow array.
///
/// # Performance
///
/// This function is `O(indices.len())`.
fn validate_sparse_indices_dyn(indices: &dyn Array, len: usize) -> Result<(), PropertyError> {
    if let Some(indices) = indices
        .as_any()
        .downcast_ref::<PrimitiveArray<arrow_array::types::UInt16Type>>()
    {
        return validate_sparse_indices::<u16>(indices, len);
    }
    if let Some(indices) = indices
        .as_any()
        .downcast_ref::<PrimitiveArray<arrow_array::types::UInt32Type>>()
    {
        return validate_sparse_indices::<u32>(indices, len);
    }
    if let Some(indices) = indices
        .as_any()
        .downcast_ref::<PrimitiveArray<arrow_array::types::UInt64Type>>()
    {
        return validate_sparse_indices::<u64>(indices, len);
    }
    Err(PropertyError::SnapshotDescriptorMismatch {
        reason: "sparse property index column is not UInt16, UInt32, or UInt64",
    })
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

/// Validates a dense primitive layer selection.
///
/// # Performance
///
/// This function is `O(layer.len())` for the null check.
fn validate_dense_primitive_selection<Id, I, P>(
    layer: &PropertyLayer<Id, I>,
    expected: IdFamily,
    required: usize,
) -> Result<&PrimitiveArray<P>, PropertyError>
where
    I: PropertyIndex,
    P: ArrowPrimitiveType,
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
type SparsePrimitiveSelection<'layer, I, P> = (
    &'layer PrimitiveArray<<I as PropertyIndex>::ArrowType>,
    &'layer PrimitiveArray<P>,
    <P as ArrowPrimitiveType>::Native,
);

/// Validates a sparse primitive layer selection.
///
/// # Performance
///
/// This function is `O(1)` plus default downcast.
fn validate_sparse_primitive_selection<I, P, Id>(
    layer: &PropertyLayer<Id, I>,
    expected: IdFamily,
    required: usize,
) -> Result<SparsePrimitiveSelection<'_, I, P>, PropertyError>
where
    I: PropertyIndex,
    P: ArrowPrimitiveType,
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
    ensure_no_nulls(primitive)?;
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
fn sparse_value<I, P>(
    indices: &PrimitiveArray<I::ArrowType>,
    values: &PrimitiveArray<P>,
    default: P::Native,
    index: usize,
) -> P::Native
where
    I: PropertyIndex,
    P: ArrowPrimitiveType,
    P::Native: Copy,
{
    let Some(target) = I::from_usize(index) else {
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
mod tests;

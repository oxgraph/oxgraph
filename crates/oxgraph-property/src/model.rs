//! Core property-layer data model, error type, and shared validation helpers.
//!
//! Holds the domain newtypes/enums, the Arrow-backed layer types, the
//! identity-mode snapshot records, the concrete [`PropertyError`] enum, the
//! snapshot tag codecs for those enums, and the layer-construction validation
//! helpers shared across the crate.

use std::{error::Error, fmt, string::String, sync::Arc, vec::Vec};

use arrow_array::{Array, ArrayRef, PrimitiveArray};
use arrow_schema::Field;
use oxgraph_snapshot::SectionViewError;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::width::{
    PropertyIndex, PropertySnapshotMetaWord, le_word, le_word_to_u32, le_word_to_usize,
};

/// Stable numeric identifier for one property layer.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayerId<Id>(pub Id);

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

/// Converts an ID family to its snapshot tag.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) const fn id_family_tag(id_family: IdFamily) -> u32 {
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
pub(crate) const fn id_family_from_tag(tag: u32) -> Result<IdFamily, PropertyError> {
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
pub(crate) const fn layer_role_tag(role: LayerRole) -> u32 {
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
pub(crate) const fn layer_role_from_tag(tag: u32) -> Result<LayerRole, PropertyError> {
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
pub(crate) const fn storage_tag(storage: StorageMode) -> u32 {
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
pub(crate) const fn missing_policy_tag(storage: StorageMode) -> u32 {
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
pub(crate) const fn storage_from_tags(
    storage: u32,
    missing: u32,
) -> Result<StorageMode, PropertyError> {
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
pub(crate) fn ensure_arrow_type<Id, I>(
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
pub(crate) fn ensure_no_nulls(array: &dyn Array) -> Result<(), PropertyError> {
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
pub(crate) fn validate_sparse_indices<I>(
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

/// Converts an Arrow error into a property error.
///
/// # Performance
///
/// This function is `O(error message length)`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Arrow result adapters hand over owned errors and this helper consumes them into messages"
)]
pub(crate) fn map_arrow_error(error: arrow_schema::ArrowError) -> PropertyError {
    PropertyError::Arrow {
        message: error.to_string(),
    }
}

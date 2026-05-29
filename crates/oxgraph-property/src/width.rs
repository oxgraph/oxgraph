//! Property index width contracts, per-axis markers, and metadata-word codecs.
//!
//! Defines the sealed [`PropertyIndex`] / [`PropertySnapshotMetaWord`] width
//! traits, the three built-in [`PropertyAxis`] markers and their [`AxisIndex`]
//! topology-bound dispatch, the snapshot section-kind constants keyed by
//! metadata width, and the little-endian metadata-word conversion helpers.

use std::vec::Vec;

use arrow_array::{PrimitiveArray, types::ArrowPrimitiveType};
use oxgraph_topology::{ElementIndex, IncidenceIndex, RelationIndex, TopologyBase};
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned,
    byteorder::{LE, U16, U32, U64},
};

use crate::model::{IdFamily, PropertyError};

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

/// Converts `value` into a little-endian metadata word.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) fn le_word<W>(value: usize) -> Result<W::LittleEndianWord, PropertyError>
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
pub(crate) fn le_word_to_usize<W>(word: W::LittleEndianWord) -> Result<usize, PropertyError>
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
pub(crate) fn le_word_to_u64<W>(word: W::LittleEndianWord) -> u64
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
pub(crate) fn le_word_to_u32<W>(word: W::LittleEndianWord) -> Result<u32, PropertyError>
where
    W: PropertySnapshotMetaWord,
{
    let value = le_word_to_u64::<W>(word);
    u32::try_from(value).map_err(|_error| PropertyError::SnapshotDescriptorMismatch {
        reason: "metadata word does not fit u32 tag",
    })
}

//! Arrow-backed named property layers for `OxGraph` topology views.
//!
//! `oxgraph-property` is a higher layer than topology. It stores named typed
//! Arrow arrays keyed by topology ID family and adapts selected total primitive
//! layers into topology weight capabilities. Foundation crates do not depend on
//! this crate, Arrow, or named properties.
//!
//! # Module layout
//!
//! The crate is split into focused modules and re-exports its full public API
//! from this facade:
//!
//! | Module | Responsibility |
//! | ------ | -------------- |
//! | [`width`] | Index/metadata width contracts, axis markers, section-kind constants |
//! | [`model`] | Layer data model, error type, identity records |
//! | [`weights`] | Dense/sparse topology weight adapters and layer partitions |
//! | [`rekey`] | Descriptor uniqueness checks and canonical-to-local rekeying |
//! | [`snapshot`] | Snapshot encode/validate/decode and wire records |
//!
//! # Snapshot section kinds
//!
//! | Constant family | Base | Description |
//! | --------------- | ---- | ----------- |
//! | `PROPERTY_DESCRIPTORS` | [`SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_BASE`] | Per-layer descriptor records (header + records + string table) |
//! | `PROPERTY_DATA` | [`SNAPSHOT_KIND_PROPERTY_DATA_BASE`] | Concatenated Arrow IPC value and sparse-default streams |
//!
//! Each persisted kind is `BASE | WIDTH_CODE`, where the two-bit width code
//! (`0b00` = `u16`, `0b01` = `u32`, `0b10` = `u64`) selects the descriptor
//! metadata word width; [`PropertySnapshotMetaWord`] derives the per-width
//! kinds. The payload format is owned by this crate and remains an
//! OxGraph-internal ABI candidate while the snapshot bytes are not stable. All
//! section-kind constants are `perf: unspecified` — compile-time `u32` tags.
// kani-skip: property layers depend on Arrow heap arrays and snapshot byte streams outside Kani's
// bounded no-std proof scope.

pub mod export;
pub mod model;
pub mod rekey;
pub mod snapshot;
pub mod weights;
pub mod width;

pub use model::{
    IdFamily, IdentityMapMode, IdentityModeRecord, IdentityModeSummary, IdentitySnapshotSummary,
    LayerId, LayerName, LayerRole, MissingPolicy, PropertyError, PropertyLayer, PropertyLayerData,
    PropertyLayerDescriptor, StorageMode,
};
pub use rekey::{rekey_layer_to_local, validate_unique_layer_ids, validate_unique_names};
pub use snapshot::{
    DecodedPropertyData, DecodedPropertyLayer, EncodedPropertySnapshot, PropertySnapshotRecord,
    PropertySnapshotSummary, encode_graph_property_snapshot, encode_hyper_property_snapshot,
    encode_property_snapshot, validate_identity_snapshot, validate_property_sections,
    validate_property_snapshot,
};
pub use weights::{DenseWeights, GraphPropertyLayers, HyperPropertyLayers, SparseWeights};
pub use width::{
    AxisIndex, ElementAxis, IncidenceAxis, PropertyAxis, PropertyIndex, PropertySnapshotMetaWord,
    RelationAxis, SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_BASE, SNAPSHOT_KIND_IDENTITY_MODES_BASE,
    SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_BASE, SNAPSHOT_KIND_PROPERTY_DATA_BASE,
    SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_BASE, SNAPSHOT_KIND_RELATION_IDENTITY_MAP_BASE,
    SNAPSHOT_PROPERTY_VERSION,
};

#[cfg(test)]
mod tests;

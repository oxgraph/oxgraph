//! Shared snapshot-export helpers for topology builders that attach property
//! layers.
//!
//! Every layout exporter (CSR, BCSR) validates its property layers against the
//! topology counts and then appends the SAME section tail — identity modes,
//! one explicit identity map, property descriptors, property data — in the
//! canonical order. The layout-specific parts (which counts are required,
//! which identity modes apply, which map kind carries the explicit map) stay
//! in each layout crate; the mechanics live here once.

use oxgraph_layout_util::SnapshotWidth;
use oxgraph_snapshot::{PlanError, SnapshotBuilder};

use crate::{
    EncodedPropertySnapshot, IdFamily, IdentityModeRecord, PropertyIndex, PropertyLayer,
    PropertySnapshotMetaWord, SNAPSHOT_PROPERTY_VERSION,
};

/// A property layer shorter than the topology requires.
///
/// Carries the family, the required minimum row count, and the offending
/// layer's length; layout crates map this into their typed
/// `PropertyLayerTooShort` build-error variant.
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerLengthError {
    /// Family whose layer fell short.
    pub id_family: IdFamily,
    /// Minimum row count the topology requires.
    pub required: usize,
    /// The offending layer's actual length.
    pub actual: usize,
}

impl core::fmt::Display for LayerLengthError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "{:?} property layer holds {} rows but the topology requires {}",
            self.id_family, self.actual, self.required
        )
    }
}

impl core::error::Error for LayerLengthError {}

/// Validates that every layer holds at least `required` rows for `id_family`.
///
/// # Errors
///
/// Returns [`LayerLengthError`] naming the first layer shorter than
/// `required`.
///
/// # Performance
///
/// This function is `O(layers.len())`.
pub fn validate_layer_lengths<Id, I>(
    layers: &[PropertyLayer<Id, I>],
    id_family: IdFamily,
    required: usize,
) -> Result<(), LayerLengthError>
where
    I: PropertyIndex,
{
    for layer in layers {
        if layer.len() < required {
            return Err(LayerLengthError {
                id_family,
                required,
                actual: layer.len(),
            });
        }
    }
    Ok(())
}

/// Appends the canonical property-export section tail: identity modes, the
/// explicit identity map under `identity_map_kind`, then the encoded property
/// descriptors and data.
///
/// # Errors
///
/// Returns [`PlanError`] when section planning rejects a section (duplicate
/// kind, count, or alignment).
///
/// # Performance
///
/// This function is `O(identity map len + encoded property bytes)`.
pub fn append_identity_and_property_sections<W, MapW>(
    builder: &mut SnapshotBuilder,
    identity_modes: &[IdentityModeRecord<W>],
    identity_map_kind: u32,
    identity_map: &[MapW],
    encoded: EncodedPropertySnapshot,
) -> Result<(), PlanError>
where
    W: PropertySnapshotMetaWord,
    MapW: SnapshotWidth,
{
    builder.add_section_little_endian(
        W::IDENTITY_MODES_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        identity_modes,
    )?;
    builder.add_section_widths(identity_map_kind, SNAPSHOT_PROPERTY_VERSION, identity_map)?;
    builder.add_section(
        W::PROPERTY_DESCRIPTORS_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        encoded.descriptors,
    )?;
    builder.add_section(
        W::PROPERTY_DATA_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        encoded.data,
    )?;
    Ok(())
}

//! Descriptor-set uniqueness checks and canonical-to-local layer rekeying.
//!
//! Defines [`validate_unique_names`] / [`validate_unique_layer_ids`] and
//! [`rekey_layer_to_local`], which remaps a property layer from canonical order
//! into snapshot-local order for both dense and sparse storage.

use std::{collections::BTreeSet, sync::Arc, vec, vec::Vec};

use arrow_select::take::take;

use crate::{
    model::{
        IdFamily, LayerId, PropertyError, PropertyLayer, PropertyLayerData,
        PropertyLayerDescriptor, ensure_arrow_type, map_arrow_error,
    },
    width::PropertyIndex,
};

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

    // Validate every mapping entry up front, for both storage modes, so the
    // dense arm enforces the same structured contract as the sparse arm rather
    // than relying on Arrow `take` to surface a generic out-of-range error.
    // `rekey` is a remap into snapshot-local order: every canonical index named
    // by `local_to_canonical` must be a valid index into the source layer.
    let layer_len = layer.len();
    for canonical in local_to_canonical.iter().copied() {
        let in_range = canonical.to_usize().is_some_and(|index| index < layer_len);
        if !in_range {
            return Err(PropertyError::SparseIndexOutOfBounds {
                index: canonical.into(),
                len: layer_len,
            });
        }
    }

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
                let Some(canonical_usize) = canonical.to_usize() else {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.into(),
                        len: layer.len(),
                    });
                };
                if canonical_usize >= layer.len() {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.into(),
                        len: layer.len(),
                    });
                }
                canonical_to_local[canonical_usize] = Some(I::from_usize(local).ok_or(
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
                        index: canonical.into(),
                        len: layer.len(),
                    });
                };
                if canonical_usize >= canonical_to_local.len() {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.into(),
                        len: layer.len(),
                    });
                }
                let Some(local) = canonical_to_local[canonical_usize] else {
                    return Err(PropertyError::SparseIndexOutOfBounds {
                        index: canonical.into(),
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

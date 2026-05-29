//! Topology weight adapters over selected primitive property layers.
//!
//! Defines the borrowed [`GraphPropertyLayers`] / [`HyperPropertyLayers`]
//! partitions, the [`DenseWeights`] / [`SparseWeights`] adapters that opt a
//! selected primitive layer into the per-axis topology weight traits, and the
//! selection-validation helpers backing their constructors.

use arrow_array::{Array, PrimitiveArray, types::ArrowPrimitiveType};
use oxgraph_topology::{
    ElementIndex, ElementWeight, IncidenceBase, IncidenceIndex, IncidenceWeight, RelationIndex,
    RelationWeight, TopologyBase,
};

use crate::{
    model::{IdFamily, PropertyError, PropertyLayer, PropertyLayerData, ensure_no_nulls},
    width::{AxisIndex, ElementAxis, IncidenceAxis, PropertyAxis, PropertyIndex, RelationAxis},
};

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

/// Stamps the `TopologyBase` impl shared by every axis of a weight-storage type.
///
/// Each `(storage, axis)` pair forwards the same associated `ElementId` /
/// `RelationId` types from the underlying topology view; this keeps that one
/// body in a single place instead of six hand-written copies.
macro_rules! impl_axis_topology_base {
    ($storage:ident, $axis:ty, $index_trait:ident) => {
        impl<T, Id, I, P> TopologyBase for $storage<'_, $axis, T, Id, I, P>
        where
            T: $index_trait,
            I: PropertyIndex,
            P: ArrowPrimitiveType,
        {
            type ElementId = T::ElementId;
            type RelationId = T::RelationId;
        }
    };
}

/// Stamps the per-axis topology weight impls for [`DenseWeights`].
///
/// Each axis (`Element` / `Relation` / `Incidence`) opts dense storage into the
/// matching topology weight trait. The three axis weight traits expose distinct
/// method names (`element_weight` / `relation_weight` / `incidence_weight`) and
/// parameter types, so a single blanket `impl` over the axis marker is not
/// expressible; this macro keeps the one dense lookup body — read the value
/// slot at the topology's dense index for the axis ID — and stamps it per axis
/// instead of hand-writing three near-identical blocks.
macro_rules! impl_dense_axis_weight {
    (
        $axis:ty, $index_trait:ident, $id_ty:ident, $index_fn:ident,
        $weight_trait:ident, $weight_fn:ident
    ) => {
        impl<T, Id, I, P> $weight_trait for DenseWeights<'_, $axis, T, Id, I, P>
        where
            T: $index_trait,
            I: PropertyIndex,
            P: ArrowPrimitiveType,
            P::Native: Copy,
        {
            type Weight = P::Native;

            fn $weight_fn(&self, id: Self::$id_ty) -> Self::Weight {
                self.values.value(self.topology.$index_fn(id))
            }
        }
    };
}

/// Stamps the per-axis topology weight impls for [`SparseWeights`].
///
/// Mirrors [`impl_dense_axis_weight`] for sparse storage: the one sparse lookup
/// body — binary-search the explicit indexes via [`sparse_value`], falling back
/// to the totalizing default — is kept here and stamped per axis rather than
/// repeated across three near-identical blocks.
macro_rules! impl_sparse_axis_weight {
    (
        $axis:ty, $index_trait:ident, $id_ty:ident, $index_fn:ident,
        $weight_trait:ident, $weight_fn:ident
    ) => {
        impl<T, Id, I, P> $weight_trait for SparseWeights<'_, $axis, T, Id, I, P>
        where
            T: $index_trait,
            I: PropertyIndex,
            P: ArrowPrimitiveType,
            P::Native: Copy,
        {
            type Weight = P::Native;

            fn $weight_fn(&self, id: Self::$id_ty) -> Self::Weight {
                sparse_value::<I, P>(
                    self.indices,
                    self.values,
                    self.default,
                    self.topology.$index_fn(id),
                )
            }
        }
    };
}

/// Stamps the `IncidenceBase` impl shared by the incidence axis of a storage type.
///
/// Only the incidence axis carries incidence-specific associated types; this
/// keeps that one forwarding body in a single place for both storage types.
macro_rules! impl_axis_incidence_base {
    ($storage:ident) => {
        impl<T, Id, I, P> IncidenceBase for $storage<'_, IncidenceAxis, T, Id, I, P>
        where
            T: IncidenceIndex,
            I: PropertyIndex,
            P: ArrowPrimitiveType,
        {
            type IncidenceId = T::IncidenceId;
            type Role = T::Role;
        }
    };
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

impl_axis_topology_base!(DenseWeights, ElementAxis, ElementIndex);
impl_axis_topology_base!(DenseWeights, RelationAxis, RelationIndex);
impl_axis_topology_base!(DenseWeights, IncidenceAxis, IncidenceIndex);
impl_axis_incidence_base!(DenseWeights);

impl_dense_axis_weight!(
    ElementAxis,
    ElementIndex,
    ElementId,
    element_index,
    ElementWeight,
    element_weight
);
impl_dense_axis_weight!(
    RelationAxis,
    RelationIndex,
    RelationId,
    relation_index,
    RelationWeight,
    relation_weight
);
impl_dense_axis_weight!(
    IncidenceAxis,
    IncidenceIndex,
    IncidenceId,
    incidence_index,
    IncidenceWeight,
    incidence_weight
);

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

impl_axis_topology_base!(SparseWeights, ElementAxis, ElementIndex);
impl_axis_topology_base!(SparseWeights, RelationAxis, RelationIndex);
impl_axis_topology_base!(SparseWeights, IncidenceAxis, IncidenceIndex);
impl_axis_incidence_base!(SparseWeights);

impl_sparse_axis_weight!(
    ElementAxis,
    ElementIndex,
    ElementId,
    element_index,
    ElementWeight,
    element_weight
);
impl_sparse_axis_weight!(
    RelationAxis,
    RelationIndex,
    RelationId,
    relation_index,
    RelationWeight,
    relation_weight
);
impl_sparse_axis_weight!(
    IncidenceAxis,
    IncidenceIndex,
    IncidenceId,
    incidence_index,
    IncidenceWeight,
    incidence_weight
);

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
    if layer.descriptor().id_family != expected {
        return Err(PropertyError::IdFamilyMismatch {
            expected,
            actual: layer.descriptor().id_family,
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
            name: layer.descriptor().name.clone(),
        });
    };
    let primitive = values
        .as_any()
        .downcast_ref::<PrimitiveArray<P>>()
        .ok_or_else(|| PropertyError::ArrowTypeMismatch {
            name: layer.descriptor().name.clone(),
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
    if layer.descriptor().id_family != expected {
        return Err(PropertyError::IdFamilyMismatch {
            expected,
            actual: layer.descriptor().id_family,
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
            name: layer.descriptor().name.clone(),
        });
    };
    let Some(default_array) = default else {
        return Err(PropertyError::SparseNullMissingNotTotal {
            name: layer.descriptor().name.clone(),
        });
    };
    let primitive = values
        .as_any()
        .downcast_ref::<PrimitiveArray<P>>()
        .ok_or_else(|| PropertyError::ArrowTypeMismatch {
            name: layer.descriptor().name.clone(),
        })?;
    ensure_no_nulls(primitive)?;
    let default_primitive = default_array
        .as_any()
        .downcast_ref::<PrimitiveArray<P>>()
        .ok_or_else(|| PropertyError::ArrowTypeMismatch {
            name: layer.descriptor().name.clone(),
        })?;
    if default_primitive.len() != 1 || default_primitive.is_null(0) {
        return Err(PropertyError::DefaultPolicyMismatch {
            name: layer.descriptor().name.clone(),
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

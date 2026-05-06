//! `PyO3` bindings for `OxGraph` builders, frozen views, properties, snapshots, BFS, and
//! `PageRank`.
//!
//! The native module is packaged as `oxgraph._oxgraph`; the Python package
//! re-exports it as `oxgraph`. Python objects own their frozen Rust views so
//! builder edits do not create stale borrowed views.
#![expect(
    missing_docs,
    reason = "PyO3 create_exception macro emits undocumented Python exception structs"
)]
#![expect(
    clippy::missing_const_for_fn,
    reason = "PyO3 pymethods and Python-callable helpers are kept as regular functions"
)]
#![expect(
    clippy::needless_pass_by_value,
    reason = "PyO3 owns converted Python inputs such as Vec and error mapping consumes Rust errors"
)]
#![expect(
    clippy::too_many_arguments,
    reason = "Python facade methods mirror keyword-rich Python APIs"
)]

use std::{collections::BTreeMap, fmt, sync::Arc, vec::Vec};

use arrow_array::{Array, Float64Array, UInt64Array, types::Float64Type};
use arrow_schema::{DataType, Field};
use oxgraph_algo::{
    PageRankConfig, PageRankError, breadth_first_search, hypergraph_pagerank,
    hypergraph_pagerank_weighted, pagerank, pagerank_weighted,
};
use oxgraph_csr::CsrSnapshotGraph;
use oxgraph_graph::{GraphCounts, TopologyCounts};
use oxgraph_graph_build::{FrozenGraph, GraphBuildError, GraphBuilder, GraphEdgeId, GraphNodeId};
use oxgraph_hyper::HypergraphCounts;
use oxgraph_hyper_bcsr::{BcsrHypergraph, BcsrValidation};
use oxgraph_hyper_build::{
    FrozenHypergraph, HyperBuildError, HyperParticipantId, HyperVertexId, HyperedgeId,
    HypergraphBuilder,
};
use oxgraph_property::{
    DenseIncidenceWeights, DenseRelationWeights, IdFamily, LayerId, LayerRole, MissingPolicy,
    PropertyError, PropertyLayer, PropertyLayerData, PropertyLayerDescriptor,
    SNAPSHOT_KIND_IDENTITY_MODES, SNAPSHOT_KIND_PROPERTY_DATA, SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
    SparseIncidenceWeights, SparseRelationWeights, StorageMode, validate_identity_snapshot,
    validate_property_snapshot,
};
use oxgraph_snapshot::Snapshot;
use oxgraph_topology::{
    CanonicalElementIdentity, CanonicalRelationIdentity, ContainsElement, ContainsIncidence,
    ContainsRelation, ElementWeight, IncidenceWeight, LocalElementIdentity, LocalRelationIdentity,
    RelationWeight,
};
use pyo3::{create_exception, exceptions::PyValueError, prelude::*};

// Base Python exception for OxGraph binding errors.
create_exception!(_oxgraph, OxGraphError, PyValueError);
// Python exception for graph builder or graph view errors.
create_exception!(_oxgraph, GraphError, OxGraphError);
// Python exception for hypergraph builder or hypergraph view errors.
create_exception!(_oxgraph, HypergraphError, OxGraphError);
// Python exception for PageRank configuration, input, or convergence errors.
create_exception!(_oxgraph, PageRankPythonError, OxGraphError);
// Python exception for snapshot open or validation errors.
create_exception!(_oxgraph, SnapshotPythonError, OxGraphError);
// Python exception for property descriptor, layer, or selection errors.
create_exception!(_oxgraph, PropertyPythonError, OxGraphError);

/// Python boundary graph builder specialization over `CPython` float weights.
type PythonGraphBuilder = GraphBuilder<f64, f64>;
/// Python boundary frozen graph specialization over `CPython` float weights.
type PythonFrozenGraph = FrozenGraph<f64, f64>;
/// Python boundary hypergraph builder specialization over `CPython` float weights.
type PythonHypergraphBuilder = HypergraphBuilder<f64, f64, f64>;
/// Python boundary frozen hypergraph specialization over `CPython` float weights.
type PythonFrozenHypergraph = FrozenHypergraph<f64, f64, f64>;

/// Python wrapper around [`GraphBuilder`].
#[pyclass(name = "GraphBuilder")]
struct PyGraphBuilder {
    /// Rust graph builder.
    inner: PythonGraphBuilder,
    /// Python label to canonical node ID map.
    labels: BTreeMap<String, u32>,
}

impl Default for PyGraphBuilder {
    fn default() -> Self {
        Self {
            inner: GraphBuilder::new(0.0, 1.0),
            labels: BTreeMap::new(),
        }
    }
}

#[pymethods]
impl PyGraphBuilder {
    /// Constructs an empty graph builder.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Adds an isolated node and optionally records a Python label.
    #[pyo3(signature = (label=None))]
    fn add_node(&mut self, label: Option<String>) -> PyResult<u32> {
        if let Some(name) = label.as_ref()
            && self.labels.contains_key(name)
        {
            return Err(py_graph_message(format_args!("duplicate label '{name}'")));
        }
        let node = self.inner.add_node().map_err(py_graph_error)?;
        if let Some(name) = label {
            insert_label(&mut self.labels, name, node.0)?;
        }
        Ok(node.0)
    }

    /// Adds a directed edge, optionally setting its relation weight.
    #[pyo3(signature = (source, target, weight=None))]
    fn add_edge(&mut self, source: u32, target: u32, weight: Option<f64>) -> PyResult<u32> {
        let edge = self
            .inner
            .add_edge(GraphNodeId(source), GraphNodeId(target))
            .map_err(py_graph_error)?;
        if let Some(value) = weight {
            self.inner
                .set_relation_weight(edge, value)
                .map_err(py_graph_error)?;
        }
        Ok(edge.0)
    }

    /// Sets an existing node's element weight.
    fn set_element_weight(&mut self, node: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_element_weight(GraphNodeId(node), weight)
            .map_err(py_graph_error)
    }

    /// Sets an existing edge's relation weight.
    fn set_relation_weight(&mut self, edge: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_relation_weight(GraphEdgeId(edge), weight)
            .map_err(py_graph_error)
    }

    /// Returns the node ID for a Python label, if present.
    fn node_for_label(&self, label: &str) -> Option<u32> {
        self.labels.get(label).copied()
    }

    /// Returns the number of nodes added so far.
    fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Returns the number of edges added so far.
    fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    /// Freezes the builder into an owned immutable graph view.
    fn freeze(&self) -> PyResult<PyFrozenGraph> {
        Ok(PyFrozenGraph {
            inner: self.inner.freeze().map_err(py_graph_error)?,
            labels: self.labels.clone(),
        })
    }
}

/// Python wrapper around an owned frozen graph.
#[pyclass(name = "FrozenGraph")]
struct PyFrozenGraph {
    /// Rust frozen graph.
    inner: PythonFrozenGraph,
    /// Python label to canonical node ID map carried from the builder.
    labels: BTreeMap<String, u32>,
}

#[pymethods]
impl PyFrozenGraph {
    /// Returns the number of nodes in the frozen graph.
    fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Returns the number of edges in the frozen graph.
    fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    /// Returns the element weight for a node.
    fn element_weight(&self, node: u32) -> PyResult<f64> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self.inner.element_weight(GraphNodeId(node)))
    }

    /// Returns the relation weight for an edge.
    fn relation_weight(&self, edge: u32) -> PyResult<f64> {
        ensure_graph_edge(&self.inner, edge)?;
        Ok(self.inner.relation_weight(GraphEdgeId(edge)))
    }

    /// Returns the canonical node ID for a local node ID.
    fn canonical_node_id(&self, node: u32) -> PyResult<u32> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self.inner.canonical_element_id(GraphNodeId(node)).0)
    }

    /// Returns the local node ID for a canonical node ID, if visible.
    fn local_node_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_element_id(GraphNodeId(canonical))
            .map(|id| id.0)
    }

    /// Returns the canonical edge ID for a local edge ID.
    fn canonical_edge_id(&self, edge: u32) -> PyResult<u32> {
        ensure_graph_edge(&self.inner, edge)?;
        Ok(self.inner.canonical_relation_id(GraphEdgeId(edge)).0)
    }

    /// Returns the local edge ID for a canonical edge ID, if visible.
    fn local_edge_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_relation_id(GraphEdgeId(canonical))
            .map(|id| id.0)
    }

    /// Returns the node ID for a Python label, if present.
    fn node_for_label(&self, label: &str) -> Option<u32> {
        self.labels.get(label).copied()
    }

    /// Returns the first Python label for a node, if present.
    fn label_for_node(&self, node: u32) -> PyResult<Option<String>> {
        ensure_graph_node(&self.inner, node)?;
        Ok(label_for_id(&self.labels, node))
    }

    /// Runs BFS from `start` and returns visited node IDs in traversal order.
    fn bfs(&self, start: u32) -> PyResult<Vec<u32>> {
        ensure_graph_node(&self.inner, start)?;
        let traversal = breadth_first_search(&self.inner, GraphNodeId(start))
            .map_err(|error| GraphError::new_err(error.to_string()))?;
        Ok(traversal.map(|node| node.0).collect())
    }

    /// Runs unweighted `PageRank` over all nodes.
    #[pyo3(signature = (damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn pagerank(
        &self,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>> {
        let config = pagerank_config(damping, tolerance, max_iterations);
        let elements = graph_elements(&self.inner);
        let mut ranks = vec![0.0; self.inner.element_count()];
        pagerank(
            &self.inner,
            elements,
            config,
            personalization.as_deref(),
            &mut ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok(ranks)
    }

    /// Runs relation-weighted `PageRank` over all nodes using frozen relation weights.
    #[pyo3(signature = (damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn weighted_pagerank(
        &self,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>> {
        let config = pagerank_config(damping, tolerance, max_iterations);
        let elements = graph_elements(&self.inner);
        let mut ranks = vec![0.0; self.inner.element_count()];
        pagerank_weighted(
            &self.inner,
            &self.inner,
            elements,
            config,
            personalization.as_deref(),
            &mut ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok(ranks)
    }

    /// Runs relation-weighted `PageRank` using a dense property layer.
    #[pyo3(signature = (layer, damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn pagerank_with_dense_relation_weights(
        &self,
        layer: &PyDenseF64Layer,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>> {
        let selected = DenseRelationWeights::<_, Float64Type>::new(&self.inner, &layer.inner)
            .map_err(py_property_error)?;
        self.pagerank_with_relation_weights(
            &selected,
            damping,
            tolerance,
            max_iterations,
            personalization,
        )
    }

    /// Runs relation-weighted `PageRank` using a sparse totalizing property layer.
    #[pyo3(signature = (layer, damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn pagerank_with_sparse_relation_weights(
        &self,
        layer: &PySparseF64Layer,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>> {
        let selected = SparseRelationWeights::<_, Float64Type>::new(&self.inner, &layer.inner)
            .map_err(py_property_error)?;
        self.pagerank_with_relation_weights(
            &selected,
            damping,
            tolerance,
            max_iterations,
            personalization,
        )
    }

    /// Serializes CSR, identity, and property sections into an `OxGraph` snapshot byte vector.
    fn to_csr_snapshot(&self) -> PyResult<Vec<u8>> {
        self.inner.to_csr_snapshot().map_err(py_graph_error)
    }
}

impl PyFrozenGraph {
    /// Runs weighted `PageRank` with an arbitrary selected relation-weight view.
    fn pagerank_with_relation_weights<W>(
        &self,
        weights: &W,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>>
    where
        W: RelationWeight<ElementId = GraphNodeId, RelationId = GraphEdgeId, Weight = f64>,
    {
        let config = pagerank_config(damping, tolerance, max_iterations);
        let elements = graph_elements(&self.inner);
        let mut ranks = vec![0.0; self.inner.element_count()];
        pagerank_weighted(
            &self.inner,
            weights,
            elements,
            config,
            personalization.as_deref(),
            &mut ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok(ranks)
    }
}

/// Python wrapper around [`HypergraphBuilder`].
#[pyclass(name = "HypergraphBuilder")]
struct PyHypergraphBuilder {
    /// Rust hypergraph builder.
    inner: PythonHypergraphBuilder,
    /// Python label to canonical vertex ID map.
    labels: BTreeMap<String, u32>,
}

impl Default for PyHypergraphBuilder {
    fn default() -> Self {
        Self {
            inner: HypergraphBuilder::new(0.0, 1.0, 1.0),
            labels: BTreeMap::new(),
        }
    }
}

#[pymethods]
impl PyHypergraphBuilder {
    /// Constructs an empty hypergraph builder.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Adds an isolated vertex and optionally records a Python label.
    #[pyo3(signature = (label=None))]
    fn add_vertex(&mut self, label: Option<String>) -> PyResult<u32> {
        if let Some(name) = label.as_ref()
            && self.labels.contains_key(name)
        {
            return Err(py_hyper_message(format_args!("duplicate label '{name}'")));
        }
        let vertex = self.inner.add_vertex().map_err(py_hyper_error)?;
        if let Some(name) = label {
            insert_hyper_label(&mut self.labels, name, vertex.0)?;
        }
        Ok(vertex.0)
    }

    /// Adds a directed hyperedge from source vertices to target vertices.
    #[pyo3(signature = (sources, targets, weight=None))]
    fn add_hyperedge(
        &mut self,
        sources: Vec<u32>,
        targets: Vec<u32>,
        weight: Option<f64>,
    ) -> PyResult<u32> {
        let source_ids: Vec<HyperVertexId> = sources.into_iter().map(HyperVertexId).collect();
        let target_ids: Vec<HyperVertexId> = targets.into_iter().map(HyperVertexId).collect();
        let hyperedge = self
            .inner
            .add_hyperedge(&source_ids, &target_ids)
            .map_err(py_hyper_error)?;
        if let Some(value) = weight {
            self.inner
                .set_relation_weight(hyperedge, value)
                .map_err(py_hyper_error)?;
        }
        Ok(hyperedge.0)
    }

    /// Sets a vertex element weight.
    fn set_element_weight(&mut self, vertex: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_element_weight(HyperVertexId(vertex), weight)
            .map_err(py_hyper_error)
    }

    /// Sets a hyperedge relation weight.
    fn set_relation_weight(&mut self, hyperedge: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_relation_weight(HyperedgeId(hyperedge), weight)
            .map_err(py_hyper_error)
    }

    /// Sets a source-side incidence weight by hyperedge-local source position.
    fn set_source_incidence_weight(
        &mut self,
        hyperedge: u32,
        position: usize,
        weight: f64,
    ) -> PyResult<()> {
        self.inner
            .set_source_incidence_weight(HyperedgeId(hyperedge), position, weight)
            .map_err(py_hyper_error)
    }

    /// Sets a target-side incidence weight by hyperedge-local target position.
    fn set_target_incidence_weight(
        &mut self,
        hyperedge: u32,
        position: usize,
        weight: f64,
    ) -> PyResult<()> {
        self.inner
            .set_target_incidence_weight(HyperedgeId(hyperedge), position, weight)
            .map_err(py_hyper_error)
    }

    /// Returns the vertex ID for a Python label, if present.
    fn vertex_for_label(&self, label: &str) -> Option<u32> {
        self.labels.get(label).copied()
    }

    /// Returns the number of vertices added so far.
    fn vertex_count(&self) -> usize {
        self.inner.vertex_count()
    }

    /// Returns the number of hyperedges added so far.
    fn hyperedge_count(&self) -> usize {
        self.inner.hyperedge_count()
    }

    /// Freezes the builder into an owned immutable hypergraph view.
    fn freeze(&self) -> PyResult<PyFrozenHypergraph> {
        Ok(PyFrozenHypergraph {
            inner: self.inner.freeze().map_err(py_hyper_error)?,
            labels: self.labels.clone(),
        })
    }
}

/// Python wrapper around an owned frozen hypergraph.
#[pyclass(name = "FrozenHypergraph")]
struct PyFrozenHypergraph {
    /// Rust frozen hypergraph.
    inner: PythonFrozenHypergraph,
    /// Python label to canonical vertex ID map carried from the builder.
    labels: BTreeMap<String, u32>,
}

#[pymethods]
impl PyFrozenHypergraph {
    /// Returns the number of vertices in the frozen hypergraph.
    fn vertex_count(&self) -> usize {
        self.inner.vertex_count()
    }

    /// Returns the number of hyperedges in the frozen hypergraph.
    fn hyperedge_count(&self) -> usize {
        self.inner.hyperedge_count()
    }

    /// Returns an element weight for a vertex.
    fn element_weight(&self, vertex: u32) -> PyResult<f64> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self.inner.element_weight(HyperVertexId(vertex)))
    }

    /// Returns a relation weight for a hyperedge.
    fn relation_weight(&self, hyperedge: u32) -> PyResult<f64> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self.inner.relation_weight(HyperedgeId(hyperedge)))
    }

    /// Returns an incidence weight for a participant ID.
    fn incidence_weight(&self, participant: u32) -> PyResult<f64> {
        ensure_hyper_incidence(&self.inner, participant)?;
        Ok(self.inner.incidence_weight(HyperParticipantId(participant)))
    }

    /// Returns the canonical vertex ID for a local vertex ID.
    fn canonical_vertex_id(&self, vertex: u32) -> PyResult<u32> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self.inner.canonical_element_id(HyperVertexId(vertex)).0)
    }

    /// Returns the local vertex ID for a canonical vertex ID, if visible.
    fn local_vertex_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_element_id(HyperVertexId(canonical))
            .map(|id| id.0)
    }

    /// Returns the canonical hyperedge ID for a local hyperedge ID.
    fn canonical_hyperedge_id(&self, hyperedge: u32) -> PyResult<u32> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self.inner.canonical_relation_id(HyperedgeId(hyperedge)).0)
    }

    /// Returns the local hyperedge ID for a canonical hyperedge ID, if visible.
    fn local_hyperedge_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_relation_id(HyperedgeId(canonical))
            .map(|id| id.0)
    }

    /// Returns the vertex ID for a Python label, if present.
    fn vertex_for_label(&self, label: &str) -> Option<u32> {
        self.labels.get(label).copied()
    }

    /// Returns the first Python label for a vertex, if present.
    fn label_for_vertex(&self, vertex: u32) -> PyResult<Option<String>> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(label_for_id(&self.labels, vertex))
    }

    /// Runs BFS from `start` and returns visited vertex IDs in traversal order.
    fn bfs(&self, start: u32) -> PyResult<Vec<u32>> {
        ensure_hyper_vertex(&self.inner, start)?;
        let traversal = breadth_first_search(&self.inner, HyperVertexId(start))
            .map_err(|error| HypergraphError::new_err(error.to_string()))?;
        Ok(traversal.map(|vertex| vertex.0).collect())
    }

    /// Runs unweighted incidence/bipartite `PageRank`.
    #[pyo3(signature = (damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn pagerank(
        &self,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<(Vec<f64>, Vec<f64>)> {
        let config = pagerank_config(damping, tolerance, max_iterations);
        let elements = hyper_elements(&self.inner);
        let relations = hyper_relations(&self.inner);
        let mut element_ranks = vec![0.0; self.inner.vertex_count()];
        let mut relation_ranks = vec![0.0; self.inner.hyperedge_count()];
        hypergraph_pagerank(
            &self.inner,
            elements,
            relations,
            config,
            personalization.as_deref(),
            &mut element_ranks,
            &mut relation_ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok((element_ranks, relation_ranks))
    }

    /// Runs weighted incidence/bipartite `PageRank`.
    #[pyo3(signature = (damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn weighted_pagerank(
        &self,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<(Vec<f64>, Vec<f64>)> {
        let config = pagerank_config(damping, tolerance, max_iterations);
        let elements = hyper_elements(&self.inner);
        let relations = hyper_relations(&self.inner);
        let mut element_ranks = vec![0.0; self.inner.vertex_count()];
        let mut relation_ranks = vec![0.0; self.inner.hyperedge_count()];
        hypergraph_pagerank_weighted(
            &self.inner,
            &self.inner,
            &self.inner,
            elements,
            relations,
            config,
            personalization.as_deref(),
            &mut element_ranks,
            &mut relation_ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok((element_ranks, relation_ranks))
    }

    /// Runs weighted incidence/bipartite `PageRank` using dense property layers.
    #[pyo3(signature = (relation_layer, incidence_layer, damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn pagerank_with_dense_weight_layers(
        &self,
        relation_layer: &PyDenseF64Layer,
        incidence_layer: &PyDenseF64Layer,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<(Vec<f64>, Vec<f64>)> {
        let relations =
            DenseRelationWeights::<_, Float64Type>::new(&self.inner, &relation_layer.inner)
                .map_err(py_property_error)?;
        let incidences =
            DenseIncidenceWeights::<_, Float64Type>::new(&self.inner, &incidence_layer.inner)
                .map_err(py_property_error)?;
        self.pagerank_with_weight_views(
            &relations,
            &incidences,
            damping,
            tolerance,
            max_iterations,
            personalization,
        )
    }

    /// Runs weighted incidence/bipartite `PageRank` using sparse totalizing layers.
    #[pyo3(signature = (relation_layer, incidence_layer, damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn pagerank_with_sparse_weight_layers(
        &self,
        relation_layer: &PySparseF64Layer,
        incidence_layer: &PySparseF64Layer,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<(Vec<f64>, Vec<f64>)> {
        let relations =
            SparseRelationWeights::<_, Float64Type>::new(&self.inner, &relation_layer.inner)
                .map_err(py_property_error)?;
        let incidences =
            SparseIncidenceWeights::<_, Float64Type>::new(&self.inner, &incidence_layer.inner)
                .map_err(py_property_error)?;
        self.pagerank_with_weight_views(
            &relations,
            &incidences,
            damping,
            tolerance,
            max_iterations,
            personalization,
        )
    }

    /// Serializes BCSR, identity, and property sections into an `OxGraph` snapshot byte vector.
    fn to_bcsr_snapshot(&self) -> PyResult<Vec<u8>> {
        self.inner.to_bcsr_snapshot().map_err(py_hyper_error)
    }
}

impl PyFrozenHypergraph {
    /// Runs weighted hypergraph `PageRank` with arbitrary selected weight views.
    fn pagerank_with_weight_views<RW, IW>(
        &self,
        relation_weights: &RW,
        incidence_weights: &IW,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<(Vec<f64>, Vec<f64>)>
    where
        RW: RelationWeight<ElementId = HyperVertexId, RelationId = HyperedgeId, Weight = f64>,
        IW: IncidenceWeight<
                ElementId = HyperVertexId,
                RelationId = HyperedgeId,
                IncidenceId = HyperParticipantId,
                Weight = f64,
            >,
    {
        let config = pagerank_config(damping, tolerance, max_iterations);
        let elements = hyper_elements(&self.inner);
        let relations = hyper_relations(&self.inner);
        let mut element_ranks = vec![0.0; self.inner.vertex_count()];
        let mut relation_ranks = vec![0.0; self.inner.hyperedge_count()];
        hypergraph_pagerank_weighted(
            &self.inner,
            relation_weights,
            incidence_weights,
            elements,
            relations,
            config,
            personalization.as_deref(),
            &mut element_ranks,
            &mut relation_ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok((element_ranks, relation_ranks))
    }
}

/// Python-owned dense f64 property layer.
#[pyclass(name = "DenseF64Layer")]
struct PyDenseF64Layer {
    /// Rust property layer.
    inner: PropertyLayer,
}

#[pymethods]
impl PyDenseF64Layer {
    /// Creates a dense f64 property layer.
    #[new]
    fn new(
        layer_id: u64,
        name: &str,
        id_family: &str,
        role: Option<&str>,
        values: Vec<f64>,
    ) -> PyResult<Self> {
        let family = parse_id_family(id_family)?;
        let role = parse_layer_role(role.unwrap_or("weight"))?;
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(layer_id),
            name,
            family,
            role,
            StorageMode::Dense,
            Field::new(name, DataType::Float64, false),
        )
        .map_err(py_property_error)?;
        let layer = PropertyLayer::try_new_dense(descriptor, Arc::new(Float64Array::from(values)))
            .map_err(py_property_error)?;
        Ok(Self { inner: layer })
    }

    /// Returns the layer name.
    fn name(&self) -> String {
        self.inner.descriptor().name.as_str().to_owned()
    }

    /// Returns the logical length.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the layer has no values.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns a checked dense value.
    fn value(&self, index: usize) -> PyResult<f64> {
        if index >= self.inner.len() {
            return Err(py_property_message(format_args!(
                "property index {index} is out of bounds"
            )));
        }
        dense_f64_value(&self.inner, index)
    }
}

/// Python-owned sparse f64 property layer.
#[pyclass(name = "SparseF64Layer")]
struct PySparseF64Layer {
    /// Rust property layer.
    inner: PropertyLayer,
}

#[pymethods]
impl PySparseF64Layer {
    /// Creates a sparse f64 property layer with an optional default.
    #[new]
    #[pyo3(signature = (layer_id, name, id_family, len, indices, values, default=None, role=None))]
    fn new(
        layer_id: u64,
        name: &str,
        id_family: &str,
        len: usize,
        indices: Vec<u64>,
        values: Vec<f64>,
        default: Option<f64>,
        role: Option<&str>,
    ) -> PyResult<Self> {
        let family = parse_id_family(id_family)?;
        let role = parse_layer_role(role.unwrap_or("weight"))?;
        let missing = if default.is_some() {
            MissingPolicy::Default
        } else {
            MissingPolicy::Null
        };
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(layer_id),
            name,
            family,
            role,
            StorageMode::Sparse { missing },
            Field::new(name, DataType::Float64, false),
        )
        .map_err(py_property_error)?;
        let default_array = default.map(|value| Arc::new(Float64Array::from(vec![value])) as _);
        let layer = PropertyLayer::try_new_sparse(
            descriptor,
            len,
            Arc::new(UInt64Array::from(indices)),
            Arc::new(Float64Array::from(values)),
            default_array,
        )
        .map_err(py_property_error)?;
        Ok(Self { inner: layer })
    }

    /// Returns the layer name.
    fn name(&self) -> String {
        self.inner.descriptor().name.as_str().to_owned()
    }

    /// Returns the logical length.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the layer has no logical values.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns a sparse value, applying the missing policy.
    fn value(&self, index: usize) -> PyResult<f64> {
        if index >= self.inner.len() {
            return Err(py_property_message(format_args!(
                "property index {index} is out of bounds"
            )));
        }
        sparse_f64_value(&self.inner, index)
    }
}

/// Python snapshot inspection result.
#[pyclass(name = "SnapshotInfo", skip_from_py_object)]
#[derive(Clone, Debug)]
struct PySnapshotInfo {
    /// Number of sections.
    section_count: usize,
    /// Section kind values.
    section_kinds: Vec<u32>,
    /// Whether property sections validated.
    property_layer_count: Option<usize>,
    /// Whether identity sections validated.
    identity_family_count: Option<usize>,
}

#[pymethods]
impl PySnapshotInfo {
    /// Returns the number of sections.
    fn section_count(&self) -> usize {
        self.section_count
    }

    /// Returns section kind values.
    fn section_kinds(&self) -> Vec<u32> {
        self.section_kinds.clone()
    }

    /// Returns validated property layer count, if property sections were present.
    fn property_layer_count(&self) -> Option<usize> {
        self.property_layer_count
    }

    /// Returns validated identity family count, if identity sections were present.
    fn identity_family_count(&self) -> Option<usize> {
        self.identity_family_count
    }
}

/// Opens a generic snapshot and validates known property/identity sections when present.
#[pyfunction]
fn open_snapshot(bytes: &[u8]) -> PyResult<PySnapshotInfo> {
    snapshot_info(bytes, SnapshotKind::Generic)
}

/// Opens a CSR graph snapshot and validates known property/identity sections when present.
#[pyfunction]
fn open_csr_snapshot(bytes: &[u8]) -> PyResult<PySnapshotInfo> {
    snapshot_info(bytes, SnapshotKind::Csr)
}

/// Opens a BCSR hypergraph snapshot and validates known property/identity sections when present.
#[pyfunction]
fn open_bcsr_snapshot(bytes: &[u8]) -> PyResult<PySnapshotInfo> {
    snapshot_info(bytes, SnapshotKind::Bcsr)
}

/// Snapshot topology validation target.
#[derive(Clone, Copy, Debug)]
enum SnapshotKind {
    /// Generic container only.
    Generic,
    /// CSR graph sections.
    Csr,
    /// BCSR hypergraph sections.
    Bcsr,
}

/// Creates the `_oxgraph` native module.
#[pymodule]
fn _oxgraph(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("OxGraphError", py.get_type::<OxGraphError>())?;
    module.add("GraphError", py.get_type::<GraphError>())?;
    module.add("HypergraphError", py.get_type::<HypergraphError>())?;
    module.add("PageRankError", py.get_type::<PageRankPythonError>())?;
    module.add("SnapshotError", py.get_type::<SnapshotPythonError>())?;
    module.add("PropertyError", py.get_type::<PropertyPythonError>())?;
    module.add_class::<PyGraphBuilder>()?;
    module.add_class::<PyFrozenGraph>()?;
    module.add_class::<PyHypergraphBuilder>()?;
    module.add_class::<PyFrozenHypergraph>()?;
    module.add_class::<PyDenseF64Layer>()?;
    module.add_class::<PySparseF64Layer>()?;
    module.add_class::<PySnapshotInfo>()?;
    module.add_function(wrap_pyfunction!(open_snapshot, module)?)?;
    module.add_function(wrap_pyfunction!(open_csr_snapshot, module)?)?;
    module.add_function(wrap_pyfunction!(open_bcsr_snapshot, module)?)?;
    Ok(())
}

/// Converts Rust graph builder errors into Python graph errors.
fn py_graph_error(error: GraphBuildError) -> PyErr {
    GraphError::new_err(error.to_string())
}

/// Converts Rust hypergraph builder errors into Python hypergraph errors.
fn py_hyper_error(error: HyperBuildError) -> PyErr {
    HypergraphError::new_err(error.to_string())
}

/// Converts Rust `PageRank` errors into Python `PageRank` errors.
fn py_pagerank_error(error: PageRankError) -> PyErr {
    PageRankPythonError::new_err(error.to_string())
}

/// Converts Rust snapshot errors into Python snapshot errors.
fn py_snapshot_error<E: fmt::Display>(error: E) -> PyErr {
    SnapshotPythonError::new_err(error.to_string())
}

/// Converts Rust property errors into Python property errors.
fn py_property_error(error: PropertyError) -> PyErr {
    PropertyPythonError::new_err(error.to_string())
}

/// Builds a property error from formatted message arguments.
fn py_property_message(args: fmt::Arguments<'_>) -> PyErr {
    PropertyPythonError::new_err(args.to_string())
}

/// Builds a graph error from formatted message arguments.
fn py_graph_message(args: fmt::Arguments<'_>) -> PyErr {
    GraphError::new_err(args.to_string())
}

/// Builds a hypergraph error from formatted message arguments.
fn py_hyper_message(args: fmt::Arguments<'_>) -> PyErr {
    HypergraphError::new_err(args.to_string())
}

/// Builds a `PageRank` config from optional Python arguments.
fn pagerank_config(
    damping: Option<f64>,
    tolerance: Option<f64>,
    max_iterations: Option<usize>,
) -> PageRankConfig<f64> {
    let default = PageRankConfig::<f64>::default();
    PageRankConfig::new(
        damping.unwrap_or(default.damping),
        tolerance.unwrap_or(default.tolerance),
        max_iterations.unwrap_or(default.max_iterations),
    )
}

/// Inserts a Python graph label into a facade map.
fn insert_label(labels: &mut BTreeMap<String, u32>, name: String, id: u32) -> PyResult<()> {
    if labels.contains_key(&name) {
        return Err(py_graph_message(format_args!("duplicate label '{name}'")));
    }
    labels.insert(name, id);
    Ok(())
}

/// Inserts a Python hypergraph label into a facade map.
fn insert_hyper_label(labels: &mut BTreeMap<String, u32>, name: String, id: u32) -> PyResult<()> {
    if labels.contains_key(&name) {
        return Err(py_hyper_message(format_args!("duplicate label '{name}'")));
    }
    labels.insert(name, id);
    Ok(())
}

/// Returns the first label for a facade ID.
fn label_for_id(labels: &BTreeMap<String, u32>, id: u32) -> Option<String> {
    labels
        .iter()
        .find_map(|(label, &value)| (value == id).then(|| label.clone()))
}

/// Checks a frozen graph node ID.
fn ensure_graph_node(graph: &PythonFrozenGraph, node: u32) -> PyResult<()> {
    if graph.contains_element(GraphNodeId(node)) {
        Ok(())
    } else {
        Err(py_graph_message(format_args!(
            "invalid graph node ID {node}"
        )))
    }
}

/// Checks a frozen graph edge ID.
fn ensure_graph_edge(graph: &PythonFrozenGraph, edge: u32) -> PyResult<()> {
    if graph.contains_relation(GraphEdgeId(edge)) {
        Ok(())
    } else {
        Err(py_graph_message(format_args!(
            "invalid graph edge ID {edge}"
        )))
    }
}

/// Checks a frozen hypergraph vertex ID.
fn ensure_hyper_vertex(hypergraph: &PythonFrozenHypergraph, vertex: u32) -> PyResult<()> {
    if hypergraph.contains_element(HyperVertexId(vertex)) {
        Ok(())
    } else {
        Err(py_hyper_message(format_args!(
            "invalid hypergraph vertex ID {vertex}"
        )))
    }
}

/// Checks a frozen hypergraph relation ID.
fn ensure_hyperedge(hypergraph: &PythonFrozenHypergraph, hyperedge: u32) -> PyResult<()> {
    if hypergraph.contains_relation(HyperedgeId(hyperedge)) {
        Ok(())
    } else {
        Err(py_hyper_message(format_args!(
            "invalid hyperedge ID {hyperedge}"
        )))
    }
}

/// Checks a frozen hypergraph incidence ID.
fn ensure_hyper_incidence(hypergraph: &PythonFrozenHypergraph, incidence: u32) -> PyResult<()> {
    if hypergraph.contains_incidence(HyperParticipantId(incidence)) {
        Ok(())
    } else {
        Err(py_hyper_message(format_args!(
            "invalid hypergraph incidence ID {incidence}"
        )))
    }
}

/// Enumerates frozen graph elements by dense first-generation ID.
fn graph_elements(graph: &PythonFrozenGraph) -> Vec<GraphNodeId> {
    (0..graph.node_count())
        .map(|index| GraphNodeId(dense_index_to_u32(index)))
        .collect()
}

/// Enumerates frozen hypergraph elements by dense first-generation ID.
fn hyper_elements(hypergraph: &PythonFrozenHypergraph) -> Vec<HyperVertexId> {
    (0..hypergraph.vertex_count())
        .map(|index| HyperVertexId(dense_index_to_u32(index)))
        .collect()
}

/// Enumerates frozen hypergraph relations by dense first-generation ID.
fn hyper_relations(hypergraph: &PythonFrozenHypergraph) -> Vec<HyperedgeId> {
    (0..hypergraph.hyperedge_count())
        .map(|index| HyperedgeId(dense_index_to_u32(index)))
        .collect()
}

/// Reads a dense f64 property-layer value.
fn dense_f64_value(layer: &PropertyLayer, index: usize) -> PyResult<f64> {
    let PropertyLayerData::Dense { values } = layer.data() else {
        return Err(py_property_message(format_args!(
            "property layer is not dense"
        )));
    };
    let values = values
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| py_property_message(format_args!("property layer is not Float64")))?;
    if values.is_null(index) {
        return Err(py_property_message(format_args!(
            "property index {index} is null"
        )));
    }
    Ok(values.value(index))
}

/// Reads a sparse f64 property-layer value with default handling.
fn sparse_f64_value(layer: &PropertyLayer, index: usize) -> PyResult<f64> {
    let PropertyLayerData::Sparse {
        indices,
        values,
        default,
    } = layer.data()
    else {
        return Err(py_property_message(format_args!(
            "property layer is not sparse"
        )));
    };
    let values = values
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| py_property_message(format_args!("property layer is not Float64")))?;
    let target = u64::try_from(index)
        .map_err(|error| py_property_message(format_args!("property index overflow: {error}")))?;
    let mut low = 0_usize;
    let mut high = indices.len();
    while low < high {
        let mid = low + ((high - low) / 2);
        if indices.value(mid) < target {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low < indices.len() && indices.value(low) == target {
        return Ok(values.value(low));
    }
    let Some(default) = default else {
        return Err(py_property_message(format_args!(
            "property index {index} is missing"
        )));
    };
    let default = default
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| py_property_message(format_args!("property default is not Float64")))?;
    Ok(default.value(0))
}

/// Converts a dense first-generation index to `u32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Python frozen views are produced by u32-ID builders, so dense counts fit u32"
)]
const fn dense_index_to_u32(index: usize) -> u32 {
    index as u32
}

/// Parses an ID-family argument.
fn parse_id_family(value: &str) -> PyResult<IdFamily> {
    match value {
        "element" | "node" | "vertex" => Ok(IdFamily::Element),
        "relation" | "edge" | "hyperedge" => Ok(IdFamily::Relation),
        "incidence" | "participant" => Ok(IdFamily::Incidence),
        _ => Err(py_property_message(format_args!(
            "unknown ID family '{value}'"
        ))),
    }
}

/// Parses a layer-role argument.
fn parse_layer_role(value: &str) -> PyResult<LayerRole> {
    match value {
        "weight" => Ok(LayerRole::Weight),
        "property" => Ok(LayerRole::Property),
        _ => Err(py_property_message(format_args!(
            "unknown layer role '{value}'"
        ))),
    }
}

/// Opens and validates snapshot sections according to `kind`.
fn snapshot_info(bytes: &[u8], kind: SnapshotKind) -> PyResult<PySnapshotInfo> {
    let snapshot = Snapshot::open(bytes).map_err(py_snapshot_error)?;
    match kind {
        SnapshotKind::Generic => {}
        SnapshotKind::Csr => {
            let _graph =
                CsrSnapshotGraph::<u32>::from_snapshot(&snapshot).map_err(py_snapshot_error)?;
        }
        SnapshotKind::Bcsr => {
            let _hypergraph = BcsrHypergraph::from_snapshot_with(&snapshot, BcsrValidation::Strict)
                .map_err(py_snapshot_error)?;
        }
    }
    let section_kinds: Vec<u32> = snapshot.sections().map(|section| section.kind()).collect();
    let property_layer_count = if snapshot
        .section(SNAPSHOT_KIND_PROPERTY_DESCRIPTORS)
        .is_some()
        || snapshot.section(SNAPSHOT_KIND_PROPERTY_DATA).is_some()
    {
        Some(
            validate_property_snapshot(&snapshot)
                .map_err(py_property_error)?
                .layer_count,
        )
    } else {
        None
    };
    let identity_family_count = if snapshot.section(SNAPSHOT_KIND_IDENTITY_MODES).is_some() {
        Some(
            validate_identity_snapshot(&snapshot)
                .map_err(py_property_error)?
                .records
                .len(),
        )
    } else {
        None
    };
    Ok(PySnapshotInfo {
        section_count: snapshot.section_count(),
        section_kinds,
        property_layer_count,
        identity_family_count,
    })
}

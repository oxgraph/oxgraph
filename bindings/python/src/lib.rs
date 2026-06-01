//! `PyO3` bindings for the `OxGraph` Python facade.
//!
//! The native module is packaged as `oxgraph._oxgraph`; the Python package
//! re-exports the reviewable facade as `oxgraph`. Python labels are owned by
//! this facade and are not Rust topology IDs.
#![expect(
    missing_docs,
    reason = "PyO3 create_exception macro emits undocumented Python exception structs"
)]
#![expect(
    clippy::missing_const_for_fn,
    reason = "PyO3 pymethods and Python-callable helpers are regular functions"
)]
#![expect(
    clippy::needless_pass_by_value,
    reason = "PyO3 owns converted Python containers such as Vec inputs"
)]
#![expect(
    clippy::too_many_arguments,
    reason = "Python facade methods mirror keyword-rich Python APIs"
)]

use std::{collections::BTreeMap, fmt, vec::Vec};

use oxgraph_algo::{
    HyperWeighted, PageRankConfig, PageRankError, Uniform, Weighted, breadth_first_search,
    pagerank_graph, pagerank_hypergraph,
};
use oxgraph_csr::{
    CsrSnapshotGraph,
    build::{
        FrozenWeightedGraph, GraphBuildError, GraphEdgeId, GraphNodeId, WeightedGraphBuilder,
        export_weighted_csr_snapshot,
    },
};
use oxgraph_graph::{EdgeEndpointGraph, GraphCounts, OutgoingGraph};
use oxgraph_hyper::{
    DirectedHyperedgeIncidences, DirectedHyperedgeParticipants, DirectedVertexHyperedges,
    HypergraphCounts,
};
use oxgraph_hyper_bcsr::{
    BcsrSnapshotHypergraph, BcsrValidation,
    build::{
        FrozenWeightedHypergraph, HyperBuildError, HyperParticipantId, HyperVertexId, HyperedgeId,
        WeightedHypergraphBuilder, export_weighted_bcsr_snapshot,
    },
};
use oxgraph_snapshot::Snapshot;
use oxgraph_topology::{
    CanonicalElementIdentity, CanonicalRelationIdentity, ContainsElement, ContainsIncidence,
    ContainsRelation, ElementSuccessors, ElementWeight, IncidenceCounts, IncidenceWeight,
    LocalElementIdentity, LocalRelationIdentity, RelationWeight,
};
use pyo3::{create_exception, exceptions::PyValueError, prelude::*};

create_exception!(_oxgraph, OxGraphError, PyValueError);
create_exception!(_oxgraph, GraphError, OxGraphError);
create_exception!(_oxgraph, HypergraphError, OxGraphError);
create_exception!(_oxgraph, PageRankPythonError, OxGraphError);
create_exception!(_oxgraph, SnapshotPythonError, OxGraphError);

/// Python graph builder specialization.
type PythonGraphBuilder = WeightedGraphBuilder<u32, u32, f64, f64>;
/// Python frozen graph specialization.
type PythonFrozenGraph = FrozenWeightedGraph<u32, u32, f64, f64>;
/// Python hypergraph builder specialization.
type PythonHypergraphBuilder = WeightedHypergraphBuilder<u32, u32, u32, f64, f64, f64>;
/// Python frozen hypergraph specialization.
type PythonFrozenHypergraph = FrozenWeightedHypergraph<u32, u32, u32, f64, f64, f64>;
/// Python graph build error specialization.
type PythonGraphBuildError = GraphBuildError<u32, u32>;
/// Python hypergraph build error specialization.
type PythonHyperBuildError = HyperBuildError<u32, u32, u32>;

/// Default Python element weight.
const DEFAULT_ELEMENT_WEIGHT: f64 = 0.0;
/// Default Python relation weight.
const DEFAULT_RELATION_WEIGHT: f64 = 1.0;
/// Default Python incidence weight.
const DEFAULT_INCIDENCE_WEIGHT: f64 = 1.0;
/// Default Python `PageRank` damping factor.
const DEFAULT_PAGERANK_DAMPING: f64 = 0.85;
/// Default Python `PageRank` convergence tolerance.
const DEFAULT_PAGERANK_TOLERANCE: f64 = 1.0e-12;
/// Default Python `PageRank` iteration cap.
const DEFAULT_PAGERANK_MAX_ITERATIONS: usize = 100;

/// Private bidirectional label/ID index for the Python facade.
///
/// Python labels are owned by this index as `String` values and are never
/// interpreted as Rust topology IDs. The index enforces uniqueness at insert
/// time through [`ensure_available`](Self::ensure_available); callers check
/// availability before mutating builders so duplicate-label errors cannot
/// leave a partially mutated builder.
///
/// Forward (`label → id`) and reverse (`id → label`) maps are kept in lockstep
/// so both directions resolve in `O(log n)`. Each label maps to exactly one ID,
/// and the reverse map records the first label inserted for an ID, matching the
/// historical [`label`](Self::label) contract.
///
/// # Performance
///
/// Insertion and lookup are `O(log n)` in the number of labels via `BTreeMap`.
/// Cloning is `O(n log n)`.
#[derive(Clone, Debug, Default)]
struct LabelIndex {
    /// Label-to-`u32`-ID mapping.
    forward: BTreeMap<String, u32>,
    /// `u32`-ID-to-label mapping holding the first label inserted per ID.
    reverse: BTreeMap<u32, String>,
}

impl LabelIndex {
    /// Creates an empty label index.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn new() -> Self {
        Self::default()
    }

    /// Returns `Ok(())` when `label` is `None` or absent from the index.
    ///
    /// Returns `Err(message)` when `label` is `Some(name)` and `name` already
    /// maps to an ID so callers can surface a typed Python error before
    /// mutating the Rust builder.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn ensure_available(&self, label: Option<&str>) -> Result<(), String> {
        if let Some(name) = label
            && self.forward.contains_key(name)
        {
            return Err(format!("duplicate label '{name}'"));
        }
        Ok(())
    }

    /// Inserts `label → id` when `label` is `Some(name)`.
    ///
    /// The caller must have already confirmed availability via
    /// [`ensure_available`](Self::ensure_available); this method does not
    /// re-check. A `None` label is a no-op. The reverse map keeps the first
    /// label inserted for an ID so [`label`](Self::label) is stable across
    /// repeated insertions for the same ID.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn insert_checked(&mut self, label: Option<String>, id: u32) {
        if let Some(name) = label {
            self.reverse.entry(id).or_insert_with(|| name.clone());
            let prev = self.forward.insert(name, id);
            debug_assert!(
                prev.is_none(),
                "insert_checked called without prior ensure_available"
            );
        }
    }

    /// Returns the ID for a label string, or `None` when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    fn id(&self, label: &str) -> Option<u32> {
        self.forward.get(label).copied()
    }

    /// Returns the first label that maps to `id`, or `None`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)` via the reverse map.
    #[must_use]
    fn label(&self, id: u32) -> Option<String> {
        self.reverse.get(&id).cloned()
    }
}

/// Python wrapper around a weighted directed graph builder.
#[pyclass(name = "GraphBuilder")]
struct PyGraphBuilder {
    /// Rust graph builder.
    inner: PythonGraphBuilder,
    /// Python label to canonical node ID map.
    labels: LabelIndex,
}

impl Default for PyGraphBuilder {
    fn default() -> Self {
        Self {
            inner: PythonGraphBuilder::new(),
            labels: LabelIndex::new(),
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

    /// Adds a node with an optional Python-owned label and element weight.
    #[pyo3(signature = (label=None, weight=None))]
    fn add_node(&mut self, label: Option<String>, weight: Option<f64>) -> PyResult<u32> {
        self.labels
            .ensure_available(label.as_deref())
            .map_err(graph_message)?;
        let node = self
            .inner
            .add_node(weight.unwrap_or(DEFAULT_ELEMENT_WEIGHT))
            .map_err(py_graph_error)?;
        self.labels.insert_checked(label, node.get());
        Ok(node.get())
    }

    /// Adds a directed edge with an optional relation weight.
    #[pyo3(signature = (source, target, weight=None))]
    fn add_edge(&mut self, source: u32, target: u32, weight: Option<f64>) -> PyResult<u32> {
        let edge = self
            .inner
            .add_edge(
                GraphNodeId::new(source),
                GraphNodeId::new(target),
                weight.unwrap_or(DEFAULT_RELATION_WEIGHT),
            )
            .map_err(py_graph_error)?;
        Ok(edge.get())
    }

    /// Sets an existing node's element weight.
    fn set_element_weight(&mut self, node: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_element_weight(GraphNodeId::new(node), weight)
            .map_err(py_graph_error)
    }

    /// Sets an existing edge's relation weight.
    fn set_relation_weight(&mut self, edge: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_relation_weight(GraphEdgeId::new(edge), weight)
            .map_err(py_graph_error)
    }

    /// Returns the node ID for a Python label, if present.
    fn node(&self, label: &str) -> Option<u32> {
        self.labels.id(label)
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
    labels: LabelIndex,
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

    /// Returns local node IDs in dense builder order.
    fn nodes(&self) -> Vec<u32> {
        dense_ids(self.inner.node_count())
    }

    /// Returns `(edge_id, source_node, target_node)` tuples in dense builder order.
    fn edges(&self) -> Vec<(u32, u32, u32)> {
        dense_ids(self.inner.edge_count())
            .into_iter()
            .map(|edge| {
                let (source, target) = self.inner.endpoints(GraphEdgeId::new(edge));
                (edge, source.get(), target.get())
            })
            .collect()
    }

    /// Returns the element weight for a node.
    fn element_weight(&self, node: u32) -> PyResult<f64> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self.inner.element_weight(GraphNodeId::new(node)))
    }

    /// Returns the relation weight for an edge.
    fn relation_weight(&self, edge: u32) -> PyResult<f64> {
        ensure_graph_edge(&self.inner, edge)?;
        Ok(self.inner.relation_weight(GraphEdgeId::new(edge)))
    }

    /// Returns the canonical node ID for a local node ID.
    fn canonical_node_id(&self, node: u32) -> PyResult<u32> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self
            .inner
            .canonical_element_id(GraphNodeId::new(node))
            .get())
    }

    /// Returns the local node ID for a canonical node ID, if visible.
    fn local_node_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_element_id(GraphNodeId::new(canonical))
            .map(GraphNodeId::get)
    }

    /// Returns the canonical edge ID for a local edge ID.
    fn canonical_edge_id(&self, edge: u32) -> PyResult<u32> {
        ensure_graph_edge(&self.inner, edge)?;
        Ok(self
            .inner
            .canonical_relation_id(GraphEdgeId::new(edge))
            .get())
    }

    /// Returns the local edge ID for a canonical edge ID, if visible.
    fn local_edge_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_relation_id(GraphEdgeId::new(canonical))
            .map(GraphEdgeId::get)
    }

    /// Returns the node ID for a Python label, if present.
    fn node(&self, label: &str) -> Option<u32> {
        self.labels.id(label)
    }

    /// Returns the first Python label for a node, if present.
    fn node_label(&self, node: u32) -> PyResult<Option<String>> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self.labels.label(node))
    }

    /// Returns `(source_node, target_node)` for a local edge ID.
    fn edge(&self, edge: u32) -> PyResult<(u32, u32)> {
        ensure_graph_edge(&self.inner, edge)?;
        let (source, target) = self.inner.endpoints(GraphEdgeId::new(edge));
        Ok((source.get(), target.get()))
    }

    /// Returns `(edge_id, target_node)` tuples whose source is `node`.
    fn out_edges(&self, node: u32) -> PyResult<Vec<(u32, u32)>> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self
            .inner
            .outgoing_edges(GraphNodeId::new(node))
            .map(|edge| {
                let (_source, target) = self.inner.endpoints(edge);
                (edge.get(), target.get())
            })
            .collect())
    }

    /// Returns successor node IDs reached from `node`.
    fn successors(&self, node: u32) -> PyResult<Vec<u32>> {
        ensure_graph_node(&self.inner, node)?;
        Ok(self
            .inner
            .element_successors(GraphNodeId::new(node))
            .map(GraphNodeId::get)
            .collect())
    }

    /// Runs BFS from `start` and returns visited node IDs in traversal order.
    fn bfs(&self, start: u32) -> PyResult<Vec<u32>> {
        ensure_graph_node(&self.inner, start)?;
        let traversal = breadth_first_search(&self.inner, GraphNodeId::new(start))
            .map_err(|error| GraphError::new_err(error.to_string()))?;
        Ok(traversal.map(GraphNodeId::get).collect())
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
        let elements = graph_elements(&self.inner);
        let mut ranks = vec![0.0; self.inner.node_count()];
        pagerank_graph(
            &self.inner,
            &Uniform,
            elements,
            pagerank_config(damping, tolerance, max_iterations),
            personalization.as_deref(),
            &mut ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok(ranks)
    }

    /// Runs relation-weighted `PageRank` over all nodes.
    #[pyo3(signature = (damping=None, tolerance=None, max_iterations=None, personalization=None))]
    fn weighted_pagerank(
        &self,
        damping: Option<f64>,
        tolerance: Option<f64>,
        max_iterations: Option<usize>,
        personalization: Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>> {
        let elements = graph_elements(&self.inner);
        let mut ranks = vec![0.0; self.inner.node_count()];
        pagerank_graph(
            &self.inner,
            &Weighted::new(&self.inner),
            elements,
            pagerank_config(damping, tolerance, max_iterations),
            personalization.as_deref(),
            &mut ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok(ranks)
    }

    /// Serializes topology and identity sections into a CSR snapshot byte vector.
    fn to_csr_snapshot(&self) -> PyResult<Vec<u8>> {
        export_weighted_csr_snapshot(&self.inner).map_err(py_graph_error)
    }
}

/// Python wrapper around a weighted directed hypergraph builder.
#[pyclass(name = "HypergraphBuilder")]
struct PyHypergraphBuilder {
    /// Rust hypergraph builder.
    inner: PythonHypergraphBuilder,
    /// Python label to canonical vertex ID map.
    labels: LabelIndex,
}

impl Default for PyHypergraphBuilder {
    fn default() -> Self {
        Self {
            inner: PythonHypergraphBuilder::new(),
            labels: LabelIndex::new(),
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

    /// Adds an isolated vertex with an optional Python label and element weight.
    #[pyo3(signature = (label=None, weight=None))]
    fn add_vertex(&mut self, label: Option<String>, weight: Option<f64>) -> PyResult<u32> {
        self.labels
            .ensure_available(label.as_deref())
            .map_err(hyper_message)?;
        let vertex = self
            .inner
            .add_vertex(weight.unwrap_or(DEFAULT_ELEMENT_WEIGHT))
            .map_err(py_hyper_error)?;
        self.labels.insert_checked(label, vertex.get());
        Ok(vertex.get())
    }

    /// Adds a directed hyperedge with optional relation and incidence weights.
    #[pyo3(signature = (sources, targets, weight=None, source_weights=None, target_weights=None))]
    fn add_hyperedge(
        &mut self,
        sources: Vec<u32>,
        targets: Vec<u32>,
        weight: Option<f64>,
        source_weights: Option<Vec<f64>>,
        target_weights: Option<Vec<f64>>,
    ) -> PyResult<u32> {
        let sources = weighted_vertices(sources, source_weights, "source_weights")?;
        let targets = weighted_vertices(targets, target_weights, "target_weights")?;
        let hyperedge = self
            .inner
            .add_hyperedge(
                &sources,
                &targets,
                weight.unwrap_or(DEFAULT_RELATION_WEIGHT),
            )
            .map_err(py_hyper_error)?;
        Ok(hyperedge.get())
    }

    /// Sets a vertex element weight.
    fn set_element_weight(&mut self, vertex: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_element_weight(HyperVertexId::new(vertex), weight)
            .map_err(py_hyper_error)
    }

    /// Sets a hyperedge relation weight.
    fn set_relation_weight(&mut self, hyperedge: u32, weight: f64) -> PyResult<()> {
        self.inner
            .set_relation_weight(HyperedgeId::new(hyperedge), weight)
            .map_err(py_hyper_error)
    }

    /// Sets one source-side incidence weight by hyperedge-local position.
    fn set_source_incidence_weight(
        &mut self,
        hyperedge: u32,
        position: usize,
        weight: f64,
    ) -> PyResult<()> {
        self.inner
            .set_source_incidence_weight(HyperedgeId::new(hyperedge), position, weight)
            .map_err(py_hyper_error)
    }

    /// Sets one target-side incidence weight by hyperedge-local position.
    fn set_target_incidence_weight(
        &mut self,
        hyperedge: u32,
        position: usize,
        weight: f64,
    ) -> PyResult<()> {
        self.inner
            .set_target_incidence_weight(HyperedgeId::new(hyperedge), position, weight)
            .map_err(py_hyper_error)
    }

    /// Returns the vertex ID for a Python label, if present.
    fn vertex(&self, label: &str) -> Option<u32> {
        self.labels.id(label)
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
    labels: LabelIndex,
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

    /// Returns the number of incidence records in the frozen hypergraph.
    fn incidence_count(&self) -> usize {
        self.inner.incidence_count()
    }

    /// Returns local vertex IDs in dense builder order.
    fn vertices(&self) -> Vec<u32> {
        dense_ids(self.inner.vertex_count())
    }

    /// Returns `(hyperedge_id, source_vertices, target_vertices)` tuples.
    fn hyperedges(&self) -> Vec<(u32, Vec<u32>, Vec<u32>)> {
        dense_ids(self.inner.hyperedge_count())
            .into_iter()
            .map(|hyperedge| {
                let sources = self.hyperedge_source_vertices(hyperedge);
                let targets = self.hyperedge_target_vertices(hyperedge);
                (hyperedge, sources, targets)
            })
            .collect()
    }

    /// Returns an element weight for a vertex.
    fn element_weight(&self, vertex: u32) -> PyResult<f64> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self.inner.element_weight(HyperVertexId::new(vertex)))
    }

    /// Returns a relation weight for a hyperedge.
    fn relation_weight(&self, hyperedge: u32) -> PyResult<f64> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self.inner.relation_weight(HyperedgeId::new(hyperedge)))
    }

    /// Returns an incidence weight for a participant ID.
    fn incidence_weight(&self, participant: u32) -> PyResult<f64> {
        ensure_hyper_incidence(&self.inner, participant)?;
        Ok(self
            .inner
            .incidence_weight(HyperParticipantId::new(participant)))
    }

    /// Returns the canonical vertex ID for a local vertex ID.
    fn canonical_vertex_id(&self, vertex: u32) -> PyResult<u32> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self
            .inner
            .canonical_element_id(HyperVertexId::new(vertex))
            .get())
    }

    /// Returns the local vertex ID for a canonical vertex ID, if visible.
    fn local_vertex_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_element_id(HyperVertexId::new(canonical))
            .map(HyperVertexId::get)
    }

    /// Returns the canonical hyperedge ID for a local hyperedge ID.
    fn canonical_hyperedge_id(&self, hyperedge: u32) -> PyResult<u32> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self
            .inner
            .canonical_relation_id(HyperedgeId::new(hyperedge))
            .get())
    }

    /// Returns the local hyperedge ID for a canonical hyperedge ID, if visible.
    fn local_hyperedge_id(&self, canonical: u32) -> Option<u32> {
        self.inner
            .local_relation_id(HyperedgeId::new(canonical))
            .map(HyperedgeId::get)
    }

    /// Returns the vertex ID for a Python label, if present.
    fn vertex(&self, label: &str) -> Option<u32> {
        self.labels.id(label)
    }

    /// Returns the first Python label for a vertex, if present.
    fn vertex_label(&self, vertex: u32) -> PyResult<Option<String>> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self.labels.label(vertex))
    }

    /// Returns `(source_vertices, target_vertices)` for a local hyperedge ID.
    fn hyperedge(&self, hyperedge: u32) -> PyResult<(Vec<u32>, Vec<u32>)> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok((
            self.hyperedge_source_vertices(hyperedge),
            self.hyperedge_target_vertices(hyperedge),
        ))
    }

    /// Returns source-side vertex IDs for a hyperedge.
    fn source_vertices(&self, hyperedge: u32) -> PyResult<Vec<u32>> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self
            .inner
            .source_participants(HyperedgeId::new(hyperedge))
            .map(HyperVertexId::get)
            .collect())
    }

    /// Returns target-side vertex IDs for a hyperedge.
    fn target_vertices(&self, hyperedge: u32) -> PyResult<Vec<u32>> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self
            .inner
            .target_participants(HyperedgeId::new(hyperedge))
            .map(HyperVertexId::get)
            .collect())
    }

    /// Returns source-side incidence IDs for a hyperedge.
    fn source_incidences(&self, hyperedge: u32) -> PyResult<Vec<u32>> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self
            .inner
            .source_incidences(HyperedgeId::new(hyperedge))
            .map(HyperParticipantId::get)
            .collect())
    }

    /// Returns target-side incidence IDs for a hyperedge.
    fn target_incidences(&self, hyperedge: u32) -> PyResult<Vec<u32>> {
        ensure_hyperedge(&self.inner, hyperedge)?;
        Ok(self
            .inner
            .target_incidences(HyperedgeId::new(hyperedge))
            .map(HyperParticipantId::get)
            .collect())
    }

    /// Returns hyperedge IDs where `vertex` is source-side.
    fn out_hyperedges(&self, vertex: u32) -> PyResult<Vec<u32>> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self
            .inner
            .outgoing_hyperedges(HyperVertexId::new(vertex))
            .map(HyperedgeId::get)
            .collect())
    }

    /// Returns successor vertex IDs reached from `vertex`.
    fn successors(&self, vertex: u32) -> PyResult<Vec<u32>> {
        ensure_hyper_vertex(&self.inner, vertex)?;
        Ok(self
            .inner
            .element_successors(HyperVertexId::new(vertex))
            .map(HyperVertexId::get)
            .collect())
    }

    /// Runs BFS from `start` and returns visited vertex IDs in traversal order.
    fn bfs(&self, start: u32) -> PyResult<Vec<u32>> {
        ensure_hyper_vertex(&self.inner, start)?;
        let traversal = breadth_first_search(&self.inner, HyperVertexId::new(start))
            .map_err(|error| HypergraphError::new_err(error.to_string()))?;
        Ok(traversal.map(HyperVertexId::get).collect())
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
        let elements = hyper_elements(&self.inner);
        let relations = hyper_relations(&self.inner);
        let mut element_ranks = vec![0.0; self.inner.vertex_count()];
        let mut relation_ranks = vec![0.0; self.inner.hyperedge_count()];
        pagerank_hypergraph(
            &self.inner,
            &Uniform,
            elements,
            relations,
            pagerank_config(damping, tolerance, max_iterations),
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
        let elements = hyper_elements(&self.inner);
        let relations = hyper_relations(&self.inner);
        let mut element_ranks = vec![0.0; self.inner.vertex_count()];
        let mut relation_ranks = vec![0.0; self.inner.hyperedge_count()];
        pagerank_hypergraph(
            &self.inner,
            &HyperWeighted::new(&self.inner, &self.inner),
            elements,
            relations,
            pagerank_config(damping, tolerance, max_iterations),
            personalization.as_deref(),
            &mut element_ranks,
            &mut relation_ranks,
        )
        .map_err(py_pagerank_error)?;
        Ok((element_ranks, relation_ranks))
    }

    /// Serializes topology and identity sections into a BCSR snapshot byte vector.
    fn to_bcsr_snapshot(&self) -> PyResult<Vec<u8>> {
        export_weighted_bcsr_snapshot(&self.inner).map_err(py_hyper_error)
    }
}

impl PyFrozenHypergraph {
    /// Returns source-side vertex IDs for a known-valid hyperedge.
    fn hyperedge_source_vertices(&self, hyperedge: u32) -> Vec<u32> {
        self.inner
            .source_participants(HyperedgeId::new(hyperedge))
            .map(HyperVertexId::get)
            .collect()
    }

    /// Returns target-side vertex IDs for a known-valid hyperedge.
    fn hyperedge_target_vertices(&self, hyperedge: u32) -> Vec<u32> {
        self.inner
            .target_participants(HyperedgeId::new(hyperedge))
            .map(HyperVertexId::get)
            .collect()
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
}

/// Opens a generic snapshot container.
#[pyfunction]
fn open_snapshot(bytes: &[u8]) -> PyResult<PySnapshotInfo> {
    snapshot_info(bytes, SnapshotKind::Generic)
}

/// Opens a CSR graph snapshot.
#[pyfunction]
fn open_csr_snapshot(bytes: &[u8]) -> PyResult<PySnapshotInfo> {
    snapshot_info(bytes, SnapshotKind::Csr)
}

/// Opens a BCSR hypergraph snapshot.
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
    module.add_class::<PyGraphBuilder>()?;
    module.add_class::<PyFrozenGraph>()?;
    module.add_class::<PyHypergraphBuilder>()?;
    module.add_class::<PyFrozenHypergraph>()?;
    module.add_class::<PySnapshotInfo>()?;
    module.add_function(wrap_pyfunction!(open_snapshot, module)?)?;
    module.add_function(wrap_pyfunction!(open_csr_snapshot, module)?)?;
    module.add_function(wrap_pyfunction!(open_bcsr_snapshot, module)?)?;
    Ok(())
}

/// Converts Rust graph builder errors into Python graph errors.
fn py_graph_error(error: PythonGraphBuildError) -> PyErr {
    GraphError::new_err(error.to_string())
}

/// Converts Rust hypergraph builder errors into Python hypergraph errors.
fn py_hyper_error(error: PythonHyperBuildError) -> PyErr {
    HypergraphError::new_err(error.to_string())
}

/// Converts Rust `PageRank` errors into Python `PageRank` errors.
fn py_pagerank_error(error: PageRankError<f64>) -> PyErr {
    PageRankPythonError::new_err(error.to_string())
}

/// Converts Rust snapshot errors into Python snapshot errors.
fn py_snapshot_error<E: fmt::Display>(error: E) -> PyErr {
    SnapshotPythonError::new_err(error.to_string())
}

/// Builds a graph error from a message.
fn graph_message(message: String) -> PyErr {
    GraphError::new_err(message)
}

/// Builds a hypergraph error from a message.
fn hyper_message(message: String) -> PyErr {
    HypergraphError::new_err(message)
}

/// Builds a `PageRank` config from optional Python arguments.
fn pagerank_config(
    damping: Option<f64>,
    tolerance: Option<f64>,
    max_iterations: Option<usize>,
) -> PageRankConfig<f64> {
    PageRankConfig::new(
        damping.unwrap_or(DEFAULT_PAGERANK_DAMPING),
        tolerance.unwrap_or(DEFAULT_PAGERANK_TOLERANCE),
        max_iterations.unwrap_or(DEFAULT_PAGERANK_MAX_ITERATIONS),
    )
}

/// Checks a frozen graph node ID.
fn ensure_graph_node(graph: &PythonFrozenGraph, node: u32) -> PyResult<()> {
    if graph.contains_element(GraphNodeId::new(node)) {
        Ok(())
    } else {
        Err(GraphError::new_err(format!("invalid graph node ID {node}")))
    }
}

/// Checks a frozen graph edge ID.
fn ensure_graph_edge(graph: &PythonFrozenGraph, edge: u32) -> PyResult<()> {
    if graph.contains_relation(GraphEdgeId::new(edge)) {
        Ok(())
    } else {
        Err(GraphError::new_err(format!("invalid graph edge ID {edge}")))
    }
}

/// Checks a frozen hypergraph vertex ID.
fn ensure_hyper_vertex(hypergraph: &PythonFrozenHypergraph, vertex: u32) -> PyResult<()> {
    if hypergraph.contains_element(HyperVertexId::new(vertex)) {
        Ok(())
    } else {
        Err(HypergraphError::new_err(format!(
            "invalid hypergraph vertex ID {vertex}"
        )))
    }
}

/// Checks a frozen hypergraph relation ID.
fn ensure_hyperedge(hypergraph: &PythonFrozenHypergraph, hyperedge: u32) -> PyResult<()> {
    if hypergraph.contains_relation(HyperedgeId::new(hyperedge)) {
        Ok(())
    } else {
        Err(HypergraphError::new_err(format!(
            "invalid hyperedge ID {hyperedge}"
        )))
    }
}

/// Checks a frozen hypergraph incidence ID.
fn ensure_hyper_incidence(hypergraph: &PythonFrozenHypergraph, incidence: u32) -> PyResult<()> {
    if hypergraph.contains_incidence(HyperParticipantId::new(incidence)) {
        Ok(())
    } else {
        Err(HypergraphError::new_err(format!(
            "invalid hypergraph incidence ID {incidence}"
        )))
    }
}

/// Enumerates frozen graph elements by dense first-generation ID.
fn graph_elements(graph: &PythonFrozenGraph) -> Vec<GraphNodeId<u32>> {
    dense_ids(graph.node_count())
        .into_iter()
        .map(GraphNodeId::new)
        .collect()
}

/// Enumerates frozen hypergraph elements by dense first-generation ID.
fn hyper_elements(hypergraph: &PythonFrozenHypergraph) -> Vec<HyperVertexId<u32>> {
    dense_ids(hypergraph.vertex_count())
        .into_iter()
        .map(HyperVertexId::new)
        .collect()
}

/// Enumerates frozen hypergraph relations by dense first-generation ID.
fn hyper_relations(hypergraph: &PythonFrozenHypergraph) -> Vec<HyperedgeId<u32>> {
    dense_ids(hypergraph.hyperedge_count())
        .into_iter()
        .map(HyperedgeId::new)
        .collect()
}

/// Enumerates dense Python local IDs.
fn dense_ids(count: usize) -> Vec<u32> {
    (0..count).map(dense_index_to_u32).collect()
}

/// Converts dense builder indexes into `u32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Python frozen views are produced by u32-ID builders, so dense counts fit u32"
)]
const fn dense_index_to_u32(index: usize) -> u32 {
    index as u32
}

/// Builds weighted hypergraph participant pairs from Python vertex and weight arrays.
fn weighted_vertices(
    vertices: Vec<u32>,
    weights: Option<Vec<f64>>,
    name: &str,
) -> PyResult<Vec<(HyperVertexId<u32>, f64)>> {
    match weights {
        Some(values) => {
            if values.len() != vertices.len() {
                return Err(HypergraphError::new_err(format!(
                    "{name} length {} does not match vertex length {}",
                    values.len(),
                    vertices.len()
                )));
            }
            Ok(vertices
                .into_iter()
                .zip(values)
                .map(|(vertex, weight)| (HyperVertexId::new(vertex), weight))
                .collect())
        }
        None => Ok(vertices
            .into_iter()
            .map(|vertex| (HyperVertexId::new(vertex), DEFAULT_INCIDENCE_WEIGHT))
            .collect()),
    }
}

/// Opens and validates snapshot sections according to `kind`.
fn snapshot_info(bytes: &[u8], kind: SnapshotKind) -> PyResult<PySnapshotInfo> {
    let snapshot = Snapshot::open(bytes).map_err(py_snapshot_error)?;
    match kind {
        SnapshotKind::Generic => {}
        SnapshotKind::Csr => {
            let _graph = CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot)
                .map_err(py_snapshot_error)?;
        }
        SnapshotKind::Bcsr => {
            let _hypergraph = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot_with(
                &snapshot,
                BcsrValidation::Strict,
            )
            .map_err(py_snapshot_error)?;
        }
    }
    Ok(PySnapshotInfo {
        section_count: snapshot.section_count(),
        section_kinds: snapshot.sections().map(|section| section.kind()).collect(),
    })
}

//! Substrate-agnostic graph algorithms over storage-agnostic topology traits.
//!
//! `oxgraph-algo` contains algorithms whose semantics generalize across
//! ordinary binary graphs and hypergraphs. BFS binds only to capability traits
//! from `oxgraph-topology`; `PageRank` additionally uses ordinary-graph and
//! hypergraph vocabulary traits from `oxgraph-graph` and `oxgraph-hyper` while
//! still avoiding concrete storage and Arrow/property dependencies. The same
//! compiled algorithms run on `oxgraph-csr::CsrGraph`,
//! `oxgraph-hyper-bcsr::BcsrHypergraph`, and any future view that exposes the
//! required capabilities.
//!
//! The default APIs are allocation-free: callers provide reusable scratch
//! storage for traversal state. Allocating convenience wrappers are available
//! behind the `alloc` feature. Standard-library optimized generic wrappers are
//! available behind the `std` feature.
//!
//! Each variant ships in two directions. Forward entry points expand through
//! [`ElementSuccessors`](oxgraph_topology::ElementSuccessors); reverse entry
//! points expand through
//! [`ElementPredecessors`](oxgraph_topology::ElementPredecessors). Both
//! directions reuse the same storage policies, so a single
//! [`BfsEpochScratch`] / [`BfsWorkspace`] serves either traversal direction.
//!
//! # BFS tiers
//!
//! | Feature | Forward API | Reverse API | Topology requirement | Performance |
//! | --- | --- | --- | --- | --- |
//! | none | [`breadth_first_search_with_scratch`] | [`reverse_breadth_first_search_with_scratch`] | dense [`ElementIndex`](oxgraph_topology::ElementIndex) + direction trait | `O(b)` setup, `O(n + m)` traversal, no allocation |
//! | none | [`breadth_first_search_with_epoch_scratch`] | [`reverse_breadth_first_search_with_epoch_scratch`] | dense [`ElementIndex`](oxgraph_topology::ElementIndex) + direction trait | amortized `O(1)` setup, `O(n + m)` traversal, no allocation |
//! | `alloc` | [`breadth_first_search`] | [`reverse_breadth_first_search`] | dense [`ElementIndex`](oxgraph_topology::ElementIndex) + direction trait | `O(b)` setup, `O(n + m)` traversal, owned storage |
//! | `alloc` | [`breadth_first_search_with_workspace`] | [`reverse_breadth_first_search_with_workspace`] | dense [`ElementIndex`](oxgraph_topology::ElementIndex) + direction trait | amortized `O(1)` setup, `O(n + m)` traversal, reusable owned storage |
//! | `alloc` | [`breadth_first_search_generic`] | [`reverse_breadth_first_search_generic`] | arbitrary element IDs | `O((n + m) log n)` traversal with `BTreeSet` membership |
//! | `std` | [`breadth_first_search_generic_hash`] | [`reverse_breadth_first_search_generic_hash`] | arbitrary element IDs | expected `O(n + m)` traversal with `HashSet` membership |
//!
//! Forward BFS follows
//! [`ElementSuccessors`](oxgraph_topology::ElementSuccessors); reverse BFS
//! follows
//! [`ElementPredecessors`](oxgraph_topology::ElementPredecessors). Both
//! traverse element-to-element directly without materializing intermediate
//! relation IDs.
//!
//! `b` is `graph.element_bound()`, `n` is the number of elements yielded, and
//! `m` is the number of expansion-target entries inspected.
#![no_std]

#[cfg(kani)]
extern crate kani;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod bfs;
pub mod pagerank;

#[cfg(feature = "alloc")]
pub mod components;
#[cfg(feature = "alloc")]
pub mod longest_path_dag;
#[cfg(feature = "alloc")]
pub mod scc;
#[cfg(feature = "alloc")]
pub mod sssp;
#[cfg(feature = "alloc")]
pub mod toposort;

pub use bfs::{
    BfsBounds, BfsEpochScratch, BfsError, BfsVisitor, BreadthFirstSearchEpochScratch,
    BreadthFirstSearchScratch, ReverseBreadthFirstSearchEpochScratch,
    ReverseBreadthFirstSearchScratch, breadth_first_search_bounded,
    breadth_first_search_bounded_both, breadth_first_search_with_epoch_scratch,
    breadth_first_search_with_scratch, reverse_breadth_first_search_bounded,
    reverse_breadth_first_search_with_epoch_scratch, reverse_breadth_first_search_with_scratch,
};
#[cfg(feature = "alloc")]
pub use bfs::{
    BfsWorkspace, BreadthFirstSearch, BreadthFirstSearchWorkspace, GenericBreadthFirstSearch,
    GenericReverseBreadthFirstSearch, ReverseBreadthFirstSearch,
    ReverseBreadthFirstSearchWorkspace, breadth_first_search, breadth_first_search_generic,
    breadth_first_search_with_workspace, reverse_breadth_first_search,
    reverse_breadth_first_search_generic, reverse_breadth_first_search_with_workspace,
};
#[cfg(feature = "std")]
pub use bfs::{
    HashBreadthFirstSearch, HashReverseBreadthFirstSearch, breadth_first_search_generic_hash,
    reverse_breadth_first_search_generic_hash,
};
#[cfg(feature = "alloc")]
pub use components::connected_components;
#[cfg(feature = "alloc")]
pub use longest_path_dag::{LongestPathError, longest_path_dag};
pub use pagerank::{
    HyperWeighted, HypergraphOutgoingDistribution, HypergraphPageRankScratch, IntoPageRankScalar,
    OutgoingDistribution, PageRankConfig, PageRankError, PageRankHypergraph, PageRankReport,
    PageRankScalar, PageRankScratch, Uniform, Weighted, pagerank_graph_with_scratch,
    pagerank_hypergraph_with_scratch,
};
#[cfg(feature = "alloc")]
pub use pagerank::{
    HypergraphPageRankWorkspace, PageRankWorkspace, pagerank_graph, pagerank_graph_with_workspace,
    pagerank_hypergraph, pagerank_hypergraph_with_workspace,
};
#[cfg(feature = "alloc")]
pub use scc::strongly_connected_components;
#[cfg(feature = "alloc")]
pub use sssp::shortest_path_lengths;
#[cfg(feature = "alloc")]
pub use toposort::{ToposortError, topological_sort};

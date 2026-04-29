//! Graph algorithms over storage-agnostic graph traits.
//!
//! `oxgraph-algo` contains algorithms whose semantics are graph-specific. The
//! algorithms depend on capability traits from `oxgraph-graph`, not on concrete
//! storage crates such as `oxgraph-csr` or `oxgraph-snapshot`.
//!
//! The default APIs are allocation-free: callers provide reusable scratch
//! storage for traversal state. Allocating convenience wrappers are available
//! behind the `alloc` feature. Standard-library optimized generic wrappers are
//! available behind the `std` feature.
//!
//! # BFS tiers
//!
//! | Feature | API | Graph requirement | Performance |
//! | --- | --- | --- | --- |
//! | none | [`breadth_first_search_with_scratch`] | dense [`NodeIndex`](oxgraph_graph::NodeIndex) | `O(b)` setup, `O(n + m)` traversal, no allocation |
//! | none | [`breadth_first_search_with_epoch_scratch`] | dense [`NodeIndex`](oxgraph_graph::NodeIndex) | amortized `O(1)` setup, `O(n + m)` traversal, no allocation |
//! | `alloc` | [`breadth_first_search`] | dense [`NodeIndex`](oxgraph_graph::NodeIndex) | `O(b)` setup, `O(n + m)` traversal, owned storage |
//! | `alloc` | [`breadth_first_search_with_workspace`] | dense [`NodeIndex`](oxgraph_graph::NodeIndex) | amortized `O(1)` setup, `O(n + m)` traversal, reusable owned storage |
//! | `alloc` | [`breadth_first_search_generic`] | arbitrary node IDs | `O((n + m) log n)` traversal with `BTreeSet` membership |
//! | `std` | [`breadth_first_search_generic_hash`] | arbitrary node IDs | expected `O(n + m)` traversal with `HashSet` membership |
//!
//! BFS follows [`OutgoingNeighborsGraph`](oxgraph_graph::OutgoingNeighborsGraph)
//! directly instead of materializing outgoing edge IDs and resolving each edge
//! target.
//!
//! `b` is `graph.node_bound()`, `n` is the number of reachable nodes yielded,
//! and `m` is the number of outgoing neighbor entries inspected.
#![no_std]

#[cfg(kani)]
extern crate kani;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod bfs;

pub use bfs::{
    BfsEpochScratch, BfsError, BreadthFirstSearchEpochScratch, BreadthFirstSearchScratch,
    breadth_first_search_with_epoch_scratch, breadth_first_search_with_scratch,
};
#[cfg(feature = "alloc")]
pub use bfs::{
    BfsWorkspace, BreadthFirstSearch, BreadthFirstSearchWorkspace, GenericBreadthFirstSearch,
    breadth_first_search, breadth_first_search_generic, breadth_first_search_with_workspace,
};
#[cfg(feature = "std")]
pub use bfs::{HashBreadthFirstSearch, breadth_first_search_generic_hash};

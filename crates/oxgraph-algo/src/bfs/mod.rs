//! Breadth-first traversal implementations (forward and reverse).
//!
//! This module exposes the BFS tier model:
//!
//! - strict `no_std` callers use scratch-backed indexed traversal;
//! - `alloc` callers can use owned indexed storage or a reusable workspace;
//! - `alloc` callers without dense element indexes can use a `BTreeSet` fallback;
//! - `std` callers can use a `HashSet` generic traversal for expected `O(n + m)` membership checks.
//!
//! Every variant ships as a forward `breadth_first_search_*` and a reverse
//! `reverse_breadth_first_search_*` entry point. Forward variants bind on
//! [`ElementSuccessors`](oxgraph_topology::ElementSuccessors); reverse
//! variants bind on
//! [`ElementPredecessors`](oxgraph_topology::ElementPredecessors). Both
//! directions reuse the same storage policies, so reusable scratch and
//! workspace types serve either traversal direction.
//!
//! Each variant is exposed as an opaque iterator type (e.g.
//! [`BreadthFirstSearchScratch`] / [`ReverseBreadthFirstSearchScratch`])
//! constructed through a variant-specific entry point. The internal storage
//! policy is shared across variants but is not part of the public API.

mod bounded;
mod core;
mod epoch;
mod error;
mod scratch;

#[cfg(feature = "alloc")]
mod generic_set;
#[cfg(feature = "alloc")]
mod indexed;
#[cfg(feature = "alloc")]
mod workspace;

pub use bounded::{
    BfsBounds, BfsVisitor, breadth_first_search_bounded, breadth_first_search_bounded_both,
    reverse_breadth_first_search_bounded,
};
pub use epoch::{
    BfsEpochScratch, BreadthFirstSearchEpochScratch, ReverseBreadthFirstSearchEpochScratch,
    breadth_first_search_with_epoch_scratch, reverse_breadth_first_search_with_epoch_scratch,
};
pub use error::BfsError;
#[cfg(feature = "alloc")]
pub use generic_set::{
    GenericBreadthFirstSearch, GenericReverseBreadthFirstSearch, breadth_first_search_generic,
    reverse_breadth_first_search_generic,
};
#[cfg(feature = "std")]
pub use generic_set::{
    HashBreadthFirstSearch, HashReverseBreadthFirstSearch, breadth_first_search_generic_hash,
    reverse_breadth_first_search_generic_hash,
};
#[cfg(feature = "alloc")]
pub use indexed::{
    BreadthFirstSearch, ReverseBreadthFirstSearch, breadth_first_search,
    reverse_breadth_first_search,
};
pub use scratch::{
    BreadthFirstSearchScratch, ReverseBreadthFirstSearchScratch, breadth_first_search_with_scratch,
    reverse_breadth_first_search_with_scratch,
};
#[cfg(feature = "alloc")]
pub use workspace::{
    BfsWorkspace, BreadthFirstSearchWorkspace, ReverseBreadthFirstSearchWorkspace,
    breadth_first_search_with_workspace, reverse_breadth_first_search_with_workspace,
};

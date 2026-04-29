//! Directed breadth-first traversal implementations.
//!
//! This module exposes the BFS tier model:
//!
//! - strict `no_std` callers use scratch-backed indexed traversal;
//! - `alloc` callers can use owned indexed storage or a reusable workspace;
//! - `alloc` callers without dense node indexes can use a `BTreeSet` fallback;
//! - `std` callers can use a `HashSet` generic traversal for expected `O(n + m)` membership checks.
//!
//! Each variant is exposed as an opaque iterator type (e.g.
//! [`BreadthFirstSearchScratch`]) constructed through a variant-specific
//! `breadth_first_search_*` entry point. The internal storage policy is shared
//! across variants but is not part of the public API.

mod core;
mod epoch;
mod error;
mod scratch;

#[cfg(feature = "alloc")]
mod generic_btree;
#[cfg(feature = "std")]
mod generic_hash;
#[cfg(feature = "alloc")]
mod indexed;
#[cfg(feature = "alloc")]
mod workspace;

pub use epoch::{
    BfsEpochScratch, BreadthFirstSearchEpochScratch, breadth_first_search_with_epoch_scratch,
};
pub use error::BfsError;
#[cfg(feature = "alloc")]
pub use generic_btree::{GenericBreadthFirstSearch, breadth_first_search_generic};
#[cfg(feature = "std")]
pub use generic_hash::{HashBreadthFirstSearch, breadth_first_search_generic_hash};
#[cfg(feature = "alloc")]
pub use indexed::{BreadthFirstSearch, breadth_first_search};
pub use scratch::{BreadthFirstSearchScratch, breadth_first_search_with_scratch};
#[cfg(feature = "alloc")]
pub use workspace::{
    BfsWorkspace, BreadthFirstSearchWorkspace, breadth_first_search_with_workspace,
};

//! Zero-copy query traversal (session, profile-dispatched BFS kernel).
#![expect(
    clippy::redundant_pub_crate,
    reason = "kernel entrypoints are crate-internal but live in a private parent module"
)]

mod kernel;
mod profile;
mod scratch;
mod session;

use core::num::{NonZeroU32, NonZeroUsize};

use kernel::{KernelOutcome, run_bfs_multi};
use profile::TraverseMode;
#[expect(
    clippy::redundant_pub_crate,
    reason = "re-exported to engine and builder"
)]
pub(crate) use scratch::TraverseScratch;
use session::TraverseSession;

use crate::{config::Config, error::PostgresGraphError};

/// Traversal direction matching pgGraph `direction` semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraversalDirection {
    /// Follow CSR outgoing adjacency.
    #[default]
    Out,
    /// Follow inbound CSC adjacency.
    In,
}

/// Limits for a single traverse query (result cap and optional hop bound).
///
/// ## Semantics
///
/// - `result_limit`: maximum nodes returned or counted; each seed counts toward the cap.
/// - `max_depth`: when `Some(d)`, discovers all nodes within `d` hops of a seed (boundary nodes are
///   counted but not enqueued past the depth cap). When `None`, expansion is unbounded by depth
///   (still capped by `result_limit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraverseLimits {
    /// Maximum nodes returned (each seed counts toward the cap).
    pub result_limit: NonZeroUsize,
    /// `None` = unlimited hops; `Some(d)` discovers nodes within `d` hops of a seed.
    pub max_depth: Option<NonZeroU32>,
}

impl TraverseLimits {
    /// Creates limits with only a result cap.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn bounded(result_limit: NonZeroUsize) -> Self {
        Self {
            result_limit,
            max_depth: None,
        }
    }

    /// Applies configured traverse cap from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Query`] when the capped limit is zero.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub fn capped_by(self, config: &Config) -> Result<Self, PostgresGraphError> {
        let cap = config.traverse_limit as usize;
        let capped = core::cmp::min(self.result_limit.get(), cap);
        let result_limit = NonZeroUsize::new(capped).ok_or(crate::error::QueryError::LimitZero)?;
        Ok(Self {
            result_limit,
            max_depth: self.max_depth,
        })
    }
}

/// Runs traversal through the shared BFS kernel.
pub(crate) fn traverse_core(
    engine: &mut crate::engine::Engine,
    seeds: &[u32],
    limits: TraverseLimits,
    direction: TraversalDirection,
    mode: TraverseMode,
) -> Result<KernelTraversalOutcome, PostgresGraphError> {
    let session = TraverseSession::open(engine, seeds, limits, direction, mode)?;
    match run_bfs_multi(engine, &session) {
        KernelOutcome::Nodes(nodes) => Ok(KernelTraversalOutcome::Nodes(nodes)),
        KernelOutcome::Count(count) => Ok(KernelTraversalOutcome::Count(count)),
    }
}

/// Outcome of a kernel traversal before mode validation.
pub(crate) enum KernelTraversalOutcome {
    /// Collect mode node ids in BFS first-discovery order.
    Nodes(alloc::vec::Vec<u32>),
    /// Count-only mode cardinality.
    Count(usize),
}

/// Collect-mode traversal through the shared kernel.
pub(crate) fn traverse_core_collect(
    engine: &mut crate::engine::Engine,
    seeds: &[u32],
    limits: TraverseLimits,
    direction: TraversalDirection,
) -> Result<alloc::vec::Vec<u32>, PostgresGraphError> {
    match traverse_core(engine, seeds, limits, direction, TraverseMode::Collect)? {
        KernelTraversalOutcome::Nodes(nodes) => Ok(nodes),
        KernelTraversalOutcome::Count(_) => Err(crate::error::QueryError::InternalInvariant(
            "expected nodes from collect traversal",
        )
        .into()),
    }
}

/// Count-only traversal through the shared kernel.
pub(crate) fn traverse_core_count(
    engine: &mut crate::engine::Engine,
    seeds: &[u32],
    limits: TraverseLimits,
    direction: TraversalDirection,
) -> Result<usize, PostgresGraphError> {
    match traverse_core(engine, seeds, limits, direction, TraverseMode::Count)? {
        KernelTraversalOutcome::Count(count) => Ok(count),
        KernelTraversalOutcome::Nodes(_) => Err(crate::error::QueryError::InternalInvariant(
            "expected count from count-only traversal",
        )
        .into()),
    }
}

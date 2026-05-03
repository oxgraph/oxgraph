//! Error types shared by indexed BFS implementations.

use core::fmt;

/// Error returned when caller-provided BFS scratch cannot support traversal.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only before traversal starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BfsError {
    /// The start element is not valid and visible in the topology view.
    StartElementNotContained,
    /// The visited scratch slice is smaller than `graph.element_bound()`.
    VisitedTooSmall {
        /// Required visited entries.
        needed: usize,
        /// Provided visited entries.
        actual: usize,
    },
    /// The queue scratch slice is smaller than `graph.element_bound()`.
    QueueTooSmall {
        /// Required queue entries.
        needed: usize,
        /// Provided queue entries.
        actual: usize,
    },
    /// The start element maps outside `graph.element_bound()`.
    StartIndexOutOfBounds {
        /// Dense index returned for the start element.
        index: usize,
        /// Exclusive element index bound for the topology view.
        bound: usize,
    },
    /// Traversal observed a successor or predecessor element whose dense index
    /// is at or past `graph.element_bound()`. Indicates the topology view
    /// violated its
    /// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors) or
    /// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors)
    /// contract: an expanded element ID must map below the element bound that
    /// was in effect when traversal started.
    NeighborIndexOutOfBounds {
        /// Dense index returned for the offending element.
        index: usize,
        /// Exclusive element index bound that was in effect for this traversal.
        bound: usize,
    },
}

impl fmt::Display for BfsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartElementNotContained => {
                formatter.write_str("start element is not contained in the topology view")
            }
            Self::VisitedTooSmall { needed, actual } => write!(
                formatter,
                "visited scratch is too small: needed {needed}, got {actual}"
            ),
            Self::QueueTooSmall { needed, actual } => write!(
                formatter,
                "queue scratch is too small: needed {needed}, got {actual}"
            ),
            Self::StartIndexOutOfBounds { index, bound } => write!(
                formatter,
                "start element index {index} is outside element index bound {bound}"
            ),
            Self::NeighborIndexOutOfBounds { index, bound } => write!(
                formatter,
                "expanded element index {index} is outside element index bound {bound}"
            ),
        }
    }
}

impl core::error::Error for BfsError {}

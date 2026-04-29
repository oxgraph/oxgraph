//! Error types shared by indexed BFS implementations.

use core::fmt;

/// Error returned when caller-provided BFS scratch cannot support traversal.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only before traversal starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BfsError {
    /// The start node is not valid and visible in the graph view.
    StartNodeNotContained,
    /// The visited scratch slice is smaller than `graph.node_bound()`.
    VisitedTooSmall {
        /// Required visited entries.
        needed: usize,
        /// Provided visited entries.
        actual: usize,
    },
    /// The queue scratch slice is smaller than `graph.node_bound()`.
    QueueTooSmall {
        /// Required queue entries.
        needed: usize,
        /// Provided queue entries.
        actual: usize,
    },
    /// The start node maps outside `graph.node_bound()`.
    StartIndexOutOfBounds {
        /// Dense index returned for the start node.
        index: usize,
        /// Exclusive node index bound for the graph view.
        bound: usize,
    },
    /// Traversal observed an outgoing neighbor whose dense index is at or past
    /// `graph.node_bound()`. Indicates the graph view violated its
    /// [`OutgoingNeighborsGraph`](oxgraph_graph::OutgoingNeighborsGraph)
    /// contract: a neighbor ID must map below the node bound that was in effect
    /// when traversal started.
    NeighborIndexOutOfBounds {
        /// Dense index returned for the offending neighbor.
        index: usize,
        /// Exclusive node index bound that was in effect for this traversal.
        bound: usize,
    },
}

impl fmt::Display for BfsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartNodeNotContained => {
                formatter.write_str("start node is not contained in the graph view")
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
                "start node index {index} is outside node index bound {bound}"
            ),
            Self::NeighborIndexOutOfBounds { index, bound } => write!(
                formatter,
                "neighbor node index {index} is outside node index bound {bound}"
            ),
        }
    }
}

impl core::error::Error for BfsError {}

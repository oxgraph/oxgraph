//! Search query types.

/// Search predicate over dense node ids (extension hydrates external columns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPredicate {
    /// Match a single dense node id.
    NodeId(u32),
    /// Match nodes whose id is within an inclusive range.
    NodeIdRange {
        /// Lower bound.
        start: u32,
        /// Upper bound.
        end: u32,
    },
}

impl SearchPredicate {
    /// Returns whether `node` satisfies this predicate.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn matches(self, node: u32) -> bool {
        match self {
            Self::NodeId(expected) => node == expected,
            Self::NodeIdRange { start, end } => (start..=end).contains(&node),
        }
    }
}

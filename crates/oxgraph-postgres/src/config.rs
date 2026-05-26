//! Operational configuration mirrored from extension GUCs.

use crate::error::{ConfigError, PostgresGraphError};

/// Freshness policy when overlays and sync rows are present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryFreshness {
    /// Read the frozen base artifact only (skip overlay edge inserts) while still
    /// honoring node tombstones recorded in the overlay.
    BaseOnly,
    /// Apply overlays and sync replay before each query.
    #[default]
    OverlayAware,
}

/// Library-side operational configuration (GUC values are registered in `oxgraph-pgrx`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Maximum BFS expansion steps per traverse call.
    pub traverse_limit: u32,
    /// Maximum rows returned from search.
    pub search_limit: u32,
    /// Overlay/sync freshness policy.
    pub query_freshness: QueryFreshness,
    /// Whether maintenance rebuilds are permitted.
    pub maintenance_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            traverse_limit: 10_000,
            search_limit: 10_000,
            query_freshness: QueryFreshness::OverlayAware,
            maintenance_enabled: true,
        }
    }
}

impl Config {
    /// Validates limits and returns an error for zero limits.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Config`] when a limit is zero.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub fn validate(&self) -> Result<(), PostgresGraphError> {
        if self.traverse_limit == 0 {
            return Err(ConfigError::ZeroTraverseLimit.into());
        }
        if self.search_limit == 0 {
            return Err(ConfigError::ZeroSearchLimit.into());
        }
        Ok(())
    }
}

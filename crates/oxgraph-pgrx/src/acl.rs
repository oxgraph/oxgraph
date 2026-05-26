//! Extension-side ACL checks delegating policy to the library.

use oxgraph_postgres::{GraphRole, PostgresGraphError};
use pgrx::pg_sys;

use crate::gucs;

/// Returns the effective graph role for the current backend session.
///
/// Superusers are always [`GraphRole::Admin`]. Other sessions honor
/// `oxgraph.graph_role` (`0` reader, `1` admin).
///
/// # Performance
///
/// This function is `O(1)`.
#[must_use]
pub(crate) fn current_role() -> GraphRole {
    // SAFETY: called from an active Postgres backend during statement execution.
    let is_super = unsafe { pg_sys::superuser() };
    if is_super {
        return GraphRole::Admin;
    }
    match gucs::graph_role_ordinal() {
        1 => GraphRole::Admin,
        _ => GraphRole::Reader,
    }
}

/// Ensures the current session satisfies the required graph role.
///
/// # Errors
///
/// Returns [`PostgresGraphError::AccessDenied`] when policy denies the operation.
pub(crate) fn ensure_role(required: GraphRole) -> Result<(), PostgresGraphError> {
    current_role().require(required)
}

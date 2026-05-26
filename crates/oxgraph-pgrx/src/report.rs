//! Maps library errors to Postgres `ERROR` responses.

use oxgraph_postgres::{GraphRole, PostgresGraphError};

use crate::{acl, session::log_error};

/// Ensures the session has admin role, logging access denials.
///
/// # Errors
///
/// Returns [`PostgresGraphError::AccessDenied`] when the role is insufficient.
pub(crate) fn ensure_admin() -> Result<(), PostgresGraphError> {
    match acl::ensure_role(GraphRole::Admin) {
        Err(error @ PostgresGraphError::AccessDenied { .. }) => {
            pgrx::log!("oxgraph: {error}");
            Err(error)
        }
        other => other,
    }
}

/// Ensures admin role or raises a SQL error.
pub(crate) fn ensure_admin_or_raise() {
    if let Err(error) = ensure_admin() {
        raise(error);
    }
}

/// Logs and raises a SQL error for `error`.
pub(crate) fn raise(error: PostgresGraphError) -> ! {
    log_error(&error);
    pgrx::error!("{error}");
}

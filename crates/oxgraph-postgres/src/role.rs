//! Access-control roles for graph admin and query surfaces.

use crate::error::PostgresGraphError;

/// Role required for graph operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphRole {
    /// Read-only graph queries.
    Reader,
    /// Registration, build, sync, and maintenance.
    Admin,
}

impl GraphRole {
    /// Returns Ok when `self` satisfies `required`.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::AccessDenied`] when the role is insufficient.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub const fn satisfies(self, required: Self) -> Result<(), PostgresGraphError> {
        match (self, required) {
            (Self::Admin, _) | (Self::Reader, Self::Reader) => Ok(()),
            (Self::Reader, Self::Admin) => Err(PostgresGraphError::AccessDenied {
                required,
                actual: self,
            }),
        }
    }

    /// Returns Ok when `self` satisfies `required`.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::AccessDenied`] when the role is insufficient.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub const fn require(self, required: Self) -> Result<(), PostgresGraphError> {
        self.satisfies(required)
    }
}

#[cfg(kani)]
mod proofs {
    use super::GraphRole;

    /// Admin satisfies every required role.
    #[kani::proof]
    fn admin_satisfies_all() {
        let admin = GraphRole::Admin;
        assert!(admin.satisfies(GraphRole::Reader).is_ok());
        assert!(admin.satisfies(GraphRole::Admin).is_ok());
    }

    /// Reader satisfies reader but not admin.
    #[kani::proof]
    fn reader_lattice() {
        let reader = GraphRole::Reader;
        assert!(reader.satisfies(GraphRole::Reader).is_ok());
        assert!(reader.satisfies(GraphRole::Admin).is_err());
    }
}

//! Canonical database identity newtypes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::IdFamily;

/// Declares one canonical `u64` identifier newtype.
macro_rules! id_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// # Performance
        ///
        /// Copying, comparing, ordering, and hashing are `O(1)`.
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[repr(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Creates an identifier from a raw canonical value.
            ///
            /// # Performance
            ///
            /// This function is `O(1)`.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the raw canonical value.
            ///
            /// # Performance
            ///
            /// This function is `O(1)`.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }

            /// Returns the next identifier if it fits.
            ///
            /// # Performance
            ///
            /// This function is `O(1)`.
            #[must_use]
            pub const fn checked_next(self) -> Option<Self> {
                match self.0.checked_add(1) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

/// Wires one canonical id newtype to its [`IdFamily`] discriminant, so error
/// constructors can take any family id generically.
macro_rules! id_family {
    ($name:ident, $family:ident) => {
        impl From<$name> for (IdFamily, u64) {
            fn from(id: $name) -> Self {
                (IdFamily::$family, id.get())
            }
        }
    };
}

id_newtype!(ElementId, "Stable canonical element identifier.");
id_newtype!(RelationId, "Stable canonical relation identifier.");
id_newtype!(IncidenceId, "Stable canonical incidence identifier.");
id_newtype!(RoleId, "Stable canonical structural role identifier.");
id_newtype!(LabelId, "Stable catalog label identifier.");
id_newtype!(RelationTypeId, "Stable catalog relation-type identifier.");
id_newtype!(PropertyKeyId, "Stable catalog property-key identifier.");
id_newtype!(ProjectionId, "Stable catalog projection identifier.");
id_newtype!(IndexId, "Stable catalog index identifier.");
id_family!(ElementId, Element);
id_family!(RelationId, Relation);
id_family!(IncidenceId, Incidence);
id_family!(RoleId, Role);
id_family!(LabelId, Label);
id_family!(RelationTypeId, RelationType);
id_family!(PropertyKeyId, PropertyKey);
id_family!(ProjectionId, Projection);
id_family!(IndexId, Index);

id_newtype!(CommitSeq, "Monotonic committed transaction sequence.");
id_newtype!(TransactionId, "Monotonic writer transaction identifier.");
id_newtype!(
    CheckpointGeneration,
    "Immutable checkpoint generation identifier."
);

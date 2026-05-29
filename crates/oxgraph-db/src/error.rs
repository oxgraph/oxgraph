//! Database error surface.

use std::{fmt, io};

use crate::{
    ElementId, IncidenceId, IndexId, LabelId, ProjectionId, PropertyKeyId, RelationId,
    RelationTypeId, RoleId, catalog::PropertyFamily, value::PropertyType,
};

/// Errors raised by the `OxGraph` database product.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum DbError {
    /// Database files already exist.
    AlreadyExists,
    /// Database files do not exist.
    NotFound,
    /// Canonical ID space is exhausted.
    IdOverflow,
    /// Transaction ID space is exhausted.
    TransactionIdOverflow,
    /// Commit sequence space is exhausted.
    CommitSeqOverflow,
    /// Duplicate catalog name or ID.
    DuplicateCatalogName,
    /// Duplicate canonical ID.
    DuplicateId,
    /// Unknown element ID.
    UnknownElement {
        /// Missing element ID.
        id: ElementId,
    },
    /// Unknown relation ID.
    UnknownRelation {
        /// Missing relation ID.
        id: RelationId,
    },
    /// Unknown incidence ID.
    UnknownIncidence {
        /// Missing incidence ID.
        id: IncidenceId,
    },
    /// Unknown role ID.
    UnknownRole {
        /// Missing role ID.
        id: RoleId,
    },
    /// Unknown label ID.
    UnknownLabel {
        /// Missing label ID.
        id: LabelId,
    },
    /// Unknown relation type ID.
    UnknownRelationType {
        /// Missing relation type ID.
        id: RelationTypeId,
    },
    /// Unknown property key ID.
    UnknownPropertyKey {
        /// Missing property key ID.
        id: PropertyKeyId,
    },
    /// Unknown projection ID.
    UnknownProjection {
        /// Missing projection ID.
        id: ProjectionId,
    },
    /// Unknown index ID.
    UnknownIndex {
        /// Missing index ID.
        id: IndexId,
    },
    /// Property value type mismatched the catalog schema.
    PropertyTypeMismatch {
        /// Expected property type.
        expected: PropertyType,
        /// Actual property type.
        actual: PropertyType,
    },
    /// Property subject family mismatched the catalog schema.
    WrongPropertyFamily {
        /// Expected subject family.
        expected: PropertyFamily,
        /// Actual subject family.
        actual: PropertyFamily,
    },
    /// Projection cannot be materialized as requested.
    InvalidProjection {
        /// Deterministic validation message.
        message: String,
    },
    /// Query text is empty.
    EmptyQuery,
    /// Query text is outside the pinned profile.
    UnsupportedQuery {
        /// Deterministic explanation.
        message: String,
    },
    /// Storage bytes are invalid.
    InvalidStore {
        /// Deterministic validation message.
        message: String,
    },
    /// Wraps an IO error with operation context.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Underlying IO error.
        source: io::Error,
    },
    /// Wraps a JSON codec error.
    Codec {
        /// Underlying codec error.
        source: serde_json::Error,
    },
    /// Wraps a substrate bounded-BFS traversal failure.
    Traversal {
        /// Underlying bounded-BFS error.
        source: oxgraph_algo::BfsError,
    },
}

impl DbError {
    /// Creates an IO error with operation context.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    /// Creates an unsupported-query error.
    ///
    /// # Performance
    ///
    /// This function is `O(message.len())`.
    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::UnsupportedQuery {
            message: message.into(),
        }
    }

    /// Creates an invalid-projection error.
    ///
    /// # Performance
    ///
    /// This function is `O(message.len())`.
    pub(crate) fn invalid_projection(message: impl Into<String>) -> Self {
        Self::InvalidProjection {
            message: message.into(),
        }
    }

    /// Creates an invalid-store error.
    ///
    /// # Performance
    ///
    /// This function is `O(message.len())`.
    pub(crate) fn invalid_store(message: impl Into<String>) -> Self {
        Self::InvalidStore {
            message: message.into(),
        }
    }

    /// Wraps a substrate bounded-BFS traversal failure.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn traversal(source: oxgraph_algo::BfsError) -> Self {
        Self::Traversal { source }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => formatter.write_str("database already exists"),
            Self::NotFound => formatter.write_str("database not found"),
            Self::IdOverflow => formatter.write_str("database ID overflow"),
            Self::TransactionIdOverflow => formatter.write_str("transaction ID overflow"),
            Self::CommitSeqOverflow => formatter.write_str("commit sequence overflow"),
            Self::DuplicateCatalogName => formatter.write_str("duplicate catalog name"),
            Self::DuplicateId => formatter.write_str("duplicate ID"),
            Self::UnknownElement { id } => write!(formatter, "unknown element {}", id.get()),
            Self::UnknownRelation { id } => write!(formatter, "unknown relation {}", id.get()),
            Self::UnknownIncidence { id } => write!(formatter, "unknown incidence {}", id.get()),
            Self::UnknownRole { id } => write!(formatter, "unknown role {}", id.get()),
            Self::UnknownLabel { id } => write!(formatter, "unknown label {}", id.get()),
            Self::UnknownRelationType { id } => {
                write!(formatter, "unknown relation type {}", id.get())
            }
            Self::UnknownPropertyKey { id } => {
                write!(formatter, "unknown property key {}", id.get())
            }
            Self::UnknownProjection { id } => write!(formatter, "unknown projection {}", id.get()),
            Self::UnknownIndex { id } => write!(formatter, "unknown index {}", id.get()),
            Self::PropertyTypeMismatch { expected, actual } => {
                write!(
                    formatter,
                    "property type mismatch: expected {expected:?}, got {actual:?}"
                )
            }
            Self::WrongPropertyFamily { expected, actual } => {
                write!(
                    formatter,
                    "property family mismatch: expected {expected:?}, got {actual:?}"
                )
            }
            Self::InvalidProjection { message } => {
                write!(formatter, "invalid projection: {message}")
            }
            Self::EmptyQuery => formatter.write_str("empty query"),
            Self::UnsupportedQuery { message } => write!(formatter, "unsupported query: {message}"),
            Self::InvalidStore { message } => write!(formatter, "invalid store: {message}"),
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
            Self::Codec { source } => write!(formatter, "codec error: {source}"),
            Self::Traversal { source } => write!(formatter, "traversal error: {source}"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Codec { source } => Some(source),
            Self::Traversal { source } => Some(source),
            Self::AlreadyExists
            | Self::NotFound
            | Self::IdOverflow
            | Self::TransactionIdOverflow
            | Self::CommitSeqOverflow
            | Self::DuplicateCatalogName
            | Self::DuplicateId
            | Self::UnknownElement { .. }
            | Self::UnknownRelation { .. }
            | Self::UnknownIncidence { .. }
            | Self::UnknownRole { .. }
            | Self::UnknownLabel { .. }
            | Self::UnknownRelationType { .. }
            | Self::UnknownPropertyKey { .. }
            | Self::UnknownProjection { .. }
            | Self::UnknownIndex { .. }
            | Self::PropertyTypeMismatch { .. }
            | Self::WrongPropertyFamily { .. }
            | Self::InvalidProjection { .. }
            | Self::EmptyQuery
            | Self::UnsupportedQuery { .. }
            | Self::InvalidStore { .. } => None,
        }
    }
}

impl From<serde_json::Error> for DbError {
    fn from(source: serde_json::Error) -> Self {
        Self::Codec { source }
    }
}

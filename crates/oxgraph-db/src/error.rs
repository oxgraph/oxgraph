//! Db error surface.

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
    /// Db files already exist.
    AlreadyExists,
    /// Db files do not exist.
    NotFound,
    /// The single-writer lock is already held by another writer.
    WriterLockHeld,
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
    /// The store's OXGDB format version is not supported by this build. A base
    /// written under an older format (for example one lacking the persisted
    /// `SECTION_INDEX_*` postings) is rejected here rather than silently rebuilt.
    UnsupportedFormat {
        /// Format version recorded in the store.
        found: u32,
        /// Format version this build requires.
        expected: u32,
    },
    /// Wraps an IO error with operation context.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Underlying IO error.
        source: io::Error,
    },
    /// A bounded traversal failed.
    Traversal {
        /// Deterministic reason the traversal failed.
        reason: &'static str,
    },
    /// A delta-log record is corrupt beyond the recoverable torn tail.
    LogCorrupt {
        /// Log sequence number of the offending record.
        lsn: u64,
        /// Deterministic reason the record was rejected.
        reason: &'static str,
    },
    /// A delta-log record names a different base generation than the superblock.
    BaseGenerationMismatch {
        /// Base generation named by the superblock.
        expected: u64,
        /// Base generation found in the record.
        found: u64,
    },
    /// A catalog name was not found in a bound schema.
    UnknownName {
        /// The kind of catalog entry (for example `"role"` or `"property key"`).
        kind: &'static str,
        /// The name that was not found.
        name: String,
    },
    /// A required property was absent from a subject.
    MissingProperty {
        /// The property key that was required but absent.
        key: PropertyKeyId,
    },
    /// A declared schema item conflicts with an existing catalog entry.
    SchemaConflict {
        /// The conflicting catalog name.
        name: String,
        /// Deterministic reason the declaration conflicts with the catalog.
        reason: &'static str,
    },
    /// A numeric value was outside the representable `i64` range.
    ValueOutOfRange,
    /// A property key has no associated equality index.
    NoEqualityIndex {
        /// The property key lacking an equality index.
        key: PropertyKeyId,
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

    /// Builds a traversal error from a deterministic reason.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn traversal(reason: &'static str) -> Self {
        Self::Traversal { reason }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => formatter.write_str("database already exists"),
            Self::NotFound => formatter.write_str("database not found"),
            Self::WriterLockHeld => formatter.write_str("database writer lock is held"),
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
            Self::UnsupportedFormat { found, expected } => write!(
                formatter,
                "unsupported OXGDB format version: found {found}, this build requires {expected}"
            ),
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
            Self::Traversal { reason } => write!(formatter, "traversal error: {reason}"),
            Self::LogCorrupt { lsn, reason } => {
                write!(formatter, "delta-log corrupt at lsn {lsn}: {reason}")
            }
            Self::BaseGenerationMismatch { expected, found } => write!(
                formatter,
                "base generation mismatch: superblock names {expected}, record has {found}"
            ),
            Self::UnknownName { kind, name } => write!(formatter, "unknown {kind} {name:?}"),
            Self::MissingProperty { key } => {
                write!(formatter, "missing property {}", key.get())
            }
            Self::SchemaConflict { name, reason } => {
                write!(formatter, "schema conflict for {name:?}: {reason}")
            }
            Self::ValueOutOfRange => formatter.write_str("value out of representable i64 range"),
            Self::NoEqualityIndex { key } => {
                write!(
                    formatter,
                    "no equality index for property key {}",
                    key.get()
                )
            }
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Traversal { .. }
            | Self::AlreadyExists
            | Self::NotFound
            | Self::WriterLockHeld
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
            | Self::InvalidStore { .. }
            | Self::UnsupportedFormat { .. }
            | Self::LogCorrupt { .. }
            | Self::BaseGenerationMismatch { .. }
            | Self::UnknownName { .. }
            | Self::MissingProperty { .. }
            | Self::SchemaConflict { .. }
            | Self::ValueOutOfRange
            | Self::NoEqualityIndex { .. } => None,
        }
    }
}

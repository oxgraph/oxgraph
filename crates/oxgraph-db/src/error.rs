//! Db error surface.
//!
//! [`DbError`] is the single error type on the public API. It composes four
//! subsystem enums — [`StorageError`], [`CatalogError`], [`TxnError`], and
//! [`QueryError`] — so callers can match on the failing subsystem while
//! internal code constructs the precise variant and `?`-converts via [`From`].

use std::{fmt, io};

use crate::{PropertyKeyId, catalog::PropertyFamily, value::PropertyType};

/// Canonical id family, for errors that name one.
///
/// # Performance
///
/// Copying, comparing, and formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdFamily {
    /// Canonical element ids.
    Element,
    /// Canonical relation ids.
    Relation,
    /// Canonical incidence ids.
    Incidence,
    /// Structural role ids.
    Role,
    /// Catalog label ids.
    Label,
    /// Catalog relation-type ids.
    RelationType,
    /// Catalog property-key ids.
    PropertyKey,
    /// Catalog projection ids.
    Projection,
    /// Catalog index ids.
    Index,
}

impl fmt::Display for IdFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Element => "element",
            Self::Relation => "relation",
            Self::Incidence => "incidence",
            Self::Role => "role",
            Self::Label => "label",
            Self::RelationType => "relation type",
            Self::PropertyKey => "property key",
            Self::Projection => "projection",
            Self::Index => "index",
        })
    }
}

/// Errors from the persistence layer: db files, the superblock, the base
/// store format, and the delta log.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum StorageError {
    /// Db files already exist.
    AlreadyExists,
    /// Db files do not exist.
    NotFound,
    /// Wraps an IO error with operation context.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Underlying IO error.
        source: io::Error,
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
}

impl StorageError {
    /// Creates an IO error with operation context.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
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
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => formatter.write_str("database already exists"),
            Self::NotFound => formatter.write_str("database not found"),
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
            Self::InvalidStore { message } => write!(formatter, "invalid store: {message}"),
            Self::UnsupportedFormat { found, expected } => write!(
                formatter,
                "unsupported OXGDB format version: found {found}, this build requires {expected}"
            ),
            Self::LogCorrupt { lsn, reason } => {
                write!(formatter, "delta-log corrupt at lsn {lsn}: {reason}")
            }
            Self::BaseGenerationMismatch { expected, found } => write!(
                formatter,
                "base generation mismatch: superblock names {expected}, record has {found}"
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::AlreadyExists
            | Self::NotFound
            | Self::InvalidStore { .. }
            | Self::UnsupportedFormat { .. }
            | Self::LogCorrupt { .. }
            | Self::BaseGenerationMismatch { .. } => None,
        }
    }
}

/// Errors from catalog identity: canonical ids, catalog names, and declared
/// schema shapes.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum CatalogError {
    /// A referenced canonical id is not present.
    UnknownId {
        /// Id family that was looked up.
        family: IdFamily,
        /// Raw canonical id value that was absent.
        id: u64,
    },
    /// A catalog name was not found in a bound schema.
    UnknownName {
        /// The kind of catalog entry (for example `"role"` or `"property key"`).
        kind: &'static str,
        /// The name that was not found.
        name: String,
    },
    /// Duplicate catalog name or ID.
    DuplicateName,
    /// Duplicate canonical ID.
    DuplicateId,
    /// A declared schema item conflicts with an existing catalog entry.
    SchemaConflict {
        /// The conflicting catalog name.
        name: String,
        /// Deterministic reason the declaration conflicts with the catalog.
        reason: &'static str,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId { family, id } => write!(formatter, "unknown {family} {id}"),
            Self::UnknownName { kind, name } => write!(formatter, "unknown {kind} {name:?}"),
            Self::DuplicateName => formatter.write_str("duplicate catalog name"),
            Self::DuplicateId => formatter.write_str("duplicate ID"),
            Self::SchemaConflict { name, reason } => {
                write!(formatter, "schema conflict for {name:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Errors from the transaction lifecycle: the single-writer lock and id and
/// sequence allocation.
///
/// # Performance
///
/// Formatting is `O(1)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum TxnError {
    /// The single-writer lock is already held by another writer.
    WriterLockHeld,
    /// Canonical ID space is exhausted.
    IdOverflow {
        /// Id family whose space is exhausted.
        family: IdFamily,
    },
    /// Transaction ID space is exhausted.
    TransactionIdOverflow,
    /// Commit sequence space is exhausted.
    CommitSeqOverflow,
}

impl fmt::Display for TxnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WriterLockHeld => formatter.write_str("database writer lock is held"),
            Self::IdOverflow { family } => write!(formatter, "database {family} ID overflow"),
            Self::TransactionIdOverflow => formatter.write_str("transaction ID overflow"),
            Self::CommitSeqOverflow => formatter.write_str("commit sequence overflow"),
        }
    }
}

impl std::error::Error for TxnError {}

/// Errors from query and read validation: query text, property schema checks,
/// projections, and traversals.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum QueryError {
    /// Query text is empty.
    Empty,
    /// Query text is outside the pinned profile.
    Unsupported {
        /// Deterministic explanation.
        message: String,
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
    /// A bounded traversal failed.
    Traversal {
        /// Deterministic reason the traversal failed.
        reason: &'static str,
    },
    /// A required property was absent from a subject.
    MissingProperty {
        /// The property key that was required but absent.
        key: PropertyKeyId,
    },
    /// A numeric value was outside the representable `i64` range.
    ValueOutOfRange,
    /// A property key has no associated equality index.
    NoEqualityIndex {
        /// The property key lacking an equality index.
        key: PropertyKeyId,
    },
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("empty query"),
            Self::Unsupported { message } => write!(formatter, "unsupported query: {message}"),
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
            Self::Traversal { reason } => write!(formatter, "traversal error: {reason}"),
            Self::MissingProperty { key } => {
                write!(formatter, "missing property {}", key.get())
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

impl std::error::Error for QueryError {}

/// Errors raised by the `OxGraph` database product.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum DbError {
    /// Persistence-layer failure (files, superblock, base format, delta log).
    Storage(StorageError),
    /// Catalog identity failure (canonical ids, names, schema shapes).
    Catalog(CatalogError),
    /// Transaction lifecycle failure (writer lock, id/sequence allocation).
    Txn(TxnError),
    /// Query or read validation failure.
    Query(QueryError),
}

impl DbError {
    /// Creates an IO error with operation context.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Storage(StorageError::Io { operation, source })
    }

    /// Creates an unsupported-query error.
    ///
    /// # Performance
    ///
    /// This function is `O(message.len())`.
    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::Query(QueryError::Unsupported {
            message: message.into(),
        })
    }

    /// Creates an invalid-projection error.
    ///
    /// # Performance
    ///
    /// This function is `O(message.len())`.
    pub(crate) fn invalid_projection(message: impl Into<String>) -> Self {
        Self::Query(QueryError::InvalidProjection {
            message: message.into(),
        })
    }

    /// Creates an invalid-store error.
    ///
    /// # Performance
    ///
    /// This function is `O(message.len())`.
    pub(crate) fn invalid_store(message: impl Into<String>) -> Self {
        Self::Storage(StorageError::InvalidStore {
            message: message.into(),
        })
    }

    /// Builds a traversal error from a deterministic reason.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn traversal(reason: &'static str) -> Self {
        Self::Query(QueryError::Traversal { reason })
    }

    /// Builds an unknown-id error from any canonical id newtype.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn unknown(id: impl Into<(IdFamily, u64)>) -> Self {
        let (family, id) = id.into();
        Self::Catalog(CatalogError::UnknownId { family, id })
    }

    /// Builds the id-exhaustion error for `family`'s allocator.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub(crate) const fn id_overflow(family: IdFamily) -> Self {
        Self::Txn(TxnError::IdOverflow { family })
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => fmt::Display::fmt(error, formatter),
            Self::Catalog(error) => fmt::Display::fmt(error, formatter),
            Self::Txn(error) => fmt::Display::fmt(error, formatter),
            Self::Query(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Skip-level delegation: the chain stays `DbError -> io::Error` with no
        // intermediate subsystem hop, matching the pre-split surface.
        match self {
            Self::Storage(error) => std::error::Error::source(error),
            Self::Catalog(error) => std::error::Error::source(error),
            Self::Txn(error) => std::error::Error::source(error),
            Self::Query(error) => std::error::Error::source(error),
        }
    }
}

impl From<StorageError> for DbError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<CatalogError> for DbError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<TxnError> for DbError {
    fn from(error: TxnError) -> Self {
        Self::Txn(error)
    }
}

impl From<QueryError> for DbError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}

//! Registration model for relational source tables and edges.

use core::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Stable identifier for a registered source table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TableId(pub u32);

impl fmt::Display for TableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "table:{}", self.0)
    }
}

/// Stable identifier for a registered edge mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EdgeId(pub u32);

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "edge:{}", self.0)
    }
}

/// External node key supplied by a registered table row.
///
/// Encodes `(table_id, primary_key)` as `(table_id << 32) | primary_key` so keys from
/// different registered tables remain distinct at build time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeKey(pub u64);

impl NodeKey {
    /// Builds a node key from a registered table id and SQL primary-key value.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn registered(table: TableId, primary_key: u64) -> Self {
        Self(((table.0 as u64) << 32) | primary_key)
    }

    /// Returns the registered table id embedded in this key.
    #[must_use]
    pub const fn table_id(self) -> TableId {
        TableId((self.0 >> 32) as u32)
    }

    /// Returns the SQL primary-key component embedded in this key.
    #[must_use]
    pub const fn primary_key(self) -> u64 {
        self.0 & 0xFFFF_FFFF
    }
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node:{}:{}", self.table_id(), self.primary_key())
    }
}

/// Column used for filter/search indexing at query time.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FilterColumn {
    /// Owning registered table.
    pub table: TableId,
    /// SQL column name (semantic boundary — interpreted only by the extension).
    pub column: String,
}

/// Registered node table metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RegisteredTable {
    /// Stable table id assigned at registration time.
    pub id: TableId,
    /// SQL schema name.
    pub schema: String,
    /// SQL table name.
    pub name: String,
    /// Primary-key column used to derive [`NodeKey`] values.
    pub primary_key_column: String,
}

/// Registered directed edge between two node tables.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RegisteredEdge {
    /// Stable edge id assigned at registration time.
    pub id: EdgeId,
    /// Source endpoint table.
    pub source_table: TableId,
    /// Target endpoint table.
    pub target_table: TableId,
    /// Source foreign-key column on the edge table.
    pub source_column: String,
    /// Target foreign-key column on the edge table.
    pub target_column: String,
    /// Edge table schema.
    pub schema: String,
    /// Edge table name.
    pub name: String,
}

/// In-memory catalog of registered tables, edges, and filter columns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Catalog {
    /// Registered node tables.
    pub tables: Vec<RegisteredTable>,
    /// Registered edge mappings.
    pub edges: Vec<RegisteredEdge>,
    /// Optional filter columns indexed at query time.
    pub filter_columns: Vec<FilterColumn>,
}

impl Catalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether registration is sufficient to run a snapshot rebuild.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::EmptyCatalog`] when no tables are registered.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub const fn validate_for_build(&self) -> Result<(), CatalogError> {
        if self.tables.is_empty() {
            return Err(CatalogError::EmptyCatalog);
        }
        Ok(())
    }

    /// Registers a node table after validating uniqueness.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError`] when ids or names collide.
    ///
    /// # Performance
    ///
    /// This method is `O(t)` where `t` is the number of registered tables.
    pub fn add_table(&mut self, table: RegisteredTable) -> Result<(), CatalogError> {
        if self.tables.iter().any(|existing| existing.id == table.id) {
            return Err(CatalogError::DuplicateTableId(table.id));
        }
        if self
            .tables
            .iter()
            .any(|existing| existing.schema == table.schema && existing.name == table.name)
        {
            return Err(CatalogError::DuplicateTableName {
                schema: table.schema,
                name: table.name,
            });
        }
        self.tables.push(table);
        Ok(())
    }

    /// Registers an edge mapping after validating endpoint tables exist.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError`] when endpoints are missing or ids collide.
    ///
    /// # Performance
    ///
    /// This method is `O(t + e)`.
    pub fn add_edge(&mut self, edge: RegisteredEdge) -> Result<(), CatalogError> {
        if self.edges.iter().any(|existing| existing.id == edge.id) {
            return Err(CatalogError::DuplicateEdgeId(edge.id));
        }
        if !self
            .tables
            .iter()
            .any(|table| table.id == edge.source_table)
        {
            return Err(CatalogError::MissingTable(edge.source_table));
        }
        if !self
            .tables
            .iter()
            .any(|table| table.id == edge.target_table)
        {
            return Err(CatalogError::MissingTable(edge.target_table));
        }
        self.edges.push(edge);
        Ok(())
    }

    /// Registers a filter column for search indexing.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError`] when the table id is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(t + f)`.
    pub fn add_filter_column(&mut self, column: FilterColumn) -> Result<(), CatalogError> {
        if !self.tables.iter().any(|table| table.id == column.table) {
            return Err(CatalogError::MissingTable(column.table));
        }
        self.filter_columns.push(column);
        Ok(())
    }

    /// Looks up a registered table by id.
    #[must_use]
    pub fn table(&self, id: TableId) -> Option<&RegisteredTable> {
        self.tables.iter().find(|table| table.id == id)
    }

    /// Hydrates a catalog from registration rows (SPI adapters collect rows first).
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError`] when any registration row violates catalog invariants.
    ///
    /// # Performance
    ///
    /// This method is `O(t + e + f)` for table, edge, and filter row counts.
    pub fn from_registration_rows(
        tables: impl IntoIterator<Item = RegisteredTable>,
        edges: impl IntoIterator<Item = RegisteredEdge>,
        filter_columns: impl IntoIterator<Item = FilterColumn>,
    ) -> Result<Self, CatalogError> {
        let mut catalog = Self::new();
        for table in tables {
            catalog.add_table(table)?;
        }
        for edge in edges {
            catalog.add_edge(edge)?;
        }
        for column in filter_columns {
            catalog.add_filter_column(column)?;
        }
        Ok(catalog)
    }
}

impl RegisteredEdge {
    /// Builds a validated `SELECT` that resolves endpoint [`NodeKey`] primary-key values.
    ///
    /// When endpoints reference different registered tables, the scan joins each node
    /// table on the edge foreign-key columns. When both endpoints use the same registered
    /// table and edge columns store that table's primary-key values directly, a single-table
    /// scan is used.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::InvalidSqlIdent`] when any identifier is unsafe, or
    /// [`CatalogError::MissingTable`] when endpoint tables are not registered.
    ///
    /// # Performance
    ///
    /// This method is `O(i)` where `i` is total identifier length.
    pub fn edge_scan_sql(&self, catalog: &Catalog) -> Result<alloc::string::String, CatalogError> {
        validate_sql_ident(&self.schema)?;
        validate_sql_ident(&self.name)?;
        validate_sql_ident(&self.source_column)?;
        validate_sql_ident(&self.target_column)?;

        let source = catalog
            .table(self.source_table)
            .ok_or(CatalogError::MissingTable(self.source_table))?;
        let target = catalog
            .table(self.target_table)
            .ok_or(CatalogError::MissingTable(self.target_table))?;
        validate_sql_ident(&source.schema)?;
        validate_sql_ident(&source.name)?;
        validate_sql_ident(&target.schema)?;
        validate_sql_ident(&target.name)?;
        validate_sql_ident(&source.primary_key_column)?;
        validate_sql_ident(&target.primary_key_column)?;

        if self.source_table == self.target_table {
            return Ok(alloc::format!(
                "SELECT \"{}\"::bigint, \"{}\"::bigint FROM \"{}\".\"{}\"",
                self.source_column,
                self.target_column,
                self.schema,
                self.name
            ));
        }

        Ok(alloc::format!(
            "SELECT src.\"{}\"::bigint, tgt.\"{}\"::bigint \
             FROM \"{}\".\"{}\" e \
             JOIN \"{}\".\"{}\" src ON e.\"{}\" = src.\"{}\" \
             JOIN \"{}\".\"{}\" tgt ON e.\"{}\" = tgt.\"{}\"",
            source.primary_key_column,
            target.primary_key_column,
            self.schema,
            self.name,
            source.schema,
            source.name,
            self.source_column,
            source.primary_key_column,
            target.schema,
            target.name,
            self.target_column,
            target.primary_key_column,
        ))
    }
}

/// Validates a SQL primary-key value for [`NodeKey::registered`].
///
/// # Errors
///
/// Returns [`CatalogError::InvalidPrimaryKey`] for negative values and
/// [`CatalogError::PrimaryKeyOutOfRange`] when the key does not fit the lower
/// 32 bits of a [`NodeKey`].
///
/// # Performance
///
/// This function is `O(1)`.
pub fn validate_primary_key(value: i64) -> Result<u64, CatalogError> {
    if value.is_negative() {
        return Err(CatalogError::InvalidPrimaryKey);
    }
    let primary_key = u64::try_from(value).map_err(|_| CatalogError::PrimaryKeyOutOfRange)?;
    if primary_key > u64::from(u32::MAX) {
        return Err(CatalogError::PrimaryKeyOutOfRange);
    }
    Ok(primary_key)
}

/// Parses a registered table id from SPI/catalog storage.
///
/// # Errors
///
/// Returns [`CatalogError::InvalidTableId`] when `value` is negative or overflows `u32`.
///
/// # Performance
///
/// This function is `O(1)`.
pub fn table_id_from_i32(value: i32) -> Result<TableId, CatalogError> {
    if value.is_negative() {
        return Err(CatalogError::InvalidTableId);
    }
    u32::try_from(value)
        .map(TableId)
        .map_err(|_| CatalogError::InvalidTableId)
}

/// Parses a registered edge id from SPI/catalog storage.
///
/// # Errors
///
/// Returns [`CatalogError::InvalidEdgeId`] when `value` is negative or overflows `u32`.
///
/// # Performance
///
/// This function is `O(1)`.
pub fn edge_id_from_i32(value: i32) -> Result<EdgeId, CatalogError> {
    if value.is_negative() {
        return Err(CatalogError::InvalidEdgeId);
    }
    u32::try_from(value)
        .map(EdgeId)
        .map_err(|_| CatalogError::InvalidEdgeId)
}

/// Returns whether `ident` is safe for double-quoted SQL identifiers.
///
/// # Errors
///
/// Returns [`CatalogError::InvalidSqlIdent`] when the identifier is empty,
/// starts with an unsafe character, or contains non-identifier characters.
///
/// # Performance
///
/// This function is `O(i)` where `i` is identifier length.
pub fn validate_sql_ident(ident: &str) -> Result<(), CatalogError> {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return Err(CatalogError::InvalidSqlIdent);
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(CatalogError::InvalidSqlIdent);
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(CatalogError::InvalidSqlIdent);
    }
    Ok(())
}

/// Catalog validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// No tables were registered.
    EmptyCatalog,
    /// Duplicate registered table id.
    DuplicateTableId(TableId),
    /// Duplicate registered edge id.
    DuplicateEdgeId(EdgeId),
    /// Duplicate schema-qualified table name.
    DuplicateTableName {
        /// SQL schema name.
        schema: String,
        /// SQL table name.
        name: String,
    },
    /// Referenced table id was not registered.
    MissingTable(TableId),
    /// SQL identifier failed validation.
    InvalidSqlIdent,
    /// SQL primary key was negative.
    InvalidPrimaryKey,
    /// SQL primary key does not fit in a [`NodeKey`] payload.
    PrimaryKeyOutOfRange,
    /// Registered table id was negative or overflowed `u32`.
    InvalidTableId,
    /// Registered edge id was negative or overflowed `u32`.
    InvalidEdgeId,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCatalog => f.write_str("catalog must register at least one table"),
            Self::DuplicateTableId(id) => write!(f, "duplicate table id {id}"),
            Self::DuplicateEdgeId(id) => write!(f, "duplicate edge id {id}"),
            Self::DuplicateTableName { schema, name } => {
                write!(f, "duplicate table name {schema}.{name}")
            }
            Self::MissingTable(id) => write!(f, "missing catalog table {id}"),
            Self::InvalidSqlIdent => f.write_str("invalid SQL identifier"),
            Self::InvalidPrimaryKey => f.write_str("primary key must be non-negative"),
            Self::PrimaryKeyOutOfRange => {
                f.write_str("primary key must fit in u32 for NodeKey encoding")
            }
            Self::InvalidTableId => f.write_str("invalid registered table id"),
            Self::InvalidEdgeId => f.write_str("invalid registered edge id"),
        }
    }
}

impl core::error::Error for CatalogError {}

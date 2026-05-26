//! SPI helpers for catalog persistence and relational scans.

use oxgraph_postgres::{
    BuildError, Catalog, EdgeId, EdgeRow, FilterColumn, PostgresGraphError, RegisteredEdge,
    RegisteredTable, SnapshotRebuild, SyncRow, TableId, edge_id_from_i32, edge_row_from_scan,
    resolve_sync_rows, table_id_from_i32, validate_primary_key, validate_sql_ident,
};
use pgrx::{datum::DatumWithOid, prelude::*};

fn map_spi_error(error: pgrx::spi::Error) -> PostgresGraphError {
    PostgresGraphError::Build(BuildError::Spi(format!("{error}")))
}

fn require_column<T>(value: Option<T>, field: &'static str) -> Result<T, PostgresGraphError> {
    value.ok_or_else(|| {
        PostgresGraphError::Build(BuildError::Spi(format!("null catalog column: {field}")))
    })
}

/// Loads the registration catalog from extension tables.
pub(crate) fn load_catalog_from_spi() -> Result<Catalog, PostgresGraphError> {
    let tables = Spi::connect(|client| {
        let mut rows = Vec::new();
        let result = client.select(
            "SELECT table_id, schema_name, table_name, primary_key_column \
             FROM graph._registered_tables ORDER BY table_id",
            None,
            &[],
        )?;
        for row in result {
            rows.push((
                row.get::<i32>(1)?,
                row.get::<String>(2)?,
                row.get::<String>(3)?,
                row.get::<String>(4)?,
            ));
        }
        Ok(rows)
    })
    .map_err(map_spi_error)?;

    let edges = Spi::connect(|client| {
        let mut rows = Vec::new();
        let result = client.select(
            "SELECT edge_id, source_table_id, target_table_id, source_column, target_column, \
             schema_name, table_name \
             FROM graph._registered_edges ORDER BY edge_id",
            None,
            &[],
        )?;
        for row in result {
            rows.push((
                row.get::<i32>(1)?,
                row.get::<i32>(2)?,
                row.get::<i32>(3)?,
                row.get::<String>(4)?,
                row.get::<String>(5)?,
                row.get::<String>(6)?,
                row.get::<String>(7)?,
            ));
        }
        Ok(rows)
    })
    .map_err(map_spi_error)?;

    let filters = Spi::connect(|client| {
        let mut rows = Vec::new();
        let result = client.select(
            "SELECT table_id, column_name FROM graph._registered_filter_columns",
            None,
            &[],
        )?;
        for row in result {
            rows.push((row.get::<i32>(1)?, row.get::<String>(2)?));
        }
        Ok(rows)
    })
    .map_err(map_spi_error)?;

    let mut catalog = Catalog::new();
    for (table_id, schema, name, primary_key_column) in tables {
        catalog.add_table(RegisteredTable {
            id: table_id_from_i32(require_column(table_id, "table_id")?)?,
            schema: require_column(schema, "schema_name")?,
            name: require_column(name, "table_name")?,
            primary_key_column: require_column(primary_key_column, "primary_key_column")?,
        })?;
    }
    for (edge_id, source_table, target_table, source_column, target_column, schema, name) in edges {
        catalog.add_edge(RegisteredEdge {
            id: edge_id_from_i32(require_column(edge_id, "edge_id")?)?,
            source_table: table_id_from_i32(require_column(source_table, "source_table_id")?)?,
            target_table: table_id_from_i32(require_column(target_table, "target_table_id")?)?,
            source_column: require_column(source_column, "source_column")?,
            target_column: require_column(target_column, "target_column")?,
            schema: require_column(schema, "schema_name")?,
            name: require_column(name, "table_name")?,
        })?;
    }
    for (table_id, column) in filters {
        catalog.add_filter_column(FilterColumn {
            table: table_id_from_i32(require_column(table_id, "table_id")?)?,
            column: require_column(column, "column_name")?,
        })?;
    }
    Ok(catalog)
}

/// Scans registered edge tables into edge rows for build/maintenance.
pub(crate) fn scan_edge_rows(catalog: &Catalog) -> Result<Vec<EdgeRow>, PostgresGraphError> {
    let mut rows = Vec::new();
    for edge in &catalog.edges {
        let sql = edge.edge_scan_sql(&catalog)?;
        let scanned = Spi::connect(|client| {
            let mut out = Vec::new();
            let result = client.select(sql.as_str(), None, &[])?;
            for tuple in result {
                out.push((tuple.get::<i64>(1)?, tuple.get::<i64>(2)?));
            }
            Ok(out)
        })
        .map_err(map_spi_error)?;
        for (source, target) in scanned {
            let source_pk = validate_primary_key(require_column(source, "source")?)?;
            let target_pk = validate_primary_key(require_column(target, "target")?)?;
            rows.push(edge_row_from_scan(edge, source_pk, target_pk));
        }
    }
    Ok(rows)
}

/// Returns true when `ident` is safe for double-quoted SQL identifiers.
pub(crate) fn sql_ident_public(ident: &str) -> bool {
    oxgraph_postgres::validate_sql_ident(ident).is_ok()
}

/// Rebuilds snapshot bytes from catalog tables and persists them.
pub(crate) fn rebuild_and_persist_snapshot(
    built_at_unix: u64,
) -> Result<Vec<u8>, PostgresGraphError> {
    let catalog = load_catalog_from_spi()?;
    let edges = scan_edge_rows(&catalog)?;
    let bytes = SnapshotRebuild::from_catalog_and_edges(&catalog, &edges, built_at_unix)?;
    persist_snapshot_bytes(&bytes, built_at_unix as i64)?;
    Ok(bytes)
}

/// Loads durable sync rows from `graph._sync_log` and resolves keyed actions.
pub(crate) fn load_sync_rows_from_spi() -> Result<Vec<SyncRow>, PostgresGraphError> {
    let catalog = load_catalog_from_spi()?;
    let edges = scan_edge_rows(&catalog)?;
    let raw = Spi::connect(|client| {
        let mut out = Vec::new();
        let result = client.select(
            "SELECT sequence, action_type, arg0, arg1 \
             FROM graph._sync_log ORDER BY sequence",
            None,
            &[],
        )?;
        for tuple in result {
            out.push((
                tuple.get::<i64>(1)?,
                tuple.get::<i16>(2)?,
                tuple.get::<i64>(3)?,
                tuple.get::<i64>(4)?,
            ));
        }
        Ok(out)
    })
    .map_err(map_spi_error)?;

    let mut raw_rows = Vec::with_capacity(raw.len());
    for (sequence, action_type, arg0, arg1) in raw {
        let sequence = u64::try_from(require_column(sequence, "sequence")?).map_err(|_| {
            PostgresGraphError::Build(BuildError::Spi("sync sequence overflow".into()))
        })?;
        raw_rows.push((
            sequence,
            require_column(action_type, "action_type")?,
            arg0,
            arg1,
        ));
    }
    resolve_sync_rows(&edges, &raw_rows)
}

/// Persists snapshot bytes into `graph._snapshot_store`.
pub(crate) fn persist_snapshot_bytes(
    bytes: &[u8],
    built_at_unix: i64,
) -> Result<(), PostgresGraphError> {
    Spi::connect_mut(|client| {
        // SAFETY: `DatumWithOid::new` is used with correct Postgres type OIDs.
        let args = unsafe {
            [
                DatumWithOid::new(bytes, pg_sys::BYTEAOID),
                DatumWithOid::new(built_at_unix, pg_sys::INT8OID),
            ]
        };
        client.update(
            "INSERT INTO graph._snapshot_store (id, bytes, built_at_unix) VALUES (1, $1, $2) \
             ON CONFLICT (id) DO UPDATE SET bytes = EXCLUDED.bytes, \
             built_at_unix = EXCLUDED.built_at_unix",
            None,
            &args,
        )?;
        Ok(())
    })
    .map_err(map_spi_error)
}

/// Registers a node table after catalog validation and persists the row.
pub(crate) fn register_table(
    schema_name: &str,
    table_name: &str,
    primary_key_column: &str,
) -> Result<TableId, PostgresGraphError> {
    validate_sql_ident(schema_name)?;
    validate_sql_ident(table_name)?;
    validate_sql_ident(primary_key_column)?;
    let mut catalog = load_catalog_from_spi()?;
    let table_id = TableId(next_table_id()?);
    let table = RegisteredTable {
        id: table_id,
        schema: schema_name.into(),
        name: table_name.into(),
        primary_key_column: primary_key_column.into(),
    };
    catalog.add_table(table.clone())?;
    insert_registered_table(&table)?;
    Ok(table_id)
}

/// Registers an edge mapping after catalog validation and persists the row.
pub(crate) fn register_edge(
    source_table_id: i32,
    target_table_id: i32,
    source_column: &str,
    target_column: &str,
    schema_name: &str,
    table_name: &str,
) -> Result<EdgeId, PostgresGraphError> {
    validate_sql_ident(source_column)?;
    validate_sql_ident(target_column)?;
    validate_sql_ident(schema_name)?;
    validate_sql_ident(table_name)?;
    let mut catalog = load_catalog_from_spi()?;
    let edge_id = EdgeId(next_edge_id()?);
    let edge = RegisteredEdge {
        id: edge_id,
        source_table: table_id_from_i32(source_table_id)?,
        target_table: table_id_from_i32(target_table_id)?,
        source_column: source_column.into(),
        target_column: target_column.into(),
        schema: schema_name.into(),
        name: table_name.into(),
    };
    catalog.add_edge(edge.clone())?;
    insert_registered_edge(&edge)?;
    Ok(edge_id)
}

/// Registers a filter column after catalog validation and persists the row.
pub(crate) fn register_filter_column(
    table_id: i32,
    column_name: &str,
) -> Result<(), PostgresGraphError> {
    validate_sql_ident(column_name)?;
    let mut catalog = load_catalog_from_spi()?;
    let column = FilterColumn {
        table: table_id_from_i32(table_id)?,
        column: column_name.into(),
    };
    catalog.add_filter_column(column.clone())?;
    insert_filter_column(&column)?;
    Ok(())
}

fn next_table_id() -> Result<u32, PostgresGraphError> {
    let next =
        Spi::get_one::<i32>("SELECT COALESCE(MAX(table_id), 0) + 1 FROM graph._registered_tables")
            .map_err(map_spi_error)?;
    let next = require_column(next, "table_id")?;
    u32::try_from(next)
        .map_err(|_| PostgresGraphError::Build(BuildError::Spi("table_id overflow".into())))
}

fn next_edge_id() -> Result<u32, PostgresGraphError> {
    let next =
        Spi::get_one::<i32>("SELECT COALESCE(MAX(edge_id), 0) + 1 FROM graph._registered_edges")
            .map_err(map_spi_error)?;
    let next = require_column(next, "edge_id")?;
    u32::try_from(next)
        .map_err(|_| PostgresGraphError::Build(BuildError::Spi("edge_id overflow".into())))
}

fn insert_registered_table(table: &RegisteredTable) -> Result<(), PostgresGraphError> {
    Spi::connect_mut(|client| {
        // SAFETY: `DatumWithOid::new` pairs each Rust value with the matching Postgres type OID.
        let args = unsafe {
            [
                DatumWithOid::new(table.id.0 as i32, pg_sys::INT4OID),
                DatumWithOid::new(table.schema.as_str(), pg_sys::TEXTOID),
                DatumWithOid::new(table.name.as_str(), pg_sys::TEXTOID),
                DatumWithOid::new(table.primary_key_column.as_str(), pg_sys::TEXTOID),
            ]
        };
        client.update(
            "INSERT INTO graph._registered_tables \
             (table_id, schema_name, table_name, primary_key_column) \
             VALUES ($1, $2, $3, $4)",
            None,
            &args,
        )?;
        Ok(())
    })
    .map_err(map_spi_error)
}

fn insert_registered_edge(edge: &RegisteredEdge) -> Result<(), PostgresGraphError> {
    Spi::connect_mut(|client| {
        // SAFETY: `DatumWithOid::new` pairs each Rust value with the matching Postgres type OID.
        let args = unsafe {
            [
                DatumWithOid::new(edge.id.0 as i32, pg_sys::INT4OID),
                DatumWithOid::new(edge.source_table.0 as i32, pg_sys::INT4OID),
                DatumWithOid::new(edge.target_table.0 as i32, pg_sys::INT4OID),
                DatumWithOid::new(edge.source_column.as_str(), pg_sys::TEXTOID),
                DatumWithOid::new(edge.target_column.as_str(), pg_sys::TEXTOID),
                DatumWithOid::new(edge.schema.as_str(), pg_sys::TEXTOID),
                DatumWithOid::new(edge.name.as_str(), pg_sys::TEXTOID),
            ]
        };
        client.update(
            "INSERT INTO graph._registered_edges (
                edge_id, source_table_id, target_table_id,
                source_column, target_column, schema_name, table_name
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            None,
            &args,
        )?;
        Ok(())
    })
    .map_err(map_spi_error)
}

fn insert_filter_column(column: &FilterColumn) -> Result<(), PostgresGraphError> {
    Spi::connect_mut(|client| {
        // SAFETY: `DatumWithOid::new` pairs each Rust value with the matching Postgres type OID.
        let args = unsafe {
            [
                DatumWithOid::new(column.table.0 as i32, pg_sys::INT4OID),
                DatumWithOid::new(column.column.as_str(), pg_sys::TEXTOID),
            ]
        };
        client.update(
            "INSERT INTO graph._registered_filter_columns (table_id, column_name) \
             VALUES ($1, $2)",
            None,
            &args,
        )?;
        Ok(())
    })
    .map_err(map_spi_error)
}

/// Reads persisted snapshot bytes when present.
pub(crate) fn load_persisted_snapshot_bytes() -> Result<Option<Vec<u8>>, PostgresGraphError> {
    Spi::connect(|client| {
        let result = client.select(
            "SELECT bytes FROM graph._snapshot_store WHERE id = 1",
            Some(1),
            &[],
        )?;
        result.first().get_one::<Vec<u8>>()
    })
    .map_err(map_spi_error)
}

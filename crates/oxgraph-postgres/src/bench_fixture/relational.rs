//! Small relational catalog fixtures for extension SQL integration tests.

use crate::{
    Catalog, CatalogError, EdgeId, EdgeRow, NodeKey, RegisteredEdge, RegisteredTable, TableId,
};

/// Chain graph catalog: one node table, one edge table (`public.nodes` / `public.edges`).
///
/// # Errors
///
/// Returns [`CatalogError`] when registration invariants fail.
pub fn chain_catalog() -> Result<Catalog, CatalogError> {
    let mut catalog = Catalog::new();
    catalog.add_table(RegisteredTable {
        id: TableId(1),
        schema: "public".into(),
        name: "nodes".into(),
        primary_key_column: "id".into(),
    })?;
    catalog.add_edge(RegisteredEdge {
        id: EdgeId(1),
        source_table: TableId(1),
        target_table: TableId(1),
        source_column: "src".into(),
        target_column: "dst".into(),
        schema: "public".into(),
        name: "edges".into(),
    })?;
    Ok(catalog)
}

/// Three-node chain edges for [`chain_catalog`].
#[must_use]
pub const fn chain_edges() -> [EdgeRow; 2] {
    [
        EdgeRow {
            source: NodeKey::registered(TableId(1), 1),
            target: NodeKey::registered(TableId(1), 2),
        },
        EdgeRow {
            source: NodeKey::registered(TableId(1), 2),
            target: NodeKey::registered(TableId(1), 3),
        },
    ]
}

/// SQL to create and seed `public.nodes` / `public.edges` for the chain fixture.
pub const CHAIN_RELATIONAL_DDL: &str = r"
CREATE TABLE IF NOT EXISTS public.nodes (id bigint PRIMARY KEY);
CREATE TABLE IF NOT EXISTS public.edges (src bigint NOT NULL, dst bigint NOT NULL);
TRUNCATE public.nodes, public.edges;
INSERT INTO public.nodes (id) VALUES (1), (2), (3);
INSERT INTO public.edges (src, dst) VALUES (1, 2), (2, 3);
";

/// SQL to register the chain catalog in extension tables.
pub const CHAIN_CATALOG_DML: &str = r"
TRUNCATE graph._registered_filter_columns, graph._registered_edges, graph._registered_tables;
INSERT INTO graph._registered_tables (table_id, schema_name, table_name, primary_key_column)
VALUES (1, 'public', 'nodes', 'id');
INSERT INTO graph._registered_edges (
    edge_id, source_table_id, target_table_id,
    source_column, target_column, schema_name, table_name
) VALUES (1, 1, 1, 'src', 'dst', 'public', 'edges');
";

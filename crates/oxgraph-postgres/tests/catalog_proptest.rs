//! Property tests for catalog registration invariants.

use oxgraph_postgres::{
    Catalog, CatalogError, EdgeId, FilterColumn, RegisteredEdge, RegisteredTable, TableId,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn duplicate_table_ids_rejected(
        schema in "[a-z][a-z0-9_]{0,8}",
        name_a in "[a-z][a-z0-9_]{0,8}",
        name_b in "[a-z][a-z0-9_]{0,8}",
    ) {
        let mut catalog = Catalog::new();
        catalog
            .add_table(RegisteredTable {
                id: TableId(1),
                schema: schema.clone(),
                name: name_a,
                primary_key_column: "id".into(),
            })
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(
            catalog.add_table(RegisteredTable {
                id: TableId(1),
                schema,
                name: name_b,
                primary_key_column: "id".into(),
            }),
            Err(CatalogError::DuplicateTableId(TableId(1)))
        );
    }

    #[test]
    fn filter_column_requires_registered_table(table_id in 1_u32..8) {
        let mut catalog = Catalog::new();
        prop_assert_eq!(
            catalog.add_filter_column(FilterColumn {
                table: TableId(table_id),
                column: "name".into(),
            }),
            Err(CatalogError::MissingTable(TableId(table_id)))
        );
    }

    #[test]
    fn edge_endpoints_must_exist(edge_id in 1_u32..8) {
        let mut catalog = Catalog::new();
        catalog
            .add_table(RegisteredTable {
                id: TableId(1),
                schema: "public".into(),
                name: "nodes".into(),
                primary_key_column: "id".into(),
            })
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(
            catalog.add_edge(RegisteredEdge {
                id: EdgeId(edge_id),
                source_table: TableId(2),
                target_table: TableId(1),
                source_column: "src".into(),
                target_column: "dst".into(),
                schema: "public".into(),
                name: "edges".into(),
            }),
            Err(CatalogError::MissingTable(TableId(2)))
        );
    }
}

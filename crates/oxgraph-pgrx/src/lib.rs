//! OxGraph PostgreSQL extension — thin SQL/SPI glue over `oxgraph-postgres`.
//!
//! Each backend process holds at most one in-memory [`oxgraph_postgres::Engine`] in a
//! thread-local [`session::SessionEngine`]. Other backends observe persisted snapshot bytes in
//! `graph._snapshot_store` after rebuild or maintenance.

use pgrx::prelude::*;

mod acl;
mod api;
mod gucs;
mod jobs;
mod report;
mod session;
mod spi;

/// Initializes extension GUCs and background workers.
///
/// # Panics
///
/// Panics when Postgres GUC registration fails during extension load.
pub extern "C-unwind" fn _PG_init() {
    gucs::register();
    jobs::register_maintenance_worker();
}

/// Registers the `graph` schema for pgrx SQL generation (DDL also in `bootstrap.sql`).
#[pg_schema]
mod graph {}

#[cfg(any(test, feature = "pg_test", feature = "pg_bench"))]
pub mod fixtures;

#[cfg(feature = "pg_bench")]
#[pg_schema]
mod benches {
    include!("benches.rs");
}

#[cfg(any(test, feature = "pg_test"))]
pub mod pg_test;

/// In-process SQL tests (`cargo pgrx test` expects the `tests` schema).
#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    include!("integration_tests.rs");
}

::pgrx::pg_module_magic!(name, version);

::pgrx::extension_sql_file!("../sql/bootstrap.sql", name = "oxgraph_bootstrap_sql");

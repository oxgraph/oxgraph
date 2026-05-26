//! Thread-local engine slot for the active backend session.

use core::cell::RefCell;

use oxgraph_postgres::{Config, Engine, EngineBuilder, PostgresGraphError, QueryFreshness};
use pgrx::prelude::*;

use crate::gucs;

thread_local! {
    static LOADED_ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

/// Active-session engine holder.
pub(crate) struct SessionEngine;

impl SessionEngine {
    /// Runs `f` against the loaded engine when present.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::NotLoaded`] when no engine is loaded.
    pub(crate) fn try_with<R>(
        f: impl FnOnce(&Engine) -> Result<R, PostgresGraphError>,
    ) -> Result<R, PostgresGraphError> {
        LOADED_ENGINE.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(engine) = borrowed.as_mut() else {
                return Err(PostgresGraphError::NotLoaded);
            };
            let _ = engine.set_config(config_from_gucs());
            f(engine)
        })
    }

    /// Runs `f` against the loaded engine mutably when present.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::NotLoaded`] when no engine is loaded.
    pub(crate) fn try_with_mut<R>(
        f: impl FnOnce(&mut Engine) -> Result<R, PostgresGraphError>,
    ) -> Result<R, PostgresGraphError> {
        LOADED_ENGINE.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(engine) = borrowed.as_mut() else {
                return Err(PostgresGraphError::NotLoaded);
            };
            let _ = engine.set_config(config_from_gucs());
            f(engine)
        })
    }

    /// Loads an engine from validated snapshot bytes and mirrors GUC config.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError`] when validation or open fails.
    pub(crate) fn load(bytes: &[u8]) -> Result<(), PostgresGraphError> {
        let config = config_from_gucs();
        let engine = EngineBuilder::new()
            .snapshot_owned(bytes.to_vec())
            .config(config)
            .build()?;
        LOADED_ENGINE.with(|slot| {
            *slot.borrow_mut() = Some(engine);
        });
        Ok(())
    }

    /// Clears the active engine slot.
    pub(crate) fn reset() {
        LOADED_ENGINE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// Builds library config from extension GUCs.
pub(crate) fn config_from_gucs() -> Config {
    Config {
        traverse_limit: read_int_guc("oxgraph.traverse_limit", gucs::traverse_limit()),
        search_limit: read_int_guc("oxgraph.search_limit", gucs::search_limit()),
        maintenance_enabled: read_bool_guc(
            "oxgraph.maintenance_enabled",
            gucs::maintenance_enabled(),
        ),
        query_freshness: read_query_freshness_guc(),
    }
}

/// Reads query freshness via SPI so SQL `SET` is visible to graph entrypoints.
fn read_query_freshness_guc() -> QueryFreshness {
    let sql = "SELECT NULLIF(current_setting('oxgraph.query_freshness', true), '')::int";
    Spi::get_one::<i32>(sql)
        .ok()
        .flatten()
        .map(|value| match value.clamp(0, 1) {
            0 => QueryFreshness::BaseOnly,
            _ => QueryFreshness::OverlayAware,
        })
        .unwrap_or_else(gucs::query_freshness)
}

/// Reads a session int GUC via SPI so SQL `SET` is visible to graph entrypoints.
fn read_int_guc(name: &str, fallback: u32) -> u32 {
    let sql = format!("SELECT NULLIF(current_setting('{name}', true), '')::int");
    Spi::get_one::<i32>(&sql)
        .ok()
        .flatten()
        .map(|value| value.clamp(1, i32::MAX) as u32)
        .unwrap_or(fallback)
}

/// Reads a session bool GUC via SPI so SQL `SET` is visible to graph entrypoints.
fn read_bool_guc(name: &str, fallback: bool) -> bool {
    let sql = format!("SELECT NULLIF(current_setting('{name}', true), '')::bool");
    Spi::get_one::<bool>(&sql)
        .ok()
        .flatten()
        .unwrap_or(fallback)
}

/// Logs a library error at the SQL boundary.
pub(crate) fn log_error(error: &PostgresGraphError) {
    pgrx::warning!("oxgraph: {error}");
}

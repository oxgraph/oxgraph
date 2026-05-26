//! Background job hooks for build and maintenance.

use pgrx::{
    bgworkers::{BackgroundWorker, BackgroundWorkerBuilder, BgWorkerStartTime, SignalWakeFlags},
    prelude::*,
};

use crate::{session::log_error, spi};

/// Registers the maintenance background worker at postmaster start.
pub(crate) fn register_maintenance_worker() {
    BackgroundWorkerBuilder::new("oxgraph_maintenance")
        .set_library("oxgraph_pgrx")
        .set_function("oxgraph_maintenance_main")
        .set_start_time(BgWorkerStartTime::RecoveryFinished)
        .enable_spi_access()
        .set_restart_time(Some(core::time::Duration::from_secs(60)))
        .load();
}

/// Background worker entry: rebuild snapshot from catalog and persist bytes.
#[pg_guard]
#[unsafe(no_mangle)]
pub extern "C-unwind" fn oxgraph_maintenance_main(_arg: pg_sys::Datum) {
    BackgroundWorker::attach_signal_handlers(SignalWakeFlags::SIGHUP | SignalWakeFlags::SIGTERM);
    BackgroundWorker::connect_worker_to_spi(Some("postgres"), None);

    while BackgroundWorker::wait_latch(Some(core::time::Duration::from_secs(300))) {
        if BackgroundWorker::sighup_received() {
            // Re-read GUCs on SIGHUP; engine config is applied on next session load.
        }
        if let Err(error) = run_maintenance_pass() {
            pgrx::log!("oxgraph maintenance worker failed: {error}");
        }
    }
}

/// Runs one maintenance pass: catalog scan, rebuild, persist snapshot bytes.
fn run_maintenance_pass() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    if !crate::gucs::maintenance_enabled() {
        return Ok(());
    }
    let built_at = chrono_like_unix_now();
    spi::rebuild_and_persist_snapshot(built_at).map(|_| ())
}

/// Schedules maintenance by waking the registered worker (no-op when disabled).
pub(crate) fn schedule_maintenance() {
    if !crate::gucs::maintenance_enabled() {
        return;
    }
    register_maintenance_worker();
    if let Err(error) = run_maintenance_pass() {
        log_error(&error);
    }
}

/// Returns a coarse Unix timestamp for maintenance builds (seconds).
fn chrono_like_unix_now() -> u64 {
    // SAFETY: `GetCurrentTimestamp` is valid in a backend or background worker.
    let stamp = unsafe { pg_sys::GetCurrentTimestamp() };
    // SAFETY: conversion uses Postgres epoch helpers for timestamptz.
    let secs = unsafe { pg_sys::timestamptz_to_time_t(stamp) };
    u64::try_from(secs).unwrap_or(0)
}

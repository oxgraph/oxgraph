//! GUC registration for graph runtime configuration.

use oxgraph_postgres::QueryFreshness;
use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};

/// Default traverse expansion cap mirrored into [`oxgraph_postgres::Config`].
static TRAVERSE_LIMIT: GucSetting<i32> = GucSetting::<i32>::new(10_000);

/// Default search row cap mirrored into [`oxgraph_postgres::Config`].
static SEARCH_LIMIT: GucSetting<i32> = GucSetting::<i32>::new(10_000);

/// When true, maintenance rebuild paths are permitted.
static MAINTENANCE_ENABLED: GucSetting<bool> = GucSetting::<bool>::new(true);

/// Session role override: `reader` (0) or `admin` (1). Superusers always receive admin.
static GRAPH_ROLE_ORDINAL: GucSetting<i32> = GucSetting::<i32>::new(0);

/// Query freshness: `0` = base-only, `1` = overlay-aware (default).
static QUERY_FRESHNESS_ORDINAL: GucSetting<i32> = GucSetting::<i32>::new(1);

/// Registers OxGraph GUCs.
pub(crate) fn register() {
    // SAFETY: static CStr literals are NUL-terminated.
    let traverse_name = c"oxgraph.traverse_limit";
    let search_name = c"oxgraph.search_limit";
    let maintenance_name = c"oxgraph.maintenance_enabled";
    let role_name = c"oxgraph.graph_role";
    let freshness_name = c"oxgraph.query_freshness";

    GucRegistry::define_int_guc(
        traverse_name,
        c"traverse limit",
        c"Maximum BFS expansion steps per traverse call.",
        &TRAVERSE_LIMIT,
        1,
        i32::MAX,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        search_name,
        c"search limit",
        c"Maximum rows returned from graph search.",
        &SEARCH_LIMIT,
        1,
        i32::MAX,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_bool_guc(
        maintenance_name,
        c"maintenance enabled",
        c"Whether maintenance rebuild paths are permitted.",
        &MAINTENANCE_ENABLED,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        role_name,
        c"graph role",
        c"Session graph role override: 0=reader, 1=admin (superusers are always admin).",
        &GRAPH_ROLE_ORDINAL,
        0,
        1,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        freshness_name,
        c"query freshness",
        c"Freshness policy: 0=base-only, 1=overlay-aware.",
        &QUERY_FRESHNESS_ORDINAL,
        0,
        1,
        GucContext::Userset,
        GucFlags::default(),
    );
}

/// Returns the configured traverse limit as `u32`.
#[must_use]
pub(crate) fn traverse_limit() -> u32 {
    TRAVERSE_LIMIT.get().clamp(1, i32::MAX) as u32
}

/// Returns the configured search limit as `u32`.
#[must_use]
pub(crate) fn search_limit() -> u32 {
    SEARCH_LIMIT.get().clamp(1, i32::MAX) as u32
}

/// Returns whether maintenance rebuild is enabled.
#[must_use]
pub(crate) fn maintenance_enabled() -> bool {
    MAINTENANCE_ENABLED.get()
}

/// Returns the session graph role ordinal (`0` reader, `1` admin).
#[must_use]
pub(crate) fn graph_role_ordinal() -> i32 {
    GRAPH_ROLE_ORDINAL.get().clamp(0, 1)
}

/// Returns the configured query freshness policy.
#[must_use]
pub(crate) fn query_freshness() -> QueryFreshness {
    match QUERY_FRESHNESS_ORDINAL.get().clamp(0, 1) {
        0 => QueryFreshness::BaseOnly,
        _ => QueryFreshness::OverlayAware,
    }
}

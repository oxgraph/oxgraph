//! pgrx test harness hooks for `cargo pgrx test`.

/// Configure the pgrx test database before SQL tests run.
pub fn setup(_options: Vec<&str>) {}

/// Return additional PostgreSQL settings for the pgrx test cluster.
#[must_use]
pub fn postgresql_conf_options() -> Vec<&'static str> {
    vec![]
}

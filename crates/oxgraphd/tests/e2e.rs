//! End-to-end tests for the `OxGraph` DB CLI and HTTP facade.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use oxgraph_db::Db;

/// Per-process path counter.
static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

/// Test error type.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "test harness reads error fields through derived Debug when a Result test fails"
)]
enum TestError {
    /// Db error.
    Db(oxgraph_db::DbError),
    /// Facade error.
    Facade(oxgraphd::OxgraphdError),
    /// Filesystem error.
    Io(std::io::Error),
}

impl From<oxgraph_db::DbError> for TestError {
    fn from(error: oxgraph_db::DbError) -> Self {
        Self::Db(error)
    }
}

impl From<oxgraphd::OxgraphdError> for TestError {
    fn from(error: oxgraphd::OxgraphdError) -> Self {
        Self::Facade(error)
    }
}

impl From<std::io::Error> for TestError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Builds a unique temporary database path.
fn temp_path(name: &str) -> PathBuf {
    let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("oxgraphd-{name}-{}-{id}", std::process::id()))
}

/// Removes `path` when it exists.
fn clean(path: &Path) -> Result<(), TestError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[test]
fn cli_covers_create_status_query_explain_and_validate() -> Result<(), TestError> {
    let path = temp_path("cli");
    clean(&path)?;

    let create = oxgraphd::run_cli(vec![
        "db".to_owned(),
        "create".to_owned(),
        path.display().to_string(),
    ])?;
    assert!(create.contains("\"ok\":true"));

    let mut database = Db::open(&path)?;
    database.write(|writer| {
        writer.create_element()?;
        Ok(())
    })?;

    let status = oxgraphd::run_cli(vec![
        "db".to_owned(),
        "status".to_owned(),
        path.display().to_string(),
    ])?;
    assert!(status.contains("\"elements\":1"));

    let query = oxgraphd::run_cli(vec![
        "db".to_owned(),
        "query".to_owned(),
        path.display().to_string(),
        "oxql".to_owned(),
        "MATCH".to_owned(),
        "ELEMENTS".to_owned(),
    ])?;
    assert!(query.contains("\"Element\""));

    let explain = oxgraphd::run_cli(vec![
        "db".to_owned(),
        "explain".to_owned(),
        path.display().to_string(),
        "oxql".to_owned(),
        "MATCH".to_owned(),
        "ELEMENTS".to_owned(),
    ])?;
    assert!(explain.contains("scan elements"));

    let validate = oxgraphd::run_cli(vec![
        "db".to_owned(),
        "validate".to_owned(),
        path.display().to_string(),
    ])?;
    assert!(validate.contains("\"ok\":true"));
    clean(&path)?;
    Ok(())
}

#[test]
fn http_facade_covers_status_query_explain_compact_and_validate() -> Result<(), TestError> {
    let path = temp_path("http");
    clean(&path)?;

    Db::create(&path)?;
    let status = oxgraphd::serve_http_request(&path, "GET /v1/status HTTP/1.1\r\n\r\n");
    assert!(status.contains("200 OK"));
    assert!(status.contains("\"elements\":0"));

    let query = oxgraphd::serve_http_request(
        &path,
        "POST /v1/query HTTP/1.1\r\n\r\n{\"language\":\"oxql\",\"query\":\"MATCH ELEMENTS\"}",
    );
    assert!(query.contains("200 OK"));
    assert!(query.contains("\"rows\":[]"));

    let explain = oxgraphd::serve_http_request(
        &path,
        "POST /v1/explain HTTP/1.1\r\n\r\n{\"language\":\"oxql\",\"query\":\"MATCH ELEMENTS\"}",
    );
    assert!(explain.contains("scan elements"));

    let compact = oxgraphd::serve_http_request(&path, "POST /v1/compact HTTP/1.1\r\n\r\n");
    assert!(compact.contains("\"ok\":true"));

    let validate = oxgraphd::serve_http_request(&path, "POST /v1/validate HTTP/1.1\r\n\r\n");
    assert!(validate.contains("\"ok\":true"));
    clean(&path)?;
    Ok(())
}

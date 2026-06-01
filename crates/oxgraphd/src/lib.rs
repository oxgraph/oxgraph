//! Thin HTTP server and CLI facade for `oxgraph-db`.

use std::{
    fmt,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

use oxgraph_db::{Database, DbError, PreparedQuery, QueryLanguage};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Product facade error type.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
pub enum OxgraphdError {
    /// Database operation failed.
    Database {
        /// Source database error.
        source: DbError,
    },
    /// IO operation failed.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Source IO error.
        source: std::io::Error,
    },
    /// JSON encoding or decoding failed.
    Json {
        /// Source JSON error.
        source: serde_json::Error,
    },
    /// Command usage was invalid.
    Usage {
        /// Deterministic usage message.
        message: String,
    },
}

impl fmt::Display for OxgraphdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database { source } => write!(formatter, "{source}"),
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
            Self::Json { source } => write!(formatter, "json error: {source}"),
            Self::Usage { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for OxgraphdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database { source } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Json { source } => Some(source),
            Self::Usage { .. } => None,
        }
    }
}

impl From<DbError> for OxgraphdError {
    fn from(source: DbError) -> Self {
        Self::Database { source }
    }
}

impl From<serde_json::Error> for OxgraphdError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}

/// Runs the `oxgraph` CLI.
///
/// # Errors
///
/// Returns [`OxgraphdError`] when parsing, database work, or JSON rendering
/// fails.
///
/// # Performance
///
/// This function is `O(argument bytes + command work)`.
pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<String, OxgraphdError> {
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [db, create, path] if db == "db" && create == "create" => command_create(path),
        [db, status, path] if db == "db" && status == "status" => command_status(path),
        [db, validate, path] if db == "db" && validate == "validate" => command_validate(path),
        [db, compact, path] if db == "db" && compact == "compact" => command_compact(path),
        [db, catalog, path] if db == "db" && catalog == "catalog" => command_catalog(path),
        [db, projections, path] if db == "db" && projections == "projections" => {
            command_projections(path)
        }
        [db, indexes, path] if db == "db" && indexes == "indexes" => command_indexes(path),
        [db, query, path, language, rest @ ..] if db == "db" && query == "query" => {
            command_query(path, language, rest)
        }
        [db, explain, path, language, rest @ ..] if db == "db" && explain == "explain" => {
            command_explain(path, language, rest)
        }
        _args => Err(usage()),
    }
}

/// Runs the `oxgraphd` server CLI.
///
/// # Errors
///
/// Returns [`OxgraphdError`] when argument parsing or server startup fails.
///
/// # Performance
///
/// This function is `O(argument bytes)` before entering the blocking accept
/// loop.
pub fn run_daemon_cli(args: impl IntoIterator<Item = String>) -> Result<(), OxgraphdError> {
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [path, address] => run_server(path, address),
        _args => Err(OxgraphdError::Usage {
            message: "usage: oxgraphd <database-path> <address>".to_owned(),
        }),
    }
}

/// Runs a blocking JSON HTTP server.
///
/// # Errors
///
/// Returns [`OxgraphdError`] when binding or stream IO fails before request
/// dispatch can produce an HTTP error.
///
/// # Performance
///
/// Binding is `O(address length)`; each request is `O(request bytes + command
/// work)`.
pub fn run_server(path: impl AsRef<Path>, address: &str) -> Result<(), OxgraphdError> {
    let listener = TcpListener::bind(address)
        .map_err(|source| OxgraphdError::io("bind http listener", source))?;
    for stream in listener.incoming() {
        // Per-connection failures (accept errors, read/write IO, malformed
        // framing) are logged and skipped; one bad client must never bring
        // down the daemon. Only the initial bind failure above is fatal.
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_stream(path.as_ref(), stream) {
                    eprintln!("oxgraphd: dropping connection: {error}");
                }
            }
            Err(error) => {
                eprintln!("oxgraphd: accept failed: {error}");
            }
        }
    }
    Ok(())
}

/// Default per-connection read timeout, so a stalled client cannot pin a
/// server thread indefinitely.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum bytes accepted for one request (headers plus body).
const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;

/// Serves one raw HTTP request and returns a raw HTTP response.
///
/// # Performance
///
/// This function is `O(request bytes + endpoint work)`.
#[must_use]
pub fn serve_http_request(default_path: impl AsRef<Path>, request: &str) -> String {
    match dispatch_http(default_path.as_ref(), request) {
        Ok(body) => http_response("200 OK", &body),
        Err(error) => http_response(
            "400 Bad Request",
            &json!({ "ok": false, "error": error.to_string() }).to_string(),
        ),
    }
}

impl OxgraphdError {
    /// Creates an IO error.
    const fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }
}

/// Handles one TCP stream.
fn handle_stream(path: &Path, mut stream: TcpStream) -> Result<(), OxgraphdError> {
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|source| OxgraphdError::io("set read timeout", source))?;
    let request = read_http_request(&mut stream)?;
    let response = serve_http_request(path, &request);
    stream
        .write_all(response.as_bytes())
        .map_err(|source| OxgraphdError::io("write http response", source))?;
    Ok(())
}

/// Reads one HTTP/1.1 request using `Content-Length` framing.
///
/// Reads headers up to the `\r\n\r\n` terminator, parses `Content-Length`, then
/// reads exactly that many body bytes. This does not depend on the client
/// half-closing the socket (so persistent connections work) and bounds the
/// request at [`MAX_REQUEST_BYTES`].
///
/// # Errors
///
/// Returns [`OxgraphdError`] on IO failure, when the request exceeds the size
/// bound, or when the bytes are not valid UTF-8.
///
/// # Performance
///
/// This function is `O(request bytes)`.
fn read_http_request<R: Read>(stream: &mut R) -> Result<String, OxgraphdError> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    // Phase 1: read until the end-of-headers marker (or EOF).
    let header_end = loop {
        if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            return Err(OxgraphdError::Usage {
                message: "request headers exceed the size limit".to_owned(),
            });
        }
        let read = stream
            .read(&mut chunk)
            .map_err(|source| OxgraphdError::io("read http request", source))?;
        if read == 0 {
            // Connection closed before a full header block; serve what we have.
            break buffer.len();
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    // Phase 2: read exactly Content-Length body bytes when declared.
    if let Some(content_length) = parse_content_length(&buffer[..header_end]) {
        let target = header_end.saturating_add(content_length);
        if target > MAX_REQUEST_BYTES {
            return Err(OxgraphdError::Usage {
                message: "request body exceeds the size limit".to_owned(),
            });
        }
        while buffer.len() < target {
            let read = stream
                .read(&mut chunk)
                .map_err(|source| OxgraphdError::io("read http body", source))?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        // Drop anything read past the framed body (e.g. a pipelined request);
        // this server handles one request per connection.
        buffer.truncate(target);
    }

    String::from_utf8(buffer).map_err(|_error| OxgraphdError::Usage {
        message: "request bytes are not valid UTF-8".to_owned(),
    })
}

/// Returns the start index of the first occurrence of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Parses the `Content-Length` header value from a header byte block.
///
/// Header names are matched case-insensitively. Returns `None` when the header
/// is absent or unparseable (the request is then treated as bodyless).
fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let text = core::str::from_utf8(headers).ok()?;
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _value)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_name, value)| value.trim().parse::<usize>().ok())
}

/// Dispatches one HTTP request.
fn dispatch_http(default_path: &Path, request: &str) -> Result<String, OxgraphdError> {
    let parsed = parse_http_request(request)?;
    match (parsed.method.as_str(), parsed.path.as_str()) {
        ("POST", "/v1/create") => endpoint_create(default_path),
        ("POST", "/v1/open") | ("GET", "/v1/status") => endpoint_status(default_path),
        ("POST", "/v1/query") => endpoint_query(default_path, parsed.body),
        ("POST", "/v1/explain") => endpoint_explain(default_path, parsed.body),
        ("POST", "/v1/compact") => endpoint_compact(default_path),
        ("POST", "/v1/validate") => endpoint_validate(default_path),
        ("GET", "/v1/catalog") => endpoint_catalog(default_path),
        ("GET", "/v1/projections") => endpoint_projections(default_path),
        ("GET", "/v1/indexes") => endpoint_indexes(default_path),
        _route => Err(OxgraphdError::Usage {
            message: "unknown endpoint".to_owned(),
        }),
    }
}

/// Creates a database from HTTP.
fn endpoint_create(default_path: &Path) -> Result<String, OxgraphdError> {
    Database::create(default_path)?;
    Ok(json!({ "ok": true }).to_string())
}

/// Returns status from HTTP.
fn endpoint_status(default_path: &Path) -> Result<String, OxgraphdError> {
    command_status_path(default_path)
}

/// Runs query from HTTP.
fn endpoint_query(default_path: &Path, body: &str) -> Result<String, OxgraphdError> {
    let request: QueryRequest = serde_json::from_str(body)?;
    let database = Database::open(default_path)?;
    let read = database.begin_read();
    let prepared = prepare_request(&database, &request)?;
    let result = read.execute(&prepared)?;
    serde_json::to_string(&json!({ "ok": true, "result": result })).map_err(Into::into)
}

/// Explains query from HTTP.
fn endpoint_explain(default_path: &Path, body: &str) -> Result<String, OxgraphdError> {
    let request: QueryRequest = serde_json::from_str(body)?;
    let database = Database::open(default_path)?;
    let prepared = prepare_request(&database, &request)?;
    Ok(json!({ "ok": true, "explain": prepared.explain() }).to_string())
}

/// Compacts a database from HTTP.
fn endpoint_compact(default_path: &Path) -> Result<String, OxgraphdError> {
    let mut database = Database::open(default_path)?;
    database.compact()?;
    Ok(json!({ "ok": true }).to_string())
}

/// Validates a database from HTTP.
fn endpoint_validate(default_path: &Path) -> Result<String, OxgraphdError> {
    Database::validate_path(default_path)?;
    Ok(json!({ "ok": true }).to_string())
}

/// Returns catalog JSON from HTTP.
fn endpoint_catalog(default_path: &Path) -> Result<String, OxgraphdError> {
    command_catalog_path(default_path)
}

/// Returns projection JSON from HTTP.
fn endpoint_projections(default_path: &Path) -> Result<String, OxgraphdError> {
    command_projections_path(default_path)
}

/// Returns index JSON from HTTP.
fn endpoint_indexes(default_path: &Path) -> Result<String, OxgraphdError> {
    command_indexes_path(default_path)
}

/// Creates a database from CLI.
fn command_create(path: &str) -> Result<String, OxgraphdError> {
    Database::create(path)?;
    Ok(json!({ "ok": true, "path": path }).to_string())
}

/// Returns status from CLI.
fn command_status(path: &str) -> Result<String, OxgraphdError> {
    command_status_path(Path::new(path))
}

/// Validates a database from CLI.
fn command_validate(path: &str) -> Result<String, OxgraphdError> {
    Database::validate_path(path)?;
    Ok(json!({ "ok": true }).to_string())
}

/// Compacts a database from CLI.
fn command_compact(path: &str) -> Result<String, OxgraphdError> {
    let mut database = Database::open(path)?;
    database.compact()?;
    Ok(json!({ "ok": true }).to_string())
}

/// Returns catalog metadata from CLI.
fn command_catalog(path: &str) -> Result<String, OxgraphdError> {
    command_catalog_path(Path::new(path))
}

/// Returns projections from CLI.
fn command_projections(path: &str) -> Result<String, OxgraphdError> {
    command_projections_path(Path::new(path))
}

/// Returns indexes from CLI.
fn command_indexes(path: &str) -> Result<String, OxgraphdError> {
    command_indexes_path(Path::new(path))
}

/// Runs a query from CLI.
fn command_query(
    path: &str,
    language: &str,
    query_parts: &[String],
) -> Result<String, OxgraphdError> {
    let database = Database::open(path)?;
    let request = QueryRequest {
        language: language.to_owned(),
        query: query_parts.join(" "),
    };
    let prepared = prepare_request(&database, &request)?;
    let result = database.begin_read().execute(&prepared)?;
    serde_json::to_string(&json!({ "ok": true, "result": result })).map_err(Into::into)
}

/// Explains a query from CLI.
fn command_explain(
    path: &str,
    language: &str,
    query_parts: &[String],
) -> Result<String, OxgraphdError> {
    let database = Database::open(path)?;
    let request = QueryRequest {
        language: language.to_owned(),
        query: query_parts.join(" "),
    };
    let prepared = prepare_request(&database, &request)?;
    Ok(json!({ "ok": true, "explain": prepared.explain() }).to_string())
}

/// Returns status JSON for a path.
fn command_status_path(path: &Path) -> Result<String, OxgraphdError> {
    let database = Database::open(path)?;
    let status = database.status();
    Ok(json!({
        "ok": true,
        "visible_commit_seq": status.visible_commit_seq.get(),
        "last_transaction_id": status.last_transaction_id.get(),
        "elements": status.element_count,
        "relations": status.relation_count,
        "incidences": status.incidence_count,
        "catalog": {
            "roles": status.catalog.role_count,
            "labels": status.catalog.label_count,
            "relation_types": status.catalog.relation_type_count,
            "property_keys": status.catalog.property_key_count,
            "projections": status.catalog.projection_count,
            "indexes": status.catalog.index_count
        }
    })
    .to_string())
}

/// Returns catalog JSON for a path.
fn command_catalog_path(path: &Path) -> Result<String, OxgraphdError> {
    let database = Database::open(path)?;
    let read = database.begin_read();
    let catalog = read.catalog();
    Ok(json!({
        "ok": true,
        "roles": catalog.roles().collect::<Vec<_>>(),
        "labels": catalog.labels().collect::<Vec<_>>(),
        "relation_types": catalog.relation_types().collect::<Vec<_>>(),
        "property_keys": catalog.property_keys().collect::<Vec<_>>()
    })
    .to_string())
}

/// Returns projection JSON for a path.
fn command_projections_path(path: &Path) -> Result<String, OxgraphdError> {
    let database = Database::open(path)?;
    let read = database.begin_read();
    Ok(json!({
        "ok": true,
        "projections": read.catalog().projections().collect::<Vec<_>>()
    })
    .to_string())
}

/// Returns index JSON for a path.
fn command_indexes_path(path: &Path) -> Result<String, OxgraphdError> {
    let database = Database::open(path)?;
    let read = database.begin_read();
    Ok(json!({
        "ok": true,
        "indexes": read.catalog().indexes().collect::<Vec<_>>()
    })
    .to_string())
}

/// Prepares a request query.
fn prepare_request(
    database: &Database,
    request: &QueryRequest,
) -> Result<PreparedQuery, OxgraphdError> {
    let language = parse_language(&request.language)?;
    database
        .prepare(language, &request.query)
        .map_err(Into::into)
}

/// Parses a language token.
fn parse_language(value: &str) -> Result<QueryLanguage, OxgraphdError> {
    match value.to_ascii_lowercase().as_str() {
        "oxql" => Ok(QueryLanguage::Oxql),
        "cypher" => Ok(QueryLanguage::Cypher),
        _other => Err(OxgraphdError::Usage {
            message: "language must be oxql or cypher".to_owned(),
        }),
    }
}

/// Parses a raw HTTP request.
fn parse_http_request(request: &str) -> Result<HttpRequest<'_>, OxgraphdError> {
    let (head, body) = request.split_once("\r\n\r\n").unwrap_or((request, ""));
    let mut lines = head.lines();
    let request_line = lines.next().ok_or_else(|| OxgraphdError::Usage {
        message: "missing request line".to_owned(),
    })?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or_else(|| OxgraphdError::Usage {
        message: "missing method".to_owned(),
    })?;
    let path = parts.next().ok_or_else(|| OxgraphdError::Usage {
        message: "missing path".to_owned(),
    })?;
    Ok(HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        body,
    })
}

/// Builds a raw JSON HTTP response.
fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Returns CLI usage.
fn usage() -> OxgraphdError {
    OxgraphdError::Usage {
        message: "usage: oxgraph db <create|status|query|explain|compact|validate|catalog|projections|indexes> ...".to_owned(),
    }
}

/// HTTP request fields needed by the facade.
struct HttpRequest<'request> {
    /// HTTP method.
    method: String,
    /// HTTP path.
    path: String,
    /// Request body.
    body: &'request str,
}

/// Query request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct QueryRequest {
    /// Language token.
    language: String,
    /// Query text.
    query: String,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{find_subsequence, parse_content_length, read_http_request};

    #[test]
    fn parses_content_length_case_insensitively() {
        let headers = b"POST /v1/query HTTP/1.1\r\nContent-Length: 42\r\n\r\n";
        assert_eq!(parse_content_length(headers), Some(42));
        let lower = b"POST / HTTP/1.1\r\ncontent-length: 7\r\n\r\n";
        assert_eq!(parse_content_length(lower), Some(7));
        let absent = b"GET /v1/status HTTP/1.1\r\n\r\n";
        assert_eq!(parse_content_length(absent), None);
    }

    #[test]
    fn finds_header_terminator() {
        assert_eq!(find_subsequence(b"ab\r\n\r\ncd", b"\r\n\r\n"), Some(2));
        assert_eq!(find_subsequence(b"no terminator", b"\r\n\r\n"), None);
    }

    #[test]
    fn reads_exactly_content_length_body_without_eof() -> Result<(), super::OxgraphdError> {
        // A persistent client sends the request then trailing bytes it has not
        // framed; the reader must stop at Content-Length and not consume them.
        let body = "{\"language\":\"oxql\",\"query\":\"MATCH ELEMENTS\"}";
        let request = format!(
            "POST /v1/query HTTP/1.1\r\ncontent-length: {}\r\n\r\n{body}TRAILING",
            body.len()
        );
        let mut cursor = Cursor::new(request.into_bytes());
        let parsed = read_http_request(&mut cursor)?;
        assert!(parsed.ends_with(body), "body must end the parsed request");
        assert!(!parsed.contains("TRAILING"), "must not read past body");
        Ok(())
    }

    #[test]
    fn reads_bodyless_request() -> Result<(), super::OxgraphdError> {
        let request = b"GET /v1/status HTTP/1.1\r\n\r\n".to_vec();
        let mut cursor = Cursor::new(request);
        let parsed = read_http_request(&mut cursor)?;
        assert!(parsed.starts_with("GET /v1/status"));
        Ok(())
    }
}

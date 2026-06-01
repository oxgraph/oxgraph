//! Native `OxQL` and pinned Cypher-profile query execution.

use oxgraph_graph::{
    CanonicalElementIdentity, CanonicalRelationIdentity, EdgeSourceGraph, EdgeTargetGraph,
    TopologyCounts,
};
use serde::{Deserialize, Serialize};

use crate::{
    DbError, ElementId, IncidenceRecord, ProjectionId, PropertyKeyId, PropertySubject,
    PropertyValue, RelationId,
    catalog::{ProjectionDefinition, PropertyFamily},
    projection::GraphProjection,
    state::DatabaseState,
    traversal::{self, TraversalDirection, TraversalOptions},
    value::parse_value_token,
};

/// Query frontend language.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum QueryLanguage {
    /// Native topology-first `OxQL` profile.
    Oxql,
    /// Pinned Cypher language profile over property-graph projections.
    Cypher,
}

/// Prepared query plan.
///
/// # Performance
///
/// Cloning is `O(query length)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedQuery {
    /// Source language.
    language: QueryLanguage,
    /// Original query text.
    text: String,
    /// Physical plan.
    plan: QueryPlan,
}

impl PreparedQuery {
    /// Parses and prepares one query string against current catalog metadata.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::EmptyQuery`] or [`DbError::UnsupportedQuery`] when the
    /// query is outside the supported profile.
    ///
    /// # Performance
    ///
    /// This function is `O(query length + catalog lookup cost)`.
    pub(crate) fn prepare(
        language: QueryLanguage,
        query: &str,
        state: &DatabaseState,
    ) -> Result<Self, DbError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(DbError::EmptyQuery);
        }
        let logical = match language {
            QueryLanguage::Oxql => parse_oxql(trimmed)?,
            QueryLanguage::Cypher => parse_cypher(trimmed)?,
        };
        let plan = bind_and_lower(logical, state)?;
        Ok(Self {
            language,
            text: trimmed.to_owned(),
            plan,
        })
    }

    /// Returns the query language.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn language(&self) -> QueryLanguage {
        self.language
    }

    /// Returns the source query text.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns a stable physical explanation.
    ///
    /// # Performance
    ///
    /// This method is `O(plan size)`.
    #[must_use]
    pub fn explain(&self) -> String {
        self.plan.explain()
    }

    /// Executes this prepared plan.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when a referenced projection cannot be materialized.
    ///
    /// # Performance
    ///
    /// This method is `O(plan output + projection build cost when used)`.
    pub(crate) fn execute(&self, state: &DatabaseState) -> Result<QueryResult, DbError> {
        self.plan.execute(state)
    }
}

/// Query result materialized for JSON, CLI, and embedded consumers.
///
/// # Performance
///
/// Iterating rows is `O(row count)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueryResult {
    /// Materialized rows.
    rows: Vec<QueryRow>,
}

impl QueryResult {
    /// Creates a result from rows.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` excluding row ownership transfer.
    #[must_use]
    pub(crate) const fn new(rows: Vec<QueryRow>) -> Self {
        Self { rows }
    }

    /// Returns result rows.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn rows(&self) -> &[QueryRow] {
        &self.rows
    }
}

/// One query result row.
///
/// # Performance
///
/// Cloning is `O(value count + string/property bytes)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueryRow {
    /// Values carried by the row.
    pub values: Vec<QueryValue>,
}

impl QueryRow {
    /// Creates a single-value row.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` excluding value ownership transfer.
    #[must_use]
    pub(crate) fn single(value: QueryValue) -> Self {
        Self {
            values: vec![value],
        }
    }
}

/// Query value variants used by `OxQL` and the pinned Cypher profile.
///
/// # Performance
///
/// Cloning is `O(value bytes)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum QueryValue {
    /// Canonical element id.
    Element(ElementId),
    /// Canonical relation id.
    Relation(RelationId),
    /// Canonical incidence record.
    Incidence(IncidenceRecord),
    /// Property subject.
    Subject(PropertySubject),
    /// Typed property value.
    Property(PropertyValue),
    /// Human-readable text value.
    Text(String),
    /// Projection id.
    Projection(ProjectionId),
}

/// Private physical query plan.
#[derive(Clone, Debug, Eq, PartialEq)]
enum QueryPlan {
    /// Scan all elements.
    ElementScan,
    /// Scan all relations.
    RelationScan,
    /// Scan all incidences.
    IncidenceScan,
    /// Scan elements with a label.
    ElementLabelScan {
        /// Label name for explanations.
        label: String,
        /// Label ID.
        label_id: crate::LabelId,
    },
    /// Scan elements by property equality.
    ElementPropertyEqual {
        /// Property key name for explanations.
        key: String,
        /// Property key ID.
        key_id: PropertyKeyId,
        /// Required property value.
        value: PropertyValue,
    },
    /// Scan relations by relation type.
    RelationTypeScan {
        /// Relation type name for explanations.
        relation_type: String,
        /// Relation type ID.
        relation_type_id: crate::RelationTypeId,
    },
    /// Walk a graph projection.
    GraphWalk {
        /// Projection ID.
        projection: ProjectionId,
        /// Canonical starting element.
        element: ElementId,
        /// Traversal options.
        options: TraversalOptions,
    },
    /// Cypher directed edge triple scan.
    CypherDirectedTriples {
        /// Graph projection ID.
        projection: ProjectionId,
    },
    /// Catalog metadata scan.
    CatalogScan,
}

impl QueryPlan {
    /// Explains this physical plan.
    fn explain(&self) -> String {
        match self {
            Self::ElementScan => "oxql scan elements".to_owned(),
            Self::RelationScan => "oxql scan relations".to_owned(),
            Self::IncidenceScan => "oxql scan incidences".to_owned(),
            Self::ElementLabelScan { label, .. } => {
                format!("oxql label index lookup elements label={label}")
            }
            Self::ElementPropertyEqual { key, value, .. } => {
                format!("oxql property equality lookup elements {key}={value}")
            }
            Self::RelationTypeScan { relation_type, .. } => {
                format!("oxql relation-type index lookup type={relation_type}")
            }
            Self::GraphWalk {
                projection,
                element,
                options,
            } => format!(
                "oxql graph projection {projection} walk from {element} depth {} direction {:?} limit {}",
                options.max_depth, options.direction, options.limit,
            ),
            Self::CypherDirectedTriples { projection } => {
                format!("cypher directed triple scan projection={projection}")
            }
            Self::CatalogScan => "catalog metadata scan".to_owned(),
        }
    }

    /// Executes this physical plan.
    fn execute(&self, state: &DatabaseState) -> Result<QueryResult, DbError> {
        match self {
            Self::ElementScan => Ok(scan_elements(state)),
            Self::RelationScan => Ok(scan_relations(state)),
            Self::IncidenceScan => Ok(scan_incidences(state)),
            Self::ElementLabelScan { label_id, .. } => {
                Ok(scan_elements_with_label(state, *label_id))
            }
            Self::ElementPropertyEqual { key_id, value, .. } => {
                scan_elements_with_property(state, *key_id, value)
            }
            Self::RelationTypeScan {
                relation_type_id, ..
            } => Ok(scan_relations_with_type(state, *relation_type_id)),
            Self::GraphWalk {
                projection,
                element,
                options,
            } => execute_graph_walk(state, *projection, *element, *options),
            Self::CypherDirectedTriples { projection } => {
                execute_cypher_triples(state, *projection)
            }
            Self::CatalogScan => Ok(scan_catalog(state)),
        }
    }
}

/// Resolved logical operation emitted by parsing, before catalog binding.
///
/// The parsers ([`parse_oxql`], [`parse_cypher`]) are purely token/string
/// driven and never touch the catalog; they produce one of these ops carrying
/// raw names and literals. [`bind_and_lower`] then resolves names to catalog
/// IDs, validates value families/types, and produces the physical
/// [`QueryPlan`]. Cypher node and label scans lower onto the shared element ops
/// rather than duplicating them.
#[derive(Clone, Debug, Eq, PartialEq)]
enum LogicalOp {
    /// Scan all elements.
    ElementScan,
    /// Scan all relations.
    RelationScan,
    /// Scan all incidences.
    IncidenceScan,
    /// Scan catalog metadata.
    CatalogScan,
    /// Scan elements carrying a named label.
    ElementLabelScan {
        /// Unresolved label name.
        label: String,
    },
    /// Scan relations of a named relation type.
    RelationTypeScan {
        /// Unresolved relation type name.
        relation_type: String,
    },
    /// Scan elements whose named property equals a literal token.
    ElementPropertyEqual {
        /// Unresolved property key name.
        key: String,
        /// Unparsed property value token.
        value: String,
    },
    /// Walk a named graph projection from a raw element-ID token.
    GraphWalk {
        /// Unresolved projection name.
        projection: String,
        /// Unparsed element-ID token.
        element: String,
        /// Parsed traversal options.
        options: TraversalOptions,
    },
    /// Scan the first graph projection as directed source/relation/target triples.
    CypherDirectedTriples,
}

/// Parses native `OxQL` into a logical operation without catalog access.
fn parse_oxql(query: &str) -> Result<LogicalOp, DbError> {
    let tokens = query.split_whitespace().collect::<Vec<_>>();
    let upper = tokens
        .iter()
        .map(|token| token.to_ascii_uppercase())
        .collect::<Vec<_>>();
    match upper.as_slice() {
        [command] if command == "CATALOG" => Ok(LogicalOp::CatalogScan),
        [verb, family] if verb == "MATCH" && family == "ELEMENTS" => Ok(LogicalOp::ElementScan),
        [verb, family] if verb == "MATCH" && family == "RELATIONS" => Ok(LogicalOp::RelationScan),
        [verb, family] if verb == "MATCH" && family == "INCIDENCES" => Ok(LogicalOp::IncidenceScan),
        [verb, family, has, object, _name]
            if verb == "MATCH" && family == "ELEMENTS" && has == "HAS" && object == "LABEL" =>
        {
            Ok(LogicalOp::ElementLabelScan {
                label: tokens[4].to_owned(),
            })
        }
        [verb, family, r#type, _name]
            if verb == "MATCH" && family == "RELATIONS" && r#type == "TYPE" =>
        {
            Ok(LogicalOp::RelationTypeScan {
                relation_type: tokens[3].to_owned(),
            })
        }
        [verb, family, r#where, _key, equals, _value]
            if verb == "MATCH" && family == "ELEMENTS" && r#where == "WHERE" && equals == "=" =>
        {
            Ok(LogicalOp::ElementPropertyEqual {
                key: tokens[3].to_owned(),
                value: tokens[5].to_owned(),
            })
        }
        [graph, _projection, neighbors, _element]
            if graph == "GRAPH" && neighbors == "NEIGHBORS" =>
        {
            Ok(LogicalOp::GraphWalk {
                projection: tokens[1].to_owned(),
                element: tokens[3].to_owned(),
                options: TraversalOptions::default(),
            })
        }
        [
            graph,
            _projection,
            walk,
            from,
            _element,
            depth,
            _max_depth,
            ..,
        ] if graph == "GRAPH" && walk == "WALK" && from == "FROM" && depth == "DEPTH" => {
            parse_graph_walk(&tokens, &upper)
        }
        _tokens => Err(DbError::unsupported("unsupported OxQL profile query")),
    }
}

/// Parses the pinned Cypher language profile into a logical operation.
fn parse_cypher(query: &str) -> Result<LogicalOp, DbError> {
    if query == "MATCH (n) RETURN n" {
        return Ok(LogicalOp::ElementScan);
    }
    if query == "MATCH (n)-[r]->(m) RETURN n,r,m" {
        return Ok(LogicalOp::CypherDirectedTriples);
    }
    if let Some(label) = query
        .strip_prefix("MATCH (n:")
        .and_then(|rest| rest.strip_suffix(") RETURN n"))
    {
        return Ok(LogicalOp::ElementLabelScan {
            label: label.to_owned(),
        });
    }
    Err(DbError::unsupported("unsupported Cypher profile query"))
}

/// Parses the optional `DIRECTION`/`LIMIT` clauses of an `OxQL` graph walk.
fn parse_graph_walk(tokens: &[&str], upper: &[String]) -> Result<LogicalOp, DbError> {
    let max_depth = tokens[6]
        .parse::<usize>()
        .map_err(|_error| DbError::unsupported("walk depth must be an integer"))?;
    let mut options = TraversalOptions {
        max_depth,
        ..TraversalOptions::default()
    };
    let mut saw_direction = false;
    let mut saw_limit = false;
    let mut index = 7;
    while index < tokens.len() {
        let Some(value) = tokens.get(index + 1) else {
            return Err(DbError::unsupported("walk option requires a value"));
        };
        match upper[index].as_str() {
            "DIRECTION" if !saw_direction => {
                options.direction = parse_walk_direction(&upper[index + 1])?;
                saw_direction = true;
            }
            "DIRECTION" => return Err(DbError::unsupported("walk direction specified twice")),
            "LIMIT" if !saw_limit => {
                options.limit = value
                    .parse::<usize>()
                    .map_err(|_error| DbError::unsupported("walk limit must be an integer"))?;
                saw_limit = true;
            }
            "LIMIT" => return Err(DbError::unsupported("walk limit specified twice")),
            _option => return Err(DbError::unsupported("unsupported walk option")),
        }
        index += 2;
    }
    Ok(LogicalOp::GraphWalk {
        projection: tokens[1].to_owned(),
        element: tokens[4].to_owned(),
        options,
    })
}

/// Parses one `OxQL` graph walk direction.
fn parse_walk_direction(direction: &str) -> Result<TraversalDirection, DbError> {
    match direction {
        "OUTGOING" => Ok(TraversalDirection::Outgoing),
        "INCOMING" => Ok(TraversalDirection::Incoming),
        "BOTH" => Ok(TraversalDirection::Both),
        _direction => Err(DbError::unsupported(
            "walk direction must be outgoing, incoming, or both",
        )),
    }
}

/// Binds a logical operation against the catalog and lowers it to a plan.
///
/// This is the single seam where names resolve to catalog IDs and where
/// property literals are parsed and family/type-validated. Parsing above never
/// touches the catalog; execution below never resolves names.
fn bind_and_lower(op: LogicalOp, state: &DatabaseState) -> Result<QueryPlan, DbError> {
    match op {
        LogicalOp::ElementScan => Ok(QueryPlan::ElementScan),
        LogicalOp::RelationScan => Ok(QueryPlan::RelationScan),
        LogicalOp::IncidenceScan => Ok(QueryPlan::IncidenceScan),
        LogicalOp::CatalogScan => Ok(QueryPlan::CatalogScan),
        LogicalOp::ElementLabelScan { label } => {
            let label_id = state
                .catalog()
                .label_id(&label)
                .ok_or_else(|| DbError::unsupported(format!("unknown label {label}")))?;
            Ok(QueryPlan::ElementLabelScan { label, label_id })
        }
        LogicalOp::RelationTypeScan { relation_type } => {
            let relation_type_id = state
                .catalog()
                .relation_type_id(&relation_type)
                .ok_or_else(|| {
                    DbError::unsupported(format!("unknown relation type {relation_type}"))
                })?;
            Ok(QueryPlan::RelationTypeScan {
                relation_type,
                relation_type_id,
            })
        }
        LogicalOp::ElementPropertyEqual { key, value } => {
            let key_id = state
                .catalog()
                .property_key_id(&key)
                .ok_or_else(|| DbError::unsupported(format!("unknown property key {key}")))?;
            let value = parse_value_token(&value).map_err(DbError::unsupported)?;
            state.validate_lookup_value_for_family(key_id, PropertyFamily::Element, &value)?;
            Ok(QueryPlan::ElementPropertyEqual { key, key_id, value })
        }
        LogicalOp::GraphWalk {
            projection,
            element,
            options,
        } => lower_graph_walk(&projection, &element, options, state),
        LogicalOp::CypherDirectedTriples => Ok(QueryPlan::CypherDirectedTriples {
            projection: first_graph_projection(state)?,
        }),
    }
}

/// Resolves and validates an `OxQL` graph traversal into a physical walk.
fn lower_graph_walk(
    projection: &str,
    element: &str,
    options: TraversalOptions,
    state: &DatabaseState,
) -> Result<QueryPlan, DbError> {
    let projection = state
        .catalog()
        .projection_id(projection)
        .ok_or_else(|| DbError::unsupported(format!("unknown projection {projection}")))?;
    let entry = state
        .catalog()
        .projection(projection)
        .ok_or(DbError::UnknownProjection { id: projection })?;
    if matches!(&entry.definition, ProjectionDefinition::Hypergraph(_)) {
        return Err(DbError::invalid_projection("projection is not a graph"));
    }
    let raw = element
        .parse::<u64>()
        .map_err(|_error| DbError::unsupported("element id must be an integer"))?;
    Ok(QueryPlan::GraphWalk {
        projection,
        element: ElementId::new(raw),
        options,
    })
}

/// Scans all elements.
fn scan_elements(state: &DatabaseState) -> QueryResult {
    QueryResult::new(
        state
            .elements()
            .map(|record| QueryRow::single(QueryValue::Element(record.id)))
            .collect(),
    )
}

/// Scans all relations.
fn scan_relations(state: &DatabaseState) -> QueryResult {
    QueryResult::new(
        state
            .relations()
            .map(|record| QueryRow::single(QueryValue::Relation(record.id)))
            .collect(),
    )
}

/// Scans all incidences.
fn scan_incidences(state: &DatabaseState) -> QueryResult {
    QueryResult::new(
        state
            .incidences()
            .map(|record| QueryRow::single(QueryValue::Incidence(*record)))
            .collect(),
    )
}

/// Scans elements with one label.
fn scan_elements_with_label(state: &DatabaseState, label_id: crate::LabelId) -> QueryResult {
    QueryResult::new(
        state
            .elements_with_label(label_id)
            .into_iter()
            .map(|id| QueryRow::single(QueryValue::Element(id)))
            .collect(),
    )
}

/// Scans elements with a property value.
fn scan_elements_with_property(
    state: &DatabaseState,
    key: PropertyKeyId,
    value: &PropertyValue,
) -> Result<QueryResult, DbError> {
    Ok(QueryResult::new(
        state
            .typed_property_equal_for_family(key, PropertyFamily::Element, value)?
            .into_iter()
            .filter_map(|subject| match subject {
                PropertySubject::Element(id) => Some(QueryRow::single(QueryValue::Element(id))),
                PropertySubject::Relation(_) | PropertySubject::Incidence(_) => None,
            })
            .collect(),
    ))
}

/// Scans relations with one relation type.
fn scan_relations_with_type(
    state: &DatabaseState,
    relation_type: crate::RelationTypeId,
) -> QueryResult {
    QueryResult::new(
        state
            .relations_with_type(relation_type)
            .into_iter()
            .map(|id| QueryRow::single(QueryValue::Relation(id)))
            .collect(),
    )
}

/// Scans catalog entries.
fn scan_catalog(state: &DatabaseState) -> QueryResult {
    let rows = state
        .catalog()
        .projections()
        .map(|entry| QueryRow {
            values: vec![
                QueryValue::Projection(entry.id),
                QueryValue::Text(entry.definition.name().to_owned()),
            ],
        })
        .collect();
    QueryResult::new(rows)
}

/// Executes a graph walk.
fn execute_graph_walk(
    state: &DatabaseState,
    projection: ProjectionId,
    element: ElementId,
    options: TraversalOptions,
) -> Result<QueryResult, DbError> {
    let graph = graph_projection(state, projection)?;
    let traversal = traversal::traverse_graph_projection(&graph, &[element], options)?;
    let rows = traversal
        .rows()
        .iter()
        .map(|row| QueryRow::single(QueryValue::Element(row.element)))
        .collect();
    Ok(QueryResult::new(rows))
}

/// Executes the Cypher directed triple profile.
fn execute_cypher_triples(
    state: &DatabaseState,
    projection: ProjectionId,
) -> Result<QueryResult, DbError> {
    let graph = graph_projection(state, projection)?;
    let mut rows = Vec::with_capacity(graph.relation_count());
    for index in 0..graph.relation_count() {
        let edge = projection_relation(index)?;
        rows.push(QueryRow {
            values: vec![
                QueryValue::Element(graph.canonical_element_id(graph.source(edge))),
                QueryValue::Relation(graph.canonical_relation_id(edge)),
                QueryValue::Element(graph.canonical_element_id(graph.target(edge))),
            ],
        });
    }
    Ok(QueryResult::new(rows))
}

/// Materializes a graph projection by ID.
fn graph_projection(
    state: &DatabaseState,
    projection: ProjectionId,
) -> Result<GraphProjection, DbError> {
    let entry = state
        .catalog()
        .projection(projection)
        .ok_or(DbError::UnknownProjection { id: projection })?;
    match &entry.definition {
        ProjectionDefinition::Graph(definition) => {
            GraphProjection::from_state(state, definition.clone())
        }
        ProjectionDefinition::Hypergraph(_definition) => {
            Err(DbError::invalid_projection("projection is not a graph"))
        }
    }
}

/// Finds the first graph projection in catalog order.
fn first_graph_projection(state: &DatabaseState) -> Result<ProjectionId, DbError> {
    state
        .catalog()
        .projections()
        .find_map(|entry| match &entry.definition {
            ProjectionDefinition::Graph(_definition) => Some(entry.id),
            ProjectionDefinition::Hypergraph(_definition) => None,
        })
        .ok_or_else(|| DbError::unsupported("Cypher profile requires a graph projection"))
}

/// Builds a projection relation ID for query execution.
fn projection_relation(index: usize) -> Result<crate::ProjectionRelationId, DbError> {
    u32::try_from(index)
        .map(crate::ProjectionRelationId::new)
        .map_err(|_error| DbError::IdOverflow)
}

//! YAML to hypergraph with full write-back loop.
//!
//! Pipeline:
//!
//! 1. Stream YAML events from a `services.yaml` fixture via `saphyr-parser` and accumulate them
//!    into a `ServicesModel` — the meaning-layer source of truth.
//! 2. Ask the model for a fresh BCSR snapshot: it builds a new `HypergraphBuilder`, allocates
//!    vertices in canonical (alphabetical) order, emits one directed hyperedge per `depends_on:
//!    [...]` declaration, freezes, attaches a UTF-8 `vertex_name` property layer, and exports
//!    snapshot bytes.
//! 3. Write the bytes through a small `SnapshotSink` trait. Two impls are shown (`File`,
//!    `Vec<u8>`). The same trait is the plug-in point for database BLOBs, KV stores, S3, etc.
//! 4. Reopen the snapshot, decode the property layer back to a `StringArray` via
//!    `DecodedPropertyLayer::decode_all`, walk every hyperedge, and rebuild a `ServicesModel` from
//!    the topology + names. The substrate stays name-blind; only the property layer carries names
//!    across the boundary.
//! 5. Demonstrate write-back: apply edits to the model (add a `metrics` service, remove `worker`),
//!    rebuild a brand-new snapshot from scratch (append-only substrate; the deletion is handled at
//!    the meaning layer simply by not re-adding `worker`), emit YAML, and write it to disk.
//!
//! Run with:
//! `cargo run -p oxgraph-hyper-bcsr --features build,build-property-arrow \
//!  --example yaml_to_hypergraph`

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::Path,
    sync::Arc,
};

use arrow_array::{Array, StringArray};
use arrow_schema::{DataType, Field};
use oxgraph_hyper::DirectedHyperedgeParticipants;
use oxgraph_hyper_bcsr::{
    BcsrHyperedgeId, BcsrSnapshotError, BcsrSnapshotHypergraph,
    build::{
        HyperBuildError, HyperVertexId, HypergraphBuilder, export_bcsr_snapshot_with_properties,
    },
};
use oxgraph_property::{
    DecodedPropertyData, DecodedPropertyLayer, HyperPropertyLayers, IdFamily, LayerId, LayerRole,
    PropertyError, PropertyLayer, PropertyLayerDescriptor, StorageMode,
};
use oxgraph_snapshot::{Snapshot, SnapshotError};
use saphyr_parser::{Event, Parser, ScanError};

/// Inline copy of the example fixture, embedded so the binary is path-free.
const SERVICES_YAML: &str = include_str!("services.yaml");

/// Errors raised by the example pipeline.
#[derive(Debug)]
enum DemoError {
    /// YAML parsing failed.
    Yaml(ScanError),
    /// Hypergraph build or snapshot export failed.
    Build(HyperBuildError<u32, u32, u32>),
    /// Property descriptor or layer construction failed.
    Property(PropertyError),
    /// Snapshot byte container could not be opened.
    Snapshot(SnapshotError),
    /// BCSR view could not be opened over the snapshot.
    BcsrSnapshot(BcsrSnapshotError),
    /// File-system or in-memory sink reported an I/O failure.
    Io(io::Error),
    /// Expected an Arrow `Utf8` column for the `vertex_name` layer.
    UnexpectedLayerType {
        /// Layer name decoded from the snapshot.
        name: String,
        /// Arrow data type that was actually present.
        actual: DataType,
    },
    /// The reopened snapshot did not contain the expected `vertex_name` layer.
    MissingNameLayer,
    /// A decoded name array length disagreed with the hypergraph vertex count.
    NameLayerLengthMismatch {
        /// Vertex count reported by the BCSR view.
        vertex_count: usize,
        /// Element count carried by the property layer.
        layer_len: usize,
    },
}

impl fmt::Display for DemoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Yaml(error) => write!(formatter, "yaml: {error}"),
            Self::Build(error) => write!(formatter, "build: {error}"),
            Self::Property(error) => write!(formatter, "property: {error}"),
            Self::Snapshot(error) => write!(formatter, "snapshot: {error}"),
            Self::BcsrSnapshot(error) => write!(formatter, "bcsr snapshot: {error}"),
            Self::Io(error) => write!(formatter, "io: {error}"),
            Self::UnexpectedLayerType { name, actual } => {
                write!(
                    formatter,
                    "expected Utf8 vertex_name layer `{name}`, found {actual}"
                )
            }
            Self::MissingNameLayer => formatter.write_str("snapshot missing vertex_name layer"),
            Self::NameLayerLengthMismatch {
                vertex_count,
                layer_len,
            } => write!(
                formatter,
                "vertex_name layer length {layer_len} != {vertex_count} vertices"
            ),
        }
    }
}

impl std::error::Error for DemoError {}

impl From<ScanError> for DemoError {
    fn from(error: ScanError) -> Self {
        Self::Yaml(error)
    }
}

impl From<HyperBuildError<u32, u32, u32>> for DemoError {
    fn from(error: HyperBuildError<u32, u32, u32>) -> Self {
        Self::Build(error)
    }
}

impl From<PropertyError> for DemoError {
    fn from(error: PropertyError) -> Self {
        Self::Property(error)
    }
}

impl From<SnapshotError> for DemoError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<BcsrSnapshotError> for DemoError {
    fn from(error: BcsrSnapshotError) -> Self {
        Self::BcsrSnapshot(error)
    }
}

impl From<io::Error> for DemoError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Sink trait for snapshot bytes.
///
/// The substrate produces a snapshot as `Vec<u8>`; everything downstream is
/// "where do those bytes go?". A `File`, a `Vec<u8>`, or any `io::Write` is a
/// valid sink. The blanket impl below means that adding a Postgres BLOB, an
/// S3 multipart upload, an `RocksDB` column family, or a KV store amounts to
/// implementing `io::Write` for the target type.
trait SnapshotSink {
    /// Writes one full snapshot in a single call.
    fn write_snapshot(&mut self, bytes: &[u8]) -> io::Result<()>;
}

impl<W: Write> SnapshotSink for W {
    fn write_snapshot(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(bytes)
    }
}

// Imagine an additional `SnapshotSink` impl for a database BLOB column, a KV
// store entry, or an object-storage key:
//
// ```ignore
// struct PostgresBlobSink<'tx> { tx: &'tx mut Transaction, snapshot_id: i64 }
// impl SnapshotSink for PostgresBlobSink<'_> {
//     fn write_snapshot(&mut self, bytes: &[u8]) -> io::Result<()> { /* COPY ... BYTEA */ }
// }
// ```
//
// Similar adapters apply to an `RocksDB` column family, an Azure Blob block,
// or an S3 multipart upload.

/// Meaning-layer model of a `services.yaml` manifest.
///
/// `ServicesModel` is the YAML editor's authoritative state. The substrate
/// (`oxgraph-hyper-bcsr`) only knows topology — vertices and hyperedges — and
/// is append-only by design. Names live here, in the property-layer-adjacent
/// world above the substrate; removals live here too, because the substrate
/// has no concept of removal.
///
/// The write-back loop is:
///
/// 1. Apply edits to the model (`set_service`, `remove_service`).
/// 2. Call [`to_snapshot`](ServicesModel::to_snapshot) — it constructs a brand new
///    `HypergraphBuilder` from scratch, freezes, attaches the name property layer, and exports
///    bytes. There is no in-place mutation of the substrate.
/// 3. Call [`to_yaml`](ServicesModel::to_yaml) to render the manifest text.
/// 4. Write the YAML through any byte sink.
///
/// # Ordering
///
/// `BTreeMap` is chosen so output is deterministic across runs: services are
/// emitted in alphabetical order. A read-modify-write loop that wants to
/// preserve the original document's service order should swap to an
/// order-preserving map such as `IndexMap`.
///
/// `depends_on` lists go through a second canonicalization: [`to_snapshot`]
/// allocates vertex IDs in alphabetical-by-name order and the BCSR substrate
/// sorts hyperedge participants by vertex ID at freeze time, so a round-trip
/// rewrites every list in alphabetical order. Any application that needs to
/// preserve user-authored dep order must store it in a separate sidecar
/// property layer (a vertex- or hyperedge-keyed Arrow column).
///
/// [`to_snapshot`]: Self::to_snapshot
///
/// # Performance
///
/// All operations are dominated by the underlying `BTreeMap` and `Vec` costs.
struct ServicesModel {
    /// Service name → ordered list of dependency names (in declaration order).
    services: BTreeMap<String, Vec<String>>,
}

impl ServicesModel {
    /// Returns an empty model.
    const fn new() -> Self {
        Self {
            services: BTreeMap::new(),
        }
    }

    /// Returns the number of declared services.
    fn service_count(&self) -> usize {
        self.services.len()
    }

    /// Returns the total number of declared `depends_on` entries.
    fn dependency_count(&self) -> usize {
        self.services.values().map(Vec::len).sum()
    }

    /// Inserts or replaces a service entry.
    ///
    /// Dependency targets are stored verbatim; if a target name does not
    /// appear as its own service entry, the snapshot path materializes it as
    /// a vertex with no outgoing hyperedge (i.e. a stub).
    fn set_service<S>(&mut self, name: S, deps: Vec<String>)
    where
        S: Into<String>,
    {
        self.services.insert(name.into(), deps);
    }

    /// Removes a service entry and strips any `depends_on` edge pointing at it.
    ///
    /// This is the substrate-bypass for deletion: the next call to
    /// [`to_snapshot`](Self::to_snapshot) simply does not re-allocate a vertex
    /// for the removed service, so the rebuilt topology has no trace of it.
    fn remove_service(&mut self, name: &str) {
        self.services.remove(name);
        for deps in self.services.values_mut() {
            deps.retain(|dep| dep != name);
        }
    }

    /// Parses YAML into a model using a streaming SAX walk.
    ///
    /// The whole document is never held as an owned tree: events are consumed
    /// one at a time, and the model only retains the service/dependency text.
    fn from_yaml(yaml: &str) -> Result<Self, DemoError> {
        let mut ingest = YamlIngest::new();
        let mut parser = Parser::new_from_str(yaml);
        while let Some(event) = parser.next_event() {
            let (event, _span) = event?;
            ingest.handle(event);
        }
        Ok(ingest.into_model())
    }

    /// Rebuilds the model from a frozen BCSR snapshot.
    ///
    /// Reads the `vertex_name` property layer, walks every hyperedge, and
    /// reconstructs the (service → deps) map. Names known only as dependency
    /// targets — i.e. vertices that never appeared as a top-level service in
    /// the original YAML — are still registered as zero-dependency entries so
    /// the round-trip preserves the full vertex set.
    fn from_snapshot(bytes: &[u8]) -> Result<Self, DemoError> {
        let snapshot = Snapshot::open(bytes)?;
        let view = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot)?;
        let layers = DecodedPropertyLayer::decode_all::<u32>(&snapshot)?;
        let name_layer = layers
            .iter()
            .find(|layer| layer.id_family == IdFamily::Element && layer.name == "vertex_name")
            .ok_or(DemoError::MissingNameLayer)?;
        let DecodedPropertyData::Dense {
            values: name_values,
        } = &name_layer.data
        else {
            return Err(DemoError::UnexpectedLayerType {
                name: name_layer.name.clone(),
                actual: name_layer
                    .data
                    .data_type()
                    .cloned()
                    .unwrap_or(DataType::Utf8),
            });
        };
        let names = name_values
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| DemoError::UnexpectedLayerType {
                name: name_layer.name.clone(),
                actual: name_values.data_type().clone(),
            })?;
        if names.len() != view.vertex_count() {
            return Err(DemoError::NameLayerLengthMismatch {
                vertex_count: view.vertex_count(),
                layer_len: names.len(),
            });
        }

        let mut services: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for vertex_index in 0..names.len() {
            services.insert(names.value(vertex_index).to_owned(), Vec::new());
        }
        // The BCSR substrate canonicalizes hyperedge participants to vertex-ID
        // order at freeze time; since we allocate IDs in alphabetical name
        // order in `to_snapshot`, deps end up alphabetical here.
        let hyperedge_count = u32::try_from(view.hyperedge_count()).unwrap_or(u32::MAX);
        for hyperedge in 0..hyperedge_count {
            let id = BcsrHyperedgeId(hyperedge);
            let targets: Vec<String> = view
                .target_participants(id)
                .map(|v| names.value(v.0 as usize).to_owned())
                .collect();
            for source in view.source_participants(id) {
                let source_name = names.value(source.0 as usize).to_owned();
                merge_deps(services.entry(source_name).or_default(), &targets);
            }
        }
        Ok(Self { services })
    }

    /// Renders the model as canonical YAML.
    ///
    /// Services and their dependency lists are emitted in `BTreeMap` order so
    /// the output is byte-stable for any given model state.
    fn to_yaml(&self) -> String {
        let mut out = String::from("services:\n");
        for (name, deps) in &self.services {
            out.push_str("  ");
            out.push_str(name);
            if deps.is_empty() {
                out.push_str(": {}\n");
            } else {
                out.push_str(":\n    depends_on: [");
                out.push_str(&deps.join(", "));
                out.push_str("]\n");
            }
        }
        out
    }

    /// Saves the model's snapshot bytes to a file path through a
    /// `SnapshotSink`. Convenience over `to_snapshot` + manual file write.
    fn save_snapshot_to(&self, path: &Path) -> Result<(), DemoError> {
        let bytes = self.to_snapshot()?;
        let mut sink = BufWriter::new(File::create(path)?);
        sink.write_snapshot(&bytes)?;
        sink.flush()?;
        Ok(())
    }

    /// Saves the model's YAML rendering to a file path. Convenience over
    /// `to_yaml` + manual file write.
    fn save_yaml_to(&self, path: &Path) -> io::Result<()> {
        let mut sink = BufWriter::new(File::create(path)?);
        sink.write_all(self.to_yaml().as_bytes())?;
        sink.flush()
    }

    /// Builds a brand new BCSR snapshot from the current model state.
    ///
    /// This is the rebuild-on-commit step of the write-back loop. The previous
    /// builder is discarded and a fresh one is constructed: every vertex is
    /// re-allocated, every hyperedge is re-emitted, and the name property
    /// layer is regenerated aligned with the new vertex IDs.
    fn to_snapshot(&self) -> Result<Vec<u8>, DemoError> {
        let mut all_names: BTreeSet<&str> = BTreeSet::new();
        for (name, deps) in &self.services {
            all_names.insert(name.as_str());
            for dep in deps {
                all_names.insert(dep.as_str());
            }
        }

        let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
        let mut id_by_name: BTreeMap<&str, HyperVertexId<u32>> = BTreeMap::new();
        for name in all_names {
            let id = builder.add_vertex()?;
            id_by_name.insert(name, id);
        }

        for (source_name, deps) in &self.services {
            if deps.is_empty() {
                continue;
            }
            let Some(&source_id) = id_by_name.get(source_name.as_str()) else {
                continue;
            };
            let target_ids: Vec<HyperVertexId<u32>> = deps
                .iter()
                .filter_map(|dep| id_by_name.get(dep.as_str()).copied())
                .collect();
            if !target_ids.is_empty() {
                builder.add_hyperedge(&[source_id], &target_ids)?;
            }
        }

        let frozen = builder.freeze()?;

        let mut ordered: Vec<&str> = vec![""; id_by_name.len()];
        for (name, id) in &id_by_name {
            let slot = id.0 as usize;
            if slot >= ordered.len() {
                return Err(DemoError::NameLayerLengthMismatch {
                    vertex_count: id_by_name.len(),
                    layer_len: slot + 1,
                });
            }
            ordered[slot] = *name;
        }
        let array = Arc::new(StringArray::from(ordered));
        let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(1_u32),
            "vertex_name",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("vertex_name", DataType::Utf8, false),
        )?;
        let name_layer = PropertyLayer::try_new_dense(descriptor, array)?;
        let element_layers = [name_layer];
        Ok(export_bcsr_snapshot_with_properties(
            &frozen,
            HyperPropertyLayers {
                element: &element_layers,
                relation: &[],
                incidence: &[],
            },
        )?)
    }
}

/// Streaming YAML ingest state.
///
/// Tracks a frame stack so the parser can walk arbitrary YAML without holding
/// the document as a single owned tree. Each frame records what the next
/// scalar means at that depth. The output is a `ServicesModel`; no
/// `HypergraphBuilder` is touched here.
struct YamlIngest {
    /// Model under construction.
    model: ServicesModel,
    /// Parser frame stack; the top frame governs how scalars are interpreted.
    frames: Vec<Frame>,
}

/// One frame of the YAML structural stack.
enum Frame {
    /// Top-level document mapping; expecting a `services:` key.
    TopMapping {
        /// Most recent key seen at this level (None means we expect a key next).
        pending_key: Option<String>,
    },
    /// Inside the `services` mapping; keys are service names, values are bodies.
    ServicesMapping {
        /// Service name awaiting its body, if any.
        pending_service: Option<String>,
    },
    /// Inside a single service body mapping; we only care about `depends_on`.
    ServiceBody {
        /// Name of the enclosing service.
        service_name: String,
        /// Most recent key seen at this level.
        pending_key: Option<String>,
    },
    /// Inside a `depends_on` sequence; scalars are dependency names.
    DependsOnSeq {
        /// Name of the enclosing service.
        service_name: String,
        /// Dependency names accumulated so far, in declaration order.
        deps: Vec<String>,
    },
    /// Mapping or sequence we do not interpret; events are discarded.
    Skip,
}

/// Extends `entry` with every element of `additions` not already present.
///
/// Defends `ServicesModel::from_snapshot` against a snapshot that carries
/// multiple hyperedges sharing the same source vertex: their target sets are
/// merged into a single deduplicated dependency list, preserving first-seen
/// order.
fn merge_deps(entry: &mut Vec<String>, additions: &[String]) {
    for addition in additions {
        if !entry.iter().any(|existing| existing == addition) {
            entry.push(addition.clone());
        }
    }
}

/// Sets `pending` if empty, otherwise clears it.
///
/// Mapping rules: the first scalar at a mapping depth is a key; the next event
/// (scalar, mapping, or sequence) is its value; after a value, the mapping is
/// back in key-mode. This helper handles the scalar-as-value case where we do
/// not retain the value.
fn assign_pending(pending: &mut Option<String>, value: Cow<'_, str>) {
    if pending.is_none() {
        *pending = Some(value.into_owned());
    } else {
        *pending = None;
    }
}

impl YamlIngest {
    /// Returns a fresh ingest with an empty model and empty frame stack.
    const fn new() -> Self {
        Self {
            model: ServicesModel::new(),
            frames: Vec::new(),
        }
    }

    /// Consumes the ingest, returning the populated model.
    fn into_model(self) -> ServicesModel {
        self.model
    }

    /// Handles one event from the SAX parser.
    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Scalar(value, ..) => self.handle_scalar(value),
            Event::MappingStart(_, _) => self.handle_mapping_start(),
            Event::SequenceStart(_, _) => self.handle_sequence_start(),
            Event::MappingEnd | Event::SequenceEnd => self.handle_close(),
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(_)
            | Event::DocumentEnd
            | Event::Alias(_) => {}
        }
    }

    /// Handles a scalar event, either as a pending key or as a value.
    fn handle_scalar(&mut self, value: Cow<'_, str>) {
        match self.frames.last_mut() {
            None | Some(Frame::Skip) => {}
            Some(
                Frame::TopMapping {
                    pending_key: pending,
                }
                | Frame::ServicesMapping {
                    pending_service: pending,
                }
                | Frame::ServiceBody {
                    pending_key: pending,
                    ..
                },
            ) => assign_pending(pending, value),
            Some(Frame::DependsOnSeq { deps, .. }) => {
                deps.push(value.into_owned());
            }
        }
    }

    /// Pushes the appropriate frame on a `MappingStart` event.
    fn handle_mapping_start(&mut self) {
        let new_frame = match self.frames.last_mut() {
            None => Frame::TopMapping { pending_key: None },
            Some(Frame::TopMapping { pending_key }) => {
                let key = pending_key.take();
                if key.as_deref() == Some("services") {
                    Frame::ServicesMapping {
                        pending_service: None,
                    }
                } else {
                    Frame::Skip
                }
            }
            Some(Frame::ServicesMapping { pending_service }) => {
                if let Some(name) = pending_service.take() {
                    // Register the service even if it ends up with no deps
                    // (e.g. `db: {}`); the model must see every declared name.
                    self.model.services.entry(name.clone()).or_default();
                    Frame::ServiceBody {
                        service_name: name,
                        pending_key: None,
                    }
                } else {
                    Frame::Skip
                }
            }
            Some(Frame::ServiceBody { pending_key, .. }) => {
                *pending_key = None;
                Frame::Skip
            }
            Some(Frame::DependsOnSeq { .. } | Frame::Skip) => Frame::Skip,
        };
        self.frames.push(new_frame);
    }

    /// Pushes the appropriate frame on a `SequenceStart` event.
    fn handle_sequence_start(&mut self) {
        let new_frame = match self.frames.last_mut() {
            Some(Frame::ServiceBody {
                pending_key,
                service_name,
            }) => {
                let key = pending_key.take();
                if key.as_deref() == Some("depends_on") {
                    Frame::DependsOnSeq {
                        service_name: service_name.clone(),
                        deps: Vec::new(),
                    }
                } else {
                    Frame::Skip
                }
            }
            _ => Frame::Skip,
        };
        self.frames.push(new_frame);
    }

    /// Pops the current frame and flushes accumulated deps into the model.
    fn handle_close(&mut self) {
        if let Some(Frame::DependsOnSeq { service_name, deps }) = self.frames.pop()
            && !deps.is_empty()
        {
            self.model
                .services
                .entry(service_name)
                .or_default()
                .extend(deps);
        }
    }
}

/// Extracts the value-array `DataType` for diagnostics.
///
/// `DecodedPropertyData` is `#[non_exhaustive]`, so the wildcard arm guards
/// against the substrate adding future variants without breaking the example.
trait DataTypeHint {
    /// Returns the Arrow data type of the layer's value column, if known.
    fn data_type(&self) -> Option<&DataType>;
}

impl DataTypeHint for DecodedPropertyData {
    fn data_type(&self) -> Option<&DataType> {
        match self {
            Self::Dense { values } | Self::Sparse { values, .. } => Some(values.data_type()),
            _ => None,
        }
    }
}

/// Runs the streaming YAML to BCSR snapshot demonstration with write-back.
///
/// The program lays out three labeled phases — `seed`, `read`/`mutate`/`save`
/// — that map one-to-one onto a real read-modify-write workflow against a
/// persisted oxgraph snapshot. The snapshot file on disk is the durable
/// substrate; everything else is in-process meaning-layer state derived from
/// or written to that file.
fn main() -> Result<(), DemoError> {
    let snapshot_path = std::env::temp_dir().join("services.oxsnap");
    let yaml_path = std::env::temp_dir().join("services.written.yaml");

    // === Phase 1: SEED ===
    //
    // First ingest. Parse the YAML fixture into a meaning-layer `ServicesModel`,
    // rebuild a fresh BCSR snapshot from it, and persist the bytes to disk via
    // the `SnapshotSink` trait. Also demonstrate the trait against an
    // in-memory `Vec<u8>` so it is clear the same interface targets files,
    // memory, databases, or object stores.
    let seed_model = ServicesModel::from_yaml(SERVICES_YAML)?;
    seed_model.save_snapshot_to(&snapshot_path)?;
    let seed_bytes = seed_model.to_snapshot()?;
    let mut memory_sink: Vec<u8> = Vec::with_capacity(seed_bytes.len());
    memory_sink.write_snapshot(&seed_bytes)?;
    println!("--- seed ---");
    println!("services    = {}", seed_model.service_count());
    println!("deps        = {}", seed_model.dependency_count());
    println!("snapshot    = {} bytes", seed_bytes.len());
    println!("disk sink   = {}", snapshot_path.display());
    println!("memory sink = {} bytes", memory_sink.len());

    // === Phase 2: READ ===
    //
    // Re-read the snapshot we just wrote from disk and decode it back into a
    // fresh `ServicesModel`. The substrate stays name-blind: every service
    // name comes from the decoded `vertex_name` property layer, and the
    // dependency edges come from walking hyperedges in the BCSR view.
    let read_bytes = fs::read(&snapshot_path)?;
    let mut model = ServicesModel::from_snapshot(&read_bytes)?;
    println!("--- read ---");
    println!("source      = {}", snapshot_path.display());
    println!("services    = {}", model.service_count());
    println!("deps        = {}", model.dependency_count());
    print!("{}", model.to_yaml());

    // === Phase 3: MUTATE ===
    //
    // Editor operations on the in-process model. The on-disk snapshot is not
    // touched here, and the BCSR substrate has no concept of removal — the
    // deletion of `worker` is handled at the meaning layer simply by not
    // re-adding it on the next `to_snapshot` call.
    model.remove_service("worker");
    model.set_service("metrics", vec!["db".to_owned(), "cache".to_owned()]);
    println!("--- mutate ---");
    println!(
        "services    = {} (after remove worker / add metrics)",
        model.service_count()
    );
    println!("deps        = {}", model.dependency_count());

    // === Phase 4: SAVE ===
    //
    // Persist the mutated model. `save_snapshot_to` rebuilds a brand-new
    // snapshot from scratch and overwrites the same file on disk. The YAML is
    // saved from the *re-decoded* snapshot rather than the in-memory model so
    // that the file we write to disk reflects the substrate's canonical form
    // (dep lists sorted by vertex ID). The two files together are the visible
    // result of the read-mutate-save loop: an updated oxgraph snapshot and a
    // YAML file that mirrors the topology actually persisted.
    model.save_snapshot_to(&snapshot_path)?;
    let canonical = ServicesModel::from_snapshot(&fs::read(&snapshot_path)?)?;
    canonical.save_yaml_to(&yaml_path)?;
    let saved_yaml = canonical.to_yaml();
    println!("--- save ---");
    println!("snapshot    = {}", snapshot_path.display());
    println!(
        "yaml        = {} bytes -> {}",
        saved_yaml.len(),
        yaml_path.display()
    );
    print!("{saved_yaml}");

    Ok(())
}

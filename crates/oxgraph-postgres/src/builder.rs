//! Engine construction: validate artifact once and attach typed topology views.

use alloc::{boxed::Box, vec::Vec};
use core::cell::{Cell, RefCell};

use oxgraph_snapshot::Snapshot;
use yoke::Yoke;

use crate::{
    artifact::read_metadata,
    config::Config,
    engine::{Engine, EngineCart, EngineState},
    error::{BuildError, PostgresGraphError},
    overlay::OverlayState,
    topology::{GraphTopology, UniqueAdjacency},
    traverse::TraverseScratch,
};

/// Configures and builds a loaded [`Engine`] from OXGTOPO bytes.
#[derive(Clone, Debug, Default)]
pub struct EngineBuilder {
    /// Owned snapshot bytes to validate and open.
    backing: Option<Vec<u8>>,
    /// Overlay state applied after topology open.
    overlay: OverlayState,
    /// Operational configuration validated at build time.
    config: Config,
}

impl EngineBuilder {
    /// Creates an empty builder (no snapshot bytes yet).
    #[must_use]
    pub fn new() -> Self {
        Self {
            backing: None,
            overlay: OverlayState::default(),
            config: Config::default(),
        }
    }

    /// Supplies owned snapshot bytes to load.
    #[must_use]
    pub fn snapshot_owned(mut self, bytes: Vec<u8>) -> Self {
        self.backing = Some(bytes);
        self
    }

    /// Sets overlay state applied after open.
    #[must_use]
    pub fn overlay(mut self, overlay: OverlayState) -> Self {
        self.overlay = overlay;
        self
    }

    /// Sets operational configuration validated at build time.
    #[must_use]
    #[expect(
        clippy::missing_const_for_fn,
        reason = "Config is not a const-constructible snapshot"
    )]
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Validates the artifact and opens forward CSR plus inbound CSC views once.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError`] when bytes are missing, invalid, or layouts disagree.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)`; queries are `O(1)` to borrow topology afterward.
    pub fn build(self) -> Result<Engine, PostgresGraphError> {
        let backing = self
            .backing
            .ok_or(PostgresGraphError::Build(BuildError::MissingSnapshotBytes))?;
        self.config.validate()?;
        let snapshot = Snapshot::open(&backing)?;
        let metadata = read_metadata(&snapshot)?;
        let cart = Box::new(EngineCart { backing, metadata });
        let inner = Yoke::try_attach_to_cart(cart, |cart: &EngineCart| {
            let snapshot = Snapshot::open(cart.backing.as_slice())?;
            let topology = GraphTopology::open(&snapshot)?;
            Ok::<EngineState<'_>, PostgresGraphError>(EngineState { topology })
        })?;
        let mut overlay = self.overlay;
        if !overlay.added_edges.is_empty() {
            overlay.rebuild_indexes();
        }
        let node_count = inner.backing_cart().metadata.node_count.get() as usize;
        let mut traverse_scratch = TraverseScratch::default();
        traverse_scratch.resize_for_nodes(node_count);
        Ok(Engine::from_parts(
            inner,
            overlay,
            self.config,
            traverse_scratch,
            RefCell::new(UniqueAdjacency::default()),
            Cell::new(false),
        ))
    }
}

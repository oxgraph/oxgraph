//! Tests for substrate-agnostic `PageRank` over concrete `OxGraph` layouts.
//!
//! The algorithms are exercised against:
//!
//! - `oxgraph-csr::CsrGraph` for ordinary graph `PageRank`;
//! - `oxgraph-hyper-bcsr::BcsrHypergraph` for incidence/bipartite hypergraph `PageRank`.

#![cfg(feature = "alloc")]

use std::{
    error::Error,
    fmt,
    ops::{Add, AddAssign, Div, Mul, Sub},
};

use oxgraph_algo::{
    HypergraphPageRankWorkspace, PageRankConfig, PageRankError, PageRankScalar, PageRankScratch,
    PageRankWorkspace, hypergraph_pagerank, hypergraph_pagerank_weighted,
    hypergraph_pagerank_with_workspace, pagerank, pagerank_weighted,
    pagerank_weighted_with_workspace, pagerank_with_scratch, pagerank_with_workspace,
};
use oxgraph_csr::{CsrEdgeId, CsrError, CsrNativeGraph, CsrNodeId};
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrNativeHypergraph, BcsrParticipantId, BcsrRole, BcsrSections,
    BcsrVertexId,
};
use oxgraph_topology::{
    ElementIndex, IncidenceBase, IncidenceWeight, RelationIndex, RelationWeight, TopologyBase,
};
use proptest::prelude::*;

/// Shared convergence configuration for layout tests.
const CONFIG: PageRankConfig<f64> = PageRankConfig::new(0.85, 1.0e-12, 500);

/// Absolute tolerance for stationary-rank assertions.
const EPSILON: f64 = 1.0e-8;

/// Absolute tolerance for f32 stationary-rank assertions.
const EPSILON_F32: f32 = 1.0e-5;

/// Error returned by concrete-layout `PageRank` fixtures.
#[derive(Debug)]
enum PageRankFixtureError {
    /// CSR fixture validation failed.
    Csr(CsrError<u32, u32>),
    /// BCSR fixture validation failed.
    Bcsr(BcsrError),
    /// `PageRank` rejected input or failed to converge.
    PageRank(PageRankError<f64>),
}

impl fmt::Display for PageRankFixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Csr(error) => write!(formatter, "CSR fixture failed: {error}"),
            Self::Bcsr(error) => write!(formatter, "BCSR fixture failed: {error}"),
            Self::PageRank(error) => write!(formatter, "PageRank failed: {error}"),
        }
    }
}

impl Error for PageRankFixtureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Csr(error) => Some(error),
            Self::Bcsr(error) => Some(error),
            Self::PageRank(error) => Some(error),
        }
    }
}

impl From<CsrError<u32, u32>> for PageRankFixtureError {
    fn from(error: CsrError<u32, u32>) -> Self {
        Self::Csr(error)
    }
}

impl From<BcsrError> for PageRankFixtureError {
    fn from(error: BcsrError) -> Self {
        Self::Bcsr(error)
    }
}

impl From<PageRankError<f64>> for PageRankFixtureError {
    fn from(error: PageRankError<f64>) -> Self {
        Self::PageRank(error)
    }
}

/// Relation weights selected for the CSR fixture.
#[derive(Debug)]
struct CsrRelationWeights<'view> {
    /// Dense weights keyed by `CsrEdgeId`.
    values: &'view [f64],
}

impl TopologyBase for CsrRelationWeights<'_> {
    type ElementId = CsrNodeId<u32>;
    type RelationId = CsrEdgeId<u32>;
}

impl RelationWeight for CsrRelationWeights<'_> {
    type Weight = f64;

    fn relation_weight(&self, relation: CsrEdgeId<u32>) -> Self::Weight {
        self.values[relation.0 as usize]
    }
}

/// Integer relation weights selected for generic-conversion coverage.
#[derive(Debug)]
struct CsrIntegerWeights<'view> {
    /// Dense integer weights keyed by `CsrEdgeId`.
    values: &'view [u32],
}

impl TopologyBase for CsrIntegerWeights<'_> {
    type ElementId = CsrNodeId<u32>;
    type RelationId = CsrEdgeId<u32>;
}

impl RelationWeight for CsrIntegerWeights<'_> {
    type Weight = u32;

    fn relation_weight(&self, relation: CsrEdgeId<u32>) -> Self::Weight {
        self.values[relation.0 as usize]
    }
}

/// Relation weights selected for the BCSR fixture.
#[derive(Debug)]
struct BcsrRelationWeights<'view> {
    /// Dense weights keyed by `BcsrHyperedgeId`.
    values: &'view [f64],
}

impl TopologyBase for BcsrRelationWeights<'_> {
    type ElementId = BcsrVertexId<u32>;
    type RelationId = BcsrHyperedgeId<u32>;
}

impl RelationWeight for BcsrRelationWeights<'_> {
    type Weight = f64;

    fn relation_weight(&self, relation: BcsrHyperedgeId<u32>) -> Self::Weight {
        self.values[relation.0 as usize]
    }
}

/// Incidence weights selected for the BCSR fixture.
#[derive(Debug)]
struct BcsrIncidenceWeights<'view> {
    /// Dense weights keyed by `BcsrParticipantId`.
    values: &'view [f64],
}

impl TopologyBase for BcsrIncidenceWeights<'_> {
    type ElementId = BcsrVertexId<u32>;
    type RelationId = BcsrHyperedgeId<u32>;
}

impl IncidenceBase for BcsrIncidenceWeights<'_> {
    type IncidenceId = BcsrParticipantId<u32>;
    type Role = BcsrRole;
}

impl IncidenceWeight for BcsrIncidenceWeights<'_> {
    type Weight = f64;

    fn incidence_weight(&self, incidence: BcsrParticipantId<u32>) -> Self::Weight {
        self.values[incidence.0 as usize]
    }
}

/// Minimal custom scalar used to prove `PageRank` is not hard-wired to primitives.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct TestScalar(f64);

impl Add for TestScalar {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for TestScalar {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul for TestScalar {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl Div for TestScalar {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        Self(self.0 / rhs.0)
    }
}

impl AddAssign for TestScalar {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl PageRankScalar for TestScalar {
    const ZERO: Self = Self(0.0);
    const ONE: Self = Self(1.0);
    const INFINITY: Self = Self(f64::INFINITY);

    #[expect(
        clippy::cast_precision_loss,
        reason = "custom scalar test mirrors f64 degree conversion"
    )]
    fn from_usize(value: usize) -> Self {
        Self(value as f64)
    }

    fn from_f64(value: f64) -> Self {
        Self(value)
    }

    fn abs(self) -> Self {
        Self(self.0.abs())
    }

    fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

#[test]
fn pagerank_runs_over_csr_layout() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut ranks = [0.0; 4];

    let report = pagerank(&graph, elements, CONFIG, None, &mut ranks)?;

    assert!(report.iterations > 0);
    assert!(report.delta <= CONFIG.tolerance);
    assert_probability_mass(&[&ranks]);
    assert_close(ranks[0], 0.204_581_549_974_233_1);
    assert_close(ranks[1], 0.378_475_867_452_851_8);
    assert_close(ranks[2], 0.369_323_534_953_867_5);
    assert_close(ranks[3], 0.047_619_047_619_047_63);

    Ok(())
}

#[test]
fn weighted_pagerank_runs_over_csr_layout() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let weights = CsrRelationWeights {
        values: &[1.0, 1.0, 4.0, 1.0],
    };
    let mut ranks = [0.0; 4];

    let report = pagerank_weighted(&graph, &weights, elements, CONFIG, None, &mut ranks)?;

    assert!(report.iterations > 0);
    assert!(report.delta <= CONFIG.tolerance);
    assert_probability_mass(&[&ranks]);
    assert_close(ranks[0], 0.276_339_530_869_789_63);
    assert_close(ranks[1], 0.339_687_769_671_171_26);
    assert_close(ranks[2], 0.336_353_651_839_991_5);
    assert_close(ranks[3], 0.047_619_047_619_047_63);

    Ok(())
}

#[test]
fn pagerank_runs_with_f32_rank_scalar() -> Result<(), Box<dyn Error>> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let config = PageRankConfig::new(0.85_f32, 1.0e-6_f32, 500);
    let mut ranks = [0.0_f32; 4];

    let report = pagerank(&graph, elements, config, None, &mut ranks)?;

    assert!(report.iterations > 0);
    assert!(report.delta <= config.tolerance);
    assert_probability_mass_f32(&[&ranks]);
    assert_close_f32(ranks[0], 0.204_581_55);
    assert_close_f32(ranks[1], 0.378_475_87);
    Ok(())
}

#[test]
fn pagerank_compiles_with_custom_rank_scalar() -> Result<(), Box<dyn Error>> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let config = PageRankConfig::new(TestScalar(0.85), TestScalar(1.0e-10), 500);
    let mut ranks = [TestScalar::ZERO; 4];

    let report = pagerank(&graph, elements, config, None, &mut ranks)?;

    assert!(report.iterations > 0);
    assert!(report.delta <= config.tolerance);
    let total = ranks.iter().fold(0.0, |acc, rank| acc + rank.0);
    assert_close(total, 1.0);
    Ok(())
}

#[test]
fn weighted_pagerank_accepts_integer_weights_into_f32_ranks() -> Result<(), Box<dyn Error>> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let weights = CsrIntegerWeights {
        values: &[1, 1, 4, 1],
    };
    let config = PageRankConfig::new(0.85_f32, 1.0e-6_f32, 500);
    let mut ranks = [0.0_f32; 4];

    let report = pagerank_weighted(&graph, &weights, elements, config, None, &mut ranks)?;

    let mut workspace = PageRankWorkspace::for_graph(&graph);
    let mut workspace_ranks = [0.0_f32; 4];
    pagerank_weighted_with_workspace(
        &graph,
        &weights,
        elements,
        config,
        None,
        &mut workspace_ranks,
        &mut workspace,
    )?;

    assert!(report.iterations > 0);
    assert!(report.delta <= config.tolerance);
    assert_probability_mass_f32(&[&ranks]);
    assert_close_f32(ranks[0], 0.276_339_53);
    for (left, right) in ranks.into_iter().zip(workspace_ranks) {
        assert_close_f32(left, right);
    }
    Ok(())
}

#[test]
fn graph_scratch_and_workspace_match_allocating() -> Result<(), Box<dyn Error>> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut allocating = [0.0; 4];
    pagerank(&graph, elements, CONFIG, None, &mut allocating)?;

    let mut teleport = [0.0; 4];
    let mut next = [0.0; 4];
    let mut visible = [0; 4];
    let mut scratch_ranks = [0.0; 4];
    pagerank_with_scratch(
        &graph,
        elements,
        CONFIG,
        None,
        &mut scratch_ranks,
        PageRankScratch::new(&mut teleport, &mut next, &mut visible),
    )?;

    let mut workspace = PageRankWorkspace::for_graph(&graph);
    let mut workspace_ranks = [0.0; 4];
    pagerank_with_workspace(
        &graph,
        elements,
        CONFIG,
        None,
        &mut workspace_ranks,
        &mut workspace,
    )?;

    for ((left, middle), right) in allocating
        .into_iter()
        .zip(scratch_ranks)
        .zip(workspace_ranks)
    {
        assert_close(left, middle);
        assert_close(left, right);
    }
    assert!(workspace.element_bound_capacity() >= graph.element_bound());
    Ok(())
}

#[test]
fn borrowed_scratch_rejects_undersized_storage() -> Result<(), Box<dyn Error>> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut ranks = [0.0; 4];
    let mut teleport = [0.0; 4];
    let mut next = [0.0; 2];
    let mut visible = [0; 4];

    let error = pagerank_with_scratch(
        &graph,
        elements,
        CONFIG,
        None,
        &mut ranks,
        PageRankScratch::new(&mut teleport, &mut next, &mut visible),
    );

    assert!(matches!(error, Err(PageRankError::ScratchTooShort { .. })));
    Ok(())
}

#[test]
fn pagerank_runs_over_hyper_bcsr_layout() -> Result<(), PageRankFixtureError> {
    let hypergraph = bcsr_fixture()?;
    let elements = [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)];
    let relations = [BcsrHyperedgeId(0), BcsrHyperedgeId(1)];
    let mut element_ranks = [0.0; 3];
    let mut relation_ranks = [0.0; 2];

    let report = hypergraph_pagerank(
        &hypergraph,
        elements,
        relations,
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    )?;

    assert!(report.iterations > 0);
    assert!(report.delta <= CONFIG.tolerance);
    assert_probability_mass(&[&element_ranks, &relation_ranks]);
    assert_close(element_ranks[0], 0.092_615_357_854_846_62);
    assert_close(element_ranks[1], 0.148_705_533_955_766_8);
    assert_close(element_ranks[2], 0.368_325_634_440_799_9);
    assert_close(relation_ranks[0], 0.131_976_884_943_133_06);
    assert_close(relation_ranks[1], 0.258_376_588_805_453_7);

    Ok(())
}

#[test]
fn weighted_pagerank_runs_over_hyper_bcsr_layout() -> Result<(), PageRankFixtureError> {
    let hypergraph = bcsr_fixture()?;
    let elements = [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)];
    let relations = [BcsrHyperedgeId(0), BcsrHyperedgeId(1)];
    let relation_weights = BcsrRelationWeights {
        values: &[4.0, 1.0],
    };
    let incidence_weights = BcsrIncidenceWeights {
        values: &[1.0, 1.0, 1.0, 3.0, 1.0, 1.0],
    };
    let mut element_ranks = [0.0; 3];
    let mut relation_ranks = [0.0; 2];

    let report = hypergraph_pagerank_weighted(
        &hypergraph,
        &relation_weights,
        &incidence_weights,
        elements,
        relations,
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    )?;

    assert!(report.iterations > 0);
    assert!(report.delta <= CONFIG.tolerance);
    assert_probability_mass(&[&element_ranks, &relation_ranks]);
    assert_close(element_ranks[0], 0.086_736_681_962_031_99);
    assert_close(element_ranks[1], 0.179_631_668_343_342_1);
    assert_close(element_ranks[2], 0.333_745_188_011_389_1);
    assert_close(relation_ranks[0], 0.145_717_625_696_089_43);
    assert_close(relation_ranks[1], 0.254_168_835_987_147_57);

    Ok(())
}

#[test]
fn hypergraph_workspace_matches_allocating() -> Result<(), Box<dyn Error>> {
    let hypergraph = bcsr_fixture()?;
    let elements = [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)];
    let relations = [BcsrHyperedgeId(0), BcsrHyperedgeId(1)];
    let mut allocating_elements = [0.0; 3];
    let mut allocating_relations = [0.0; 2];
    hypergraph_pagerank(
        &hypergraph,
        elements,
        relations,
        CONFIG,
        None,
        &mut allocating_elements,
        &mut allocating_relations,
    )?;

    let mut workspace = HypergraphPageRankWorkspace::for_hypergraph(&hypergraph);
    let mut workspace_elements = [0.0; 3];
    let mut workspace_relations = [0.0; 2];
    hypergraph_pagerank_with_workspace(
        &hypergraph,
        elements,
        relations,
        CONFIG,
        None,
        &mut workspace_elements,
        &mut workspace_relations,
        &mut workspace,
    )?;

    for (left, right) in allocating_elements.into_iter().zip(workspace_elements) {
        assert_close(left, right);
    }
    for (left, right) in allocating_relations.into_iter().zip(workspace_relations) {
        assert_close(left, right);
    }
    assert!(workspace.element_bound_capacity() >= hypergraph.element_bound());
    assert!(workspace.relation_bound_capacity() >= hypergraph.relation_bound());
    Ok(())
}

#[test]
fn invalid_graph_config_is_rejected() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut ranks = [0.0; 4];

    assert!(matches!(
        pagerank(
            &graph,
            elements,
            PageRankConfig::new(f64::NAN, 1.0e-9, 10),
            None,
            &mut ranks
        ),
        Err(PageRankError::InvalidDamping { .. })
    ));
    assert!(matches!(
        pagerank(
            &graph,
            elements,
            PageRankConfig::new(0.85, -1.0, 10),
            None,
            &mut ranks
        ),
        Err(PageRankError::InvalidTolerance { .. })
    ));
    assert!(matches!(
        pagerank(
            &graph,
            elements,
            PageRankConfig::new(0.85, 1.0e-9, 0),
            None,
            &mut ranks
        ),
        Err(PageRankError::InvalidMaxIterations)
    ));
    Ok(())
}

#[test]
fn graph_personalization_and_lengths_are_validated() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut ranks = [0.0; 4];
    let report = pagerank(
        &graph,
        elements,
        CONFIG,
        Some(&[1.0, 0.0, 0.0, 0.0]),
        &mut ranks,
    )?;
    assert!(report.delta <= CONFIG.tolerance);
    assert_probability_mass(&[&ranks]);
    assert!(matches!(
        pagerank(
            &graph,
            elements,
            CONFIG,
            Some(&[0.0, 0.0, 0.0, 0.0]),
            &mut ranks
        ),
        Err(PageRankError::ZeroPersonalization)
    ));
    assert!(matches!(
        pagerank(&graph, elements, CONFIG, Some(&[1.0]), &mut ranks),
        Err(PageRankError::PersonalizationTooShort { .. })
    ));
    let mut short = [0.0; 2];
    assert!(matches!(
        pagerank(&graph, elements, CONFIG, None, &mut short),
        Err(PageRankError::OutputTooShort { .. })
    ));
    Ok(())
}

#[test]
fn graph_visible_subset_ignores_omitted_targets() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1)];
    let mut ranks = [0.0; 4];

    let report = pagerank(&graph, elements, CONFIG, None, &mut ranks)?;

    assert!(report.delta <= CONFIG.tolerance);
    assert_close(ranks[0], 0.350_877_192_982_456_1);
    assert_close(ranks[1], 0.649_122_807_017_543_9);
    assert_close(ranks[2], 0.0);
    assert_close(ranks[3], 0.0);
    assert_probability_mass(&[&ranks[..2]]);

    let weights = CsrRelationWeights {
        values: &[1.0, f64::NAN, 1.0, 1.0],
    };
    let mut weighted_ranks = [0.0; 4];
    pagerank_weighted(
        &graph,
        &weights,
        elements,
        CONFIG,
        None,
        &mut weighted_ranks,
    )?;
    for (left, right) in ranks.into_iter().zip(weighted_ranks) {
        assert_close(left, right);
    }
    Ok(())
}

#[test]
fn graph_duplicate_visible_elements_are_rejected() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(1)];
    let mut ranks = [0.0; 4];

    let error = pagerank(&graph, elements, CONFIG, None, &mut ranks);

    assert!(matches!(
        error,
        Err(PageRankError::DuplicateElement { index: 1 })
    ));
    Ok(())
}

#[test]
fn graph_invalid_weights_are_rejected() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut ranks = [0.0; 4];
    for invalid in [-1.0, f64::NAN, f64::INFINITY] {
        let weights = CsrRelationWeights {
            values: &[1.0, invalid, 1.0, 1.0],
        };
        assert!(matches!(
            pagerank_weighted(&graph, &weights, elements, CONFIG, None, &mut ranks),
            Err(PageRankError::InvalidRelationWeight { .. })
        ));
    }
    Ok(())
}

#[test]
fn zero_weight_rows_are_dangling_rows() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let weights = CsrRelationWeights {
        values: &[0.0, 0.0, 0.0, 0.0],
    };
    let mut ranks = [0.0; 4];
    let report = pagerank_weighted(&graph, &weights, elements, CONFIG, None, &mut ranks)?;
    assert!(report.delta <= CONFIG.tolerance);
    assert_probability_mass(&[&ranks]);
    for rank in ranks {
        assert_close(rank, 0.25);
    }
    Ok(())
}

#[test]
fn non_convergence_reports_last_nonzero_delta() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut ranks = [0.0; 4];
    let error = pagerank(
        &graph,
        elements,
        PageRankConfig::new(0.85, 0.0, 1),
        None,
        &mut ranks,
    );
    assert!(matches!(
        error,
        Err(PageRankError::NonConverged { delta, .. }) if delta > 0.0
    ));
    Ok(())
}

#[test]
fn deterministic_output_repeats() -> Result<(), PageRankFixtureError> {
    let graph = csr_fixture()?;
    let elements = [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)];
    let mut first = [0.0; 4];
    let mut second = [0.0; 4];
    pagerank(&graph, elements, CONFIG, None, &mut first)?;
    pagerank(&graph, elements, CONFIG, None, &mut second)?;
    for (left, right) in first.into_iter().zip(second) {
        assert_close(left, right);
    }
    Ok(())
}

#[test]
fn hypergraph_visible_subset_ignores_omitted_states() -> Result<(), PageRankFixtureError> {
    let hypergraph = bcsr_fixture()?;
    let elements = [BcsrVertexId(0), BcsrVertexId(1)];
    let relations = [BcsrHyperedgeId(0)];
    let mut element_ranks = [0.0; 3];
    let mut relation_ranks = [0.0; 2];

    let report = hypergraph_pagerank(
        &hypergraph,
        elements,
        relations,
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    )?;

    assert!(report.delta <= CONFIG.tolerance);
    assert_close(element_ranks[2], 0.0);
    assert_close(relation_ranks[1], 0.0);
    assert_probability_mass(&[&element_ranks[..2], &relation_ranks[..1]]);
    Ok(())
}

#[test]
fn hypergraph_duplicate_visible_states_are_rejected() -> Result<(), PageRankFixtureError> {
    let hypergraph = bcsr_fixture()?;
    let mut element_ranks = [0.0; 3];
    let mut relation_ranks = [0.0; 2];

    let duplicate_element = hypergraph_pagerank(
        &hypergraph,
        [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(1)],
        [BcsrHyperedgeId(0), BcsrHyperedgeId(1)],
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    );
    assert!(matches!(
        duplicate_element,
        Err(PageRankError::DuplicateElement { index: 1 })
    ));

    let duplicate_relation = hypergraph_pagerank(
        &hypergraph,
        [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)],
        [BcsrHyperedgeId(0), BcsrHyperedgeId(0)],
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    );
    assert!(matches!(
        duplicate_relation,
        Err(PageRankError::DuplicateRelation { index: 0 })
    ));
    Ok(())
}

#[test]
fn hypergraph_invalid_weights_and_personalization_are_rejected() -> Result<(), PageRankFixtureError>
{
    let hypergraph = bcsr_fixture()?;
    let elements = [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)];
    let relations = [BcsrHyperedgeId(0), BcsrHyperedgeId(1)];
    let relation_weights = BcsrRelationWeights {
        values: &[1.0, -1.0],
    };
    let incidence_weights = BcsrIncidenceWeights {
        values: &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
    };
    let mut element_ranks = [0.0; 3];
    let mut relation_ranks = [0.0; 2];
    assert!(matches!(
        hypergraph_pagerank_weighted(
            &hypergraph,
            &relation_weights,
            &incidence_weights,
            elements,
            relations,
            CONFIG,
            None,
            &mut element_ranks,
            &mut relation_ranks,
        ),
        Err(PageRankError::InvalidRelationWeight { .. })
    ));
    let relation_weights = BcsrRelationWeights {
        values: &[1.0, 1.0],
    };
    let incidence_weights = BcsrIncidenceWeights {
        values: &[1.0, 1.0, 1.0, f64::INFINITY, 1.0, 1.0],
    };
    assert!(matches!(
        hypergraph_pagerank_weighted(
            &hypergraph,
            &relation_weights,
            &incidence_weights,
            elements,
            relations,
            CONFIG,
            None,
            &mut element_ranks,
            &mut relation_ranks,
        ),
        Err(PageRankError::InvalidIncidenceWeight { .. })
    ));
    assert!(matches!(
        hypergraph_pagerank(
            &hypergraph,
            elements,
            relations,
            CONFIG,
            Some(&[0.0, 0.0, 0.0, 0.0, 0.0]),
            &mut element_ranks,
            &mut relation_ranks,
        ),
        Err(PageRankError::ZeroPersonalization)
    ));
    Ok(())
}

#[test]
fn hypergraph_non_convergence_reports_last_nonzero_delta() -> Result<(), PageRankFixtureError> {
    let hypergraph = bcsr_fixture()?;
    let elements = [BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)];
    let relations = [BcsrHyperedgeId(0), BcsrHyperedgeId(1)];
    let mut element_ranks = [0.0; 3];
    let mut relation_ranks = [0.0; 2];
    let error = hypergraph_pagerank(
        &hypergraph,
        elements,
        relations,
        PageRankConfig::new(0.85, 0.0, 1),
        None,
        &mut element_ranks,
        &mut relation_ranks,
    );
    assert!(matches!(
        error,
        Err(PageRankError::NonConverged { delta, .. }) if delta > 0.0
    ));
    Ok(())
}

proptest! {
    /// Generated CSR fixtures preserve probability mass and tier equivalence.
    #[test]
    fn generated_graph_pagerank_preserves_mass_visibility_and_tier_equivalence(
        node_count in 1_u32..12,
        edges in prop::collection::vec((0_u32..12, 0_u32..12), 0..48),
        visible_seed in prop::collection::vec(any::<bool>(), 1..12),
    ) {
        let (offsets, targets) = generated_csr_arrays(node_count, &edges);
        let graph = CsrNativeGraph::<u32, u32>::validate(node_count, &offsets, &targets)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let node_len = node_count as usize;
        let mut visible = vec![false; node_len];
        for (node, slot) in visible.iter_mut().enumerate() {
            *slot = visible_seed[node % visible_seed.len()];
        }
        if !visible.iter().any(|value| *value) {
            visible[0] = true;
        }
        let elements: Vec<CsrNodeId<u32>> = visible
            .iter()
            .enumerate()
            .filter_map(|(node, is_visible)| {
                is_visible.then_some(CsrNodeId(u32::try_from(node).ok()?))
            })
            .collect();
        let config = PageRankConfig::new(0.85, 1.0e-9, 1_000);

        let mut allocating = vec![0.0; node_len];
        pagerank(&graph, elements.iter().copied(), config, None, &mut allocating)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        let mut teleport = vec![0.0; node_len];
        let mut next = vec![0.0; node_len];
        let mut visible_scratch = vec![0; node_len];
        let mut scratch = vec![0.0; node_len];
        pagerank_with_scratch(
            &graph,
            elements.iter().copied(),
            config,
            None,
            &mut scratch,
            PageRankScratch::new(&mut teleport, &mut next, &mut visible_scratch),
        )
        .map_err(|error| TestCaseError::fail(error.to_string()))?;

        let mut workspace = PageRankWorkspace::for_graph(&graph);
        let mut workspace_ranks = vec![0.0; node_len];
        pagerank_with_workspace(
            &graph,
            elements.iter().copied(),
            config,
            None,
            &mut workspace_ranks,
            &mut workspace,
        )
        .map_err(|error| TestCaseError::fail(error.to_string()))?;

        let total: f64 = allocating.iter().sum();
        prop_assert!((total - 1.0).abs() <= 1.0e-7, "rank mass was {total}");
        for (node, ((alloc_rank, scratch_rank), workspace_rank)) in allocating
            .iter()
            .zip(&scratch)
            .zip(&workspace_ranks)
            .enumerate()
        {
            prop_assert!(alloc_rank.is_finite());
            prop_assert!(*alloc_rank >= 0.0);
            prop_assert!(
                (*alloc_rank - *scratch_rank).abs() <= 1.0e-8,
                "scratch rank diverged at node {node}: {alloc_rank} vs {scratch_rank}"
            );
            prop_assert!(
                (*alloc_rank - *workspace_rank).abs() <= 1.0e-8,
                "workspace rank diverged at node {node}: {alloc_rank} vs {workspace_rank}"
            );
            if !visible[node] {
                prop_assert!(
                    alloc_rank.abs() <= 1.0e-12,
                    "invisible node {node} received rank {alloc_rank}"
                );
            }
        }
    }
}

/// Builds generated CSR arrays for `PageRank` proptests.
fn generated_csr_arrays(node_count: u32, edges: &[(u32, u32)]) -> (Vec<u32>, Vec<u32>) {
    let node_len = node_count as usize;
    let mut buckets = vec![Vec::<u32>::new(); node_len];
    for &(source, target) in edges {
        if source < node_count && target < node_count {
            buckets[source as usize].push(target);
        }
    }
    let mut offsets = Vec::with_capacity(node_len + 1);
    let mut targets = Vec::new();
    offsets.push(0);
    for bucket in buckets {
        targets.extend(bucket);
        offsets.push(u32::try_from(targets.len()).unwrap_or(u32::MAX));
    }
    (offsets, targets)
}

/// Opens the CSR graph fixture used by ordinary graph `PageRank` tests.
fn csr_fixture() -> Result<CsrNativeGraph<'static, u32, u32>, CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0, 1, 2, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 0, 1];

    CsrNativeGraph::<u32, u32>::validate(4, OFFSETS, TARGETS)
}

/// Opens the BCSR hypergraph fixture used by incidence `PageRank` tests.
fn bcsr_fixture() -> Result<BcsrNativeHypergraph<'static, u32, u32, u32>, BcsrError> {
    static HEAD_OFFSETS: &[u32] = &[0, 1, 3];
    static HEAD_PARTICIPANTS: &[u32] = &[0, 0, 1];
    static TAIL_OFFSETS: &[u32] = &[0, 2, 3];
    static TAIL_PARTICIPANTS: &[u32] = &[1, 2, 2];
    static VERTEX_OUTGOING_OFFSETS: &[u32] = &[0, 2, 3, 3];
    static VERTEX_OUTGOING_HYPEREDGES: &[u32] = &[0, 1, 1];
    static VERTEX_INCOMING_OFFSETS: &[u32] = &[0, 0, 1, 3];
    static VERTEX_INCOMING_HYPEREDGES: &[u32] = &[0, 0, 1];

    BcsrNativeHypergraph::<u32, u32, u32>::open(BcsrSections {
        head_offsets: HEAD_OFFSETS,
        head_participants: HEAD_PARTICIPANTS,
        tail_offsets: TAIL_OFFSETS,
        tail_participants: TAIL_PARTICIPANTS,
        vertex_outgoing_offsets: VERTEX_OUTGOING_OFFSETS,
        vertex_outgoing_hyperedges: VERTEX_OUTGOING_HYPEREDGES,
        vertex_incoming_offsets: VERTEX_INCOMING_OFFSETS,
        vertex_incoming_hyperedges: VERTEX_INCOMING_HYPEREDGES,
    })
}

/// Asserts that one or more rank slices form a finite probability vector.
fn assert_probability_mass(parts: &[&[f64]]) {
    let mut total = 0.0;
    for part in parts {
        for value in *part {
            assert!(value.is_finite(), "rank is not finite: {value}");
            assert!(*value >= 0.0, "rank is negative: {value}");
            total += value;
        }
    }
    assert_close(total, 1.0);
}

/// Asserts approximate equality for stationary ranks.
fn assert_close(actual: f64, expected: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= EPSILON,
        "actual {actual} differs from expected {expected} by {delta}"
    );
}

/// Asserts that one or more f32 rank slices form a finite probability vector.
fn assert_probability_mass_f32(parts: &[&[f32]]) {
    let mut total = 0.0_f32;
    for part in parts {
        for value in *part {
            assert!(value.is_finite(), "rank is not finite: {value}");
            assert!(*value >= 0.0, "rank is negative: {value}");
            total += value;
        }
    }
    assert_close_f32(total, 1.0);
}

/// Asserts approximate equality for f32 stationary ranks.
fn assert_close_f32(actual: f32, expected: f32) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= EPSILON_F32,
        "actual {actual} differs from expected {expected} by {delta}"
    );
}

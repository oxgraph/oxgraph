"""Thin Python facade for the OxGraph native module."""

from ._oxgraph import (
    FrozenGraph,
    FrozenHypergraph,
    GraphBuilder,
    HypergraphBuilder,
    SnapshotInfo,
    GraphError,
    HypergraphError,
    OxGraphError,
    PageRankError,
    SnapshotError,
    open_bcsr_snapshot,
    open_csr_snapshot,
    open_snapshot,
)

__all__ = [
    "FrozenGraph",
    "FrozenHypergraph",
    "GraphBuilder",
    "GraphError",
    "HypergraphBuilder",
    "HypergraphError",
    "OxGraphError",
    "PageRankError",
    "SnapshotError",
    "SnapshotInfo",
    "open_bcsr_snapshot",
    "open_csr_snapshot",
    "open_snapshot",
]

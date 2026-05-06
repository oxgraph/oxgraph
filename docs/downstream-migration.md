# Downstream migration sketch

This repository keeps transition-style fixtures local to OxGraph. A downstream Python project can migrate later without OxGraph depending on that project:

1. Build domain states as Python labels on `GraphBuilder` or `HypergraphBuilder`.
2. Add transitions as graph edges or directed hyperedges.
3. Store transition strengths as relation weights or as named f64 property layers selected into PageRank.
4. Freeze to owned views before running BFS or PageRank.
5. Persist snapshots with topology, identity, and property sections; reopen them through snapshot helpers before constructing downstream caches.
6. Keep domain meaning outside OxGraph: labels and property names are facade/domain data, not topology semantics.

Rust coverage for this pattern lives in `crates/oxgraph-algo/tests/transition_fixture.rs`. Python facade coverage lives in `crates/oxgraph-python/tests/test_oxgraph.py`.

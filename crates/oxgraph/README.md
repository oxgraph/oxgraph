# oxgraph

High-performance, zero-copy graph and topology substrate.

Version `0.1.0` is the first DB-inclusive crates.io release of the umbrella
crate. The default feature set is empty; enable the layers you need explicitly.

Core topology, graph, hypergraph, CSR, BCSR, and builder features avoid Arrow.
Use `graph-snapshot` or `hyper-snapshot` for topology snapshot export. Use
`property-arrow`, `graph-property-arrow`, or `hyper-property-arrow` when
Arrow-backed property layers or property snapshot export are required. Enable
`db` for the embedded OxGraph-native database API re-exported as `oxgraph::db`.

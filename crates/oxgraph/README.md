# oxgraph

High-performance, zero-copy graph and topology substrate.

Core topology, graph, hypergraph, CSR, BCSR, and builder features avoid Arrow.
Use `graph-snapshot` or `hyper-snapshot` for topology snapshot export. Use
`property-arrow`, `graph-property-arrow`, or `hyper-property-arrow` when
Arrow-backed property layers or property snapshot export are required.

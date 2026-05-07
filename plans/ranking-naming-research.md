# Ranking naming research

## Question

What names do graph libraries, graph databases, and graph theory sources use for PageRank-like algorithms besides `PageRank`, and what should OxGraph call the primitive module/API?

## Working hypothesis

`PageRank` is the dominant user-facing name for the canonical damped random-surfer algorithm. More primitive names appear in mathematics and variants: stationary distribution, eigenvector centrality, random walk with restart, personalized PageRank, stochastic/Markov ranking, graph diffusion. OxGraph may want a broad `rank` module with `pagerank` as the canonical preset.

## Findings

### Library/API names

- NetworkX exposes the canonical algorithm as `pagerank()` under link analysis. Its docs say PageRank ranks nodes based on incoming links, was designed for web pages, supports damping `alpha`, personalization, `weight`, dangling handling, and power-iteration convergence.
- graph-tool exposes `graph_tool.centrality.pagerank()` with damping, personalization, edge weight, and output property-map parameters. It also groups PageRank alongside centrality algorithms such as eigenvector, Katz, HITS, and Eigentrust.
- Neo4j Graph Data Science calls it `PageRank`, describes it as measuring node importance from incoming relationships and source-node importance, and exposes Personalized PageRank. It also has `ArticleRank` as a named PageRank variant.
- Apache Spark GraphX calls the algorithm `PageRank` and exposes static and dynamic implementations.
- JGraphT exposes class `PageRank<V,E>` under `org.jgrapht.alg.scoring`, implementing a generic `VertexScoringAlgorithm`.

### Mathematical / conceptual names

- PageRank is commonly described as a random-surfer model.
- PageRank can be understood as a Markov chain over pages/vertices where transitions follow links plus teleport/damping.
- The result is a stationary distribution / principal eigenvector of a modified stochastic matrix.
- PageRank is often presented as a variant of eigenvector centrality.
- Common named relatives include Personalized PageRank, random walk with restart, ArticleRank, HITS, Katz centrality, Eigentrust, graph diffusion, and Markov-chain centrality/stationary ranking.

## Naming recommendation

- Use `rank` as the broad OxGraph algorithm module because the family includes PageRank, personalized PageRank, bipartite/hypergraph ranking, and future centrality/ranking algorithms.
- Keep `pagerank` as the user-facing function/preset for canonical PageRank. This is the dominant name across major graph libraries/databases and is what users will search for.
- Internally name the more primitive abstraction around `stationary_rank` or `stochastic_rank` only if we actually expose a generic Markov/stochastic fixed-point solver. Do not rename canonical PageRank away from `pagerank` prematurely.
- Candidate module layout:

```text
oxgraph-algo::rank
  pagerank          # canonical PageRank API/config/result/errors
  stochastic        # later generic stochastic ranking kernel if it proves reusable
  policies          # normalization/dangling/teleport policy types
```

## Implication for OxGraph

`PageRank` is not too narrow as a public algorithm name. The primitive crate/module should be broader (`rank`), while the canonical algorithm remains `pagerank`.

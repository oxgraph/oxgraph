# oxgraph-property

Arrow-backed named property layers for OxGraph topology views.

[![crates.io](https://img.shields.io/crates/v/oxgraph-property.svg)](https://crates.io/crates/oxgraph-property)
[![docs.rs](https://docs.rs/oxgraph-property/badge.svg)](https://docs.rs/oxgraph-property)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The property layer of the [oxgraph](https://github.com/oxgraph/oxgraph)
crate family: topology stays meaning-free, values live here.

## What it is

`oxgraph-property` stores named, typed Arrow arrays keyed by topology ID
family (element, relation, or incidence) and adapts selected total primitive
layers into the topology weight capabilities that algorithms consume. It is
deliberately a higher layer: the foundation crates do not depend on this
crate, Arrow, or named properties.

The crate is split into focused modules behind one facade:

| Module | Responsibility |
| --- | --- |
| `width` | Index/metadata width contracts, axis markers, section-kind constants |
| `model` | Layer data model, error type, identity records |
| `weights` | Dense/sparse topology weight adapters and layer partitions |
| `rekey` | Descriptor uniqueness checks and canonical-to-local rekeying |
| `snapshot` | Snapshot encode/validate/decode and wire records |

Property layers persist into `oxgraph-snapshot` sections: one width-coded
kind for per-layer descriptor records and one for the concatenated Arrow IPC
value streams. The payload format is owned by this crate and remains an
OxGraph-internal ABI candidate while the snapshot bytes are not stable.

## Where it sits

```text
oxgraph-topology                  capability traits (weights bind here)
└── oxgraph-property            ← this crate (Arrow-backed layers)
    ├── persists into oxgraph-snapshot sections
    └── exported by the csr / hyper-bcsr builders
        (`build-property-arrow` features)
```

## Documentation

See [docs.rs/oxgraph-property](https://docs.rs/oxgraph-property) for the
full API and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features property-arrow`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).

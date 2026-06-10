# Section-kind registry

Single authority over the OXGT section-kind namespace. The container assigns
no semantics to kinds; every in-tree layer declares its constants inside the
band reserved here (`oxgraph_snapshot::kinds`), so distinct subsystems cannot
collide. The umbrella crate's `section_kind_registry` test
(`crates/oxgraph/tests/section_kind_registry.rs`, `--features full`) asserts
all derived values are pairwise distinct.

## Width encoding

Width-parameterized sections derive their kind as `BASE | WIDTH_CODE`, where
`BASE` is 4-aligned and `WIDTH_CODE`
(`oxgraph_layout_util::SnapshotWidth::WIDTH_CODE`) occupies the low two bits:

| Width | Code   |
| ----- | ------ |
| `u16` | `0b00` |
| `u32` | `0b01` |
| `u64` | `0b10` |
| —     | `0b11` (reserved) |

Within one band, bases ascend by at least 4 in the exporter's emission order,
so the derived kinds stay strictly ascending (the OXGT v2 table invariant) for
any width mix. Widthless kinds (Postgres catalog/metadata, all OXGDB kinds)
use the raw value.

## Bands and bases

| Band | Range | Owner | Constant | Base / value |
| ---- | ----- | ----- | -------- | ------------ |
| `CSR_BAND` | `0x0001..0x0020` | `oxgraph-csr` | `SNAPSHOT_KIND_CSR_OFFSETS_BASE` | `0x0004` |
| | | | `SNAPSHOT_KIND_CSR_TARGETS_BASE` | `0x0008` |
| `BCSR_BAND` | `0x0020..0x0100` | `oxgraph-hyper-bcsr` | `SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE` | `0x0020` |
| | | | `SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE` | `0x0024` |
| | | | `SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE` | `0x0028` |
| | | | `SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE` | `0x002C` |
| | | | `SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE` | `0x0030` |
| | | | `SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE` | `0x0034` |
| | | | `SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE` | `0x0038` |
| | | | `SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE` | `0x003C` |
| `PROPERTY_BAND` | `0x0100..0x0200` | `oxgraph-property` | `SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_BASE` | `0x0100` |
| | | | `SNAPSHOT_KIND_PROPERTY_DATA_BASE` | `0x0104` |
| | | | `SNAPSHOT_KIND_IDENTITY_MODES_BASE` | `0x0110` |
| | | | `SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_BASE` | `0x0114` |
| | | | `SNAPSHOT_KIND_RELATION_IDENTITY_MAP_BASE` | `0x0118` |
| | | | `SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_BASE` | `0x011C` |
| `POSTGRES_BAND` | `0x0200..0x0300` | `oxgraph-postgres` | `SNAPSHOT_KIND_PG_CATALOG` (widthless) | `0x0200` |
| | | | `SNAPSHOT_KIND_PG_INBOUND_OFFSETS_BASE` (pins `u32` → `0x0205`) | `0x0204` |
| | | | `SNAPSHOT_KIND_PG_INBOUND_TARGETS_BASE` (pins `u32` → `0x0209`) | `0x0208` |
| | | | `SNAPSHOT_KIND_PG_METADATA` (widthless) | `0x020C` |
| `DATABASE_BAND` | `0x0300..0x0400` | `oxgraph-db` | `SECTION_DB_HEADER` … `SECTION_STRING_TABLE` (22 widthless kinds, emission order; see `crates/oxgraph-db/src/wire.rs`) | `0x0300..=0x0315` |
| custom | `0x0400..` | applications | `CUSTOM_BASE` | `0x0400` |

## Per-exporter emission order (ascending-kind proof)

- **CSR** (`oxgraph-csr::build`): offsets (`EdgeIndex` width) then targets
  (`NodeIndex` width). Worst case `0x0004 | 0b10 = 0x0006 < 0x0008`.
- **BCSR** (`oxgraph-hyper-bcsr::build`): the eight sections in base order;
  offsets sections use the incidence width, participants the vertex width,
  vertex-major hyperedges the relation width. Each `BASE + 3 <` next base.
- **Property tail** (`oxgraph-property::export`): descriptors, data, identity
  modes (metadata width), then one identity map (map width). Bases ascend by
  at least 4 in that order, and the whole band sits above CSR/BCSR topology
  sections.
- **Postgres artifact** (`oxgraph-postgres::artifact`): forward sections
  (`< 0x0200`), inbound offsets `0x0205`, inbound targets `0x0209`, metadata
  `0x020C`.
- **OXGDB** (`oxgraph-db::freeze`): kinds assigned contiguously in emission
  order; a compile-time check in `wire.rs` pins band membership and ascending
  order.

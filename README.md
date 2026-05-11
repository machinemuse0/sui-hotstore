# Sui HotStore

Sui HotStore is a ToplingDB-powered local KV serving layer for Sui custom indexers, RPC hot paths, and archive-style query workloads.

[Demo Video](#demo-video) | [Benchmark Summary](#benchmark-summary) | [Known Limitations](#known-limitations) | [Run Commands](#run-commands)

## Architecture Diagram

```mermaid
flowchart TD
    A["Sui Data Sources"] --> B["Bounded-Range Ingest Pipeline"]
    A1["Checkpoint RPC"] --> B
    A2["Synthetic Dataset"] --> B
    A3["Prepared Real Dataset"] --> B

    B --> C["Storage Abstraction"]
    C --> D["RocksDB Baseline"]
    C --> E["ToplingDB Backend"]

    D --> F["HotStore Column Families"]
    E --> F

    F --> F1["cf_checkpoint"]
    F --> F2["cf_tx_by_digest"]
    F --> F3["cf_object_version"]
    F --> F4["cf_object_last_seen"]
    F --> F5["cf_event_by_type"]
    F --> F6["cf_owner_touched_objects"]

    F --> G["Admin + Benchmark Tooling"]
    G --> G1["stats / checksum / compare-checksum"]
    G --> G2["DB-level workloads"]
    G --> G3["summary + reports"]
```

## Run Commands

### 1. Format, check, test

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
```

### 2. Route 1 server run

This is the current end-to-end path for bounded real-data benchmarking:

```bash
bash scripts/run-route1-benchmark-server.sh \
  --network mainnet \
  --first-checkpoint 270700000 \
  --last-checkpoint 270759999 \
  --base-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb \
  --backend rocksdb \
  --cargo-profile release \
  --requests 500000 \
  --concurrency 1,4,8,16,32,64 \
  --batch-size 10 \
  --tx-batch-size 50
```

To restart from scratch instead of resuming:

```bash
bash scripts/run-route1-benchmark-server.sh \
  --network mainnet \
  --first-checkpoint 270700000 \
  --last-checkpoint 270759999 \
  --base-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb \
  --backend rocksdb \
  --cargo-profile release \
  --reset-state
```

For a strict RocksDB vs ToplingDB comparison, prefer pinning an identical explicit checkpoint range for both runs:

```bash
bash scripts/run-route1-benchmark-server.sh \
  --network mainnet \
  --first-checkpoint 270700000 \
  --last-checkpoint 270759999 \
  --base-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb \
  --backend rocksdb \
  --cargo-profile release
```

### 3. DB-only benchmark on an existing HotStore DB

```bash
bash scripts/run-benchmark-suite.sh \
  --backend rocksdb \
  --db-path /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/db-a \
  --keys-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/keys \
  --report-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/reports \
  --dataset mainnet-270700000-270759999 \
  --requests 500000 \
  --warmup-requests 50000 \
  --concurrency 1,4,8,16,32,64 \
  --access-pattern uniform \
  --scan-mode count \
  --cache-state hot \
  --min-hit-rate 1.0 \
  --batch-size 10 \
  --cargo-profile release
```

### 4. Checksum and stats

```bash
cargo run --release --bin hotstore-admin -- \
  stats \
  --backend rocksdb \
  --db-path /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/db-a \
  --output /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/reports/stats.json

cargo run --release --bin hotstore-admin -- \
  checksum \
  --backend rocksdb \
  --db-path /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/db-a \
  --output /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/reports/checksum/checksum.json
```

## Benchmark Summary

Current benchmark focus:

- checksum and data equality
- DB-level workloads
- point lookup
- multi-get
- event prefix scan
- mixed RPC-style workload

Current implemented workloads:

- `get-tx`
- `get-object-version`
- `get-object-last-seen`
- `multi-get-tx`
- `multi-get-object-version`
- `scan-events`
- `mixed-rpc`

Current public summary artifact:

- [reports/summary.md](reports/summary.md)

Current headline comparison:

- RocksDB results are based on the `master` branch.
- ToplingDB results are based on the `topling` branch.

| Metric | RocksDB | ToplingDB | ToplingDB vs RocksDB |
|---|---:|---:|---:|
| Checkpoint range | `270700000..270759999` | `270700000..270759999` | same data |
| Imported checkpoints | `60,000` | `60,000` | same data |
| Logical entries | `6,242,137` | `6,242,137` | same data |
| Disk usage | `5.82 GiB` | `1.25 GiB` | `78.6%` less |
| get-tx | `1.36M rps` | `4.51M rps` | `3.31x` |
| get-object-version | `0.81M rps` | `3.66M rps` | `4.52x` |
| get-object-last-seen | `2.01M rps` | `4.85M rps` | `2.42x` |
| multi-get-tx | `0.15M rps` | `2.15M rps` | `14.53x` |
| multi-get-object-version | `0.08M rps` | `1.22M rps` | `15.24x` |
| scan-events | `1.47M rps` | `1.58M rps` | `1.08x` |
| mixed-rpc | `0.33M rps` | `2.72M rps` | `8.24x` |

Current benchmark-data note:

- We restored about `200G` of Sui data via the formal snapshot path.
- In practice, bringing a local `sui-node` on top of that dataset to a stable benchmark-ready state currently involves a long pruning / compaction window.
- Because of that, the current public benchmark evidence is based on a bounded route 1 workflow over checkpoints `270700000..270759999`.
- The current comparison covers `60,000` checkpoints, `6,242,137` logical entries, and DB-level workloads with matching benchmark-facing data column-family checksums.

Related benchmark materials:

- [docs/benchmark_runbook.md](docs/benchmark_runbook.md)
- [docs/sui-overflow-submission.md](docs/sui-overflow-submission.md)

## Known Limitations

- This is not a fullnode replacement.
- This is not validator storage.
- Current dataset is bounded.
- `object_last_seen` is latest observed within imported dataset.
- `owner_touched_objects` is not complete wallet inventory.
- Benchmark results are hardware-specific.

Additional current-state caveats:

- API serving is still a scaffold; the current benchmark focus is DB-level workloads.
- Real-data ingestion currently uses bounded-range Sui JSON-RPC sourcing, not full genesis replay.
- Formal snapshot and deeper Sui-native storage integration are still follow-on work.
- We already downloaded a much larger formal-snapshot dataset, but local Sui-node pruning / compaction time is still too long for that path to be the main benchmark evidence source today.

## Demo Video

- [Demo video link placeholder](#demo-video)
- Recording script: [docs/sui-overflow-demo-script.md](docs/sui-overflow-demo-script.md)

Recording status:

```text
Pending final benchmark capture from the current server runs.
```

## Related Projects

- [topling/rust-toplingdb](https://github.com/topling/rust-toplingdb)  
  RocksDB-compatible Rust binding / backend line used for ToplingDB integration and backend switching experiments in this project.

- [rockeet/sui - `use-toplingdb` branch](https://github.com/rockeet/sui/tree/use-toplingdb)  
  Sui integration branch used for ToplingDB-related experiments, configuration alignment, and comparison work alongside Sui HotStore.

## Project Scope

Sui HotStore is aimed at infra teams building on Sui:

- custom indexer operators
- RPC providers
- explorer backends
- wallet backends
- analytics and archive-query services

The current MVP is centered on:

- a bounded-range ingest path
- deterministic key encoding
- RocksDB baseline and ToplingDB backend comparison
- checksum validation
- reproducible benchmark tooling

## Repository Guide

- Core schema and records:
  - [crates/hotstore-core](crates/hotstore-core)
- Storage abstraction and backends:
  - [crates/hotstore-db](crates/hotstore-db)
- Real-data bounded-range ingest:
  - [crates/hotstore-sui-source](crates/hotstore-sui-source)
- Admin tooling:
  - [crates/hotstore-admin](crates/hotstore-admin)
- DB benchmark runner:
  - [crates/hotstore-bench](crates/hotstore-bench)
- Submission and demo materials:
  - [docs](docs)

## Roadmap

Near-term:

- add API-level benchmark coverage on top of the DB-level evidence
- capture repeat runs for variance and cold-cache behavior
- package Sui Overflow demo materials

Later:

- fuller API layer
- deeper Sui-native storage integration
- formal snapshot bootstrap path
- broader benchmark coverage and observability

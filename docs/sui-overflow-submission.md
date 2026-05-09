# Sui Overflow Submission Draft — Sui HotStore

> File: `docs/sui-overflow-submission.md`  
> Project: **Sui HotStore**  
> Status: Draft for Sui Overflow / Sui ecosystem demo submission  
> Recommended target track: **Infra & Tooling**  
> Secondary target track: **Programmable Storage / Data Infrastructure**, if available  
> Note: final track names should be updated after the official Sui Overflow 2026 track announcement.

---

## 1. Project Name

```text
Sui HotStore
```

---

## 2. One-liner

```text
Sui HotStore is a ToplingDB-powered local KV serving layer for Sui custom indexers, RPC hot paths, and archive-style query workloads.
```

---

## 3. Short Description

Sui HotStore provides a high-performance local key-value serving layer for Sui data applications.

It ingests Sui-style transaction, event, object, owner, and checkpoint data into a RocksDB-compatible ToplingDB backend, then exposes low-latency APIs for common Sui data access patterns:

```text
- transaction lookup by digest
- object version lookup
- latest-observed object lookup within an imported range
- event prefix scan by event type
- owner touched-object scan
- multi_get workloads for RPC-style batched queries
```

The project is designed for:

```text
- custom indexers
- RPC providers
- explorer backends
- wallet backends
- DeFi analytics services
- archive-query services
```

It is **not** a replacement for Sui full node storage or validator state storage. It is a KV-native hot path for Sui data applications.

---

## 4. Problem

Sui applications need fast access to chain data such as:

```text
- transactions
- checkpoints
- events
- objects
- object versions
- owner/object relationships
```

Many Sui data services rely on full node RPC, GraphQL, PostgreSQL-backed indexers, or custom pipelines. These are powerful, but some high-frequency serving paths are naturally KV-shaped:

```text
tx_digest -> transaction/effects
object_id + version -> object version
object_id -> latest observed object
event_type + checkpoint -> event stream
owner + object_type -> touched objects
```

For RPC providers, wallets, explorers, DeFi dashboards, trading analytics, and archive services, repeatedly serving these hot queries through general-purpose databases or remote RPC can create:

```text
- high p99 latency
- expensive storage and compute overhead
- inefficient multi_get behavior
- heavy pressure on PostgreSQL or RPC nodes
- difficulty separating hot serving paths from analytical workloads
```

Sui HotStore addresses this gap by providing a local, KV-native, benchmarkable data layer that sits beside custom indexers and RPC services.

---

## 5. Solution

Sui HotStore turns Sui chain data into column-family based KV indexes.

### Core architecture

```text
Sui data source
  ├── exported indexed data sample
  ├── checkpoint sample
  ├── synthetic Sui-like workload
  └── future: full checkpoint / gRPC stream / formal snapshot pipeline
        |
        v
Sui HotStore ingest pipeline
        |
        v
Storage abstraction
  ├── RocksDB baseline
  └── ToplingDB backend
        |
        v
Column families
  ├── cf_checkpoint
  ├── cf_tx_by_digest
  ├── cf_object_version
  ├── cf_object_last_seen
  ├── cf_event_by_type
  └── cf_owner_touched_objects
        |
        v
Serving API / benchmark suite
  ├── point lookup
  ├── multi_get
  ├── prefix scan
  └── mixed RPC-style workload
```

### Why ToplingDB?

ToplingDB is positioned as a RocksDB-compatible high-performance KV engine. This makes it a practical candidate for Sui data infrastructure because many blockchain storage and indexing workloads are already KV-heavy.

For this demo, ToplingDB is evaluated against a RocksDB baseline using the same schema, same data, and same query keysets.

---

## 6. Target Users

### RPC providers

Need:

```text
- fast tx lookup
- fast object lookup
- batched object/tx multi_get
- lower p99 latency
- lower pressure on full nodes
```

Sui HotStore provides:

```text
- local KV cache / hot path
- multi_get benchmark path
- point lookup APIs
```

---

### Custom indexer operators

Need:

```text
- event stream lookup
- prefix scans by event type
- bounded retention policies
- separation between serving path and analytics DB
```

Sui HotStore provides:

```text
- event_by_type prefix indexes
- owner/object indexes
- benchmarked serving workload
```

---

### Explorer and wallet backends

Need:

```text
- transaction detail lookup
- object version lookup
- owner-object relationship lookup
- fast API response
```

Sui HotStore provides:

```text
- tx_by_digest
- object_version
- owner_touched_objects
```

---

### Archive-query services

Need:

```text
- historical object versions
- checkpoint-scoped data
- raw or structured record serving
- lower-cost local archive indexes
```

Sui HotStore provides:

```text
- object_version index
- checkpoint index
- future raw record storage mode
```

---

## 7. MVP Scope

The current MVP focuses on a bounded Sui dataset and a benchmarkable KV schema.

### Included

```text
- Rust workspace
- storage abstraction
- RocksDB baseline backend
- ToplingDB backend or feature-gated ToplingDB integration
- deterministic key encoding
- synthetic Sui-like data ingestion
- prepared real Sui dataset ingestion path
- bounded real-checkpoint ingestion via Sui JSON-RPC
- transaction lookup
- object version lookup
- object_last_seen lookup
- event_type prefix scan
- owner_touched_objects scan
- multi_get workloads
- DB-level benchmark suite
- checksum validation
- benchmark summary template
- benchmark key generation from checkpoint RPC
- benchmark key generation from existing HotStore DB
- benchmark key emission during real-data ingest
- resume support for interrupted bounded-range ingest runs
```

### Explicitly not included in MVP

```text
- replacing Sui full node storage
- replacing validator storage
- full chain replay from genesis
- complete global object_latest semantics
- complete global address inventory semantics
- production archival service
- distributed compaction deployment
- full Sui custom indexer BYOS implementation
- production-ready API service
- completed API benchmark suite
```

### Platform note

```text
- macOS is suitable for documentation work, RocksDB smoke checks, and general development
- practical ToplingDB backend validation and benchmark evidence should be collected on Linux
- current ToplingDB integration follows upstream native build assumptions, so our authoritative ToplingDB benchmark environment is Linux
```

---

## 8. Important Semantics

This project currently supports **bounded checkpoint range / bounded dataset semantics**.

That means:

```text
cf_object_last_seen
```

means:

```text
the latest object version observed within the imported dataset or checkpoint range
```

It does **not** yet mean:

```text
the global latest object state on the entire Sui network
```

Similarly:

```text
cf_owner_touched_objects
```

means:

```text
objects observed or touched by an owner within the imported dataset or checkpoint range
```

It does **not** yet mean:

```text
the owner's complete object inventory across the full chain
```

Full global latest-state semantics require one of the following future paths:

```text
- genesis replay
- full-history checkpoint ingestion
- snapshot-based bootstrap
- formal snapshot / consistent store integration
- production-grade Sui custom indexer pipeline
```

---

## 9. Column Families

### `cf_checkpoint`

Stores checkpoint metadata.

```text
key:
  checkpoint_seq_be

value:
  CheckpointRecord {
    network,
    sequence_number,
    timestamp_ms,
    tx_count,
    event_count,
    object_change_count,
    source
  }
```

---

### `cf_tx_by_digest`

Stores transaction lookup records.

```text
key:
  tx_digest

value:
  TxRecord {
    digest,
    checkpoint,
    tx_index,
    sender,
    status,
    gas_used,
    event_count,
    changed_object_count,
    raw_effects_bytes optional
  }
```

---

### `cf_object_version`

Stores historical object versions.

```text
key:
  object_id || version_be

value:
  ObjectRecord {
    object_id,
    version,
    checkpoint,
    owner,
    type_tag,
    raw_object_bytes optional
  }
```

---

### `cf_object_last_seen`

Stores the latest observed object record within the imported range.

```text
key:
  object_id

value:
  ObjectRecord
```

---

### `cf_event_by_type`

Stores event prefix indexes.

```text
key:
  event_type_hash || checkpoint_be || tx_index_be || event_index_be

value:
  EventRecord {
    event_type,
    checkpoint,
    tx_digest,
    sender,
    package_id,
    module,
    event_name,
    payload
  }
```

---

### `cf_owner_touched_objects`

Stores owner-object observations within the imported range.

```text
key:
  owner || type_tag_hash || object_id || version_be

value:
  OwnerTouchedObjectRecord {
    owner,
    object_id,
    version,
    checkpoint,
    type_tag
  }
```

---

## 10. Demo APIs

The planned demo API surface is:

```text
GET  /health
GET  /stats
GET  /checkpoint/{seq}

GET  /tx/{digest}
POST /multi-get/txs

GET  /object/{object_id}
GET  /object/{object_id}/version/{version}
GET  /object/{object_id}/versions?limit=50
POST /multi-get/objects

GET  /owner/{owner}/touched-objects?type={type_tag}&limit=100

GET  /events?type={event_type}&from_checkpoint={seq}&limit=100

GET  /bench/summary
```

Current status:

```text
- hotstore-api crate exists as a scaffold
- DB-level benchmark tooling is implemented first
- API routes and API benchmark remain later-phase work
```

---

## 11. Benchmark Plan

The project benchmarks ToplingDB against RocksDB using:

```text
same dataset
same key encoding
same column families
same query keysets
same workload definitions
```

### Benchmark layers

```text
Layer 1: checksum and data equality
Layer 2: DB-level benchmark
Layer 3: API-level benchmark (planned)
Layer 4: stability and read-under-write benchmark (planned)
```

### Core workloads

```text
get_tx
get_object_version
get_object_last_seen
multi_get_tx_50
multi_get_object_version_50
scan_events_100
mixed_rpc
```

Planned extension:

```text
scan_owner_touched_objects_100
```

### Mixed RPC workload

```text
50% get_tx
20% get_object_version
15% multi_get_object_version_50
10% scan_events_100
5% get_object_last_seen
```

### Metrics

```text
- throughput
- p50 latency
- p90 latency
- p95 latency
- p99 latency
- p999 latency
- max latency
- error rate
- disk usage
- 30-minute stability
```

---

## 12. Demo Dataset Strategy

Sui HotStore can support multiple dataset modes.

### Mode A: Mirror sample mode

Uses prepared Sui exported data, such as:

```text
transactions
events
objects
checkpoints
```

This is the fastest mode for benchmark-scale testing.

Purpose:

```text
- large benchmark dataset
- tx/event/object serving demo
- ToplingDB vs RocksDB comparison
```

---

### Mode B: Checkpoint sample mode

Uses a bounded official Sui checkpoint range.

Purpose:

```text
- source-of-truth ingestion validation
- checkpoint-scoped query validation
- archive path proof-of-concept
```

---

### Mode C: Synthetic mode

Uses deterministic Sui-like generated data.

Purpose:

```text
- reproducible tests
- controlled workload size
- CI-friendly benchmark
- feature development before full real-data ingestion
```

---

### Mode D: Snapshot / current-state mode

Future path using official formal snapshot or consistent store restore.

Purpose:

```text
- current object state
- owner/object queries
- balance-style serving path
```

---

## 13. How to Run

### Synthetic ingestion

```bash
cargo run --release --bin hotstore-ingest -- \
  --backend rocksdb \
  --db-path ./data/sui-rocksdb \
  --dataset synthetic \
  --checkpoints 1000 \
  --tx-per-checkpoint 50 \
  --objects-per-checkpoint 200 \
  --events-per-checkpoint 100 \
  --seed 42
```

```bash
cargo run --release --bin hotstore-ingest -- \
  --backend toplingdb \
  --db-path ./data/sui-toplingdb \
  --dataset synthetic \
  --checkpoints 1000 \
  --tx-per-checkpoint 50 \
  --objects-per-checkpoint 200 \
  --events-per-checkpoint 100 \
  --seed 42
```

---

### Real Sui bounded-range ingestion

Current real-data path uses JSON-RPC sourced checkpoint / transaction data and writes
bounded-range records into a HotStore DB. In the current route 1 flow, benchmark keys
are emitted during ingest and progress watermarks are persisted for resume:

```bash
cargo run --release --bin sui-hotstore-ingest-real -- \
  --network mainnet \
  --remote-store-url https://checkpoints.mainnet.sui.io \
  --first-checkpoint <FIRST_CHECKPOINT> \
  --last-checkpoint <LAST_CHECKPOINT> \
  --backend rocksdb \
  --db-path /data4/sui-hotstore-mainnet-range \
  --bench-keys-dir /data4/sui-hotstore-mainnet-range/keys \
  --record-mode lite \
  --checkpoint-batch-size 100 \
  --tx-batch-size 50 \
  --max-retries 6 \
  --retry-backoff-ms 1000 \
  --resume
```

---

### Validate data equality

```bash
cargo run --release --bin hotstore-admin -- \
  checksum \
  --backend rocksdb \
  --db-path ./data/sui-rocksdb \
  --output ./reports/checksum/rocksdb-checksum.json
```

```bash
cargo run --release --bin hotstore-admin -- \
  checksum \
  --backend toplingdb \
  --db-path ./data/sui-toplingdb \
  --output ./reports/checksum/toplingdb-checksum.json
```

```bash
cargo run --release --bin hotstore-admin -- \
  compare-checksum \
  --left ./reports/checksum/rocksdb-checksum.json \
  --right ./reports/checksum/toplingdb-checksum.json
```

---

### Run benchmark suite

```bash
bash scripts/run-benchmark-suite.sh \
  --backend rocksdb \
  --db-path ./data/sui-rocksdb \
  --keys-dir ./bench/keys \
  --report-dir ./reports/rocksdb \
  --requests 100000 \
  --concurrency 1,4,8,16,32,64 \
  --batch-size 50 \
  --cargo-profile release
```

```bash
bash scripts/run-benchmark-suite.sh \
  --backend toplingdb \
  --db-path ./data/sui-toplingdb \
  --keys-dir ./bench/keys \
  --report-dir ./reports/toplingdb \
  --requests 100000 \
  --concurrency 1,4,8,16,32,64 \
  --batch-size 50 \
  --cargo-profile release
```

---

### Route 1 server run

For the current demo, the most practical end-to-end server workflow is the route 1
script that:

```text
- resolves or reuses a fixed checkpoint range from public Sui RPC
- ingests the bounded range into HotStore
- emits benchmark keys from the same ingest path
- persists run config and watermarks for resume
- runs stats, checksum, and DB benchmark suite
```

This route is also our current evidence path for a practical reason: we already restored
roughly `200G` of Sui data through the formal snapshot workflow, but bringing a local
`sui-node` on top of that dataset to a stable benchmark-ready state still involves a long
pruning / compaction window. Because of that, the current benchmark results are based on
bounded-range live-RPC ingestion, with the current public run focused on about `10,000`
checkpoints. Larger dataset runs are still in progress.

```bash
bash scripts/run-route1-benchmark-server.sh \
  --network mainnet \
  --first-checkpoint 270700000 \
  --last-checkpoint 270709999 \
  --base-dir /data4/sui-hotstore-route1-mainnet-270700000-270709999-rocksdb \
  --backend rocksdb \
  --cargo-profile release \
  --requests 100000 \
  --concurrency 1,4,8,16,32,64 \
  --batch-size 50 \
  --tx-batch-size 50 \
  --rpc-max-retries 8 \
  --rpc-retry-backoff-ms 1500 \
  --step-max-attempts 50 \
  --step-retry-sleep-secs 30
```

For strict RocksDB vs ToplingDB comparison, both runs should use the same explicit:

```text
FIRST_CHECKPOINT
LAST_CHECKPOINT
```

ToplingDB practical benchmark runs for submission evidence should be done on Linux.

---

### Run API server (planned)

The API crate exists, but HTTP routes are not the current submission focus yet.

```bash
cargo run --release --bin hotstore-api -- \
  --backend toplingdb \
  --db-path ./data/sui-toplingdb \
  --host 0.0.0.0 \
  --port 8080
```

---

### API smoke test

```bash
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/stats
```

---

## 14. Expected Submission Narrative

### Project summary

Sui HotStore is a KV-native data serving layer for Sui applications. It uses ToplingDB as a high-performance RocksDB-compatible backend and benchmarks it against RocksDB on Sui-style data workloads.

---

### Why it matters

Many Sui data applications repeatedly query chain data in KV-shaped patterns:

```text
transaction digest lookup
object version lookup
event type scan
owner/object lookup
batched multi_get
```

Sui HotStore provides a dedicated local serving layer for these workloads, reducing pressure on full node RPC, GraphQL, and general-purpose analytical databases.

For current evidence collection, practical ToplingDB benchmark runs are Linux-based.

---

### What is new

The project contributes:

```text
- a Sui-specific KV schema
- ToplingDB-backed storage layer
- RocksDB baseline comparison
- benchmark suite for Sui data workloads
- benchmark key generation and checksum validation workflow
- API scaffold for RPC/indexer/archive-style queries
- clear roadmap toward custom indexer and archive integration
```

---

### Why it fits Infra & Tooling

Sui HotStore is infrastructure for developers and operators building on Sui.

It helps:

```text
- indexer builders
- RPC providers
- DeFi analytics teams
- wallet backend teams
- explorer teams
- archive-query services
```

It is not an end-user dApp. It is a developer/operator tool for making Sui data access faster and more benchmarkable.

---

## 15. Demo Script

### 0:00 — Problem

Sui applications need fast access to transactions, objects, events, and historical versions.

Some of these queries are naturally KV-shaped and should not always go through PostgreSQL, GraphQL, or full node RPC.

---

### 0:20 — Solution

Sui HotStore ingests bounded-range Sui data into a KV-native local serving layer.

It builds column families for:

```text
tx_by_digest
object_version
object_last_seen
event_by_type
owner_touched_objects
```

---

### 0:45 — Route 1 workflow

Show:

```bash
bash scripts/run-route1-benchmark-server.sh --help
```

---

### 1:05 — Integrity demo

Show:

```bash
cat reports/<backend>/stats.json
cat reports/<backend>/checksum/checksum.json
```

---

### 1:25 — Benchmark demo

Show benchmark summary:

```text
RocksDB vs ToplingDB
same data
same schema
same keysets
same workloads
```

Highlight:

```text
tx lookup
object version lookup
event prefix scan
multi_get_50
mixed_rpc
disk usage
checksum consistency
```

Current result summary:

```text
- mainnet checkpoint range: 270700000..270709999
- imported checkpoints: 10,000
- logical entries: 1,082,702
- checksum: all benchmark-facing data column families match; only cf_meta differs by 2 value bytes
- get-tx: ToplingDB leads by about 8.9 percent
- multi-get-tx: ToplingDB leads by about 1.2 percent
- multi-get-object-version: ToplingDB leads by about 2.5 percent
- mixed-rpc: RocksDB leads by about 7.0 percent
- scan-events: effectively tied
```

If only one backend's fresh evidence is visible in the recording window, say explicitly which side is already captured and note that ToplingDB submission evidence is collected on Linux.

---

### 1:50 — Platform and roadmap

Mention:

```text
- RocksDB smoke and development work can be done on macOS
- practical ToplingDB benchmark evidence is collected on Linux
```

Then close with future work:

Future work:

```text
- full Sui checkpoint integration
- formal snapshot bootstrap
- Sui custom indexer BYOS backend
- raw archive record mode
- ToplingDB SidePlugin observability
- distributed compaction experiment
```

---

## 16. Roadmap

### Phase 1 — MVP

```text
- synthetic and prepared dataset ingestion
- bounded real Sui checkpoint ingestion
- RocksDB baseline
- ToplingDB backend
- benchmark suite
- checksum validation
- API scaffold
```

---

### Phase 2 — Real Sui pipeline

```text
- official checkpoint sample ingestion
- real event/object/tx extraction
- better event type indexing
- improved object version handling
- GraphQL/gRPC validation samples
```

---

### Phase 3 — Custom indexer integration

```text
- Sui custom indexer storage adapter
- Store / Connection implementation
- configurable retention
- live ingestion with polling fallback
```

---

### Phase 4 — Archive and snapshot

```text
- formal snapshot bootstrap
- object state cache
- raw checkpoint/archive record mode
- historical object query service
```

---

### Phase 5 — ToplingDB advanced features

```text
- SidePlugin configuration
- metrics and observability
- Prometheus/Grafana dashboard
- distributed compaction experiment
- compression configuration experiments
```

---

## 17. Submission Package Checklist

To turn this draft into a clean Sui Overflow submission package, prepare:

```text
- final project description (this document, trimmed for the form length)
- 1 short demo video
- 3 to 5 benchmark result screenshots or tables
- checksum consistency evidence
- repo link
- short architecture diagram
- one benchmark summary markdown or PDF
```

Recommended concrete artifacts:

```text
- docs/sui-overflow-submission.md
- docs/sui-overflow-demo-script.md
- docs/sui-overflow-form-answers.md
- docs/benchmark_runbook.md
- reports/summary.md
- reports/<backend>/stats.json
- reports/<backend>/checksum/checksum.json
- reports/<backend>/db/*.json
```

Suggested demo evidence to capture:

```text
- route 1 server run output
- stats output
- checksum compare output
- get-tx / multi-get / scan-events benchmark summary
- RocksDB vs ToplingDB comparison table
```

---

## 17. Risks and Limitations

### Current limitations

```text
- bounded dataset semantics
- not full-chain latest object state
- not complete address inventory
- not fullnode storage replacement
- not validator storage replacement
- ToplingDB native build may require environment-specific setup
```

### Benchmark caveats

```text
- results are hardware-specific
- warm cache and cold cache must be reported separately
- API benchmark includes serialization and HTTP overhead
- DB-level benchmark is required for backend comparison
- p99 and p999 are more important than average latency
```

---

## 18. What We Want Feedback On

We are looking for feedback from:

```text
- Sui infra engineers
- RPC providers
- custom indexer builders
- explorer teams
- wallet backend teams
- DeFi analytics teams
- archive data service operators
```

Questions:

```text
1. Which Sui data access patterns are most painful today?
2. Which queries need a local KV hot path?
3. What retention windows matter most?
4. Should the next integration target custom indexer BYOS, checkpoint ingestion, or formal snapshot bootstrap?
5. What benchmark workloads would be most useful for RPC providers and data teams?
```

---

## 19. Submission Form Draft Answers

### What are you building?

We are building Sui HotStore, a ToplingDB-powered local KV serving layer for Sui custom indexers, RPC hot paths, and archive-style workloads.

It indexes Sui-style transactions, events, objects, owners, and checkpoints into column families, then exposes low-latency query APIs and benchmarks them against a RocksDB baseline.

---

### Who is it for?

It is for:

```text
- Sui custom indexer builders
- RPC providers
- wallet backend teams
- explorer teams
- DeFi analytics teams
- archive-query services
```

---

### What problem does it solve?

It solves the problem of serving high-frequency KV-shaped Sui queries efficiently.

Examples:

```text
- tx digest lookup
- object version lookup
- event type scan
- owner touched-object lookup
- multi_get for batched RPC-style requests
```

These hot paths can be inefficient or expensive when served only through full node RPC, GraphQL, PostgreSQL, or analytical databases.

---

### What makes it unique?

Sui HotStore combines:

```text
- Sui-specific data access schema
- ToplingDB backend
- RocksDB-compatible baseline comparison
- benchmark suite
- API serving layer
- clear bounded-range semantics
```

It focuses on serving-path infrastructure rather than end-user dApp functionality.

---

### What did you build during the hackathon?

During the hackathon/demo window, we built:

```text
- Rust workspace for Sui HotStore
- storage abstraction
- RocksDB baseline
- ToplingDB integration path
- Sui-style column family schema
- deterministic key encoding
- ingestion path for prepared datasets
- synthetic workload generator
- API server
- benchmark suite
- checksum validator
- benchmark report template
```

---

### How does it use Sui?

It uses Sui-style transactions, events, objects, object versions, owners, and checkpoints as the core data model.

The project is designed to integrate with Sui data sources such as:

```text
- exported indexed Sui datasets
- checkpoint samples
- future full checkpoint ingestion
- future custom indexer pipeline
- future formal snapshot bootstrap
```

---

### What is the technical architecture?

```text
Sui data source
        |
        v
Sui HotStore ingestion
        |
        v
Storage abstraction
  ├── RocksDB baseline
  └── ToplingDB backend
        |
        v
Column families
  ├── checkpoint
  ├── tx_by_digest
  ├── object_version
  ├── object_last_seen
  ├── event_by_type
  └── owner_touched_objects
        |
        v
Serving API and benchmark suite
```

---

### What are the next steps?

```text
1. Integrate official Sui checkpoint ingestion.
2. Add formal snapshot bootstrap for current-state object/owner queries.
3. Implement Sui custom indexer storage adapter.
4. Expand raw archive record mode.
5. Benchmark ToplingDB advanced features such as SidePlugin observability and distributed compaction.
6. Work with Sui RPC providers and indexer teams to define production workloads.
```

---

## 20. Final Positioning

```text
Sui HotStore is not a fullnode replacement.

It is a KV-native hot path for Sui data applications.

It helps custom indexers, RPC providers, explorers, wallets, DeFi analytics services, and archive-query systems serve high-frequency Sui data queries more efficiently.
```

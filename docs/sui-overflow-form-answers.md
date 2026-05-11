# Sui Overflow Form Answers — Sui HotStore

> Purpose: copy-paste answers for Sui Overflow submission forms  
> Style: concise, reviewer-friendly, aligned with the current MVP  
> Project: **Sui HotStore**

---

## 1. Project Name

```text
Sui HotStore
```

---

## 2. One-Liner

### Primary version

```text
Sui HotStore is a ToplingDB-powered local KV serving layer for Sui custom indexers, RPC hot paths, and archive-style query workloads.
```

### Shorter version

```text
A KV-native serving layer and benchmark stack for Sui data applications.
```

---

## 3. Elevator Pitch

### 280-character version

```text
Sui HotStore turns bounded-range Sui chain data into a local KV serving layer optimized for transaction lookup, object version lookup, event scans, and batched reads, with a RocksDB baseline and a ToplingDB backend for reproducible infra benchmarking.
```

### 500-character version

```text
Sui HotStore is an infra tool for Sui data applications. It ingests bounded-range Sui data into a Sui-specific KV schema, supports a RocksDB baseline plus a ToplingDB backend, and benchmarks hot-path workloads like tx lookup, object version lookup, event scans, and multi-get. The current route 1 workflow fixes a checkpoint range, emits benchmark keys during ingest, supports resume on interruption, and gives indexers, RPC providers, explorers, and analytics services a faster, more local, and more benchmarkable serving layer.
```

---

## 4. Problem Statement

### Short version

```text
Many Sui data services repeatedly serve KV-shaped queries such as tx digest lookup, object version lookup, event prefix scans, and batched reads. These hot paths can create latency and infrastructure pressure when routed through remote RPC, GraphQL, or general-purpose analytical databases.
```

### Longer version

```text
Sui applications like explorers, wallets, RPC providers, and analytics systems need fast access to transactions, object versions, events, and checkpoint-scoped data. Many of these requests are naturally KV-shaped, but are often served through remote RPC or heavier storage layers. That can increase p99 latency, amplify load on full nodes or indexers, and make it harder to separate hot-path serving from analytical storage.
```

---

## 5. Solution

### Short version

```text
Sui HotStore provides a local KV-native serving layer for Sui data workloads. It uses deterministic key encoding, a Sui-specific column-family schema, checksum validation, and a benchmark suite to compare a RocksDB baseline against a ToplingDB backend on the same fixed dataset and query keysets.
```

### Longer version

```text
Sui HotStore transforms bounded-range Sui data into a benchmarkable KV schema with indexes for checkpoint metadata, tx lookup by digest, object versions, latest-observed objects within the imported range, and event prefix scans. Its current route 1 workflow fixes a checkpoint range, ingests real data, emits benchmark keys during ingest, and persists resume watermarks. It keeps backend comparisons disciplined through the same schema, same dataset, same keysets, and checksum validation, so performance claims can be tied to reproducible evidence rather than ad hoc measurements.
```

---

## 6. Why It Matters For Sui

```text
Sui has a growing set of infra-heavy applications: custom indexers, RPC providers, explorers, wallets, and analytics services. Those systems need fast and predictable access to chain data. Sui HotStore focuses on the serving side of that problem by providing a KV-native layer for the query patterns that appear repeatedly in real Sui data infrastructure.
```

---

## 7. Why This Fits Infra & Tooling

```text
Sui HotStore is not an end-user dApp. It is a developer and operator tool. It helps infrastructure teams build faster query paths, reduce repeated pressure on upstream RPC or analytical storage, and benchmark backend choices with the same workloads and the same data model.
```

---

## 8. Current MVP Status

```text
The current MVP focuses on bounded checkpoint-range semantics. It includes a Rust workspace, deterministic key encoding, a RocksDB baseline, a ToplingDB backend path, bounded-range real-data ingestion via Sui JSON-RPC, checksum validation, benchmark key generation during ingest, resume support for interrupted runs, and a DB-level benchmark suite for tx lookup, object version lookup, event scan, multi-get, and mixed RPC-style workloads.
```

```text
For benchmark evidence, we have already restored roughly 200G of Sui data via the formal snapshot path, but local sui-node pruning / compaction time on top of that dataset is still too long for it to be the main demo route today. As a result, the current public benchmark run uses a bounded live-RPC route over checkpoints 270700000..270759999, covering 60,000 checkpoints and 6,242,137 logical entries.
```

---

## 9. Important Semantic Disclaimer

```text
This MVP currently supports bounded checkpoint-range semantics. For example, object_last_seen means the latest object version observed within the imported range, not the global latest object state across the full Sui network.
```

---

## 10. Demo Summary

```text
The demo shows a bounded-range real Sui dataset being ingested into HotStore, benchmark keys emitted from the same ingest path, checksum and stats outputs produced for integrity validation, and DB-level benchmark results collected for query workloads such as get-tx, get-object-version, multi-get, and scan-events. For strict backend comparison, RocksDB and ToplingDB runs use the same explicit checkpoint range.
```

```text
The current public benchmark evidence is intentionally bounded. We already have a much larger formal-snapshot dataset on disk, but because local sui-node pruning / compaction still takes a long time on that path, the current demo run uses checkpoints 270700000..270759999 sourced through the route 1 workflow. We will keep extending the benchmark corpus with larger datasets.
```

```text
In the current mainnet route 1 run over checkpoints 270700000..270759999, both backends ingest the same bounded dataset with 6,242,137 logical entries. All benchmark-facing data column families match by checksum, with the only mismatch appearing in cf_meta, where ToplingDB differs by 2 value bytes. On performance, ToplingDB is ahead across the DB-level workloads in this report, including about 3.3x RocksDB throughput on get-tx, about 14.5x on multi-get-tx, about 15.2x on multi-get-object-version, and about 8.2x on mixed-rpc.
```

---

## 11. What Is Novel

```text
- a Sui-specific KV schema for hot-path data access
- reproducible backend comparison between RocksDB and ToplingDB
- checksum-first validation workflow for storage equality
- benchmark tooling shaped around Sui-style serving workloads
- a practical path toward deeper custom indexer and archive integration
```

---

## 12. Target Users

```text
- custom indexer operators
- RPC providers
- explorer backends
- wallet backends
- analytics and archive-query services
```

---

## 13. Technical Keywords

```text
Sui, ToplingDB, RocksDB, key-value storage, custom indexer, RPC infrastructure, event indexing, archive queries, multi-get benchmarking, storage benchmarking
```

---

## 14. Platform Note

```text
ToplingDB practical benchmark runs are currently treated as Linux-only in this project. macOS is still useful for documentation work, RocksDB smoke checks, and general development, but the ToplingDB-backed benchmark evidence and backend validation should be collected on Linux.
```

---

## 15. Submission Form Variants

### If the form asks “What did you build?”

```text
We built Sui HotStore, a KV-native serving layer and benchmark stack for Sui data applications. It ingests bounded-range Sui data into a Sui-specific schema, keeps a RocksDB baseline, evaluates a ToplingDB backend, validates data equality with checksum tooling, and benchmarks hot-path workloads like tx lookup, object version lookup, multi-get, and event scans. In the current mainnet run over 60,000 checkpoints, ToplingDB leads across the DB-level workloads and uses about 78.6 percent less disk in the report bundle.
```

### If the form asks “What problem are you solving?”

```text
We are solving the gap between general-purpose storage and the repeated KV-shaped queries that appear in Sui data infrastructure. Many Sui applications need fast lookup for transactions, object versions, and events, but those hot paths are often routed through heavier systems or repeated upstream RPC calls.
```

### If the form asks “Why did you choose Sui?”

```text
Sui has a rich data surface and a strong ecosystem need for better infra around indexers, RPC, and archive-style access. Its workload mix makes it a strong fit for a local KV-native serving layer and for benchmarking backend choices in a reproducible way.
```

### If the form asks “What is your roadmap?”

```text
Next steps include deeper real Sui ingestion, formal snapshot bootstrap, fuller custom indexer integration, richer event and owner/object query support, and broader backend benchmarking and observability.
```

---

## 16. Judge-Friendly 3-Sentence Version

```text
Sui HotStore is a KV-native serving layer for Sui data applications. It focuses on bounded-range ingestion, deterministic indexing, checksum validation, resume-aware benchmark workflows, and reproducible benchmarking of RocksDB versus ToplingDB on real Sui-style query workloads. In the current 60,000-checkpoint mainnet run, ToplingDB leads across point lookup, multi-get, scan-events, and mixed-rpc DB workloads.
```

---

## 17. Very Short Taglines

```text
- Faster Sui data hot paths, backed by KV-native storage.
- Benchmarkable Sui storage paths for indexers and RPC providers.
- A local KV serving layer for Sui data infrastructure.
```

---

## 18. What To Fill In Later

The current benchmark run already gives us these concrete anchor points:

```text
- checkpoint range: 270700000..270759999
- imported checkpoints: 60,000
- logical entry count: 6,242,137
- RocksDB disk usage: 6,244,820,049 bytes
- ToplingDB disk usage: 1,337,044,105 bytes
- get-tx: ToplingDB about 231 percent faster at best-throughput point
- mixed-rpc: ToplingDB about 8.2x RocksDB throughput
- multi-get-tx: ToplingDB about 14.5x RocksDB throughput
- multi-get-object-version: ToplingDB about 15.2x RocksDB throughput
- scan-events: ToplingDB about 7.8 percent faster
- checksum status: all benchmark-facing data CFs match; cf_meta differs by 2 value bytes
```

Still worth filling in before a polished final submission if available:

```text
- exact CPU / memory / disk model for the server run
- test date and cache-state note
- whether we want to quote best-throughput points only or also a fixed-concurrency comparison
```

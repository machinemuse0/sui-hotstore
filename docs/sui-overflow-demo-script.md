# Sui Overflow Demo Script — Sui HotStore

> Goal: a clean submission video script aligned with the current project state  
> Project: **Sui HotStore**  
> Target length: about 2 minutes 30 seconds  
> Important: this script stays honest about current implementation status, especially around the API layer

---

## 1. Recording Principle

This demo should feel polished, but it should not overclaim.

Use this rule throughout:

```text
- show implemented DB ingest / checksum / benchmark evidence directly
- present API routes as the planned serving interface unless they are actually live in the recording environment
- present ToplingDB benchmark evidence from Linux
```

Platform note:

```text
- macOS is suitable for docs, RocksDB smoke checks, and general development
- practical ToplingDB benchmark evidence should be collected on Linux
```

---

## 2. Video Outline

### 0:00 - Problem

Narration:

```text
Sui apps need fast access to transactions, objects, events, and historical versions. Many of these queries are naturally KV-shaped, but they are often served through remote RPC or heavier storage layers that are not ideal for hot-path lookup.
```

Show:

```text
- docs/sui-overflow-submission.md
- the Problem section
```

On-screen emphasis:

```text
tx digest lookup
object version lookup
event scan
historical lookup
batched reads
```

---

### 0:20 - Architecture

Narration:

```text
Sui HotStore takes bounded-range Sui data, ingests it into a KV-native schema, and benchmarks the same workloads on a RocksDB baseline and a ToplingDB backend.
```

Show:

```text
Sui data -> Sui HotStore ingest -> ToplingDB/RocksDB -> API/benchmark
```

Recommended visual:

```text
- README.md architecture diagram
or
- docs/sui-overflow-submission.md architecture section
```

Add one sentence:

```text
This is not fullnode storage. It is a local serving layer for Sui data applications.
```

---

### 0:45 - API Demo

Important honesty note:

```text
The hotstore-api crate is still a scaffold in the current repo. For submission, present these routes as the planned demo API surface unless you are recording against a separate live serving prototype.
```

Narration:

```text
The intended serving surface covers common Sui data access patterns: stats, transaction lookup, object version lookup, event scans, and batched transaction queries.
```

Show:

```text
/stats
/tx/{digest}
/object/{object_id}/version/{version}
/events
/multi-get/txs
```

Best current recording approach:

```text
- show the planned API section in docs/sui-overflow-submission.md
- if you have a separate live API prototype, replace this section with real curl calls
```

Suggested on-screen command block if using docs only:

```bash
sed -n '120,220p' docs/sui-overflow-submission.md
```

Suggested narration line to keep it accurate:

```text
The serving API is the intended interface for these lookup patterns, while the current benchmark work is focused on the DB layer underneath it.
```

---

### 1:30 - Benchmark

Narration:

```text
The benchmark suite is where the current implementation is strongest today. We validate checksum and then measure DB-level workloads such as tx lookup, object version lookup, multi-get, event scans, and a mixed RPC-style workload.
```

Optional honesty line for the current recording:

```text
We also restored about 200G through the formal snapshot path, but local sui-node pruning and compaction on top of that dataset still takes long enough that today's benchmark evidence uses a bounded live-RPC route 1 run over checkpoints 270700000 through 270759999.
```

Show:

```bash
cat reports/summary.md
```

Highlight:

```text
- get-tx
- get-object-version
- multi-get-object-version
- scan-events
- mixed-rpc
- disk usage
- checksum consistency
```

Current result summary:

```text
- mainnet checkpoint range: 270700000..270759999
- imported checkpoints: 60,000
- logical entries: 6,242,137
- checksum: all benchmark-facing data column families match; only cf_meta differs by 2 value bytes
- get-tx: ToplingDB leads by about 231 percent at the best-throughput point
- multi-get-tx: ToplingDB leads by about 13.5x
- multi-get-object-version: ToplingDB leads by about 15.2x
- mixed-rpc: ToplingDB leads by about 8.2x
- scan-events: ToplingDB leads by about 7.8 percent
- disk usage: ToplingDB uses about 78.6 percent less space in this report bundle
```

If both backends are ready, say:

```text
RocksDB and ToplingDB are compared on the same explicit checkpoint range, same schema, same key encoding, same keysets, and same workload definitions.
```

If only RocksDB evidence is ready, say:

```text
The RocksDB route is fully runnable today, and ToplingDB benchmark evidence is collected on Linux.
```

---

### 2:10 - Positioning

Narration:

```text
Sui HotStore is not a fullnode replacement. It is not validator storage. It is a KV-native hot path for Sui custom indexers, RPC cache workloads, and archive-style data queries.
```

Show:

```text
- README.md Known Limitations
or
- docs/sui-overflow-submission.md semantics / scope sections
```

Make sure to mention:

```text
- current dataset is bounded
- object_last_seen means latest observed within the imported dataset
- owner_touched_objects is not a complete wallet inventory
```

---

### 2:30 - Roadmap

Narration:

```text
From here, the roadmap is deeper checkpoint ingestion, custom indexer BYOS integration, formal snapshot bootstrap, and more advanced ToplingDB-specific capabilities.
```

Show:

```text
- checkpoint ingestion
- custom indexer BYOS
- formal snapshot bootstrap
- ToplingDB advanced features
```

Suggested on-screen source:

```text
docs/sui-overflow-submission.md roadmap section
```

---

## 3. Recommended Recording Assets

Prepare these before recording:

```text
- README.md
- docs/sui-overflow-submission.md
- reports/summary.md
- reports/<backend>/stats.json
- reports/<backend>/checksum/checksum.json
- reports/<backend>/db/get-tx.json
- reports/<backend>/db/mixed-rpc.json
```

If route 1 results are ready, also have:

```text
- the exact fixed checkpoint range used
- the route 1 output directory
- the final report directory for RocksDB
- the final report directory for ToplingDB, if complete
```

---

## 4. Suggested Terminal Sequence

### Shot A — architecture

```bash
sed -n '1,200p' README.md
```

### Shot B — submission framing

```bash
sed -n '1,260p' docs/sui-overflow-submission.md
```

### Shot C — benchmark summary

```bash
cat reports/summary.md
```

### Shot D — checksum / stats evidence

```bash
cat reports/<backend>/stats.json
cat reports/<backend>/checksum/checksum.json
```

### Shot E — route 1 workflow

```bash
bash scripts/run-route1-benchmark-server.sh --help
```

---

## 5. What Not To Claim

Do not say:

```text
- this already replaces Sui fullnodes
- this is validator storage
- this already provides full global latest-object semantics
- the API benchmark is complete
- the API demo shown is live if you are actually showing planned routes only
```

Prefer:

```text
- bounded-range MVP
- KV-native serving path
- DB-level benchmark is implemented
- API surface is planned and maps cleanly to the storage model
- ToplingDB benchmark evidence is captured on Linux
```

---

## 6. Delivery Note

I cannot directly output a finished `.mp4` from this environment, but this file is intended to be production-ready for recording or handing to an editor.

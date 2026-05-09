# Sui Overflow Video Shot List — Sui HotStore

> Companion to `docs/sui-overflow-demo-script.md`  
> Purpose: fast recording checklist for a 2m30s submission video

---

## Shot 1 — Problem (0:00 - 0:20)

Visual:

```text
docs/sui-overflow-submission.md
```

Frame:

```text
- Problem
- KV-shaped lookups
- tx / object / event / historical access
```

Voice line:

```text
Sui apps need fast transaction, object, event, and historical lookup, and many of those requests are naturally KV-shaped.
```

---

## Shot 2 — Architecture (0:20 - 0:45)

Visual:

```text
README.md architecture diagram
```

Frame:

```text
Sui data -> ingest -> ToplingDB/RocksDB -> API/benchmark
```

Voice line:

```text
Sui HotStore ingests bounded-range Sui data into a KV-native schema and supports both a RocksDB baseline and a ToplingDB backend for benchmarked serving paths.
```

---

## Shot 3 — API Surface (0:45 - 1:30)

Visual:

```text
docs/sui-overflow-submission.md API section
```

Frame:

```text
/stats
/tx/{digest}
/object/{object_id}/version/{version}
/events
/multi-get/txs
```

Voice line:

```text
The intended serving surface maps directly onto the core Sui lookup patterns we care about, while the current implementation focus is on the DB layer underneath it.
```

Note:

```text
If a live API server is not available, present this as the planned interface rather than pretending these are live endpoints in the current repo.
```

---

## Shot 4 — Benchmark (1:30 - 2:10)

Visual:

```bash
cat reports/summary.md
```

Frame:

```text
- get-tx
- get-object-version
- multi-get-object-version
- scan-events
- mixed-rpc
- checksum consistency
```

Voice line:

```text
The benchmark suite validates checksum first and then measures DB-level workloads that matter for Sui data serving.
```

---

## Shot 5 — Positioning (2:10 - 2:30)

Visual:

```text
README.md Known Limitations
```

Frame:

```text
not a fullnode replacement
not validator storage
bounded dataset
latest observed within imported range
```

Voice line:

```text
This is not a fullnode replacement and not validator storage. It is a KV-native hot path for Sui custom indexers, RPC cache workloads, and archive-style queries.
```

---

## Shot 6 — Roadmap (2:30 - end)

Visual:

```text
docs/sui-overflow-submission.md roadmap
```

Frame:

```text
checkpoint ingestion
custom indexer BYOS
formal snapshot bootstrap
ToplingDB advanced features
```

Voice line:

```text
The roadmap is deeper checkpoint ingestion, custom indexer integration, formal snapshot bootstrap, and more advanced ToplingDB-specific capabilities.
```

---

## Platform Footnote

Include this somewhere in the spoken or written demo context:

```text
ToplingDB practical benchmark evidence is collected on Linux. macOS remains useful for development, docs, and RocksDB-side smoke checks.
```

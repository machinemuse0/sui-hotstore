# Sui HotStore Benchmark Summary

## Scope

- Benchmark target: Sui HotStore DB-level workloads on a bounded real Sui dataset
- Backends:
  - RocksDB baseline
  - ToplingDB
- Status:
  - DB-level benchmark: completed for the latest `data/report1` and `data/report2` bundle
  - API-level benchmark: still out of scope for this report
- Summary selection rule: for each workload/backend pair, this summary uses the run with the highest `throughput_rps`

## Dataset

- Source: route 1 bounded-range ingestion from live Sui JSON-RPC
- Network: mainnet
- Checkpoint range: `270700000..270709999`
- Imported checkpoints: `10,000`
- Logical records from the benchmarked DB:
  - `cf_checkpoint`: `60,000`
  - `cf_tx_by_digest`: `530,490`
  - `cf_event_by_type`: `811,079`
  - `cf_object_version`: `2,380,717`
  - `cf_object_last_seen`: `79,125`
  - `cf_owner_touched_objects`: `2,380,717`
  - `cf_meta`: `9`
  - `default`: `0`
- Important note:
  - We already restored about `200G` through the formal snapshot path, but local `sui-node` pruning / compaction time on top of that dataset is still too long for it to be the main benchmark evidence path today.
  - Because of that, the current public benchmark evidence is based on this bounded `10,000` checkpoint route 1 run.
  - Larger dataset runs are still in progress.

## Hardware

- CPU: not captured in this report bundle
- Memory: not captured in this report bundle
- Disk: not captured in this report bundle
- OS: Linux server run
- Test date: not explicitly captured in the report JSON; report directories were updated on `2026-05-07`
- Cache state: not explicitly captured

## Integrity Summary

- Total logical entry count:
  - RocksDB: `6,242,137`
  - ToplingDB: `6,242,137`
- Total key bytes:
  - RocksDB: `613,806,478`
  - ToplingDB: `613,806,478`
- Total value bytes:
  - RocksDB: `2,319,130,899`
  - ToplingDB: `2,319,130,901`
- Total checksum:
  - RocksDB: `7adfc56a357db9d03612cfb9b8ae541c1e458abcee17f93f7e02d8f3b5d50e13`
  - ToplingDB: `786e797963a7e3640e894cd68572438da3e63a792f3fd061ba40aa70e2c13a10`
- Per-column-family checksum result:
  - `cf_checkpoint`: match
  - `cf_event_by_type`: match
  - `cf_object_last_seen`: match
  - `cf_object_version`: match
  - `cf_owner_touched_objects`: match
  - `cf_tx_by_digest`: match
  - `default`: match
  - `cf_meta`: mismatch
- `cf_meta` detail:
  - RocksDB `value_bytes`: `290`
  - ToplingDB `value_bytes`: `292`
  - This `2`-byte difference is enough to make the total DB checksum differ, even though all benchmark-facing data column families match.

## DB-Level Results

| Workload | Backend | Best Concurrency | Requests | Throughput RPS | p50 ms | p95 ms | p99 ms | p999 ms | Errors |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| get-tx | RocksDB | 1 | 500000 | 1349023.70 | 0.000 | 0.000 | 0.000 | 0.016 | 0 |
| get-tx | ToplingDB | 1 | 500000 | 1164037.52 | 0.000 | 0.000 | 0.001 | 0.016 | 0 |
| get-object-version | RocksDB | 1 | 500000 | 532473.18 | 0.000 | 0.001 | 0.002 | 0.019 | 0 |
| get-object-version | ToplingDB | 1 | 500000 | 564267.68 | 0.000 | 0.001 | 0.001 | 0.018 | 0 |
| get-object-last-seen | RocksDB | 8 | 500000 | 3629940.65 | 0.001 | 0.002 | 0.002 | 0.021 | 0 |
| get-object-last-seen | ToplingDB | 8 | 500000 | 3305584.17 | 0.001 | 0.002 | 0.002 | 0.022 | 0 |
| multi-get-tx | RocksDB | 8 | 500000 | 91808.29 | 0.069 | 0.111 | 0.133 | 0.163 | 0 |
| multi-get-tx | ToplingDB | 8 | 500000 | 95154.50 | 0.062 | 0.099 | 0.122 | 0.180 | 0 |
| multi-get-object-version | RocksDB | 8 | 500000 | 65803.36 | 0.089 | 0.140 | 0.165 | 0.208 | 0 |
| multi-get-object-version | ToplingDB | 8 | 500000 | 75193.18 | 0.076 | 0.134 | 0.162 | 0.197 | 0 |
| scan-events | RocksDB | 64 | 500000 | 1223579.85 | 0.032 | 0.112 | 0.159 | 0.260 | 0 |
| scan-events | ToplingDB | 64 | 500000 | 1249729.43 | 0.030 | 0.109 | 0.153 | 0.265 | 0 |
| mixed-rpc | RocksDB | 4 | 500000 | 170028.23 | 0.000 | 0.072 | 0.090 | 0.131 | 0 |
| mixed-rpc | ToplingDB | 4 | 500000 | 165324.73 | 0.000 | 0.072 | 0.091 | 0.130 | 0 |

## Disk Usage

| Backend | Disk usage bytes |
|---|---:|
| RocksDB | 4000002170 |
| ToplingDB | 4000004537 |

Difference:

- ToplingDB uses `2,367` more bytes in this run, which is effectively negligible at the current dataset size.

## Source Reports

- RocksDB report dir: [data/report1](../data/report1)
- ToplingDB report dir: [data/report2](../data/report2)
- RocksDB DB path: `/data4/sui-hotstore-route1-mainnet-270700000-270709999-rocksdb/db-a`
- ToplingDB DB path: `/data4/sui-hotstore-route1-mainnet-270700000-270709999-toplingdb/db-a`

## Observations

- Data equality:
  - All benchmark-facing data column families match by checksum.
  - The only mismatch is `cf_meta`, which differs by `2` value bytes, so total DB checksum does not match byte-for-byte.
- Point lookup:
  - `get-tx`: RocksDB is about `13.7%` faster at its best run.
  - `get-object-version`: ToplingDB is about `6.0%` faster at its best run and has slightly better p99/p999 latency.
  - `get-object-last-seen`: RocksDB is about `8.9%` faster at its best run, with near-identical p95/p99 latency.
- Multi-get:
  - `multi-get-tx`: ToplingDB is about `3.6%` faster and has better p50/p95/p99 latency, while RocksDB has a slightly better p999 latency.
  - `multi-get-object-version`: ToplingDB is about `14.3%` faster and has better p50/p95/p99/p999 latency.
- Prefix scan:
  - `scan-events`: ToplingDB is about `2.1%` faster and slightly better on p50/p95/p99 latency; RocksDB is slightly better at p999.
- Mixed RPC:
  - `mixed-rpc`: RocksDB is about `2.8%` faster at the best-throughput point.
  - Tail latency is effectively tied, with ToplingDB very slightly better at p999 and RocksDB very slightly better at p99.
- Tail latency:
  - Single-key lookups remain effectively sub-millisecond for both backends.
  - The clearest ToplingDB latency win in this run is `multi-get-object-version`.
  - The clearest RocksDB throughput wins in this run are `get-tx` and `get-object-last-seen`.
- Disk footprint:
  - Disk usage is effectively identical in this run.
- Overall:
  - This larger report bundle is mixed rather than uniformly favorable to one backend.
  - ToplingDB is ahead on `get-object-version`, `multi-get-tx`, `multi-get-object-version`, and `scan-events`.
  - RocksDB is ahead on `get-tx`, `get-object-last-seen`, and `mixed-rpc`.
  - ToplingDB's strongest result is `multi-get-object-version`; RocksDB's strongest result is `get-tx`.

## Caveats

- This benchmark uses a bounded Sui dataset, not a full-history archive.
- `object_last_seen` means latest observed within the imported range.
- `owner_touched_objects` is not complete wallet inventory.
- Benchmark results are hardware-specific.
- API benchmark results are intentionally omitted until DB-level results are stable and repeatable.
- The current public benchmark evidence is based on a `10,000` checkpoint route 1 run because the much larger formal-snapshot path still has long local `sui-node` pruning / compaction time before it becomes benchmark-ready.

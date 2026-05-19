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

## Additional Result: 2026-05-18 RocksDB Snappy vs ToplingDB

Detailed report: [summary-2026-05-18-rocksdb-snappy-vs-toplingdb.md](summary-2026-05-18-rocksdb-snappy-vs-toplingdb.md)

This run uses the same logical dataset on both backends, verified by identical entry counts and checksum. The benchmark JSON `dataset` labels were passed incorrectly during the run, so this summary treats `stats.json`, `checksum.json`, and the DB paths as the source of truth.

| Metric | RocksDB snappy | ToplingDB | Result |
|---|---:|---:|---|
| logical entries | 67,204,261 | 67,204,261 | match |
| checksum | `6edbe080a8d3...` | `6edbe080a8d3...` | match |
| disk usage | 6.51 GiB | 3.37 GiB | ToplingDB 48.3% less |
| get-tx peak rps | 2,887,184 | 9,303,347 | ToplingDB 3.22x |
| get-object-version peak rps | 2,478,623 | 7,387,090 | ToplingDB 2.98x |
| get-object-last-seen peak rps | 3,537,405 | 13,571,198 | ToplingDB 3.84x |
| multi-get-tx peak rps | 358,761 | 1,289,705 | ToplingDB 3.59x |
| multi-get-object-version peak rps | 298,336 | 1,160,008 | ToplingDB 3.89x |
| mixed-rpc peak rps | 1,161,423 | 3,115,361 | ToplingDB 2.68x |
| scan-events peak rps | 2,398,263 | 1,722,725 | RocksDB 1.39x |

Compression materially changes the RocksDB footprint: the earlier uncompressed RocksDB run was about `27.49 GiB`, while this snappy-compressed RocksDB run is `6.51 GiB`, a `76.3%` reduction. ToplingDB remains smaller at `3.37 GiB` and remains faster on point lookup, multi-get, and mixed RPC workloads; RocksDB is faster on `scan-events`.

## Dataset

- Source: route 1 bounded-range ingestion from live Sui checkpoint data
- Network: mainnet
- Checkpoint range: `270700000..270759999`
- Imported checkpoints: `60,000`
- Benchmark command shape:
  - Requests per run: `500,000`
  - Warmup requests per run: `50,000`
  - Concurrency sweep: `1,4,8,16,32,64`
  - Access pattern: `uniform`
  - Batch / scan limit: `10`
  - Scan mode: `count`
  - Cache state: `hot`
  - Seed: `6840346605343600653`
- Logical records from the benchmarked DB:
  - `cf_checkpoint`: `60,000`
  - `cf_tx_by_digest`: `530,490`
  - `cf_event_by_type`: `811,079`
  - `cf_object_version`: `2,380,717`
  - `cf_object_last_seen`: `79,125`
  - `cf_owner_touched_objects`: `2,380,717`
  - `cf_meta`: `9`
  - `default`: `0`

## Hardware

- Hostname: `station`
- CPU: `Intel(R) Xeon(R) CPU E5-2682 v4 @ 2.50GHz`
- Memory: `135,056,142,336` bytes
- OS: Linux `x86_64`
- Rust: `rustc 1.90.0 (1159e78c4 2025-09-14)`
- RocksDB report start: `2026-05-11 08:58:34 CST`
- ToplingDB report start: `2026-05-10 23:08:57 CST`
- ToplingDB config: `/data7/osc/sui/crates/typed-store/config/topling_sui.yaml`

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
  - RocksDB: `16e2f4720c874e41630cbb40b3b69439e374d8c8f8ac65ec371380e9e34809d1`
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
  - The `2`-byte metadata difference changes the total DB checksum, while all benchmark-facing data column families match.

## DB-Level Results

| Workload | Backend | Best Concurrency | Requests | Throughput RPS | p50 ms | p95 ms | p99 ms | p999 ms | Errors |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| get-tx | RocksDB | 64 | 500000 | 1,361,160.74 | 0.032713 | 0.095865 | 0.185756 | 0.542569 | 0 |
| get-tx | ToplingDB | 16 | 500000 | 4,505,924.15 | 0.002839 | 0.004405 | 0.006167 | 0.042284 | 0 |
| get-object-version | RocksDB | 64 | 500000 | 810,844.61 | 0.060759 | 0.163501 | 0.270301 | 0.624569 | 0 |
| get-object-version | ToplingDB | 32 | 500000 | 3,663,629.65 | 0.007045 | 0.011840 | 0.029747 | 0.060765 | 0 |
| get-object-last-seen | RocksDB | 64 | 500000 | 2,006,191.20 | 0.021844 | 0.070945 | 0.136572 | 0.348743 | 0 |
| get-object-last-seen | ToplingDB | 16 | 500000 | 4,850,173.58 | 0.002642 | 0.004110 | 0.005675 | 0.032095 | 0 |
| multi-get-tx | RocksDB | 32 | 500000 | 147,620.54 | 0.207511 | 0.298457 | 0.403023 | 0.616534 | 0 |
| multi-get-tx | ToplingDB | 64 | 500000 | 2,145,447.31 | 0.025150 | 0.038165 | 0.072123 | 0.124968 | 0 |
| multi-get-object-version | RocksDB | 32 | 500000 | 80,317.22 | 0.387713 | 0.516063 | 0.627478 | 0.834797 | 0 |
| multi-get-object-version | ToplingDB | 64 | 500000 | 1,224,063.04 | 0.037295 | 0.066563 | 0.097495 | 0.192934 | 0 |
| scan-events | RocksDB | 64 | 500000 | 1,469,092.00 | 0.034659 | 0.075698 | 0.132257 | 0.271179 | 0 |
| scan-events | ToplingDB | 64 | 500000 | 1,583,457.04 | 0.033855 | 0.063404 | 0.103031 | 0.178078 | 0 |
| mixed-rpc | RocksDB | 32 | 500000 | 330,293.65 | 0.035573 | 0.438297 | 0.527992 | 0.691973 | 0 |
| mixed-rpc | ToplingDB | 64 | 500000 | 2,722,918.70 | 0.011939 | 0.059402 | 0.090327 | 0.151685 | 0 |

## Head-to-Head

| Workload | ToplingDB best-throughput advantage | ToplingDB p99 change at best-throughput points |
|---|---:|---:|
| get-tx | `+231.0%` | `-96.7%` |
| get-object-version | `+351.8%` | `-89.0%` |
| get-object-last-seen | `+141.8%` | `-95.8%` |
| multi-get-tx | `+1353.4%` | `-82.1%` |
| multi-get-object-version | `+1424.0%` | `-84.5%` |
| scan-events | `+7.8%` | `-22.1%` |
| mixed-rpc | `+724.4%` | `-82.9%` |

## Disk Usage

| Backend | Disk usage bytes | Approx size |
|---|---:|---:|
| RocksDB | 6,244,820,049 | 5.82 GiB |
| ToplingDB | 1,337,044,105 | 1.25 GiB |

ToplingDB uses about `78.6%` less disk than RocksDB in this report bundle, or roughly `4.7x` less physical space for the same logical benchmark-facing data.

## Source Reports

- RocksDB report dir: [data/report1](../data/report1)
- ToplingDB report dir: [data/report2](../data/report2)
- RocksDB DB path: `/data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb/db-a`
- ToplingDB DB path: `/data4/sui-hotstore-route1-mainnet-270700000-270759999-toplingdb/db-a`

## Observations

- Data equality:
  - The benchmark-facing data column families match by entry count, key bytes, value bytes, and checksum.
  - The only checksum mismatch is `cf_meta`, with a `2`-byte value difference.
- Point lookup:
  - ToplingDB is faster on all three point lookup workloads at the best-throughput point.
  - The strongest point lookup win is `get-object-version`, where ToplingDB is about `4.5x` RocksDB throughput.
- Multi-get:
  - ToplingDB is over an order of magnitude faster on both multi-get workloads.
  - `multi-get-object-version` is the largest win in this report: about `15.2x` RocksDB throughput.
- Prefix scan:
  - `scan-events` is the closest workload.
  - ToplingDB is still ahead by about `7.8%` throughput and has lower p99/p999 latency at the best-throughput point.
- Mixed RPC:
  - ToplingDB is about `8.2x` RocksDB throughput on the mixed workload.
  - This is the clearest end-to-end DB-level serving win in the current report because it combines point lookups, multi-get, and event scan behavior.
- Tail latency:
  - ToplingDB has lower p99 at the selected best-throughput point for every workload in this run.
  - The biggest p99 reductions are on point lookups, where ToplingDB's p99 is roughly one order of magnitude lower.
- Disk footprint:
  - ToplingDB's physical footprint is much smaller in this run despite matching the benchmark-facing logical data.

## Caveats

- This benchmark uses a bounded Sui dataset, not a full-history archive.
- `object_last_seen` means latest observed within the imported range.
- `owner_touched_objects` is not complete wallet inventory.
- Benchmark results are hardware-specific.
- API benchmark results are intentionally omitted until DB-level results are stable and repeatable.
- The RocksDB and ToplingDB binaries were built from different git SHAs in this report bundle; the workload metadata captures those SHAs for traceability.

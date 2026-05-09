# AGENTS.md

## Project

This repository implements Sui HotStore, a ToplingDB-powered local KV serving layer for Sui custom indexers, RPC cache, and archive workloads.

## Priorities

1. Keep the MVP small and runnable.
2. Prefer deterministic tests and fixed synthetic seeds.
3. Keep RocksDB baseline and ToplingDB backend behavior identical at the schema layer.
4. Use bounded checkpoint range semantics. Do not claim full latest-state semantics unless genesis/full-history ingestion is implemented.
5. Add smoke tests and checksum validation for every storage backend.
6. Avoid modifying Sui node source code.
7. Avoid introducing distributed compaction in the MVP.

## Commands

Run formatting:

```bash
cargo fmt --all
```

Run checks:

```bash
cargo check --workspace
```

Run tests:

```bash
cargo test --workspace
```

Run synthetic smoke test:

```bash
./scripts/ingest-synthetic.sh rocksdb small
./scripts/ingest-synthetic.sh toplingdb small
./scripts/compare-checksum.sh
```

## Coding style

- Use `anyhow` for application-level errors.
- Use typed records and explicit key encoding.
- Prefer big-endian numeric key encoding for ordered scans.
- Keep API responses JSON-serializable.
- Avoid hidden global state.
- Keep scripts idempotent where possible.

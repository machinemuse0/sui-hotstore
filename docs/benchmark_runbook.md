# Sui HotStore Benchmark Runbook

## 1. 目标

这份 runbook 面向当前 demo 阶段的 Sui HotStore benchmark，目标是用同一批 Sui 数据、同一套 key files、同一台机器上的同一套参数，对比：

- RocksDB baseline
- ToplingDB backend

当前重点回答下面几个问题：

1. 两个 backend 的写入结果是否一致。
2. 在 DB 级 workload 上，ToplingDB 在 point lookup、multi-get、prefix scan、mixed workload 上是否有优势。
3. p99 / p999 是否稳定。
4. 磁盘占用是否有差异。

当前已实现并可运行的范围：

- 数据一致性验证
  - `hotstore-admin stats`
  - `hotstore-admin checksum`
  - `hotstore-admin compare-checksum`
- DB-level benchmark
  - `get-tx`
  - `get-object-version`
  - `get-object-last-seen`
  - `multi-get-tx`
  - `multi-get-object-version`
  - `scan-events`
  - `mixed-rpc`

当前未纳入正式流程的范围：

- API benchmark
  - k6 mixed API benchmark
  - wrk get_tx benchmark
- 长时间 soak test
- 读写混合 / compaction stress

## 2. 基本原则

1. 先做数据一致性验证，再看性能。
2. Layer 2 和 Layer 3 分开测，不要把 DB benchmark 和 API benchmark 混在一起。
3. RocksDB 和 ToplingDB 必须使用同一批 checkpoint 区间。
4. RocksDB 和 ToplingDB 必须使用同一批 key files。
5. 同一轮对比建议在同一台 Linux 机器上完成。
6. 如果不能清 page cache，就不要宣称自己测的是 cold cache。

## 3. 当前 benchmark 组件

### CLI 和脚本

- 数据导入：
  - [crates/hotstore-sui-source/src/cli.rs](../crates/hotstore-sui-source/src/cli.rs#L9)
- Admin 校验：
  - [crates/hotstore-admin/src/main.rs](../crates/hotstore-admin/src/main.rs#L13)
- DB benchmark：
  - [crates/hotstore-bench/src/main.rs](../crates/hotstore-bench/src/main.rs#L15)
- 一键 DB suite：
  - [scripts/run-benchmark-suite.sh](../scripts/run-benchmark-suite.sh#L1)
- 服务器版 route 1 一键执行：
  - [scripts/run-route1-benchmark-server.sh](../scripts/run-route1-benchmark-server.sh#L1)
- 生成 benchmark keys：
  - [scripts/gen-bench-keys-from-checkpoints.sh](../scripts/gen-bench-keys-from-checkpoints.sh#L1)
- 从现有 DB 抽样生成 benchmark keys：
  - [scripts/gen-bench-keys-from-db.sh](../scripts/gen-bench-keys-from-db.sh#L1)
- 从现有 Sui formal/fullnode DB 生成 benchmark keys：
  - [scripts/gen-bench-keys-from-sui-db.sh](../scripts/gen-bench-keys-from-sui-db.sh#L1)
- Linux ToplingDB demo：
  - [scripts/run-toplingdb-benchmark-demo.sh](../scripts/run-toplingdb-benchmark-demo.sh#L1)
- 自动回填 summary：
  - [scripts/fill-benchmark-summary.sh](../scripts/fill-benchmark-summary.sh#L1)
- 汇总模板：
  - [reports/summary.md](../reports/summary.md#L1)

### 当前 key 文件

当前 `hotstore-bench` 需要以下 4 个文件：

- `tx_digests.txt`
- `object_versions.txt`
- `object_ids.txt`
- `event_types.txt`

格式约定：

- `tx_digests.txt`
  - 每行一个 tx digest
- `object_versions.txt`
  - 每行 `object_id,version`
- `object_ids.txt`
  - 每行一个 object id
- `event_types.txt`
  - 每行一个 event type

## 4. 环境要求

### 正式对比建议

- OS: Linux
- CPU: 8C / 16T 或更高
- Memory: 32GB+
- Disk: NVMe SSD

### demo / 冒烟

- macOS 可以跑 RocksDB baseline
- ToplingDB 正式 benchmark 建议只在 Linux 跑

### 系统信息采集

建议测试前记录：

```bash
mkdir -p reports/sys
date
uname -a
lscpu
free -h
lsblk
df -h
ulimit -n
```

建议将结果保存到：

- `reports/sys/env.txt`
- `reports/sys/lscpu.txt`
- `reports/sys/free.txt`
- `reports/sys/lsblk.txt`
- `reports/sys/df.txt`

### 持续资源采样

正式测试时建议后台记录：

```bash
mkdir -p reports/sys
iostat -xz 1 > reports/sys/iostat.log &
vmstat 1 > reports/sys/vmstat.log &
pidstat -dru 1 > reports/sys/pidstat.log &
```

也可以用仓库内的内存采样脚本直接包住整轮 DB benchmark。它会采样 benchmark 进程树的 RSS / VSZ，并输出 `memory-samples.csv` 和 `memory-summary.json`，后续 summary 可以读取其中的 `peak_rss_bytes`：

```bash
scripts/monitor-benchmark-memory.sh \
  --label rocksdb-snappy \
  --interval 1 \
  --output-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb-snappy/reports/memory \
  -- scripts/run-benchmark-suite.sh \
    --backend rocksdb \
    --db-path /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb-snappy/db-a \
    --keys-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb-snappy/keys \
    --report-dir /data4/sui-hotstore-route1-mainnet-270700000-270759999-rocksdb-snappy/reports \
    --dataset mainnet-270700000-270759999 \
    --requests 1000000 \
    --warmup-requests 100000 \
    --concurrency 1,4,8,16,32,64 \
    --access-pattern uniform \
    --scan-mode count \
    --cache-state hot \
    --min-hit-rate 1.0 \
    --batch-size 10 \
    --cargo-profile release
```

如果 benchmark 已经在运行，也可以监控已有 PID：

```bash
scripts/monitor-benchmark-memory.sh \
  --label rocksdb-running \
  --output-dir reports/memory/rocksdb-running \
  --pid <benchmark-pid>
```

注意：

- 这个脚本统计的是目标进程树的 RSS / VSZ，不包含系统 page cache 的整体变化。
- RocksDB block cache 和 ToplingDB 进程内常驻内存会反映在 RSS 中；mmap 或 page cache 行为仍建议结合 `vmstat` / `pidstat` / `/proc/meminfo` 一起看。

## 5. Backend 切换策略

### RocksDB baseline

使用 `main` 分支或未打 Topling patch 的工作树。

当前默认依赖形状：

```toml
rocksdb = { version = "0.22.0", default-features = false, features = ["multi-threaded-cf"] }
```

### ToplingDB

使用分支：

```bash
git switch codex-toplingdb-cargo-patch
```

这个分支在 [Cargo.toml](../Cargo.toml#L32) 加入了：

```toml
[patch.crates-io]
rocksdb = { git = "https://github.com/topling/rust-toplingdb" }
```

同时要求：

```bash
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/sui/crates/typed-store/config/topling_sui.yaml
```

## 6. 数据集建议

### Demo 级

先使用一个很小的 checkpoint 区间，确保工具链可跑通：

- 网络：`testnet`
- checkpoint range：`331445801..331445803`

### 正式对比级

建议使用更大的 bounded range，例如：

- 最近 1,000 checkpoints
- 最近 10,000 checkpoints
- 固定 60,000 checkpoints，例如当前报告使用的 `270700000..270759999`

注意：

- 当前 ingest 是 bounded-range 语义
- `object_last_seen` 表示“导入区间内最后一次观察到的 object 状态”
- 不能把当前结果宣传成 full-history latest-state archive benchmark

### 当前建议

如果现有机器上已经准备好了完整 benchmark 数据目录，例如已经下载并恢复好了约 200G 的 DB，那么：

1. 不再重复 ingest
2. key 生成优先走“路线 1”：
   - 直接用稳定的公网 RPC 拉一段 checkpoint / tx 元数据
   - 冒烟先试最近 `10,000` 个 checkpoint；正式报告建议固定显式区间，例如 `270700000..270759999`
   - 这是当前最稳的公开 benchmark 路线，因为我们虽然已经用 formal snapshot 拉下来了约 `200G` 数据，但本地 `sui-node` 基于这批数据启动后，pruning / compaction 收敛时间仍然偏长
3. 如果本地 formal/fullnode 节点能稳定提供历史 checkpoint，再考虑“路线 2”：
   - 复用现有 Sui DB
   - 通过本地 RPC 抽样
   - 这条路保留，并且后续会继续推进到更大数据集测试
4. 对于当前已实现的 benchmark 工具链：
   - key 生成可以直接走 checkpoint RPC
   - `run-benchmark-suite.sh` 仍然面向 HotStore schema 的 DB-level workloads
   - 直接针对 Sui fullnode 内部 DB 的 DB-level adapter 需要单独实现

## 7. Benchmark 分层计划

### Group A: 数据一致性验证

目标：

- 验证两次 ingest 或两个 backend 的逻辑结果是否一致

执行项：

1. `stats`
2. `checksum`
3. `compare-checksum`

产物：

- `stats.json`
- `checksum/checksum.json`
- `checksum/compare-checksum.json`

### Group B: 单点读取 DB benchmark

执行项：

- `get-tx`
- `get-object-version`
- `get-object-last-seen`

目标：

- 看 point lookup throughput
- 看 p50/p95/p99/p999

### Group C: MultiGet DB benchmark

执行项：

- `multi-get-tx`
- `multi-get-object-version`

目标：

- 看批量读取对两个 backend 的放大差异

### Group D: Prefix Scan DB benchmark

执行项：

- `scan-events`

目标：

- 看 event type prefix scan 的延迟和返回记录数

### Group E: Mixed DB benchmark

执行项：

- `mixed-rpc`

当前固定配比：

- 50% `get-tx`
- 20% `get-object-version`
- 15% `multi-get-object-version`
- 10% `scan-events`
- 5% `get-object-last-seen`

目标：

- 模拟更接近真实 KV serving path 的混合读负载

### Group F: API benchmark

当前状态：

- 暂未纳入正式流程

后续计划：

- k6 mixed JSON-RPC benchmark
- wrk single-endpoint benchmark

## 8. 标准执行流程

### Flow A: macOS 上跑 RocksDB 小量测试

1. 准备环境：

```bash
cd /path/to/sui-hotstore
export HTTPS_PROXY=http://127.0.0.1:7897
export https_proxy=http://127.0.0.1:7897
export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
export DYLD_FALLBACK_LIBRARY_PATH=/Library/Developer/CommandLineTools/usr/lib
```

2. 检查 workspace：

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
```

3. 导入两份同区间 RocksDB 数据：

```bash
cargo run --bin sui-hotstore-ingest-real -- \
  --network testnet \
  --remote-store-url https://checkpoints.testnet.sui.io \
  --first-checkpoint 331445801 \
  --last-checkpoint 331445803 \
  --backend rocksdb \
  --db-path /private/tmp/sui-hotstore-rocksdb-a \
  --record-mode lite \
  --checkpoint-batch-size 2 \
  --tx-batch-size 25

cargo run --bin sui-hotstore-ingest-real -- \
  --network testnet \
  --remote-store-url https://checkpoints.testnet.sui.io \
  --first-checkpoint 331445801 \
  --last-checkpoint 331445803 \
  --backend rocksdb \
  --db-path /private/tmp/sui-hotstore-rocksdb-b \
  --record-mode lite \
  --checkpoint-batch-size 2 \
  --tx-batch-size 25
```

4. 生成 keys：

```bash
scripts/gen-bench-keys-from-checkpoints.sh \
  --network testnet \
  --first-checkpoint 331445801 \
  --last-checkpoint 331445803 \
  --out-dir /private/tmp/hotstore-bench-keys-testnet-331445801-331445803 \
  --tx-batch-size 25
```

5. 生成第二份 DB 的 checksum：

```bash
cargo run --bin hotstore-admin -- \
  checksum \
  --backend rocksdb \
  --db-path /private/tmp/sui-hotstore-rocksdb-b \
  --output /private/tmp/sui-hotstore-rocksdb-b-checksum.json
```

6. 跑完整 suite：

```bash
scripts/run-benchmark-suite.sh \
  --backend rocksdb \
  --db-path /private/tmp/sui-hotstore-rocksdb-a \
  --keys-dir /private/tmp/hotstore-bench-keys-testnet-331445801-331445803 \
  --report-dir /private/tmp/hotstore-reports-rocksdb \
  --requests 2000 \
  --concurrency 1,2,4 \
  --batch-size 10 \
  --cargo-profile dev \
  --compare-checksum-with /private/tmp/sui-hotstore-rocksdb-b-checksum.json
```

### Flow B: Linux 上跑 ToplingDB demo

1. 切到 ToplingDB 分支：

```bash
git switch codex-toplingdb-cargo-patch
```

2. 设置 Topling 配置：

```bash
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/sui/crates/typed-store/config/topling_sui.yaml
```

3. 如有代理，补充：

```bash
export HTTPS_PROXY=http://127.0.0.1:7897
export https_proxy=http://127.0.0.1:7897
```

4. 直接运行 demo 脚本：

```bash
scripts/run-toplingdb-benchmark-demo.sh
```

可选参数示例：

```bash
scripts/run-toplingdb-benchmark-demo.sh \
  --network testnet \
  --first-checkpoint 331445801 \
  --last-checkpoint 331445803 \
  --base-dir /tmp/toplingdb-demo \
  --cargo-profile dev \
  --requests 2000 \
  --concurrency 1,2,4 \
  --batch-size 10
```

这个脚本会完成：

1. `cargo check --workspace`
2. ingest `db-a`
3. ingest `db-b`
4. 生成 keys
5. 对 `db-b` 做 checksum
6. 对 `db-a` 跑完整 DB suite
7. 自动 compare-checksum

### Flow C: Linux 上做正式对比

同一台 Linux 机器建议分成两轮：

#### Run 1: RocksDB

```bash
git switch main
cargo check --workspace
```

然后使用与 Topling 完全一致的：

- network
- checkpoint range
- keys
- requests
- concurrency
- batch size

跑一轮 RocksDB suite。

#### Run 2: ToplingDB

```bash
git switch codex-toplingdb-cargo-patch
export TOPLINGDB_EASY_MIGRATE_CONF=/path/to/sui/crates/typed-store/config/topling_sui.yaml
cargo check --workspace
```

然后用同一套参数再跑一轮 ToplingDB suite。

### Flow D: 已有现成 DB 时的推荐路径

如果现有 Linux 机器上已经有可用的 RocksDB / ToplingDB 数据目录，推荐流程如下：

1. 不再重复导入数据
2. 从现有 DB 直接抽样生成 keys
3. 对现有 DB 做 checksum / compare-checksum
4. 跑 `release` 版 benchmark suite

#### 从现有 RocksDB 抽 keys

```bash
scripts/gen-bench-keys-from-db.sh \
  --backend rocksdb \
  --db-path /data/sui-hotstore-rocksdb \
  --out-dir /data/bench-keys-rocksdb \
  --tx-limit 100000 \
  --object-version-limit 100000 \
  --object-id-limit 100000 \
  --event-type-limit 1000 \
  --cargo-profile release
```

#### 从现有 ToplingDB 抽 keys

```bash
scripts/gen-bench-keys-from-db.sh \
  --backend toplingdb \
  --db-path /data/sui-hotstore-toplingdb \
  --out-dir /data/bench-keys-toplingdb \
  --tx-limit 100000 \
  --object-version-limit 100000 \
  --object-id-limit 100000 \
  --event-type-limit 1000 \
  --cargo-profile release
```

如果两边的数据目录来自同一批数据，推荐只生成一份 keys，后续两边共用。

#### 路线 1：直接从 checkpoint RPC 生成 keys

这是当前 demo 阶段最稳的一条路，尤其适合：

- 本地 fullnode 还在做 compaction / pruning
- formal snapshot 节点无法稳定返回旧 checkpoint
- 我们只是需要一批可复用的 benchmark keys

冒烟可以直接抓最近 `10,000` 个 mainnet checkpoints：

```bash
HTTPS_PROXY=http://127.0.0.1:7897 \
https_proxy=http://127.0.0.1:7897 \
bash scripts/gen-bench-keys-from-checkpoints.sh \
  --network mainnet \
  --rpc-url https://fullnode.mainnet.sui.io:443 \
  --latest-count 10000 \
  --out-dir /data/bench-keys-mainnet-latest-10000 \
  --tx-batch-size 100 \
  --rpc-timeout-secs 30
```

如果不需要代理，去掉前两行环境变量即可。

如果希望在服务器上把 ingest、keys、stats/checksum、DB benchmark 一次跑完，可以直接用：

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
  --batch-size 10
```

#### 路线 2：从现有 Sui formal DB 抽 keys

如果现有目录不是 HotStore 自己的 schema，而是 Sui fullnode / formal snapshot 恢复出来的目录，例如：

```text
/data/sui/mainnet-formal
```

推荐直接复用该目录启动本地 fullnode，然后通过本地 RPC 抽样：

```bash
bash scripts/gen-bench-keys-from-sui-db.sh \
  --db-root /data/sui/mainnet-formal \
  --network mainnet \
  --first-checkpoint 100 \
  --last-checkpoint 300 \
  --out-dir /data/bench-keys-mainnet \
  --start-node
```

如果节点已经在本地启动，也可以直接复用 RPC：

```bash
bash scripts/gen-bench-keys-from-sui-db.sh \
  --db-root /data/sui/mainnet-formal \
  --network mainnet \
  --first-checkpoint 100 \
  --last-checkpoint 300 \
  --rpc-url http://127.0.0.1:9000 \
  --out-dir /data/bench-keys-mainnet
```

如果本地 formal/fullnode 节点对历史 checkpoint 查询不稳定，可以把“本地 DB”和“抽 key 用的 RPC”拆开：

```bash
bash scripts/gen-bench-keys-from-sui-db.sh \
  --db-root /data/sui/mainnet-formal \
  --network mainnet \
  --first-checkpoint 100 \
  --last-checkpoint 300 \
  --rpc-url http://127.0.0.1:9000 \
  --key-rpc-url https://fullnode.mainnet.sui.io:443 \
  --out-dir /data/bench-keys-mainnet
```

注意：

- 这个脚本解决的是“**已有 Sui DB，如何直接抽样生成 benchmark keys**”
- 它不要求重新 ingest
- 但当前 [scripts/run-benchmark-suite.sh](../scripts/run-benchmark-suite.sh#L1) 仍然是针对 HotStore schema 的 DB-level benchmark
- 如果要**直接 benchmark Sui fullnode 内部 DB**，需要单独的 Sui-native DB adapter

#### 正式 benchmark 建议使用 release

```bash
scripts/run-benchmark-suite.sh \
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

同理再跑一轮 ToplingDB。

## 9. 结果目录建议

建议按 backend 和轮次归档：

```text
reports/
  rocksdb-run1/
    stats.json
    checksum/
    db/
  toplingdb-run1/
    stats.json
    checksum/
    db/
  sys/
    iostat.log
    vmstat.log
    pidstat.log
```

同时将摘要填入：

- [reports/summary.md](../reports/summary.md#L1)

自动回填命令：

```bash
scripts/fill-benchmark-summary.sh \
  --rocksdb-report-dir /tmp/hotstore-reports-rocksdb \
  --toplingdb-report-dir /tmp/hotstore-reports-toplingdb \
  --keys-manifest /tmp/hotstore-bench-keys/manifest.json \
  --pick-concurrency max \
  --output reports/summary.md
```

## 10. 对外汇报建议

推荐汇报结构：

1. 数据集范围
2. 硬件和系统参数
3. backend 切换方式
4. 数据一致性结果
5. DB benchmark 核心表格
6. tail latency 观察
7. 磁盘占用
8. caveats

建议重点展示这些 workload：

- `get-tx`
- `get-object-version`
- `multi-get-object-version`
- `scan-events`
- `mixed-rpc`

## 11. Caveats

1. 当前结果是 bounded dataset benchmark，不是 full-history archive benchmark。
2. 当前 ingest 主要走 RPC，不是 checkpoint 文件全量回放。
3. `object_last_seen` 只有导入区间内语义。
4. API benchmark 尚未纳入正式流程，因此当前结论仅覆盖 DB-level serving path。
5. 如果没有 root 清 cache，不要把结果标注为 cold cache。

## 12. 下一步扩展

当 DB benchmark 稳定后，建议按这个顺序继续：

1. 补 API benchmark
   - k6 mixed JSON-RPC
   - wrk single endpoint
2. 增加更大的 checkpoint 区间
3. 增加 hot/cold key 集合
4. 增加长时间稳定性测试
5. 增加 compaction / write amplification 观察

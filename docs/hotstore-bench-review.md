# hotstore-bench Review & Optimization Plan

完整 review 了 `crates/hotstore-bench`（外加它依赖的 `hotstore-db` 后端），按"严重度 / 类别"排出问题和优化建议。重点是三类：**测量方法不正确导致结果有偏**、**框架自身开销污染了被测路径**、**功能缺失让对比不严谨**。

---

## 一、Methodology bugs（会让数据失真，优先修）

### 1. `multi_get` 是假的 multi-get —— 最严重的一个

`crates/hotstore-db/src/rocksdb_backend.rs:47-49`

```rust
fn multi_get(&self, cf: ColumnFamily, keys: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>> {
    keys.iter().map(|key| self.get(cf, key)).collect()
}
```

退化成了 N 次独立 `get_cf`，没有走 RocksDB 的 `multi_get_cf` / `batched_multi_get_cf`。`multi-get-tx`、`multi-get-object-version`、以及 mixed-rpc 里的 multi-get 测出来都不是真正的 batch 性能（少了共享 LSM 路径、bloom filter 复用、I/O 并行化）。

**修法**：调用 `db.batched_multi_get_cf(&handle, keys, /*sorted=*/ false)`，对 ToplingDB 同样下沉到 native 接口。配合 trait 把签名改成 `&[&[u8]]` 避免一次额外的 `Vec<Vec<u8>>` 拷贝（见第 4 点）。预计 batch=50、并发 16+ 时 throughput 提升 3-8x，更接近真实 multi-get 上限。

### 2. 没有 warmup —— 冷启动伪影计入正式样本

`crates/hotstore-bench/src/workloads.rs:117` `started_at = Instant::now()` 之后立刻进入测量。RocksDB 的 block cache、文件句柄、OS page cache 都是冷的，前几千个请求的 latency 显著偏高，特别影响 p99/p999。

**修法**：在 `run_single_benchmark` 里先跑 `requests / 10`（或 `--warmup` 参数）丢弃样本，再 reset 计时器。也可以把 warmup 挪到打开后端之后、第一次 concurrency run 之前做一次全局热身。

### 3. Key 访问模式只支持线性 —— 不代表真实 RPC 流量

`crates/hotstore-bench/src/workloads.rs:387` `keys[index % keys.len()]`：

- worker 之间通过 `worker_start_index` 切片成连续区间，多线程几乎不重叠 → 跟真实 fanout 读完全不同。
- 线性扫描制造了 LSM block 复用，把 cache hit rate 人工拉高，吞吐被高估。
- p99 也会被低估（少了 random 跨 SST 命中）。

**修法**：加 `--access-pattern {sequential, uniform, zipfian}` + `--seed`，random 用 `SmallRng` per worker（避免共享 RNG 锁）。Zipfian (theta≈0.99) 是 YCSB 的事实标准。

### 4. 每个请求都 `clone()` key —— 框架开销混进了被测时间

多处 `.clone()` 在 `Instant::now()` 之后：

- `crates/hotstore-bench/src/workloads.rs:387` `keys[index % keys.len()].clone()`
- `crates/hotstore-bench/src/workloads.rs:398` batch 整段 clone
- `crates/hotstore-bench/src/workloads.rs:501-522` mixed-rpc 同样 clone

对 32B 的 key 也许尚可忍受，但 multi-get 的 50 个 key 每次都新分配一段 `Vec<Vec<u8>>`，加上 trait 强制 `&[Vec<u8>]` 的签名导致后端也无法 zero-copy。

**修法**：

- `WorkloadData` 内部用单一 arena `Vec<u8>` + `Vec<Range<u32>>`，请求时取出 `&[u8]`。
- trait 改成 `multi_get(&self, cf, keys: &[&[u8]])`，所有调用点借引用而不是 own buffer。
- 计时只覆盖 engine 调用本身（`key_at` 在外面预先准备好），更纯地测存储路径。

### 5. Closed-loop benchmark 会低估 tail latency（coordinated omission）

worker 是"发起 → 等返回 → 立刻发起下一条"的串行闭环。一个慢请求会推迟后面所有请求的"发起时间"，所以 p99/p999 报出来的是"被压住后的"而不是"用户视角的"延迟。这是 wrk2 / Gil Tene 反复强调的问题。

**修法**：

- 加 `--rate <rps>` 模式：用一个生成线程按预定 rate 把 `expected_start_time = t0 + i/rate` 喂进 channel，worker 从 channel 拉，latency 用 `now - expected_start_time` 而不是 `now - actually_started`。
- 闭环模式保留作为 "max throughput" 模式，但要在报告里明确标注。

### 6. 没有 compaction / cache state 控制

两次 run 之间的 LSM 形态、block cache 状态都不一样，跑不同 workload 的顺序会影响后面的结果。`run-benchmark-suite.sh` 里 9 个 workload 串行跑，第 9 个的 cache 跟第 1 个完全不同。

**修法**：开 bench 前 `db.compact_range_cf(handle, None, None)` 让 LSM 收敛到稳定形态；可选 `--drop-caches`（Linux 上 `posix_fadvise(DONTNEED)` 或 sudo `echo 3 > drop_caches`）作为"冷"基线。报告里记录"hot"/"cold"模式。

---

## 二、统计与可重复性

### 7. 用 HDR Histogram 替换全量 sort

现在每个 worker 攒 `Vec<u64>`（100k req × 64 worker = 6.4M u64 ≈ 50MB），main 线程再 `extend + sort_unstable`。sort 不算长，但：

- 内存压力大；
- p999 在 100k 样本下只有 100 个点支撑，噪声很大；
- 多 run 合并不可累加。

**修法**：用 `hdrhistogram` crate（固定内存、O(1) 插入、可合并、tail 精度更高），worker 各自 record，最后 merge。报告里直接 dump quantile distribution，必要时存压缩二进制 histogram 给后处理。

### 8. 报告里没有 environment fingerprint

`crates/hotstore-bench/src/report.rs:8-17` 只有 backend 名 + db_path。两台机器跑出来的 JSON 没法对比。

**修法**：在 `BenchmarkSuiteReport` 里加：

- 主机名、CPU model（`/proc/cpuinfo` 第一条）、核数、内存大小
- kernel 版本、文件系统、挂载选项（noatime?）
- RocksDB 版本（编译期 `env!` 或读 `librocksdb-sys` 的版本常量）、ToplingDB config 路径
- 数据集 checksum（已经有 `hotstore-admin checksum`，引用 hash 即可）
- bench 二进制 git sha（通过 `build.rs` 注入）
- ulimit、`vm.swappiness`、`vm.dirty_ratio` 这些读基线相关项

### 9. Errors 被静默吞掉

`crates/hotstore-bench/src/workloads.rs:258-260` `Err(_) => result.errors += 1;` 把所有错误归一类。如果出现是 IO 错误、key 编码错误、DB 损坏完全看不出来。

**修法**：错误也分类计数（按 `anyhow::Error` 的 root cause downcast 或加 enum），首条错误打印到 stderr，限制总量避免日志炸开。

### 10. Concurrency 切分会让小 run 数据稀疏

`base_requests = requests / concurrency; extra_requests = requests % concurrency`。当 `requests=100, concurrency=64` 时大部分 worker 只跑 1-2 个请求，每个 worker 自己的 latency vec 只有几条样本，最后合在一起虽然总样本够，但 throughput 计算的"启动同步成本"占比变高。

**修法**：检查 `requests / concurrency >= some_min`（例如 1000），不满足就警告或自动放大；或者改用基于时间 (`--duration 30s`) 而不是 request 计数。

---

## 三、可比性 / 配置缺失

### 11. RocksDB / ToplingDB 调优开关没暴露

`crates/hotstore-db/src/rocksdb_backend.rs:17-19` 全用默认 `Options`：默认 64MB block cache、无 bloom filter（取决于 rust-rocksdb 版本）、默认 write buffer。这对 read benchmark 影响巨大。两个后端的"默认"也不必相等。

**修法**：bench 不必内嵌调优，但应当 (a) 在报告中 dump `db.get_options_string()` / column-family 的实际生效配置，(b) 提供一个 `--rocksdb-options-file` 选项加载 INI，让 RocksDB vs ToplingDB 跑在等价配置下。

### 12. `scan_prefix` 强制 `Vec<(Vec<u8>, Vec<u8>)>` 物化

`crates/hotstore-db/src/rocksdb_backend.rs:71-97` 把每个 row 的 key+value 都 `to_vec()`。scan-events workload 的 latency 包含了这一段拷贝，不是纯迭代器开销。

**修法**：trait 加 `scan_prefix_count(prefix, limit) -> usize` 或 callback / iterator 形式 `scan_prefix_visit(prefix, limit, &mut FnMut(&[u8], &[u8]))`，bench 走 visitor 路径只算 row 数和总 byte 数。再加一个"用户路径模拟"workload 显式包含 deserialize 成本，二者分开报告。

### 13. Mixed workload 比例硬编码

`crates/hotstore-bench/src/workloads.rs:500-523` `match index % 100 { 0..=49 => GetTx, 50..=69 => ... }`。任何想换一种 "RPC mix" 的人都要改代码重编。

**修法**：CLI 接受 `--mix get-tx=50,get-object-version=20,multi-get-object-version=15,scan-events=10,get-object-last-seen=5` 解析成 weighted dispatcher。

### 14. `WorkloadData::clone()` per worker

`crates/hotstore-bench/src/workloads.rs:137` 整个 keys 容器（实测 100k 条 × 64B ≈ 6MB；mixed 翻 4 倍）每个 worker 复制一份。在 sweep `1,4,8,16,32,64` 时同样的数据复制 6 次。

**修法**：`Arc<WorkloadData>`，`run_worker` 拿 `&WorkloadData` 即可。

### 15. 没有 "no-op baseline" workload

没法分离 framework 开销和 storage 开销。新功能引入后，如果发现 throughput 降了，难以判断是 bench 框架变重了还是后端变慢了。

**修法**：加 `--workload noop`：不调 engine，只走时序记录路径，作为 floor。

### 16. 时间单位 / 精度

`Instant::elapsed().as_micros() as u64` —— 微秒精度对 sub-microsecond 的 cache hit 不够。`as_nanos() as u64` 即可，HDR histogram 也支持纳秒桶。

---

## 四、建议的优先修复顺序

| 优先级 | 改动 | 收益 |
|---|---|---|
| P0 | 实现真正的 `multi_get_cf` (#1) | multi-get 数字立即正确，可能是最大单点改进 |
| P0 | 加 warmup + key 随机访问模式 + seed (#2, #3) | 数据可信度 |
| P0 | 解决 coordinated omission，open-loop 模式 (#5) | tail latency 不再被掩盖 |
| P1 | `Arc<WorkloadData>`、key arena、trait 改 `&[&[u8]]` (#4, #14) | 减少 framework 噪声 |
| P1 | HDR histogram (#7) + environment fingerprint (#8) | 报告更可比 |
| P1 | 错误分类 + 最小 per-worker 校验 (#9, #10) | 防止隐藏故障 |
| P2 | 暴露 RocksDB options 日志、`scan_prefix_visit`、可配置 mix、noop baseline (#11, #12, #13, #15) | 灵活性 / 可对比性 |
| P2 | 纳秒精度、duration-based stop (#16) | 长尾分析 |

---

## 五、可顺手做的小事

- `crates/hotstore-bench/src/report.rs:46` 报告里加 `started_at_unix`、`finished_at_unix`，方便和 metrics 对齐。
- `crates/hotstore-bench/src/workloads.rs:175` `elapsed_secs.max(0.000_001)` 实际请求 0 已经在前面 bail 了；这个 saturate 没有真实场景，可移除。
- `crates/hotstore-bench/src/workloads.rs:152` `Vec::with_capacity(requests)` 在 main 线程上为合并准备的，但你已经知道每个 worker 的精确 size，可以预先 `reserve_exact` 减少 reallocation。
- 现有 `parse_concurrency_list` 没去重，传 `1,1,4` 会跑两次 1。加 `dedup` + 排序更友好。

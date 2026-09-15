# Profiling and external benchmarks

The optional `profiling` feature implements the memory snapshots, safe partial
allocation trace, lifecycle timing and benchmark workflow proposed in
[the original design report](performance/README.md). Diagnostics
are off by default. This is an observability baseline, not a CPU/call-stack
sampler or a claim of feature/performance parity with QuickJS.

Historical measurements and validation evidence:
[PocketLab baseline](performance/README.md) and
[CPU hotspot investigation](performance/README.md). Each report applies to
its recorded source and build; its optimization ordering is not a current
backlog. The [stack VM plan](primitive-vm-plan.md) selects this PR's goals
from [issue #16's post-PR19 investigation](https://github.com/pocket-stack/quickjs-oxide/issues/16#issuecomment-5634660983).
The stack VM migration is in progress; these links do not claim a new benchmark run.

## Build and run

```sh
cargo build --locked --release -p quickjs-oxide-cli --features profiling
./target/release/qjs -d workload.js
./target/release/qjs -T workload.js
./target/release/qjs -q -d
./target/release/qjs -dT --profile-json --profile-output /tmp/new-profile.jsonl workload.js
```

`-d/--dump` requests a snapshot and compile/VM cost collection; `-T/--trace` requests allocation observation.
`-q -d` also takes 100 independent lifecycle samples. `--profile-iterations N`
changes that count (1–10000). Trace holds at most 65536 events by default;
`--profile-events N` changes the limit (0–1000000). Its buffer is allocated once
before runtime creation, never grows while collecting, and counts dropped
events. Dropping the trace handle does not stop the runtime's collector.

Reports go to stderr, preserving script stdout. `--profile-output PATH` creates
a **new** file and refuses existing paths, including the input script. JSON
output is one complete object per line: `oxide-memory-v1`,
`oxide-allocation-trace-v1`, `oxide-compile-vm-cost-v1` when compilation or
execution occurred, and, for `-q -d`, `oxide-lifecycle-v1`. Multiple
records can share the output. Human-readable mode includes accounting notes
and raw lifecycle samples. File-open errors are CLI errors before execution;
subsequent write errors mark the report incomplete on stderr while preserving
the script's result, exit code and original exception.

Snapshots happen before Context destruction, after ordinary pending jobs on
success. Early execution errors still produce an `error-before-context-drop`
snapshot, without advancing jobs. `-q` snapshots are labelled
`initialized-before-context-drop`. Argument/input-file errors before runtime
construction produce no snapshot. The collector never runs getters, Proxy
traps, jobs or an extra GC, and does not retain JavaScript roots. Deferred
releases are observed as they stand, without draining them for a snapshot.
Trace serialization happens after Context and Runtime teardown.

## Data contract and coverage

| Data | Included | Unavailable / interpretation |
| --- | --- | --- |
| Heap population | Object, shape, variable-reference, Context and bytecode node counts; lifecycle states; pending jobs | Logical node counts are not allocation counts or byte totals |
| Owned storage | Arena slots/free indices/zero queue, object property slots, dense array elements, ordinary ArrayBuffer bytes | `used_bytes` measures initialized inline storage and `capacity_bytes` its reserved capacity; nested allocations and allocator headers are excluded |
| Bytecode | Unique instruction slices, deduplicated by their shared storage identity | Rc headers, nested operands, constants and debug data are excluded |
| Static property keys | `bytecode_property_keys`: linked-name count and deduplicated constant-indexed Atom slice bytes, including unused slots | Rc headers and the separate owning references in `auxiliary_atoms` are excluded; no table is allocated for functions without static names |
| Atoms and strings | Live table-backed atom count; immediate integers excluded | Full string storage/counts are unavailable because strings can be shared with atoms, bytecode and embedder values |
| Shared buffers | SharedArrayBuffer wrapper count | Shared backing bytes are unavailable, avoiding duplication across wrappers/contexts/runtimes |
| Allocation events | Actual arena Vec backing-storage allocation, growth and release; stable storage identity; sequence; old/new capacity bytes | `coverage=partial`, `scope=arena-slots-backing-storage`; successful safe Vec capacity transitions only. An `R` does not prove a libc `realloc` call, nor physical relocation |
| Lifecycle | Runtime create, Context create, Context drop, Runtime drop; every raw sample and each phase minimum | Monotonic wall nanoseconds. Excludes process startup and the construction of the host-services value. The sum of independent minima may not correspond to one iteration |

The arena's inline bytes include its record storage. Logical-only node
categories must not be converted to extra inline bytes and added again.
ArrayBuffer views/aliases do not count their backing bytes again; each ordinary
ArrayBuffer owning Vec is counted once. Other object payloads, shape lookup
maps, BigInts, strings, code metadata and host allocations are outside byte
coverage. **The sum of reported categories is not total runtime memory.**
Missing values are `null`, never zero. Total allocator requested/usable bytes,
allocation failures, peak/RSS and cumulative process allocation are explicitly
unavailable. `-T` is not a logical-object trace or a function execution trace.

The safe allocation boundary deliberately preserves `unsafe_code = "forbid"`.
No global allocator replacement is installed. The arena wrapper exposes slice
access, so all its capacity changes pass through its instrumented push, and its
backing allocation is released before the final `F`. A storage ID is scoped to
a runtime and remains stable across growth. Physical addresses are not output.
A trace reports `finished`, `dropped_events`, and `complete_within_scope`; partial
resource coverage remains partial even when no events were dropped. Allocation
requests that abort the process cannot produce a final report.

The Rust API is `Runtime::memory_snapshot()` and
`Runtime::new_with_allocation_trace(host, max_events)`. The latter returns a
runtime and a diagnostic-only `AllocationTrace`. Keep the trace handle, drop
all Context/Value/Runtime handles, then call `trace.snapshot()` for teardown
records. Ordinary runtime construction in a profiling build does not allocate
a trace buffer. Builds without the feature compile out the collector and
reject profiling flags with an explanatory error.

## Compile and VM cost diagnostics

`-d --profile-json` reuses the same CLI and benchmark workload entry. Its
`oxide-compile-vm-cost-v1` record describes the **legacy** execution path:
parse/resolution/lowering, blocks, fusion, relocation, verify and publish attempts
and inclusive/exclusive monotonic wall nanoseconds,
successfully lowered function drafts (including children), final instruction
count and inline typed-code bytes, maximum verified stack, dynamic dispatches,
successful PC publications, and operand depth observed at dispatch boundaries.
Failed parses still count as attempts; lowered drafts are not published-code
or unique-code counts. Inline code bytes exclude boxed operands and metadata.
Inclusive phase time includes nested compilation and callbacks and is not
additive. `exclusive_ns` subtracts measured child phases in the same collector;
uninstrumented work and nested collectors remain charged to their parent.
Verify covers draft authentication; publish covers flattening, linking and heap
publication (including the heap boundary's independent checks). Blocks covers
compiler block discovery; fusion covers the owned execution projection.
Relocation covers IR fragment moves and target-bearing lowered instructions,
not the entire lowering/emission loop. Fine-grained diagnostic timers introduce
overhead and must not be compared against ordinary compile-only timings.
Each phase also records the number of storage snapshots and the
largest observed partial owned-Vec capacity: the function arena, operations,
constants, bindings, scopes, local/parameter descriptors, closure variables,
eval environments, and each scope’s binding-index vector. These are boundary
samples (parse completion, resolution entry/exit, lowering entry), excluding
referenced payloads, source, hash tables, and transient worklists. They are not
continuous allocation tracking or a compiler peak-memory total. This instrumented build is not a formal throughput measurement.

The Rust entry is `CostProfile::start()` in `engine::api::profiling`, followed
by `snapshot()`. Collection covers an interval on the calling thread, across
runtimes; it is not a per-Runtime total. The innermost live profile receives
events, and dropping it restores the outer collector, including on unwinding.
The collector owns no Runtime or JavaScript values. Owned-core storage counters
are described below; total call allocations, total retain/release activity and
compiler peak memory remain unavailable.
Without the `profiling` feature, compiler and interpreter hooks are compiled out.

## Disable and measure overhead

Remove `-d/-T` to disable collection. For a binary with the feature compiled out:

```sh
cargo build --locked --release -p quickjs-oxide-cli --no-default-features \
  --target-dir target/plain
./target/plain/release/qjs workload.js
```

Cargo features are additive; avoid other dependencies explicitly enabling
`profiling`. Do not assume a compiled-but-inactive collector is free. The
experiment runner measures four modes (feature absent, compiled inactive,
dump, trace), rotating their order and checking identical stdout. It reports
raw full-process wall times including formatting/I/O, not a fabricated isolated
VM overhead percentage.

## Benchmark without vendoring workloads

Follow [the benchmark tool instructions](../scripts/benchmark/README.md).
The current workflow supports both the pinned QuickJS `tests/microbench.js` and
an **external** checkout of
[ahaoboy/js-engine-benchmark](https://github.com/ahaoboy/js-engine-benchmark).
Only our orchestration, parsers, tests, documentation and result summaries live
in this repository. Third-party benchmark source and generated bundles stay
outside it. No complete QuickJS `std`/`os` module implementation is required.

For this work, run release builds, broader tests, profiler experiments and
benchmarks in the `eric-83am` Herdr PocketLab workspace, under
`/home/eric/Documents/Sources/PocketLab/quickjs-oxide`. The development computer
is limited to minimal checks/tests. Keep timing runs serial and separate from
compilation and correctness tests on PocketLab.

Correctness remains independent of performance. A timeout, unsupported case,
missing score, swallowed benchmark error or partial result cannot become a
zero-time result or contribute to a speed ratio. Keep raw logs to distinguish
those cases, and use unchanged workloads and the same clock/harness on both
engines. QuickJS lifecycle CPU times must not be divided by Oxide wall times.

### 最终码与指令契约

`profiling` feature 下，调用 `CostProfile::capture_disassembly()` 可为该作用域随后成功完成的 lowering 捕获逐函数反汇编。`snapshot().code_disassembly` 按 lowering 完成顺序保存文本；每行包括最终 PC、指令与同一 `InstructionInfo` 的栈状态、控制流、操作数和潜在效果。默认是 `None`，重复启用不清空已有记录。该选项用于诊断，不能用于正式计时；文本不持有 Runtime roots 或原始 IR。

潜在回调/分配效果是通用语义的保守上界，不能据此断言每次 Number 运算都会调用 JS 或分配。可捕获 JS 异常与引擎分配/不变量错误分开；catch、iterator、gosub 和 resume 的动态验证不会被 nominal 栈数量代替。

The non-default `stack-vm` migration configuration adds `owned_instructions`,
`owned_bridge_exits`, `owned_sync_call_bridges`, and `owned_max_operand_depth`
to the same cost snapshot.
An owned instruction is counted after its step commits (a call commits when its
child frame is installed, before the callee returns); a bridge exit is counted
separately and resumes the untouched opcode in the previous VM. CLI reports use
`owned-stack-with-legacy-bridge` when an owned counter is nonzero. This is partial
coverage, not a claim that a whole sample ran in the new core.

`owned_sync_call_bridges` counts selected owned calls and unresolved domain
steps dispatched through synchronous Runtime entries. S05 synchronous native
families now register typed domain continuations or explicitly audited NoJs
leaves. Their property, conversion, iterator and callback requests stay in the
owned driver; old synchronous consumers use the same domain steps. Promise,
generator, module and host/API entries still have the S06/S07 migration work
listed in [the callback ledger](primitive-vm-sync-callbacks.md).
The counters describe the measured interval, not every possible path of an
intrinsic. A coverage claim requires all three legacy/bridge counters to be zero
and a source audit of the selected native leaves. PendingCall remains a counted
internal migration boundary; it is not a host delimiter.
Temporary request/continuation Box allocations and Proxy operation state storage
are outside `call_preparation` coverage.

The current S08/S09 run keeps only resume PC local; fault PC is written directly
into Frame at each actual dispatch entry. `owned_execution_events` separates:

| Counter | Current meaning |
| --- | --- |
| `run_frame_fault_pc_write` | Source-level Frame fault assignment at each actual run dispatch entry, including entries that subsequently take a cold/error exit. A fused span does not manufacture writes for its skipped canonical dispatches. |
| `run_frame_resume_pc_write` | Publication of the local resume value when its ProgramCounter guard drops on normal, Result-error, cold, suspension, or Rust-unwind exit. |
| `runtime_pc_publication` | Existing driver publication to the active Runtime frame at observation boundaries; this is not a per-instruction counter. |

These source-level counters are not machine store counts. The ordinary-loop
fixture expects fault writes greater than 100 and no greater than its committed
owned instruction count, with resume and Runtime publication each 1. That bound
is fixture-specific, not a universal relationship for failed dispatches. Earlier
both-local PC experiments had different write counts and remain historical
measurements. The local-resume choice came from ordinary paired candidate
measurements, not from assuming fewer stores are faster. See the
[S08 development evidence](performance/README.md).

`owned_storage` records SlotStore/FrameStore capacity changes, frame-depth and
slot peaks, logical owner moves, cleanup clears, value copies, and narrow hot
releases. Its slot counts distinguish logical frame extent from initialized
backing storage and allocated capacity:

| Field | Meaning |
| --- | --- |
| `slots_initialized` | Cumulative logical slots reserved by successfully installed frames, including reused slots and unused operand capacity. This compatibility counter does not count physical writes of `None`. |
| `physical_none_initializations` | Cumulative slots first written as `None` when a SlotStore's initialized backing grows. Reusing that backing adds zero. Later owner clears and binding writes are not included. Growth before a failed frame installation still counts. |
| `maximum_initialized_slots` | Largest initialized backing length observed for one SlotStore; includes inactive `None` entries retained after frame return. It can exceed the interval's successful active-extent peak after a failed installation or when collection starts after a larger frame has returned. |
| `maximum_reserved_slots` | Largest successful active frame extent (`active_end`) observed for one SlotStore, including reserved but unused operands. Inactive backing above `active_end` is excluded. |
| `maximum_live_slots` | Largest observed number of occupied binding/value slots in one SlotStore. Unused operands and inactive backing are excluded. |
| `maximum_slot_capacity` | Largest observed SlotStore `Vec::capacity()`, measured in entries. Allocated capacity can exceed initialized length; it is not allocator usable bytes or a process memory peak. |
| `slot_capacity_growths` | Number of observed increases in allocated SlotStore capacity. Reusing capacity or extending initialized length within existing capacity is not a growth event. |

A SlotStore retains its initialized high-water backing until the owning
execution releases it. Every inactive entry is `None`: frame cleanup releases
its owners, and suspension handoff moves them into `FrameStorage`. Retained
backing therefore holds no JavaScript values or roots above `active_end` and
does not consume the logical active-slot budget. Repeated calls can increase
`slots_initialized` while `physical_none_initializations` remains unchanged.
When collection starts after warm-up, a reused push observes the existing
initialized length and allocated capacity without counting earlier physical
initializations or allocation growths.
These are per-store observations within the collection interval, not additive
process-wide peaks or evidence of a throughput improvement.

Moves include entry/handoff transfers and pop; rotations count participating
owners, not machine copies. Clears count occupied slots removed by frame/arena
cleanup; a hot release followed by pop is reported separately. Empty reserved
operands count toward logical initialization and active extent, not live slots.

`copied_heap_roots` and `hot_heap_root_releases` cover Object/Symbol operations at
the narrow slot boundaries only. Primitive Rc operations, binding/cold-payload
roots, cleanup cascades, window-identity/registry containers and legacy bridge
allocations are excluded. The counters therefore cannot be subtracted to infer
leaks or treated as a total allocation/RC profile. Features compile these hooks
out of ordinary builds; instrumented timings are not formal throughput results.

An S03 debug diagnostic of a 100-iteration addition function returned `4950` and
recorded 1,510 owned instructions, 8 legacy dispatches, 2 slot capacity growths,
2,022 logical slot moves, 604 value copies, and a per-store peak of 6 live slots.
The script/print wrapper uses the bridge; this sample is explicitly mixed. The
separately measured ordinary-call regression requires zero legacy dispatches.


### 字节码调用准备成本

`oxide-compile-vm-cost-v1.call_preparation` 记录成功完成的字节码构帧准备；
函数体随后抛错也保留该事件。新旧执行器共用同一准备入口，计数包含参数与
局部 Vec 的非空 backing allocation、累计实际容量字节、初始化槽数，以及
参数 Value 复制、其中 Object/Symbol root 复制和准备函数中的 callee root
复制。缺少实参的 Undefined padding 计作槽初始化，不计作实参复制；额外
实参保留实际 arity。局部 Vec 显式预留全部定义的容量，填充不再隐含增长。

owned 入口另记录 FrameCold Box 的成功分配、captured-reuse 位标记 Vec
的非空分配和容量，以及进入该帧时独占的原始实参 Vec 容量。最后一项是
**buffer 观察，不是实参分配事件总数**：bound/apply 的中间缓冲区、闭包
快照、原生/旧桥内部容器、暂停 operation 载荷及 allocator 元数据均不在
此字段覆盖内。所有容量为累计观察值，不是同时存活峰值；不能与
`owned_storage` 的逐 arena 峰值相加。root 复制也仅覆盖列明的边界，
不等于全 Runtime retain/release 统计。正式性能仍须使用关闭诊断的构建。


### S09 调用临时缓冲区与暂停阶段诊断

owned 普通根调用和普通子调用直接初始化 SlotStore 参数/局部区，因此其
`parameter_buffer_allocations`、`local_buffer_allocations` 为零；原始 argv
独立保留，参数复制与 Undefined padding 仍分别计数。普通未绑定子调用的
`call_outgoing_tail_transferred` 表示原始 argv 从 caller 操作数尾区直接转移，
不是省略参数副本。FrameCold、捕获标记和 unwind regions 只复用清空后的容量，
按最大同时活动帧深度预留；`owned_frame_allocations` 不计复用命中。

`oxide-compile-vm-cost-v1.call_buffers` 按实际生产者分组：
`native.readable`、`bound.raw_snapshot`、`bound.rooted_snapshot`、`bound.merge`、
`apply.indexed`、`arguments.fast_raw/fast_ordering/fast_rooted`、
`function.call_suffix`、`call.boundary_argv`。Array/Arguments 快照也供 spread
使用，因此共享生产者不强行归到 apply 一类。`invoke.argv_carrier` 只观察
已经构造完成的参数容器，不重复计分配或复制。

- `capacity_growths`、`capacity_growth_bytes`、`allocated_capacity_bytes` 只在
  明确的成功 reserve/new-allocation 边界记录；分别是增长次数、净容量增长
  字节和增长后容量字节的累计和。都不是 malloc usable bytes。
- `buffers_observed`、`observed_capacity_bytes` 是缓冲区容量观察；特别是
  `collect<Result<Vec<_>>>` 的内部增长次数不可从最终容量推断，其结果只记录
  observation，不伪称一次分配。失败的 collect 内部部分分配也未覆盖。
- `slots_initialized`、`values_copied`、`values_moved` 是已观察到的初始化、
  Value 复制和 owner 移入数量。初始化包含对应复制/移入，三者不能相加。
  native Undefined padding 只计初始化；indexed apply 的 getter 回复计移入。
- `heap_root_copies` 只计 Object/Symbol root 复制或 raw 到 root 的提升；
  `primitive_rc_copies` 计 String/堆 BigInt 的共享引用复制，短 BigInt 归入
  `immediate_copies`。RawValue 的 ObjectId/Atom 复制单列为
  `raw_heap_edges_copied`，不当作 Runtime root retain；raw 的 String/堆 BigInt
  共享引用另计 `raw_primitive_rc_copies`。

各生产者代表不同真实步骤；bound raw 快照仅共享 Rc slice，记录现有长度
观察，不推断新分配或逐 RawValue 复制；root 提升和合并 argv 才各自发生
列明的 Value 复制。共享容器 Rc 的复制以 `shared_storage_clones` 单列，不计入 primitive Value Rc 字段。`call_preparation` 与 `owned_storage` 仍是另两种部分观察，
与新 map 的统计可能重叠，不能相加作为全调用总数。这里没有全局 allocator
拦截、完整 GC/析构 release 账或所有临时容器覆盖。

`vm_phases` 包含 `bytecode.prepare`、`native.prepare`、`freeze.detach`、
`freeze.owned_export`、`freeze.encode`、`thaw.decode`、`thaw.prepare_owned`。
每项记录 attempts（含错误/展开）、inclusive/exclusive 纳秒，以及前 4096 次
`[inclusive, exclusive]` 原始样本和 omitted_samples。VM 与编译计时共用同一
嵌套时钟；父阶段 exclusive 扣除直接测量子阶段，inclusive 不能相加。
这些阶段是明确的 Rust 操作边界；未计时的 GC/release 工作仍包含在所在阶段中，
不伪称单独 GC 暂停。样本截断后不能把前 4096 次的分位数称为完整调用分布。

暂停探针在事件数不超过上限时可报告完整样本 p50/p95/p99 与最大值。所有诊断
计时和样本保存均只存在于 `profiling` 构建，并有测量开销；正式吞吐和生产
RSS 比较使用关闭 profiling 的独立构建，不与诊断、测试或构建并行。

owned native continuation 现在接收已有独占 argv Vec，沿同一 metadata/realm
验证入口补齐 Undefined padding 后直接交给 NativeArguments，实际 arity
不变；`native.incoming_argv` 只记录该源容器观察。`native.readable` 对此路径
记录 buffer 所拥有 Value 的逻辑转移和实际 padding 增长，不计 argv 复制；
借用式同步入口仍记录新 readable Vec 和真实 Value 复制。两者都保留额外
实参直到 native 完成、抛错或放弃，未把状态等待期间的 owner 提前回收。

Native 调用方的空 argv 容量由 execution-owned pool 回收：`call.native_argv`
记录调用方实际 reserve 的容量增长和从 operand 栈移交的 Value owner；
`call.native_pool` 单独记录空 Vec 容器池元数据的容量增长（元素为 Vec header，
不是 Value）。池按同时存活的 active-frame 深度预留，native readable 中的参数
仅在错误物化、active-frame 退出后释放，再回收空容量。等待中的 native scope
仍独占其完整参数；丢弃 continuation 使用原有 owner 清理，不回收活值。

### Local continuation and recycler allocation producers

The call-buffer ledger additionally observes successful reserves at `query.parents`,
`query.native_scopes`, `query.spare_parents`, `query.free_pool`,
`cold.empty_pool`, `cold.capture_pool`, `cold.region_pool`, `cold.capture_flags`
and `cold.regions`. Reuse and failed reserves add no capacity growth. Recycler
metadata backing and the reusable allocations it points to are separate producers.

`query.pending_box`, `iterator.pending_box` and `cold.frame_box` record each
successful fixed-size Box allocation as capacity 0 → 1, using its payload size.
`executable.published_data_rc` records the lazy immutable metadata allocation
once and each shared snapshot handle clone separately; clones allocate no payload.
Those byte counts exclude allocator metadata and the Rc header. Existing cold-frame
allocation counters overlap `cold.frame_box`; do not add them together.
`native_activation_prepared` counts successful guard publication, not allocations.
These local counters do not constitute global allocator or retain/release totals.

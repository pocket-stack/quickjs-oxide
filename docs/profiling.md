# Profiling and external benchmarks

The optional `profiling` feature implements the memory snapshots, safe partial
allocation trace, lifecycle timing and benchmark workflow proposed in
[the original design report](reports/quickjs-profiling-plan.html). Diagnostics
are off by default. This is an observability baseline, not a CPU/call-stack
sampler or a claim of feature/performance parity with QuickJS.

Measured remote results and validation evidence: [PocketLab baseline](reports/profiler-baseline.md).
CPU sampling and targeted counters: [CPU hotspot investigation](reports/cpu-hotspots.md).

## Build and run

```sh
cargo build --locked --release -p quickjs-oxide-cli --features profiling
./target/release/qjs -d workload.js
./target/release/qjs -T workload.js
./target/release/qjs -q -d
./target/release/qjs -dT --profile-json --profile-output /tmp/new-profile.jsonl workload.js
```

`-d/--dump` requests a snapshot; `-T/--trace` requests allocation observation.
`-q -d` also takes 100 independent lifecycle samples. `--profile-iterations N`
changes that count (1–10000). Trace holds at most 65536 events by default;
`--profile-events N` changes the limit (0–1000000). Its buffer is allocated once
before runtime creation, never grows while collecting, and counts dropped
events. Dropping the trace handle does not stop the runtime's collector.

Reports go to stderr, preserving script stdout. `--profile-output PATH` creates
a **new** file and refuses existing paths, including the input script. JSON
output is one complete object per line: `oxide-memory-v1`,
`oxide-allocation-trace-v1`, and, for `-q -d`, `oxide-lifecycle-v1`. Multiple
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

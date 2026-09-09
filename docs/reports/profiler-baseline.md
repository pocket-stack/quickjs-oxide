# Profiler baseline — PocketLab, 2026-09-09

The optional profiler is implemented and exercised on `eric-83am`, in the PocketLab `quickjs-oxide` workspace. It provides memory snapshots, bounded arena backing-storage events, and lifecycle timing. It does not sample CPU stacks or cover all allocator activity. See [usage and accounting contract](../profiling.md) and [benchmark commands](../../scripts/benchmark/README.md).

The measurements show substantial performance gaps against native QuickJS: all five selected microbench cases complete, while four of eight isolated V8 v7 suites time out in Oxide. The measurements identify workloads for investigation, not CPU hotspots or causes. No whole-suite score is inferred from the successful subset.

## Revisions and reproducibility

- Base: PR #2's `refact/clean-up-structure`, `9fa0e9d4698959c59453e7b1577dbcb1d54df19c`.
- V8 v7 and full Rust/Test262 validation: `e0b3ef3bc8381ba5971adbf99650f3ae1ff85c6b`.
- Final profiler experiment, matched-clock microbench and pinned-toolchain checks: `5723fa435c62b4f649cf42185293c225276d39c0`.
- Between those revisions: CI coverage, microbench clock adaptation/tests, and one feature-gated formatting expression for the pinned Clippy policy. No interpreter algorithm changed. The binaries have different hashes; the V8 results are specifically for the earlier binary. Later report-only commits do not represent new benchmark runs.
- Machine: AMD Ryzen 7 7840HS, 16 logical CPUs, Linux 6.18.44-1-lts x86_64, glibc 2.44, Python 3.14.7. Builds and timing runs were serialized on the remote machine; no controlled CPU-frequency or machine-isolation claim is made.
- Oxide release: Rust/Cargo 1.94.1, `cargo build --locked --release -p quickjs-oxide-cli --no-default-features`, separate target directory with `--features profiling` for instrumentation. No custom RUSTFLAGS. Build receipts record exact commands, lockfile hashes and clean source revisions.
- Reference: QuickJS 2026-06-04, GCC 16.2.1 20260810, observed `-O2 -g` build commands. Its `--version` is unsupported (exit 1), but its help output reports the version; this is recorded without treating the probe as a benchmark failure.
- External workload: [ahaoboy/js-engine-benchmark at 2034d98](https://github.com/ahaoboy/js-engine-benchmark/tree/2034d98fc8c5f8044e186267593f5d5ea5232caf). Checkout and generated bundles remain outside this repository. The runner mirrors upstream load inlining/concatenation; benchmark bodies and V8 timing remain unchanged.

Exact workload, runner and binary SHA-256 hashes, build receipts, all measured sample values, lifecycle samples, memory/trace records and Test262 comparison are in [profiler-baseline.json](profiler-baseline.json). The external benchmark sources are not vendored in either the repository or the raw evidence package.

| Binary | SHA-256 |
| --- | --- |
| Oxide V8 plain | `1995b7a7b74e625885fa3e51b7c446ded54508846faf4c6bb4c080c8c1f52f60` |
| Oxide final plain | `644b35c3714d8b898f262a551bdaf4762994302fcec0ce91c61c816e7a83b82f` |
| Oxide final profiling | `93e86468463123bd99e953eccec1438ba7466deca1d5958add37374a2da524e1` |
| Native QuickJS | `40ab2a7a9471b197843a5c489d1d53c7c075d4cef0a4d42cce4d22dc07a20007` |

## Instrumentation experiment

One authored workload is run 11 times in each of four modes, with rotated serial ordering and identical verified stdout. Plain is compiled without profiling; compiled-off includes the feature but enables no diagnostics. Dump and trace enable their respective diagnostics. Full-process wall time includes startup, teardown, serialization and output I/O, and is not isolated VM execution time.

| Mode | Median ms | Range ms | Ratio to plain |
| --- | ---: | --- | ---: |
| plain | 14.580 | 14.012–16.960 | 1.000× |
| compiled-off | 14.573 | 13.382–15.343 | 0.999× |
| dump | 15.688 | 14.949–17.679 | 1.076× |
| trace | 14.604 | 13.824–15.418 | 1.002× |

The ranges overlap. These samples support neither a zero-overhead claim nor a statistically significant difference for compiled-off/trace. Dump's observed median is about 7.6% higher for this workload including report I/O. This is a single workload and machine, not a general overhead guarantee.

### Snapshot and trace

The final snapshot contains 1,171 objects, 65 shapes, 56 variable references, one Context and 383 table-backed atoms. There are 1,293 live arena slots and three vacant slots. No bytecode functions remain at this snapshot boundary; zero is not a claim that the program executed without bytecode.

| Owned inline storage | Used bytes | Capacity bytes |
| --- | ---: | ---: |
| Arena slots | 1,171,584 | 1,851,392 |
| Property slots | 92,512 | 176,160 |
| Dense array elements | 24,024 | 24,672 |
| ArrayBuffer bytes | 65,536 | 65,536 |

These are partial owned-storage measurements, excluding nested allocations and allocator overhead. They must not be interpreted as total memory or added again to logical object counts. String storage, shared backing storage, total allocator bytes, peak and RSS remain explicitly unavailable (`null`).

The trace has 11 events: one allocation, nine capacity growth events and one release after runtime destruction. Arena backing capacity grows from 3,616 to 1,851,392 bytes and ends at zero; no events were dropped, and collection finished. These successful safe Vec storage transitions do not identify physical malloc/realloc calls or failed allocation attempts. The preallocated event buffer does not grow during collection, perform I/O, invoke JavaScript or retain JS roots.

### Lifecycle

100 independent iterations, instrumentation off; monotonic wall nanoseconds:

| Phase | Median ns | Minimum ns | Maximum ns |
| --- | ---: | ---: | ---: |
| runtime_create | 5711 | 5430 | 8135 |
| context_create | 2,393,814.0 | 2369102 | 2547776 |
| context_drop | 61 | 40 | 180 |
| runtime_drop | 184648 | 181919 | 199071 |

The very short Context-drop phase measures that API boundary; retained realm cycles are cleaned up during Runtime drop. Context creation dominates this lifecycle probe. Reference QuickJS lifecycle uses CPU time, so no cross-clock speed ratio is reported. Independent phase minima need not belong to the same iteration.

## Matched-clock microbench

Five selected cases from pinned QuickJS `tests/microbench.js`, three repetitions per engine, serial alternating order, 90-second timeout. Both engines receive the identical prefix disabling optional `performance`/`os` timers; every run must confirm the `Date.now` fallback before its result is accepted. The original body bytes are preserved, and original/prepared/prefix hashes are recorded. This deliberate host adaptation is not an untouched native-clock comparison.

| Case | Oxide median ns/op | QuickJS median ns/op | Oxide / QuickJS time |
| --- | ---: | ---: | ---: |
| empty_loop | 200 | 10 | 20× |
| prop_read | 1000 | 10 | 100× |
| array_read | 500 | 10 | 50× |
| func_call | 1000 | 25 | 40× |
| int_arith | 400 | 10 | 40× |

All 30 runs succeed. Each triplicate has identical reported values because the benchmark's millisecond clock and calibration produce coarse, quantized estimates. They are upstream minimum ns/op estimates, not individual-operation distributions; identical numbers do not establish zero variance. The ratios are coarse observations.

An earlier run accidentally compared QuickJS `performance.now` with Oxide `Date.now`. Its ratios are invalid and excluded here. Original evidence is retained with an explicit warning under `microbench-invalid-clock` in the archive; the table above uses only the rerun with verified matching clocks.

## External V8 v7 suites

Each suite is isolated, with three repetitions per engine, serial alternating engine order, instrumentation off and a 90-second deadline per process. A watchdog terminates the process group on timeout. Success requires all expected harness scores, not merely exit 0, because the external harness can swallow errors. Scores are higher-is-better.

| Suite | Oxide successful runs | Oxide median score (range) | QuickJS successful runs | QuickJS median score (range) |
| --- | ---: | --- | ---: | --- |
| Richards | 3/3 | 15 (14.8–15) | 3/3 | 1352 (1351–1370) |
| DeltaBlue | 3/3 | 20.9 (20.7–20.9) | 3/3 | 1236 (1221–1245) |
| Crypto | 0/3 | timeout in all runs | 3/3 | 1549 (1479–1580) |
| RayTrace | 3/3 | 34.8 (34.8–35.3) | 3/3 | 2829 (2757–2838) |
| Earley-Boyer | 0/3 | timeout in all runs | 3/3 | 3439 (3389–3452) |
| RegExp | 0/3 | timeout in all runs | 3/3 | 627 (597–630) |
| Splay | 3/3 | 126 (125–126) | 3/3 | 4958 (4906–4971) |
| Navier-Stokes | 0/3 | timeout in all runs | 3/3 | 3108 (2931–3164) |

The runner correctly exits nonzero for this incomplete suite. Timeout establishes failure to finish under this deadline, not an infinite loop, unsupported syntax, or a quantified speed ratio. No combined V8 score is available. Process wall times are retained separately from harness scores.

## Correctness and gates

Heavy builds, tests, profiling and JS benchmarks ran through the Herdr PocketLab workspace on `eric-83am`. Local work was limited to lightweight checks and report processing.

- Remote workspace/all-target profiling validation: 2,993 Rust tests passed, one existing test ignored across 13 test binaries. Plain CLI tests also passed.
- Final Rust 1.88.0 pinned-toolchain strict Clippy and CLI profiling tests passed. Source layout/Rust-only gates passed; all six Python runner tests passed. The final CLI tests include snapshot/trace/error paths, feature gating and writer/JSON checks.
- An initial strict Clippy run on Rust 1.94.1 failed on 37 existing lint diagnostics; advisory mode passed. The pinned-toolchain run first needed its Clippy component, then exposed one new format-argument lint which was fixed before the final successful run. The status history and failed logs are retained.
- Full Test262 produced 102,037 variant outcomes: 79,982 pass; 7 parse failures; 43 runtime failures; 18,475 skipped; 3,530 classified unsupported. These pre-existing non-passes are retained.
- Every Test262 outcome row matches the PR #2 baseline: **0 changed rows, 0 new failures**, identical row SHA-256 `fa99d3349bb4b61f30ba57d7c7f275df64f19691edefc50b96b53887339fa8c3`. Baseline source fingerprint was recomputed from the exact base and matches the prior receipt.
- The frozen full-report checksum gate still exits 1: the new source fingerprint changes the TSV header and therefore its file checksum. This is not reported as a passing frozen gate, and its baseline was not silently updated. Full Test262 was run at `e0b3ef3`; later formatting/driver/CI changes received targeted validation.

## Next investigations

Property access, function calls and array access are useful starting workloads given the coarse microbench gaps. Context initialization and the 904-byte arena slot representation also warrant measurement. These are hypotheses: allocation coverage is too narrow to attribute whole-runtime memory, and this profiler supplies no CPU stack samples. Follow-up CPU sampling of representative successful and timed-out suites is needed before assigning interpreter hotspots or optimizing algorithms.

## Raw evidence

Archive: `quickjs-oxide-profiler-pocketlab-2026-09-09.tar.gz` (4,226,215 bytes).
SHA-256: `19120bc1f848dc1f7c17347de4efab5aba7c3d56e0246c864a48a4a302444e06`.

The archive is available in the local workspace's `target/` and on `eric-83am` at `/home/eric/Documents/Sources/PocketLab/quickjs-oxide/target/`. It contains raw stdout/stderr, sample journals, build/compiler evidence, validation logs, both Test262 vectors, the initial/final profiler runs and the invalid-clock run with its warning. It is an external delivery artifact, not a tracked source dependency. The tracked JSON provides the numerical evidence without requiring the archive. Remote absolute paths in receipts describe their original execution locations.

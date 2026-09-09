# Performance tools

These tools orchestrate external workloads; they do not vendor benchmark code.
Use Python 3.10+ on a Unix host. Run timing and heavier validation on PocketLab,
serially, without competing builds or tests. `run.py` uses process-group timeout
cleanup; Windows process management is not implemented by this runner.

## Prepare binaries with provenance

Commit implementation changes first. Build on the measurement host:

```sh
python3 scripts/benchmark/build.py --jobs 2
```

This builds ordinary and profiling release CLIs in separate target directories,
embeds the source commit in profiling reports, and writes `qjs.build.json`
receipts with compiler versions, command, flags and binary hash. The benchmark
runner verifies matching receipts when present. External engines without a
receipt are identified by binary hash/version output; attach their compiler
and build configuration separately when publishing comparisons.

## External V8 v7 suite

```sh
# A sibling checkout, outside quickjs-oxide; record/fix its commit for repeats.
git clone https://github.com/ahaoboy/js-engine-benchmark.git ../js-engine-benchmark
# Use the project's existing, pinned QuickJS oracle builder if needed.
reference=$(./scripts/quickjs/build-quickjs-oracle.sh)

python3 scripts/benchmark/run.py --suite v8-v7 \
  --source ../js-engine-benchmark \
  --engine oxide="$PWD/target/release/qjs" --engine quickjs="$reference" \
  --repeat 3 --timeout 120 --output target/benchmark-v8-v7
```

The runner generates bundles under the external checkout's
`dist/quickjs-oxide`, following its `scripts/build.ts` algorithm: inline `load`
calls for the complete suite, or concatenate `base.js`, one suite and the
unchanged runner for isolated cases. It does not change benchmark bodies or
timing policy. The generated complete suite can also be run directly:

```sh
./target/release/qjs ../js-engine-benchmark/dist/quickjs-oxide/run.js
```

Default selection is every isolated suite from upstream `run.js`.
Use `--case richards --case deltablue` to select suites, or `--case all` for the
original combined run. Each requested repetition gets a fresh process. Record
both the source commit and generated workload hashes: upstream can change.
The original suite reports scores, not ns/op. Require every expected suite score
and a valid aggregate `Score` even if the engine exits zero: its error callback
can swallow failures. Internal assertions remain the original suite's checks.

## Pinned QuickJS microbench

```sh
python3 scripts/benchmark/run.py --suite microbench \
  --source target/oracle/quickjs-2026-06-04/tests/microbench.js \
  --engine oxide="$PWD/target/release/qjs" \
  --engine quickjs="$PWD/target/oracle/quickjs-2026-06-04/qjs" \
  --repeat 3 --timeout 120 --output target/benchmark-microbench
```

The default initial matrix is `empty_loop`, `prop_read`, `array_read`,
`func_call`, and `int_arith`. `--case` follows the original function-name prefix
matching. The tool admits ordinary benchmark rows with N and ns/op; specialized
sort output that does not match this contract is retained as incomplete rather
than assigned a score. A shared prefix disables `performance`/`os` clock
selection on both engines, preserving every byte of the original body and using
its existing Date.now fallback. This is necessary because pinned QuickJS adds
`performance.now` even without `--std`, while Oxide currently does not. A dynamic
clock marker is checked on every run; mismatched clocks disqualify the result.
Prepared microbench source stays in an external temporary directory, with its
path/hash and exact prefix/hash recorded in metadata. Its millisecond resolution and minimum-of-many
sampling limit what the resulting ns/op says; these are not individual-operation
latency distributions. No ratio is admitted without the matching clock marker.

Reference loading/saving in JS is not needed. Python records stdout, per-run
ns/op, N, median/min/max/stdev across independent runs and per-case ratios.
Only the shared clock prefix is added; workload bodies are untouched. On engines without `std`/`fs`, the harness's
reference-file operations are no-ops; Python owns result files.

## Profiler collection and overhead

```sh
python3 scripts/benchmark/probe.py \
  --plain target/release/qjs --profile target/profile-feature/release/qjs \
  --reference target/oracle/quickjs-2026-06-04/qjs \
  --repeat 11 --output target/profile-experiment
```

This runs an explicitly authored small allocation workload, validates snapshots
and complete scoped trace lifecycles, saves the reference's original output,
and records 100 uninstrumented lifecycle samples. Four instrumentation modes
use the same workload, with mode order rotated between repetitions. Full-process
wall time includes report I/O, so this measurement is workload/host-specific and
not pure interpreter overhead. The reference trace/dump share stdout with the
script, and lifecycle CPU time is kept separate from Oxide wall time.

## Output and validation

Every invocation requires a new output directory, preserving prior evidence.
`metadata.json` captures workload hashes, engine hashes/build receipts, hardware,
OS, source revision, clock and policy. `samples.jsonl` is flushed after each
sample so interrupted work retains evidence. `results.json` and `report.md`
contain the final matrix, raw sample references and summaries. Raw stdout/stderr
remain separate files. Nonzero exit, timeout, incomplete/unsupported output and
successful samples are distinct. The runner returns nonzero if any sample is
unsuccessful. A case qualifies for a cross-engine ratio only if every requested
repetition succeeds on both engines; no overall score is inferred from a subset.

The built-in harness warm-up/calibration is unchanged and recorded as such.
Process wall times include startup, parse/compile, execution and teardown;
benchmark operation timings are reported separately. These tools do not claim
to isolate parse/compile time without a separate embedding harness.

```sh
python3 -m unittest discover -s scripts/benchmark -p 'test_*.py'
cargo test --locked -p quickjs-oxide-cli --test profiling
cargo test --locked -p quickjs-oxide-cli --test profiling --features profiling
cargo test --locked -p quickjs-oxide --lib --features profiling profiling_
```

Run the broader Rust/QuickJS comparison tests and Test262 independently of
benchmarking. Never revise conformance baselines to turn a performance change
into an apparent pass.

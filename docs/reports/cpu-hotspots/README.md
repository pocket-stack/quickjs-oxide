# CPU investigation artifacts

Read [the CPU hotspot report](../cpu-hotspots.md) first. These artifacts describe experiments at engine commit `9ed39e275d0f1b74ce82ed694ebaef797d6b79d7`; they do not change the product implementation.

- `context.svg`, `prop_fixed.svg`, `empty_fixed.svg`, `richards.svg`: cycle-weighted flame graphs from three uninstrumented frame-pointer runs each. Open an SVG directly in a browser to zoom/search; shortened display labels retain full symbols in tooltips. Richards reaches the 127-frame capture limit, so outer ancestry is incomplete.
- `diagnostic-v1.patch`: temporary counters, Context intrinsic-stage timers and compile/execute timers used for the main diagnostic experiment.
- `diagnostic-v2.patch`: the complete alternative patch, adding array classification and numeric/local-binding counters. Both patches apply independently to the same base; do not apply V2 on top of V1.

The patches were applied only to an archived source copy under the remote project's `target/`. They deliberately expose an investigation-only module and are not intended as a production API or a proposed optimization. Modes are selected with `OXIDE_CPU_PROBE`: `0` off, `1` counters, `2` counters and detailed inclusive timers, `3` coarse phase timers. The CLI prints one final diagnostic JSON record after teardown. No per-operation I/O, user callback or global allocator interception is added.

The report JSON retains the sampled hotspot summaries, every process-wall sample and all nonzero diagnostic counters/timers. Zero entries are omitted only from this compact summary. The external raw archive includes full records, the original 1,000-iteration lifecycle arrays, reproducibility scripts, exact ELF files, raw perf data and a verified SHA-256 manifest. External benchmark JS source and generated bundles are excluded.

To repeat the experiment, use a fresh output directory on the remote host. Build the unmodified source first with release optimization, debug information and frame pointers:

```sh
CARGO_PROFILE_RELEASE_DEBUG=1 \
CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C force-frame-pointers=yes' \
cargo build --locked --release -p quickjs-oxide-cli \
  --features profiling --target-dir target/cpu-profile-repro --jobs 2

perf record -e cycles:u -F 199 --call-graph fp \
  -o target/richards-repro.data -- \
  target/cpu-profile-repro/release/qjs \
  ../js-engine-benchmark/dist/quickjs-oxide/richards.js

perf report --stdio --no-children --call-graph none \
  -i target/richards-repro.data
```

Use the source/workload revisions and hashes recorded in the JSON. For probe runs, archive the base into a separate directory, apply exactly one diagnostic patch there, and build to another target directory. Compare probe-off and probe-on output and timing; do not use an instrumented binary's speed as an optimization result. Original experiment scripts in the raw archive preserve their absolute execution paths and refuse existing output directories; adapt those paths before replaying them.

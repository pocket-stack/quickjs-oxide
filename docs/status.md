# Implementation status

quickjs-oxide is an unsafe-free Rust rewrite targeting semantic Feature Parity
with QuickJS 2026-06-04. It is runnable on the command line and as the real
Rust/WASM engine in the GitHub Pages playground, but it is not yet at Feature
Parity.

## Current baseline

The authoritative R3ed-A Test262 vector has:

- 68,362 full-corpus passes out of 102,037 variants (66.997%)
- 68,414 eligible variants out of 102,037 (67.048%)
- 68,362 passes out of 68,414 runnable variants (99.924%, secondary quality
  metric)
- 50 classified failures and 2 timeouts among eligible variants

The exact profile, inputs, summary, line counts, and report hashes live in
[`dev-support/test262/current.conf`](../dev-support/test262/current.conf).

## Implemented architecture

- Rust compiler, verified bytecode, runtime, jobs, modules, and embedding API
- binary data, typed arrays, shared memory, and Atomics slices
- collections, weak references, finalization, Promises, and iterator slices
- Unicode 17 case, identifier, normalization, and property data
- synchronous static-module graphs, live namespaces, default exports, and core
  `import.meta` semantics
- native command-line execution and a Rust/WASM browser playground

The public API and Test262 runner now report the same engine diagnostics.
Detached public bytecode/VM execution has been retired, the Test262 runner
loads a data profile instead of compiling historical milestone identity tables,
and its `$262` realm/agent host is isolated behind a non-default feature.

## Remaining parity work

Major open frontiers include dynamic import, import attributes, top-level
await, remaining module-host behavior, and the unsupported/failed leaves
recorded by the current Test262 vector. A Feature Parity claim additionally
requires the acceptance contract in [`parity.md`](parity.md), including QuickJS
differential evidence and non-Test262 behavior.

The initial architecture-hygiene pass is complete. Cargo integration-test
targets fell from 186 to 60 (56 oracle targets), and repeated QuickJS transport
helpers now share support code. Path-sensitive and feature-sensitive targets
remain separate where aggregation would change their contract. Negative
diagnostics use a source-authenticated data contract; its first natural
ModuleImportBinding cohort gates the exact QuickJS error message and
line/column as well as phase and type.

## Verification

```sh
cargo test --locked --workspace --all-targets
cargo test --locked --features test262-host --lib --bins
./scripts/test-test262.sh --check
./scripts/test-test262.sh --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh --full
./scripts/test-web-playground.sh
```

Historical milestone gates, profiles, result vectors, baselines, and the former
long-form ledgers are preserved in the release archive indexed under
[`dev-support/test262/archive`](../dev-support/test262/archive/index.tsv).

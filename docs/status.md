# Implementation status

quickjs-oxide is an unsafe-free Rust rewrite targeting semantic Feature Parity
with QuickJS 2026-06-04. It is runnable on the command line and as the real
Rust/WASM engine in the GitHub Pages playground, but it is not yet at Feature
Parity.

## Current baseline

<!-- current-test262-metrics:start -->
The authoritative R3es Test262 vector has:

- 79,475 full-corpus passes out of 102,037 variants (77.888%)
- 79,525 eligible variants out of 102,037 (77.937%)
- 79,475 passes out of 79,525 runnable variants (99.937%, secondary quality
  metric)
- 50 classified failures and no timeouts among eligible variants
<!-- current-test262-metrics:end -->

The pass count includes three exact `(path, variant)` results where Rust passes
a test listed in pinned QuickJS's known-error file. These narrow target
deviations are registered in [`deviations.md`](deviations.md); they do not imply
that the pinned engine passes those tests.

The exact profile, inputs, summary, line counts, and report hashes live in
[`dev-support/test262/current.conf`](../dev-support/test262/current.conf).

## Implemented architecture

- Rust compiler, verified bytecode, runtime, jobs, modules, and embedding API
- binary data, typed arrays, shared memory, and Atomics slices
- collections, weak references, finalization, Promises, and iterator slices
- Unicode 17 case, identifier, normalization, and property data
- physical dense Array storage shared by literals, builtin results, and JSON
- public and private instance/static data fields, private methods and
  accessors across ordinary, generator, async, and async-generator forms,
  static blocks, and private brand checks with `#name in object`, including
  QuickJS-matched early errors
- static-module graphs, live namespaces, default exports, top-level await with
  async dependency/SCC scheduling, static import attributes with loader
  validation, JSON synthetic modules, and canonical host-populated
  `import.meta` objects
- Script-goal dynamic `import()` with FIFO load/finish jobs, live host-loader
  callback sampling, import attributes, cached cycle-root evaluation Promises,
  namespace reuse, Promise assimilation, and exact propagation of arbitrary
  JavaScript values thrown by normalize, attribute-check, and load callbacks
- native command-line execution, including file-module goal detection,
  filesystem dependencies, top-level-await settlement, and `import.meta`
  `url`/`main`; plus a Rust/WASM browser playground

The public API and Test262 runner now report the same engine diagnostics.
Detached public bytecode/VM execution has been retired, the Test262 runner
loads its profile and exact admissions from hash-authenticated data instead of
compiling milestone identity or admission tables, and its `$262` realm/agent
host is isolated behind a non-default feature.

## Remaining parity work

Major open frontiers include JSON5/byte-oriented host loading, initiating-
Context access and legal re-entry at module-host callbacks, and the unsupported/
failed leaves recorded by the current Test262 vector.
Failed acyclic source graphs retry like QuickJS. For a failed cycle, Rust
safely unpublishes every record that still points into the failed transaction;
it does not reproduce pinned QuickJS's dangling dependency pointer. Reclaiming
the resulting vacant module-cache slots remains architecture work. A Feature
Parity claim additionally requires the acceptance contract in
[`parity.md`](parity.md), including QuickJS differential evidence and
non-Test262 behavior.

The R3en architecture-hygiene pass is complete. Cargo integration-test targets
fell from 186 to 5 (one shared oracle harness). Repeated runtime-completion,
value, property, CLI, and QuickJS transport helper families now share support
code; a token-level gate rejects the retired and shared-provider fingerprints
without conflating domain-specific prelude or spelling helpers. Path-sensitive
non-oracle tests remain separate, while `$262` oracle modules are feature-gated
inside the shared harness. Negative
diagnostics use a source-authenticated data contract. The ModuleImportBinding,
public-class-field, public-static-initialization, private-data-field, and
private-callable cohorts gate the exact QuickJS error message and line/column
policy as well as phase and type. Logical-assignment, optional-chain-assignment,
generator-yield collateral cases, and both direct and parenthesized
assignment-target cohorts discovered during admission are exact-contracted too.
R3el adds the QuickJS-matched single-statement function, lexical, and class
declaration diagnostics, plus strict-code `with` statement diagnostics.

The active tree now retains only 23 referenced `tests/test262-*` artifacts; 313
superseded manifests and ledgers are authenticated in the R3eh history release.
Fast CI rejects any new unreferenced Test262 bookkeeping file.

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

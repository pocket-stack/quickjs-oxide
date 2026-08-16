# Implementation status

quickjs-oxide is an unsafe-free Rust rewrite targeting semantic Feature Parity
with QuickJS 2026-06-04. It is runnable on the command line and as the real
Rust/WASM engine in the GitHub Pages playground, but it is not yet at Feature
Parity.

## Current baseline

<!-- current-test262-metrics:start -->
The authoritative R3fb Test262 vector has:

- 79,597 full-corpus passes out of 102,037 variants (78.008%)
- 79,647 eligible variants out of 102,037 (78.057%)
- 79,597 passes out of 79,647 runnable variants (99.937%, secondary quality
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
- byte-exact Script and ECMAScript Module embedding APIs and loader payloads,
  with QuickJS-compatible malformed UTF-8, WTF-8/CESU-8, source retention, and
  diagnostic locations
- byte-exact strict and extended JSON module-loader payloads plus CLI file
  ingestion, preserving malformed-byte semantics and diagnostic locations
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
  validation, strict and QuickJS-extended JSON synthetic modules, and
  canonical host-populated `import.meta` objects
- Script-goal dynamic `import()` with FIFO load/finish jobs, live host-loader
  callback sampling, import attributes, cached cycle-root evaluation Promises,
  namespace reuse, Promise assimilation, and exact propagation of arbitrary
  JavaScript values thrown by normalize, attribute-check, and load callbacks
- initiating-Context access for module-host callbacks, same-Context compiled
  module results, synchronous nested loader compilation with QuickJS-matched
  callback depth and evaluation order, and catchable native-stack exhaustion
- QuickJS-ordered parse-time module publication: the construction identity is
  cache-visible before the first token, each request prefix is visible before
  its attribute callback, successful completion preserves the same identity,
  and failed/referenced identities remain deterministic without unsafe raw
  pointer reuse
- native command-line execution, including byte-preserving file-module goal
  detection and filesystem dependencies, top-level-await settlement, and
  `import.meta` `url`/`main`; qjs-compatible side-effect-free structured
  `print`/`console.log` output with byte-exact WTF-8 String transport; plus a
  Rust/WASM browser playground

The public API and Test262 runner now report the same engine diagnostics.
Detached public bytecode/VM execution has been retired, the Test262 runner
loads its profile and exact admissions from hash-authenticated data instead of
compiling milestone identity or admission tables, and its `$262` realm/agent
host is isolated behind a non-default feature.

## Remaining parity work

Major open frontiers include remaining module-host lifetime and
allocation-failure edge matrices and the unsupported/failed leaves recorded by
the current Test262 vector.
The private BC_VERSION 5 foundation now has bounded wire primitives and the
pinned BigInt payload codec, including QuickJS's asymmetric 16,385-limb writer
edge. A heap-independent WireGraph slice now validates and canonically rewrites
primitives, ordinary objects, arrays, ArrayBuffers, shared identity, and cycles
with explicit decode and emitted-traversal budgets. Its data-object semantics
include header atom interning, tagged decimal keys, first-slot/last-value
duplicate properties, compatible null-atom consumption, depth-first output
atom rebuilding, fixed-versus-resizable ArrayBuffer state, and per-buffer plus
aggregate current-backing-store byte limits. The decoder separates preorder
identity registration from value completion: every parent/root attachment now
uses one completed-subtree delivery path owned by the decode state. Its private
arena represents incomplete identities with explicit pending/ready slots;
linear node/reference reservations reject stale commits, and independently
bounded reference entries can alias pending or ready identities without
consuming another node. TypedArray, SharedArrayBuffer, Date, ObjectValue, and
the bytecode-only object tags remain rejected. It is not a public binary-object
API yet:
`num-bigint` lacks fallible construction, so heap materialization, decoder OOM
mapping, and allocator fault-injection remain hardening gates before untrusted
input admission.
Failed acyclic source graphs retry like QuickJS. Parse-time resolution success and
one-shot failure latches match the pinned callback order; an incomplete graph
is non-executable and dynamic import rejects deterministically. Rust safely
uses an `Aborted` identity where pinned QuickJS can retain a dangling pointer
whose subsequent use enters native undefined behavior. Native crash and
allocator-aliasing probes are deliberately not automated. Reclaiming the
resulting vacant module-cache slots remains architecture work. A Feature Parity
claim additionally requires the acceptance contract in
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

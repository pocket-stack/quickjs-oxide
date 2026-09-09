# Candidate A refactor

The current layout is the responsibility decomposition described at the end of this record and in [architecture](../architecture.md). Earlier sections record the preceding migration stages.

The selected architecture is implemented in the
[workspace](../architecture.md) and [responsibility boundaries](../responsibility-boundaries.md).

The former core and compiler packages are now engine modules. Code products,
primitive storage, exact source coordinates, compilation and execution share
one crate. Realm initialization, builtins, modules, jobs and Context operations
have distinct modules. Native and browser services live under adapters; the
Test262 runner lives under conformance. The follow-up consolidation described below removes the embedding facade; native provider, CLI, Web and runner package names remain available.

Source and regexp remain modules inside engine because their current consumers
and UTF-16 string representation belong to this product. Candidate A's directory
sketch does not require them to become standalone crates. Unicode tables are
byte-identical to their originals.

## Initial candidate A validation

| Check | Result |
| --- | --- |
| Workspace tests, all targets | 2,982 passed; one existing ignored oracle test |
| Rust 1.88 workspace libraries/binaries with all features and unsupported diagnostics | 2,036 passed |
| Rust 1.88 Test262 host oracle tests | 5 passed |
| Rust 1.88 Clippy, workspace/all targets/all features, warnings denied | Passed |
| Rust 1.88 formatting and whitespace diff check | Passed |
| WASM optimized build and Node playground smoke | Passed; 15 examples and catchable recursion overflow |
| Binary-object checker unit tests | 10 passed |
| Production source boundary scan | Passed |
| Binary-object mutation regression | Full run found one obsolete no-op visibility mutation; corrected it and reran all 35 scalar cases successfully. All other full-run cases were correctly rejected. |
| Rust-only, host feature/dependency and anti-special-casing checks | Passed |
| Oracle helper/module registry | Passed |
| Current Test262 runner release build and source/binary provenance | Passed |
| Test262 frozen receipt authentication | Passed; baseline explicitly reported as source-stale |

Full Test262 was not rerun. Historical receipts and their source identities were
not rewritten, and this refactor makes no new conformance or performance claim.

## Boundary maintenance

Checks now scan adapters and conformance in addition to apps and crates.
Current fingerprint tree arguments are sorted as required by the authenticated
fingerprint tool. The checker pins the new embedding routing and the crate-only
publication methods needed by Context. Private binary-object intermediate
representations retain their previous boundaries.

Mutation fixtures and target strings follow the moved owners, including primitive
constants and the engine Cargo manifest. Clippy cleanups only reorganize existing
test modules or express equivalent assertions; corresponding source snapshots
were updated after reviewing those changes, independently of Test262 receipts.

The obsolete mutation changed `pub(crate)` to itself after the Context move.
It now widens the method to `pub`, and the publication-boundary check rejects it.
The focused rerun used the repository's normal production scan, clean fixtures,
job scheduler and scalar canary definitions.


## Root package consolidation

The former engine and embedding facade are now one quickjs-oxide package at
the repository root. Source moved from crates/engine/src to src, and embedding
tests and examples moved to tests and examples. There is no crates directory.

Runtime is now the actual engine handle, without a wrapper or into_engine
conversion. Application entry points explicitly pass native or browser
HostServices. Production dependencies remain acyclic: adapters depend on the
main package; native applications depend on the package and native adapter.
The workspace has six packages: one interpreter, two adapters, two applications
and the Test262 runner.

Cargo targets, generators, CI, boundary checks, mutation fixture paths and current
source fingerprints follow the root layout. Historical Test262 receipts retain
their original source identity.

The root source files are module implementations and entry points. In particular,
heap.rs, runtime.rs and vm.rs still contain substantial code and inline tests.
Their sibling directories hold submodules. This consolidation changes package
layout and host composition; it does not further split those implementations.

### Consolidation validation

- Root embedding example: `cargo run -p quickjs-oxide --example eval -- '6 * 7'` returned `42`.

- Rust 1.88 workspace all-target tests: 2,982 passed, one existing ignored oracle.
- Rust 1.88 workspace doc-test command: passed (no doctest cases).
- Rust 1.88 all-target/all-feature Clippy with warnings denied: passed.
- Five feature-enabled Test262 host oracle tests: passed.
- Optimized WASM build and Node smoke: all 15 playground examples passed,
  including catchable recursion overflow.
- Native release Test262 runner provenance: passed for source fingerprint
  `2564b44792ed5ea843b1d6b8fd2f0037f0ba10d570f836ce363ce219f0afd539`.
- Production binary-object scan, ten checker unit tests, host dependency gate,
  anti-special-casing gate, oracle registry and Rust-only gate: passed.
- All 685 existing mutation targets were checked for applicability. The relevant
  surface/layout group rejected all 62 cases. After restoring root tests/examples
  to source ownership coverage, eight focused cases (including six repeated
  layout cases and two root-scope probes) were rejected. The entire mutation
  suite was not rerun for this consolidation.
- Formatting and whitespace checks: passed.

Full Test262 was not rerun. Existing conformance receipts remain tied to their
historical source, and do not certify this merged package.


## Responsibility decomposition

The final source layout follows the report directly: source/ and regexp/ are
siblings of engine/ under src. Only lib.rs remains as top-level Rust source.

Runtime methods have been distributed to their semantic owners: names in atom,
object allocation and internal operations in object, coercion in value, global
bindings in realm, publication and decoding in code, execution in vm, and
language builtins in builtins. The public Runtime handle belongs to api;
shared state, roots and resource cleanup belong to heap/runtime and heap.

Heap records and storage operations are separated by payload and lifecycle.
VM protocol, completion, activation, frame execution, instruction dispatch,
numeric execution and unwind logic are distinct modules. Native builtin
selectors belong to builtins. Unicode algorithms and tables belong to source.
Existing public module paths remain reexports of these owners.

The private decoder is now a child of code. Its restricted visibility names
the actual code/binary_object ancestry. ConstructorRef remains opaque; VM-only
frame helpers and heap-only storage helpers retain their enclosing-module scope.

All 7,173 function definitions were retained. Function-body comparison found no
production algorithm change after accounting for moved paths and formatting.
The only initially unmatched body was formatting in an existing VM test.

All 64 source directories have README ownership guides and file/child inventories.
The new source-layout CI check verifies the report's directory set, the single
top-level lib.rs, README coverage and links, and reachability of all 414 Rust files
through module declarations or explicit generated-table includes.

Binary boundary evidence follows registered physical owners. The checker verifies
that each owner is connected through conventional module declarations, rejecting
missing routes and conditional exclusions. Mutation rewrites use the same owner
map and reject edits that neither change source nor add a file. Frozen structural
hashes were migrated after reviewing namespace/visibility and rustfmt differences;
historical Test262 receipts were not renewed.

### Decomposition validation

- Rust 1.88 workspace all-target tests: 2,982 passed; one existing ignored oracle.
- Rust 1.88 workspace all-target/all-feature tests: 2,987 passed; one existing
  ignored oracle, including the five feature-enabled host oracle cases.
- Rust 1.88 all-target/all-feature Clippy with warnings denied: passed.
- Optimized WASM build and Node smoke: 15 examples passed, including catchable
  deep recursion overflow.
- Source layout: 414 reachable Rust files and 64 directory READMEs.
- Binary checker unit tests: 14 passed, including new module-route rejection tests.
- Production binary-object scan, host feature/dependency gate, anti-special-casing,
  oracle registry and Rust-only gate: passed.
- All 685 existing mutation targets remain applicable in the new layout.
- Full Test262 was not rerun.
- The focused surface/publication/VM/transport mutation run found one obsolete
  namespace predicate; all other selected mutations were rejected. After fixing
  that predicate and adding five nested-engine dependency probes, all 57 cases
  in the affected transport/dependency rerun were rejected. The entire mutation
  suite was not rerun for this decomposition.
- Rust 1.88 workspace doc-test command, formatting and whitespace checks: passed.
- Native release runner provenance: passed for source fingerprint
  `54f2da20d8927d98736866dfee2aebfff7b32178910bf86f1b16603f5e967758`.

## Removal of legacy Rust API routing

The embedding entry point is now `quickjs_oxide::engine::api`. `lib.rs` declares
only `engine`, `source`, and `regexp`, plus version constants. Inside `engine`,
only `api` is public. No old root module aliases or root API item reexports remain.

Removed `code/compat.rs`, both `BytecodeFunction` aliases, the historical test
`CallFrame` alias, and the redundant `BufferState` alias. Detached tests now use
`DetachedBytecode<Value>` (or the compiler's primitive value type) and `VmActivation`.
Removed cross-owner reexports from runtime, heap, rooted code, debug, error, and
module routing. Runtime glob imports were replaced with explicit owner imports.
The embedding API gathers required contracts from those owners directly; raw
compiler metadata is not an embedding export. GC statistics remain available as
the return types of the public GC operations.

Adapters, applications, conformance tools, examples and tests use the new API.
Internal native-selector assertions moved from the CLI integration test into the
owning builtin unit tests. The `test-support` feature now exposes only a small
explicit lexer/numeric observation surface for differential tests. Detached VM
fixtures, allocation-failure injection and test-only helpers are absent from
production builds. Unused diagnostic accessors and constructors were removed.
The compiler attribute-checker callback no longer has a compatibility default.

ModuleLoader now has three hooks: `normalize`, `check_attributes`, and `load`.
All receive the initiating Context; `load` also receives request attributes and
returns ModuleLoadResult. The text-only and context-free adapter chains were
removed, and every loader implementation was migrated, including the conformance
runner. The heap's compatibility `run_gc` / liveness-only / hook-only entry points
were removed. Heap tests exercise the finalization-sink API directly, with an
explicit test fixture helper where no AtomTable exists.

Pending jobs have one `execute_pending_job` entry point returning the outcome
and originating context; the old boolean wrapper and `_with_context` name are
removed. Detached bytecode no longer carries an unused historical `name` field.
Error equality and Debug include the exact native message payload instead of
hiding it to preserve the previous Rust representation.

This changes the Rust API deliberately. ECMAScript and pinned QuickJS behavior,
including language aliases and the archive calling conventions, remains the
compatibility target. The native adapter no longer exports a root HostServices alias. Its opt-in
engine-test-support bridge identifies the distinct trait identity needed by
Cargo's engine unit-test build and is enabled only by the engine dev-dependency.

Source-layout checks reject root reexports, public engine implementation modules,
and runtime facade glob imports. Compile-fail documentation tests cover the old
root Runtime import and direct heap/VM imports. Binary-boundary snapshots and
mutation anchors follow the reviewed new source routing; historical Test262
receipts are not renewed by this refactor.

### API cleanup validation

- Rust 1.98.1 workspace/all-target/all-feature tests: 2,987 passed; one existing
  ignored oracle. The native-selector unit test moved from the CLI to the engine.
- Default-feature engine unit tests: 1,881 passed.
- CI Rust 1.88 workspace/all-target/all-feature check and default-library /
  workspace Clippy with warnings denied: passed. Rust 1.98.1 Clippy additionally
  rejects 44 style diagnostics (35 collapsible `if` statements, seven manual
  `is_multiple_of` patterns and two constant-size chunk patterns); these newer
  lint migrations are outside this API cleanup.
- Three compile-fail API boundary documentation tests: passed.
- Source layout: 414 reachable Rust files, 65 directory READMEs, no root aliases
  and no public engine implementation modules.
- Binary-object production scan and 14 checker unit tests: passed. The focused
  218-case mutation run passed after updating two relocated dependency diagnostic
  expectations and a moved import anchor. All 690 mutation targets remain
  applicable; this is not a rerun of the complete mutation suite.
- Host feature/dependency, Rust-only, anti-special-casing and oracle registry
  gates: passed.
- Optimized WASM build and Node smoke: all 15 playground examples passed,
  including catchable recursion overflow. No deployment was performed.
- Full Test262 was not rerun; historical conformance receipts remain historical.

### Explicit unlinked binding construction (2026-09-09)

`UnlinkedFunction::new` now requires argument definitions, local definitions and
closure descriptors from its producer. The compiler passes its already-computed
binding metadata directly, without constructing unnamed defaults first. The two
BC5 publication paths explicitly supply the ordinary bindings admitted by their
validated profiles. The separate closure constructor and production binding
replacement builder were removed.

Hand-forged bytecode tests use explicitly named fixture builders in
`engine/code/function/fixtures.rs`, compiled only under `cfg(test)`. These builders
can generate ordinary unnamed frame bindings and replace them for validation
negative cases. They are test data construction, not production API aliases.
Function-name and parameter-environment attachment still maintain their associated
special local descriptors. Publication remains responsible for validating drafts.

Validation on Rust 1.88: workspace/all-target/all-feature tests passed (2,987 passed,
one existing ignored test); the final engine unit-test rerun passed 1,900 tests;
workspace/all-target/all-feature Clippy with warnings denied passed. The constructor
test now supplies authored definitions directly. Source layout covers 415 Rust
files and 65 directory READMEs; the binary-boundary scan and 14 checker unit tests
passed. Frozen source fingerprints were updated after reviewing the constructor
routing and the two explicit BC5 binding layouts. The focused 218-case boundary
mutation suite passed on this revision. Full Test262 was not rerun.

The earlier API-cleanup validation records successful checks on that revision;
it did not establish that every historical construction fallback had been found.
This follow-up closes the identified unlinked-function fallback.

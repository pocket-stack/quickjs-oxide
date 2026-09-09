# Responsibility boundaries

Status: candidate A implemented as source/ and regexp/ siblings of engine/ under src/. The root contains only lib.rs as Rust source. See
[workspace architecture](architecture.md) for the concrete directories and
[the research report](reports/interpreter-architecture.html#rust) for the decision
and alternatives. This supersedes the earlier separate-compiler proposal.

## Design premise

Engine is the complete interpreter product. Compiler, code, values, objects,
atoms, heap, VM, realms, builtins, modules and jobs share one runtime model inside
one crate. There is no separate core or compiler package and no requirement for
cross-runtime compilation products, background compilation or a stable bytecode
format. Prefer concrete modules and restricted visibility over new traits.

A module's shared use does not make it a separate library. Source, regex, Unicode,
string and numeric support remain named responsibilities. Their current
consumers are inside this product, so their package extraction is not a prerequisite.
Native and browser providers are separate adapters because they implement
independent environment services.

## Ownership and interfaces

| Responsibility | Owns | Boundary |
| --- | --- | --- |
| Source | Exact bytes, carrier text, positions and spans | No parsing policy, terminal rendering or live object ownership |
| Compiler | Lexing, parsing, scopes, resolution and lowering | Explicit source/options and shared engine compilation types; no provider selection |
| Code | Instructions, constants, function/module drafts, verification and debug records | Publication authenticates heap identity and owns runtime roots |
| Value | UTF-16 strings, numeric representations, primitives, Object and Symbol values | Runtime coercion may execute user code; primitive helpers cannot replace it |
| Atom and heap | Name identity, storage, retention, tracing and collection | Compiler, code and VM use the same lifetime model |
| Object and VM | Properties, calls, execution, exceptions and suspension | Observable JS behavior remains in engine |
| Realm and builtins | Context construction, intrinsic relationships and language behavior | No concrete OS/browser provider implementation |
| Modules and jobs | Module instances, graph execution, queued computation and retained roots | Applications supply loader policy and drive the queue |
| Regexp | Pattern syntax, compiled programs, matching and interruption | JS RegExp properties, coercion and callbacks belong to builtins |
| Unicode | Character properties, case mapping, normalization and generated data | Consumers apply parser, regex and builtin policy |
| Host | Clock, local timezone, random seed and output capability contracts | No concrete environment access or alternate JS value representation |
| API | Runtime/Context entrypoints, rooted public handles and explicit host injection | One engine-owned object and value system |
| Adapters | Native and browser capability implementations | No JS coercion or VM policy |
| Applications | Inputs, loading policy, diagnostics, output conversion and process behavior | Use embedding APIs |
| Conformance | Admission, harnesses, scheduling and authenticated reports | Test-only hooks remain opt-in; each worker creates its own runtime |

## Compilation and lifetime

Unlinked compilation products remain an internal preparation stage. Their
existence does not establish an independent compiler product or require a
separate bytecode-contract crate. Engine verifies and publishes them
transactionally before execution. Compile failure, name retention, nested
functions and runtime-owned roots keep their existing cleanup rules.

`PrimitiveValue` remains a restricted constant representation. `Value` represents
all JS values, including Object and Symbol identity. Symbol creation and registry
lookup happen during execution. Compiler constants do not acquire live Object
or Symbol roots simply because compiler now shares the engine crate.

Compile and regex errors become JS exceptions where required. Module-attribute
host failures preserve the exact thrown value at the runtime boundary; they are
not stringified into parser diagnostics.

## Dependency and visibility rules

1. Engine production dependencies must not include its providers, applications
   or conformance runner. Adapters depend on the engine host contract.
2. Compiler may share the engine's code, names, strings and primitive algorithms.
   It does not choose the active realm or perform environmental side effects.
3. Builtins own complete runtime coercion, property access and user-code calls.
   Regex matching and source handling do not invoke arbitrary JS behavior.
4. Internal state and operations shared across sibling modules use
   `pub(crate)`. Narrower subsystem boundaries, especially binary-object
   intermediate representations, retain their existing visibility.
5. Engine owns pending computation and its roots. Applications decide when to
   run jobs and how to obtain module source.
6. New public embedding capabilities belong in `api`; retained root-level
   aliases preserve existing consumers without copying implementation.

## Verification scenarios

Existing tests cover compilation failures, nested functions and closures, eval
and dynamic functions, host exceptions and re-entry, module linking, pending
jobs, GC and source locations. Native and Web adapters must still construct the
same engine. Tooling must scan the new paths, including adapters and conformance.

Frozen oracle and Test262 evidence is historical data. Reorganization must not
silently rewrite its source commit or claim a fresh full-suite result. Any new
conformance receipt requires an actual run. Architecture alone makes no claim
about speed, peak memory or increased language coverage.

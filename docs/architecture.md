# Workspace architecture

The repository implements candidate A from the [architecture report](reports/interpreter-architecture.html#rust).
One root package, quickjs-oxide, contains the complete interpreter. Its only
top-level Rust source file is src/lib.rs.

```text
src/
  lib.rs
  source/                 authored source, coordinates and Unicode text support
  regexp/                 regex compilation, programs, matching and interruption
  engine/
    compiler/             lexer, parser, scopes and code generation
    code/                 code representation, verification and publication
    value/                values, strings, numbers and runtime conversions
    object/               object handles, properties, shapes and internal methods
    atom/                 interned names, symbols and property keys
    heap/                 raw records, storage operations, roots and collection
    vm/                   protocols, frames, calls, dispatch, unwinding and suspension
    realm/                global bindings, prototypes and initialization
    builtins/             language builtins and native call dispatch
    modules/              loaders, module instances, linking and evaluation
    jobs/                 queued computations, retained roots and cleanup
    host/                 environment capability contracts
    api/                  Runtime/Context and embedding operations
```

Every source directory has a README describing ownership, dependencies and its
direct files/children. Start at [src/README.md](../src/README.md).

## Responsibilities and storage

Runtime is defined in api/runtime.rs. RuntimeInner and RuntimeState belong to
heap/runtime: they own the shared heap/atom domain and cleanup. Runtime methods
live with their behavior: property operations in object, conversion in value,
global bindings in realm, publication in code, execution in vm and builtin
algorithms in builtins. This uses ordinary inherent implementations of one
Runtime type; it does not introduce forwarding wrappers or parallel runtimes.

Heap records describe retained raw data. The *_records modules define identities,
payloads and state; *_storage modules maintain references and storage transitions.
Language behavior remains in the appropriate owner, even when its raw state is
stored by heap. Native builtin selectors now belong to builtins/native.

VM protocol, completion states, activation/suspension, frame execution, instruction
dispatch, numeric execution and unwinding have separate files. The private
detached test host is compiled only for unit tests. Public Value and rooted
handles continue to have one representation.

source and regexp are sibling Rust modules of engine. They currently share the
engine's exact UTF-16 string carrier; they are not standalone Cargo packages.
Unicode algorithms and their fixed generated tables belong to source/unicode.
The JS RegExp object shell remains in engine/builtins/regexp.

## Public and private boundaries

src/lib.rs declares source, regexp and engine. Only engine::api is public inside
the engine; embeddings use that boundary directly. Legacy root aliases and
cross-owner runtime/heap reexports have been removed. The test-support feature
provides a small explicit differential-testing surface; detached VM fixtures are
compiled only for unit tests. API entry points do not contain a second implementation.

The bytecode decoder is a private child of engine/code. Only
code/binary_object_publish consumes its archive models. Decoder intermediate
visibility is restricted to the actual code/binary_object ancestry. The
ConstructorRef capability remains opaque. Splitting sibling VM and heap helpers
uses their enclosing module visibility without making them public APIs.

## Packages and hosts

| Package directory | Production workspace dependencies |
| --- | --- |
| Repository root: quickjs-oxide | None |
| adapters/native: quickjs-oxide-host | quickjs-oxide |
| adapters/web: quickjs-oxide-web-host | quickjs-oxide |
| apps/cli | quickjs-oxide, native adapter |
| apps/web | quickjs-oxide, web adapter |
| conformance/test262 | quickjs-oxide with test262-host, native adapter |

Applications construct Runtime::new_with_host_services(provider), choosing the
native or browser provider. The main package has no production adapter dependency.
Unit tests use a native dev dependency and a test-only HostServices identity
bridge. Applications own files, loader policy, output presentation and job driving.

## Verification

Workspace tests cover module behavior and existing integration/oracle scenarios.
The binary boundary checker follows registered physical owners and verifies their
Rust module routes; missing or conditionally disconnected evidence fails the gate.
Mutation fixtures follow the same ownership map. Full Test262 receipts remain
authenticated against their historical source; a source refactor does not renew
a conformance claim.

See [the refactor record](reports/candidate-a-refactor.md) for validation results
and the distinction between completed checks and full suites that were not rerun.

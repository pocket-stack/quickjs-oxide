# quickjs-oxide

`quickjs-oxide` is an independent, from-scratch Rust rewrite of QuickJS. Its
compatibility target is the official **QuickJS 2026-06-04** release and its
ES2025 behavior.

This is a runnable but incomplete implementation. It is not yet a drop-in
replacement for QuickJS, and passing the current tests does not imply full
feature parity. See the [current implementation status](docs/status.md) for the
audited boundary and the [feature-parity contract](docs/parity.md) for the
definition of completion.

## Project principles

- Product code is Rust-only. It does not compile, link, embed, or delegate
  execution to the QuickJS C engine.
- The implementation follows QuickJS semantics and architecture where those
  choices affect observable behavior: lexer, compiler, bytecode VM, runtime,
  realms, objects, errors, and memory ownership are real Rust components.
- The official QuickJS binary is used only as a differential-test oracle.
- `unsafe` Rust is forbidden by the crate lint configuration.

Rust-only is an implementation boundary, not a security certification. The
project is pre-1.0, incomplete, and has not been presented as a hardened
sandbox for untrusted production workloads.

## What works today

The current repository provides:

- a `qjs` executable for `-e` evaluation and UTF-8 script files;
- a Rust lexer-to-compiler-to-bytecode-to-VM execution path with no external
  engine fallback;
- primitive values, UTF-16 strings, arbitrary-precision BigInts, symbols,
  objects, properties, accessors, functions, closures, realms, and native
  Error objects on the implemented paths;
- a practical subset of expressions, calls, construction, member access and
  assignment, operators, ordinary function expressions, and control flow;
- function-local `var`, plus simple identifier `let`/`const` declarations in
  ordinary function bodies, nested brace blocks, the shared scope of a
  `switch`, and classic `for` heads (including the nested forms in scripts),
  with TDZ and pinned-QuickJS captured-cell lifetimes across re-entry and
  abrupt control flow;
- selected Function, Number, Boolean, Symbol, BigInt, String, Object-prototype,
  global numeric, and URI behavior;
- filename, source-position, stack, strip-source, and strip-debug support for
  the implemented compiler/runtime paths;
- Rust-only and differential test gates pinned to QuickJS 2026-06-04.

The supported surface is deliberately narrower than JavaScript or QuickJS as a
whole. Exact details and evidence live in [docs/status.md](docs/status.md).

## Important gaps

Among the capabilities not yet complete are:

- program/global declaration instantiation, including direct `var`, function,
  and lexical declarations;
- lexical destructuring, including destructuring in loop heads, and the
  remaining lexical environments;
- `for-in`/`for-of`/`for-await` and `try`/`catch`/`finally`;
- function declarations and hoisting, arrow functions, classes, generators,
  async functions, and the complete `arguments` behavior;
- object/array literal coverage, Arrays and iterators, Proxy and Reflect,
  RegExp, TypedArrays, Atomics, WeakRef, Promises, and most remaining builtins;
- direct `eval`, `with`, modules, jobs, workers, `std`/`os`, and the event loop;
- the full QuickJS CLI and REPL, `qjsc`, upstream bytecode/BJSON compatibility,
  and complete Rust and C embedding APIs;
- complete recoverable out-of-memory, interruption, and termination behavior.

Unsupported syntax or behavior should fail explicitly rather than silently
falling back to another engine or pretending that the feature works.

## Requirements

- Rust 1.85 or newer
- Cargo

The full parity gate also needs `rg` and either `sha256sum` or `shasum`. It can
reuse `QJS_ORACLE` directly when that executable has the matching
`libunicode-table.h` beside it; otherwise the Unicode check falls back to the
pinned oracle cache. Populating that cache may additionally need network
access, `curl`, `tar`, `make`, and a C toolchain. The Rust engine itself builds
and runs without the oracle.

## Build

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
cargo build --release
```

The release executable is written to `target/release/qjs`.

## Run

Evaluate source text:

```sh
cargo run --bin qjs -- -e '(function (a) { return a + 1; })(41)'
```

Run a script file:

```sh
cargo run --bin qjs -- path/to/script.js
```

Inspect the currently implemented command-line options:

```sh
cargo run --bin qjs -- --help
cargo run --bin qjs -- --version
```

Like upstream QuickJS, `qjs -e` does not print the expression's completion
value. Exceptions and uncaught thrown values are written to standard error.

The crate also exposes the current Rust runtime API:

```rust
use quickjs_oxide::{Runtime, Value};

let runtime = Runtime::new();
let mut context = runtime.new_context();
let value = context.eval("(function (a) { return a + 1; })(41)").unwrap();
assert_eq!(value, Value::Int(42));
```

The embedding API is still evolving and is not a claim of QuickJS C API
compatibility.

## Test

Run the Rust test suite:

```sh
cargo test --locked --workspace --all-targets
```

Run formatting, linting, and the product-boundary audit directly:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
./scripts/check-rust-only.sh
```

Run one differential suite against an existing official QuickJS build:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_primitives -- --nocapture
```

Run the complete current parity-slice gate:

```sh
./scripts/test-parity-slice.sh
```

When `QJS_ORACLE` is unset, that script builds the checksum-pinned upstream
oracle, verifies the generated Unicode identifier tables, and then runs
formatting, tests, Clippy, and the Rust-only audit.

## Documentation

- [Implementation status](docs/status.md): implemented behavior, known gaps,
  and reproducible evidence for the current revision.
- [Feature-parity contract](docs/parity.md): the acceptance criteria for
  claiming semantic feature parity with QuickJS.
- [Pinned upstream metadata](compat/upstream.toml): release URLs, checksums,
  bytecode version, Unicode version, and Test262 baseline.
- [NOTICE](NOTICE): upstream attribution and generated-data notices.

Keep milestone bookkeeping and detailed parser/opcode notes in the status
document rather than expanding this README.

## License

`quickjs-oxide` is licensed under the [MIT License](LICENSE). QuickJS and
Unicode notices retained for compatibility work and generated data are in
[LICENSES](LICENSES/) and [NOTICE](NOTICE).

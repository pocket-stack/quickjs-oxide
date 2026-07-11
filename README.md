# quickjs-oxide

`quickjs-oxide` is an independent, from-scratch Rust rewrite of QuickJS. It
targets the semantics of the official **QuickJS 2026-06-04** release and its
ES2025 behavior.

The engine is runnable, but incomplete and pre-1.0. It is not yet a drop-in
replacement for QuickJS or a hardened sandbox for untrusted production code.

## Design

- Product code is Rust-only: QuickJS C code is never linked, embedded, or used
  as an execution fallback.
- The lexer, compiler, bytecode VM, runtime, realms, objects, errors, and memory
  ownership are native Rust components informed by upstream QuickJS behavior.
- The official QuickJS binary is used only as a pinned differential-test
  oracle.
- `unsafe` Rust is forbidden by the crate lint configuration.

## Quick start

Rust 1.85 or newer is required.

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
cargo run --bin qjs -- -e '(function (a) { return a + 1; })(41)'
cargo run --bin qjs -- path/to/script.js
```

Like upstream QuickJS, `qjs -e` does not print an expression's completion
value. Uncaught exceptions are written to standard error.

The current Rust API can also evaluate source directly:

```rust
use quickjs_oxide::{Runtime, Value};

let runtime = Runtime::new();
let mut context = runtime.new_context();
let value = context.eval("(function (a) { return a + 1; })(41)").unwrap();
assert_eq!(value, Value::Int(42));
```

The embedding API is still evolving and does not yet claim QuickJS C API
compatibility.

## Compatibility status

The repository has a real lexer-to-VM execution path, core values and objects,
closures and realms, native errors, a practical expression/control-flow
subset, and selected built-ins. Simple-name `let` and `const` declarations now
work in Program code and the implemented local lexical scopes, including TDZ
and captured-cell behavior.

Large parts of JavaScript and the QuickJS host surface remain unfinished. The
current frontier includes the remaining Program declaration paths, lexical
destructuring and environments, advanced functions and classes, iterators and
async behavior, modules, jobs, workers, the full CLI and REPL, `qjsc`, and
embedding compatibility. Unsupported paths must fail explicitly rather than
silently delegate to another engine.

See [docs/status.md](docs/status.md) for the exact implemented boundary,
known gaps, upstream anchors, and reproducible evidence. The acceptance bar for
the eventual parity claim is defined in [docs/parity.md](docs/parity.md).

## Verify

Run the Rust suite:

```sh
cargo test --locked --workspace --all-targets
```

Run the complete current parity-slice gate:

```sh
./scripts/test-parity-slice.sh
```

The gate checks formatting, tests, Clippy, the Rust-only boundary, generated
Unicode data, and differential behavior against the pinned QuickJS oracle. Set
`QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs` to reuse a matching local build.

## Reference

- [Implementation status](docs/status.md)
- [Feature-parity contract](docs/parity.md)
- [Pinned upstream metadata](compat/upstream.toml)
- [Attribution and generated-data notices](NOTICE)

Detailed milestone bookkeeping belongs in the status document, not this
README.

## License

`quickjs-oxide` is available under the [MIT License](LICENSE). QuickJS and
Unicode notices are retained in [LICENSES](LICENSES/) and [NOTICE](NOTICE).

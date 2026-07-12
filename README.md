# quickjs-oxide

`quickjs-oxide` is an independent, from-scratch Rust rewrite of QuickJS,
targeting the semantics of the official **QuickJS 2026-06-04** release and its
ES2025 behavior.

The engine is runnable, but incomplete and pre-1.0. It is not yet a drop-in
replacement for QuickJS or a hardened sandbox for untrusted production code.
Product code is Rust-only: QuickJS C code is neither linked nor used as an
execution fallback; the pinned upstream binary is only a differential-test
oracle. The crate forbids `unsafe` Rust.

## Run

Rust 1.85 or newer is required.

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
cargo run --bin qjs -- -e '(function (a) { return a + 1; })(41)'
cargo run --bin qjs -- path/to/script.js
```

Like upstream QuickJS, `qjs -e` does not print an expression's completion
value. Uncaught exceptions are written to standard error.

The evolving Rust API can also evaluate source directly:

```rust
use quickjs_oxide::{Runtime, Value};

let runtime = Runtime::new();
let mut context = runtime.new_context();
let value = context.eval("(function (a) { return a + 1; })(41)").unwrap();
assert_eq!(value, Value::Int(42));
```

## Status

Semantic feature parity with the pinned QuickJS release is the goal. The
repository already has a native lexer-to-bytecode-VM execution path, but large
parts of JavaScript and the QuickJS host/API surface remain unfinished.
Unsupported paths must fail explicitly instead of delegating to another
engine.

- [Current implementation boundary and milestone evidence](docs/status.md)
- [Feature-parity acceptance contract](docs/parity.md)
- [Pinned upstream metadata](compat/upstream.toml)
- [Attribution and generated-data notices](NOTICE)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-parity-slice.sh
```

The parity gate also checks formatting, Clippy, the Rust-only boundary,
generated Unicode data, and differential behavior. Set
`QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs` to reuse a matching oracle build.

## License

`quickjs-oxide` is available under the [MIT License](LICENSE). QuickJS and
Unicode notices are retained in [LICENSES](LICENSES/) and [NOTICE](NOTICE).

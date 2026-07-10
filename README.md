# quickjs-oxide

`quickjs-oxide` is a from-scratch, memory-safe Rust rewrite of QuickJS. The
compatibility target is the upstream **QuickJS 2026-06-04** release and its
ES2025 behavior, not merely a JavaScript-like language.

The repository is still an incomplete rewrite. The current vertical slices
execute primitive expressions, named ordinary functions and closures, and
defining-realm global bindings through the real lexer, late scope resolver,
bytecode and VM. They also implement runtime-owned
Atom/Object/Shape/Context/FunctionBytecode/VarRef nodes, ordinary properties
and accessors, native Error objects, per-function filename/PC/source debug
metadata, QuickJS-style eager `Error.stack` for the current synchronous
bytecode/native frame chain, and the first ordered `%Function.prototype%`
slice (`caller`, `arguments`, `call`, `apply`, `bind`, `toString`, and
ordinary/bound `@@hasInstance`). Bound functions have a dedicated traced heap
payload and forward calls, construction, `new.target`, realm lookup, and
instance checks along the QuickJS paths; function source text and native
fallback templates are also observable through `Function.prototype.toString`.
The implemented VM operators use completion-aware QuickJS-style Number/default
`ToPrimitive` and ordered `ToNumeric` paths, including exact BigInt/String
relational comparison and preservation of user-thrown coercion values.
Explicit reference counting and cycle removal own those metadata, frame and
intrinsic roots. Passing these tests proves only those slices;
it does not imply QuickJS feature parity. Source/debug stripping, recoverable
OOM behavior, the complete Function intrinsic/debug-getter surface, and most
of the language and builtin library remain unfinished. See
[`docs/parity.md`](docs/parity.md) for the full completion contract and
[`docs/status.md`](docs/status.md) for the current audited boundary.

## Design constraints

- Product crates are Rust-only: no QuickJS C source, generated bindings, FFI,
  or native QuickJS linkage.
- The compiler targets a stack bytecode VM, following QuickJS's execution
  model rather than interpreting an AST.
- Runtime and context/realm lifetimes remain distinct.
- Atoms, shapes, property descriptors, deterministic reference counting with
  cycle removal, jobs, modules, RegExp, Unicode, BigInt, and the standard
  library are parity requirements rather than optional extensions.
- A separately installed upstream `qjs` may be used only as a differential-test
  oracle.

## Development

```sh
cargo test --workspace --all-targets
cargo run --bin qjs -- -e '(function(a) { return a + 1; })(41)'
```

`qjs -e` intentionally follows upstream and does not print the expression's
completion value. To run the implemented primitive and Error-backtrace
differential suites against a separately built official release:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_primitives -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_error_stacks -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_vm_object_coercion -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_function_bind_to_string -- --nocapture
```

Or run the complete current gate—including checksum-verified oracle setup,
formatting, tests, Clippy, and the Rust-only dependency audit—with one command:

```sh
./scripts/test-parity-slice.sh
```

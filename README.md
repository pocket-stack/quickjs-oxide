# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. R3ao adds a QuickJS-shaped
DataView with all 11 getter/setter families and fixed or length-tracking views
over resizable ArrayBuffers. Its scoped Test262 gate is 984/984 in both Oxide
and pinned QuickJS; the complete vector is 51,707/102,037 while modules,
TypedArrays, SharedArrayBuffer, and broad built-in coverage remain incomplete.
Pinned QuickJS is the test oracle, never a product dependency; detailed
bookkeeping lives in the status documents.

## Try it

Rust 1.85 or newer is required.

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
./scripts/demo-42.sh  # 42
cargo run --quiet --bin qjs -- --print-result -e \
  '(function (a) { return a + 1; })(41)'  # 42
```

## Status

- [Implementation status and milestone ledger](docs/status.md)
- [Pinned Test262 progress baseline](docs/test262.md)
- [Parity acceptance contract](docs/parity.md)
- [Pinned upstream release](compat/upstream.toml)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-test262-array-buffer.sh
./scripts/test-test262-data-view.sh
./scripts/test-test262-proxy.sh
./scripts/test-test262-full.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

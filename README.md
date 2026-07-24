# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. The current R3ah milestone
adds public instance/static class async-generator methods by composing
QuickJS's Method grammar, class publication path, and existing async-generator
driver. Pinned QuickJS passes all 573 focused candidate paths; 256 explicit
frontier exclusions leave 317 paths and 606/606 passing Oxide variants.
Private async-generator methods, `yield*`, `for await`, async iterator closing,
modules, Proxy, and broad built-in coverage remain incomplete. The conservative
full vector gains six passes to 43,686/102,037 with no previous-pass
regression. Pinned QuickJS is the test oracle, never a product dependency. See
the status documents for detailed bookkeeping.

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
./scripts/test-test262-class-public-init.sh
./scripts/test-test262-class-private-fields.sh
./scripts/test-test262-class-private-{methods,accessors}.sh
./scripts/test-test262-class-generator-methods.sh
./scripts/test-test262-class-private-generator-methods.sh
./scripts/test-test262-class-sync-matrix.sh
./scripts/test-test262-promise-{race-try-with-resolvers,finally,all,all-settled,any}.sh
./scripts/test-test262-regexp-builtins.sh
./scripts/test-test262-generator-destructuring.sh
./scripts/test-test262-iterator-helpers.sh
./scripts/test-test262-iterator-sequencing.sh
./scripts/test-test262-async-{function,arrow,object-method,class-method,private-class-method,generator}-core.sh
./scripts/test-test262-async-generator-object-method-core.sh
./scripts/test-test262-async-generator-class-method-core.sh
./scripts/test-r3z-async-function-core-oracle.sh --oxide ./target/debug/qjs
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

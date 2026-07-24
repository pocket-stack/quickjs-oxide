# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. The current R3ad milestone
adds public ordinary async instance/static class methods on the existing
`Method+Async` and `HomeObject` path. Pinned QuickJS passes all 313 candidate
paths; 19 private-async/async-generator exclusions leave 294 paths and 568/568
passing Oxide variants. The conservative full vector is 43,661/102,037 with no
previous-pass regression. Private async class methods, async generators,
modules, Proxy, and broad built-in coverage remain incomplete. Pinned QuickJS
is the test oracle, never a product dependency. See the status documents below
for historical milestones and reproducible R3ad evidence.

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
./scripts/test-test262-async-function-core.sh
./scripts/test-test262-async-arrow-core.sh
./scripts/test-test262-async-object-method-core.sh
./scripts/test-test262-async-class-method-core.sh
./scripts/test-r3z-async-function-core-oracle.sh --oxide ./target/debug/qjs
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

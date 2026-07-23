# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. The current R3ab milestone
adds async arrows with pinned QuickJS token timing and lexical-capture
semantics. Its canonical language tree has no exclusions: Oxide passes 110/110
variants across 60 paths, and QuickJS 2026-06-04 passes all 60 paths. The
conservative full vector is 43,655/102,037 after 12 already-admitted consumers
advance, while broad async feature/host admission stays fail-closed until
async methods and generators are complete. Modules, Proxy, and broad built-in
coverage also remain incomplete. Pinned QuickJS is the test oracle, never a
product dependency. See the status documents below for the R3z/R3aa history
and reproducible R3ab evidence.

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
./scripts/test-r3z-async-function-core-oracle.sh --oxide ./target/debug/qjs
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

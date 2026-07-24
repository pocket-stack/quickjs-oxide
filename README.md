# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. The current R3aj milestone
adds async-generator `yield*` delegation, including async iterators and
Async-from-Sync adaptation. Pinned QuickJS passes all 775 focused paths, and
Oxide passes all 1,550 sloppy/strict variants with deterministic reports.
The complete 102,037-variant vector is byte-identical at 43,686 passes.
This is not complete async iteration: `for await` is the next frontier, while
closing an independently active outer iterator on `.return()` remains a
separate follow-up. Modules, Proxy, and broad built-in coverage also remain
incomplete. Pinned QuickJS is the test oracle, never a product dependency.
See the status documents for detailed bookkeeping.

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
./scripts/test-test262-async-generator-private-class-method-core.sh
./scripts/test-test262-async-generator-yield-star.sh
./scripts/test-r3z-async-function-core-oracle.sh --oxide ./target/debug/qjs
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

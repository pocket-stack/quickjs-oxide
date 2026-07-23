# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. R3u globally admits the
pinned synchronous `generators` + `destructuring-binding` cohort after Oxide
and pinned QuickJS both pass 6,593/6,593 variants. The conservative full
Test262 vector records 43,521/102,037 passes with no engine fault. Modules,
async functions/generators, Proxy, and broad built-in coverage remain
incomplete. Unsupported paths fail explicitly; pinned QuickJS is the semantic
oracle, not a product dependency.

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
./scripts/test-test262-promise-{race-try-with-resolvers,finally,all,all-settled,any}.sh
./scripts/test-test262-regexp-builtins.sh
./scripts/test-test262-generator-destructuring.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

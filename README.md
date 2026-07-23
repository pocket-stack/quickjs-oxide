# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free engine is runnable but incomplete. Synchronous classes and
generators are available. R3q completes the aggregate Promise statics:
`all` passes 196/196 focused Test262 variants, `allSettled` 208/208, and `any`
188/188. The earlier `race`/`try`/`withResolvers` gate passes 224/224;
`Promise.prototype.finally` passes 56/58, with only two Proxy-dependent
variants failing. This is not full-suite parity: modules, async
functions/generators, Proxy, and broad built-in coverage remain incomplete.
Unsupported paths fail explicitly; pinned QuickJS is only an oracle.

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
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

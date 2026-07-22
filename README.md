# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The `unsafe`-free Rust engine and CLI are runnable but incomplete. Its
synchronous class path includes public/private instance and static data fields,
static blocks, private-`in`, and ordinary private instance/static methods with
per-class-side brands. The focused R3g/R3h/R3i cohorts pass 767/767,
1,260/1,260, and 534/534 Test262 variants respectively. That is not a full-suite
parity claim: modules, async/generators, private accessors, async/generator class
forms, and broad built-in coverage remain incomplete. Unsupported paths fail
explicitly, and pinned QuickJS is used only as a test oracle.

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
./scripts/test-test262-class-private-methods.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

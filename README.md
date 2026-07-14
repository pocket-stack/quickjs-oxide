# quickjs-oxide

An independent Rust rewrite of QuickJS, targeting semantic feature parity with
the official **QuickJS 2026-06-04** release and its ES2025 behavior.

The engine is runnable but incomplete. It is not yet a drop-in replacement or
a production sandbox. Product code is Rust-only and forbids `unsafe`; the
pinned QuickJS binary is used only as a differential-test oracle.

## Quick start

Rust 1.85 or newer is required.

```sh
git clone https://github.com/pocket-stack/quickjs-oxide.git
cd quickjs-oxide
./scripts/demo-42.sh
cargo run --bin qjs -- -e '(function (a) { return a + 1; })(41)'
cargo run --bin qjs -- path/to/script.js
```

## Project status

Semantic parity is the goal; unsupported paths fail explicitly instead of
falling back to another engine. Detailed progress and maintenance records live
outside this overview:

- [Implementation status and milestone ledger](docs/status.md)
- [Pinned Test262 progress baseline](docs/test262.md)
- [Parity acceptance contract](docs/parity.md)
- [Pinned upstream release](compat/upstream.toml)

## Verify

```sh
cargo test --locked --workspace --all-targets
./scripts/test-parity-slice.sh
./scripts/test-test262-smoke.sh
```

## License

[MIT](LICENSE). Third-party notices are retained in [NOTICE](NOTICE) and
[LICENSES](LICENSES/).

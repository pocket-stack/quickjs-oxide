# Test ownership

- `src`: unit tests
  beside the implementation they inspect.
- `tests`: public Rust embedding integration tests.
- `apps/cli/tests/cli.rs`: qjs process behavior, using Cargo's binary path.
- `apps/cli/tests/oracle/main.rs`: one oracle integration executable organized
  into ordinary behavior modules, including opt-in Test262 host cases.
- `apps/cli/tests/fixtures/inputs` and `expected`: fixed inputs and frozen outputs.
- `conformance/test262/src`: runner unit tests; active generated data remains under
  `dev-support/test262`.

Use the owning package for a focused run, for example:

```sh
cargo test -p quickjs-oxide-cli --test cli
cargo test -p quickjs-oxide --lib
cargo test -p quickjs-oxide-cli --features test262-host --test oracle test262_
```

The shared oracle suite is intentionally not run for every small source move.

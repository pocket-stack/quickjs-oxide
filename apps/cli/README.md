# qjs command-line application

Owns arguments, files, module-loading policy, diagnostics and process exits. It uses quickjs-oxide with the native adapter. CLI/oracle integration tests and fixed fixtures live under tests.

```sh
cargo test -p quickjs-oxide-cli --test cli
```

See [workspace architecture](../../docs/architecture.md) for dependency and lifetime boundaries.

## Optional diagnostics

Build with `--features profiling` for `-d`, `-T`, `-q -d` and JSON Lines reports.
[Profiling documentation](../../docs/profiling.md) defines coverage and timing
boundaries. `src/profiling.rs` owns CLI collection policy and output formatting.

# Test262 conformance runner

Owns test admission, metadata, harness construction, scheduling, execution and reporting. Package name remains quickjs-oxide-test262 and the binary is run-test262. It explicitly enables test262-host; applications retain their normal defaults. Frozen inputs and receipts live under dev-support/test262; preparation and verification live under scripts/test262.

```sh
cargo test -p quickjs-oxide-test262
```

See [workspace architecture](../../docs/architecture.md) for dependency and lifetime boundaries.

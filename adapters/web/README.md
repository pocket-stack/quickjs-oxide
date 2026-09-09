# Browser host adapter

Implements engine HostServices using browser Date and Math services. It owns no WASM product exports or result formatting; those belong to apps/web. Package name is `quickjs-oxide-web-host`.

```sh
cargo check -p quickjs-oxide-web-host --target wasm32-unknown-unknown
```

See [workspace architecture](../../docs/architecture.md) for dependency and lifetime boundaries.

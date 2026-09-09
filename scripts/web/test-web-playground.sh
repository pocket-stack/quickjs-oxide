#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
wasm_target="wasm32-unknown-unknown"
wasm_stem="quickjs_oxide_web"
cargo_target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
if [[ "${cargo_target_dir}" != /* ]]; then
  cargo_target_dir="${repo_root}/${cargo_target_dir}"
fi
wasm_file="${cargo_target_dir}/${wasm_target}/web/${wasm_stem}.wasm"
node_dir="${repo_root}/target/web-playground-node"

cd "${repo_root}"

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep (rg) is required for the playground anti-delegation gate" >&2
  exit 1
fi

if ! grep -Fqx \
  'quickjs-oxide = { workspace = true, default-features = false }' \
  apps/web/Cargo.toml; then
  echo "web wrapper must path-depend on quickjs-oxide without dev-support features" >&2
  exit 1
fi

if ! grep -Fqx 'wasm-bindgen = "=0.2.126"' Cargo.toml \
  || ! grep -Fqx \
    'js-sys = { version = "=0.3.103", default-features = false }' \
    Cargo.toml; then
  echo "web bindings must stay exactly pinned with js-sys unsafe-eval disabled" >&2
  exit 1
fi

rust_host_pattern='js_sys::(eval|Function)|Function::new'
browser_host_pattern='(^|[^.$[:alnum:]_])eval[[:space:]]*[(]|(globalThis|window|self)[.]eval[[:space:]]*[(]|new[[:space:]]+Function[[:space:]]*[(]|(^|[^.$[:alnum:]_])Function[[:space:]]*[(]'
if rg -n "${rust_host_pattern}" apps/web adapters/web \
  || rg -n --glob '!pkg/**' "${browser_host_pattern}" apps/web/site; then
  echo "playground source must not delegate evaluation to the browser host" >&2
  exit 1
fi

./scripts/web/build-web-playground.sh

rm -rf "${node_dir}"
mkdir -p "${node_dir}"
wasm-bindgen \
  "${wasm_file}" \
  --out-dir "${node_dir}" \
  --out-name "${wasm_stem}" \
  --target nodejs \
  --no-typescript

node "${repo_root}/scripts/web/test-web-playground-node.mjs"

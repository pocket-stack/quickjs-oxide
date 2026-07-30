#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wasm_bindgen_version="0.2.126"
wasm_target="wasm32-unknown-unknown"
wasm_package="quickjs-oxide-web"
wasm_stem="quickjs_oxide_web"
pages_dir="${repo_root}/target/pages"
site_dir="${repo_root}/web/site"
cargo_target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
if [[ "${cargo_target_dir}" != /* ]]; then
  cargo_target_dir="${repo_root}/${cargo_target_dir}"
fi
wasm_file="${cargo_target_dir}/${wasm_target}/web/${wasm_stem}.wasm"

if ! rustup run stable rustc --version >/dev/null 2>&1; then
  echo "missing the rustup stable toolchain; run: rustup toolchain install stable" >&2
  exit 1
fi

stable_rustc="$(rustup which --toolchain stable rustc)"
stable_rustdoc="$(rustup which --toolchain stable rustdoc)"

if ! rustup target list --installed --toolchain stable | grep -Fxq "${wasm_target}"; then
  echo "missing Rust target ${wasm_target}; run:" >&2
  echo "  rustup target add --toolchain stable ${wasm_target}" >&2
  exit 1
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "missing wasm-bindgen ${wasm_bindgen_version}; run:" >&2
  echo "  cargo install wasm-bindgen-cli --version ${wasm_bindgen_version} --locked" >&2
  exit 1
fi

actual_wasm_bindgen_version="$(wasm-bindgen --version | awk '{print $2}')"
if [[ "${actual_wasm_bindgen_version}" != "${wasm_bindgen_version}" ]]; then
  echo "wasm-bindgen CLI ${actual_wasm_bindgen_version} does not match crate ${wasm_bindgen_version}" >&2
  exit 1
fi

if [[ ! -d "${site_dir}" ]]; then
  echo "missing static playground source: ${site_dir}" >&2
  exit 1
fi

cd "${repo_root}"
RUSTC="${stable_rustc}" RUSTDOC="${stable_rustdoc}" rustup run stable cargo build \
  --locked \
  --profile web \
  --target "${wasm_target}" \
  --package "${wasm_package}"

rm -rf "${pages_dir}"
mkdir -p "${pages_dir}/pkg"
cp -R "${site_dir}/." "${pages_dir}/"
wasm-bindgen \
  "${wasm_file}" \
  --out-dir "${pages_dir}/pkg" \
  --out-name "${wasm_stem}" \
  --target no-modules \
  --no-typescript

test -f "${pages_dir}/index.html"
test -f "${pages_dir}/pkg/${wasm_stem}.js"
test -f "${pages_dir}/pkg/${wasm_stem}_bg.wasm"

echo "Built GitHub Pages artifact at ${pages_dir}"

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wasm_bindgen_version="0.2.126"
wasm_target="wasm32-unknown-unknown"
wasm_package="quickjs-oxide-web"
wasm_stem="quickjs_oxide_web"
build_commit="${QUICKJS_OXIDE_COMMIT:-${GITHUB_SHA:-local}}"
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
QUICKJS_OXIDE_COMMIT="${build_commit}" \
  RUSTC="${stable_rustc}" \
  RUSTDOC="${stable_rustdoc}" \
  rustup run stable cargo build \
  --locked \
  --profile web \
  --target "${wasm_target}" \
  --package "${wasm_package}"

rm -rf "${pages_dir}"
mkdir -p "${pages_dir}/pkg"
cp -R "${site_dir}/." "${pages_dir}/"
node "${repo_root}/scripts/current-test262-metrics.mjs" \
  --render "${pages_dir}/index.html"
wasm-bindgen \
  "${wasm_file}" \
  --out-dir "${pages_dir}/pkg" \
  --out-name "${wasm_stem}" \
  --target no-modules \
  --no-typescript

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

replace_literal() {
  local file="$1"
  local old="$2"
  local new="$3"

  if ! grep -Fq "${old}" "${file}"; then
    echo "missing expected Pages reference ${old} in ${file}" >&2
    exit 1
  fi
  OLD="${old}" NEW="${new}" perl -0pi -e \
    's/\Q$ENV{OLD}\E/$ENV{NEW}/g' "${file}"
}

# The Pages CDN can expose fixed-path files from different deployments during
# propagation. The authenticated index therefore points through a fully
# content-addressed JavaScript chain to the exact generated WASM bytes.
glue_sha256="$(sha256_file "${pages_dir}/pkg/${wasm_stem}.js")"
wasm_sha256="$(sha256_file "${pages_dir}/pkg/${wasm_stem}_bg.wasm")"
examples_sha256="$(sha256_file "${pages_dir}/examples.js")"
glue_asset="${wasm_stem}.${glue_sha256}.js"
wasm_asset="${wasm_stem}_bg.${wasm_sha256}.wasm"
examples_asset="examples.${examples_sha256}.js"
cp "${pages_dir}/pkg/${wasm_stem}.js" "${pages_dir}/pkg/${glue_asset}"
cp "${pages_dir}/pkg/${wasm_stem}_bg.wasm" "${pages_dir}/pkg/${wasm_asset}"
cp "${pages_dir}/examples.js" "${pages_dir}/${examples_asset}"

replace_literal \
  "${pages_dir}/worker.js" \
  "./pkg/${wasm_stem}.js" \
  "./pkg/${glue_asset}"
replace_literal \
  "${pages_dir}/worker.js" \
  "./pkg/${wasm_stem}_bg.wasm" \
  "./pkg/${wasm_asset}"
worker_sha256="$(sha256_file "${pages_dir}/worker.js")"
worker_asset="worker.${worker_sha256}.js"
cp "${pages_dir}/worker.js" "${pages_dir}/${worker_asset}"

replace_literal \
  "${pages_dir}/app.js" \
  './examples.js' \
  "./${examples_asset}"
replace_literal \
  "${pages_dir}/app.js" \
  './worker.js' \
  "./${worker_asset}"
app_sha256="$(sha256_file "${pages_dir}/app.js")"
app_asset="app.${app_sha256}.js"
cp "${pages_dir}/app.js" "${pages_dir}/${app_asset}"

style_sha256="$(sha256_file "${pages_dir}/style.css")"
style_asset="style.${style_sha256}.css"
cp "${pages_dir}/style.css" "${pages_dir}/${style_asset}"
replace_literal "${pages_dir}/index.html" './style.css' "./${style_asset}"
replace_literal "${pages_dir}/index.html" './app.js' "./${app_asset}"

test -f "${pages_dir}/index.html"
test -f "${pages_dir}/pkg/${wasm_stem}.js"
test -f "${pages_dir}/pkg/${wasm_stem}_bg.wasm"
test -f "${pages_dir}/pkg/${glue_asset}"
test -f "${pages_dir}/pkg/${wasm_asset}"
test -f "${pages_dir}/${app_asset}"
test -f "${pages_dir}/${examples_asset}"
test -f "${pages_dir}/${style_asset}"
test -f "${pages_dir}/${worker_asset}"

echo "Built GitHub Pages artifact at ${pages_dir}"

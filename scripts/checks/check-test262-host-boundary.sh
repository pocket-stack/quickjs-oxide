#!/usr/bin/env bash
# Keep the Test262 host out of default library, CLI, and WASM builds.

set -euo pipefail
export LC_ALL=C

tool_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
script_dir=$(CDPATH= cd -- "$tool_dir/.." && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

die() {
    echo "error: $*" >&2
    exit 1
}

command -v cargo >/dev/null 2>&1 || die "cargo is required"
command -v python3 >/dev/null 2>&1 || die "python3 is required"

metadata=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-metadata.XXXXXX")
fake_runner=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-fake-runner.XXXXXX")
fake_runner_marker=$fake_runner.invoked
trap 'rm -f -- "$metadata" "$fake_runner" "$fake_runner_marker"' EXIT
cargo metadata --locked --format-version 1 --no-deps > "$metadata"

python3 "$tool_dir/test262-host-metadata.py" "$metadata"

printf '%s\n' '#!/bin/sh' \
    'printf invoked > "$0.invoked"' \
    "printf '%s\\n' 'run-test262 provenance: engine_semantics_sha256=0000000000000000000000000000000000000000000000000000000000000000'" \
    > "$fake_runner"
chmod +x "$fake_runner"
expected_override_error='error: TEST262_RUNNER is retired; use CARGO_TARGET_DIR to reuse Cargo-authenticated builds'
for runner_override in '' "$fake_runner" /definitely/stale/run-test262; do
    set +e
    TEST262_RUNNER="$runner_override" \
        ./scripts/test262/test-test262.sh --runner-provenance > "$metadata" 2>&1
    status=$?
    set -e
    [[ "$status" == 1 ]] || die 'retired TEST262_RUNNER override was not rejected'
    [[ "$(cat "$metadata")" == "$expected_override_error" ]] \
        || die 'retired TEST262_RUNNER rejection drifted or executed the override'
done
[[ ! -e "$fake_runner_marker" ]] || die 'retired TEST262_RUNNER executed a spoofing shim'

echo "Test262 host feature and runner provenance boundary passed."

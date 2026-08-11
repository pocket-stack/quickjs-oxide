#!/usr/bin/env bash
# Keep the Test262 host out of default library, CLI, and WASM builds.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

die() {
    echo "error: $*" >&2
    exit 1
}

command -v cargo >/dev/null 2>&1 || die "cargo is required"
command -v python3 >/dev/null 2>&1 || die "python3 is required"

metadata=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-metadata.XXXXXX")
trap 'rm -f -- "$metadata"' EXIT
cargo metadata --locked --format-version 1 --no-deps > "$metadata"

python3 - "$metadata" <<'PY'
import json
from pathlib import Path
import sys


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


metadata = json.loads(Path(sys.argv[1]).read_text())
packages = {package["name"]: package for package in metadata["packages"]}

engine = packages.get("quickjs-oxide")
if engine is None:
    fail("cargo metadata omitted the quickjs-oxide package")
if engine["features"].get("default") != []:
    fail("quickjs-oxide default feature set must stay empty")
if engine["features"].get("test262-host") != []:
    fail("quickjs-oxide must expose an empty, opt-in test262-host feature")

runner = [target for target in engine["targets"] if target["name"] == "run-test262"]
if len(runner) != 1:
    fail("cargo metadata must contain exactly one run-test262 target")
if runner[0].get("required-features") != ["test262-host"]:
    fail("run-test262 must require exactly the test262-host feature")

integration_targets = [
    target
    for target in engine["targets"]
    if target.get("kind") == ["test"]
]
integration_tests = {target["name"] for target in integration_targets}
expected_integration_tests = {
    "checked_string_construction",
    "cli",
    "oracle",
    "rust_only",
    "unsupported_diagnostics",
}
if (
    len(integration_targets) != len(expected_integration_tests)
    or integration_tests != expected_integration_tests
):
    fail(
        "quickjs-oxide integration targets must be exactly "
        f"{sorted(expected_integration_tests)}; found {sorted(integration_tests)}"
    )

oracle = [target for target in engine["targets"] if target["name"] == "oracle"]
if len(oracle) != 1:
    fail("cargo metadata must contain exactly one oracle target")
if oracle[0].get("required-features"):
    fail("the shared oracle target must not require a feature")

retired_host_targets = {
    "oracle_create_realm",
    "oracle_host_gc",
    "oracle_is_html_dda",
}
if integration_tests & retired_host_targets:
    fail("Test262 host oracles must be feature-gated modules in the shared oracle target")

web = packages.get("quickjs-oxide-web")
if web is None:
    fail("cargo metadata omitted the quickjs-oxide-web package")
engine_dependencies = [
    dependency
    for dependency in web["dependencies"]
    if dependency["name"] == "quickjs-oxide"
]
if len(engine_dependencies) != 1:
    fail("WASM wrapper must have exactly one quickjs-oxide dependency")
dependency = engine_dependencies[0]
if dependency["uses_default_features"] or dependency["features"]:
    fail("WASM wrapper must disable default features and enable no dev-support feature")

feature = '#[cfg(feature = "test262-host")]'


def require_gated(path: str, declarations: tuple[str, ...]) -> None:
    lines = Path(path).read_text().splitlines()
    for declaration in declarations:
        matches = [index for index, line in enumerate(lines) if line == declaration]
        if len(matches) != 1:
            fail(f"{path} must contain exactly one declaration: {declaration.strip()}")
        index = matches[0]
        attributes = []
        cursor = index - 1
        while cursor >= 0 and lines[cursor].strip().startswith("#["):
            attributes.append(lines[cursor].strip())
            cursor -= 1
        if feature not in attributes:
            fail(f"{path} must gate {declaration.strip()} with test262-host")


require_gated(
    "tests/oracle.rs",
    (
        "mod test262_create_realm;",
        "mod test262_host_gc;",
        "mod test262_is_html_dda;",
    ),
)
require_gated(
    "src/runtime.rs",
    (
        "mod test262_agent;",
        "mod test262_host;",
        "pub use self::test262_agent::{Test262AgentError, Test262AgentSession};",
        "use crate::heap::Test262AgentKind;",
        "    fn call_test262_gc(&self, invocation: NativeInvocation) -> Result<Completion, RuntimeError> {",
        "    fn set_object_is_html_dda(&self, object: &ObjectRef) -> Result<(), RuntimeError> {",
        "    pub fn new_code_point_range_function(&mut self) -> Result<CallableRef, RuntimeError> {",
        "    pub fn new_test262_gc_function(&mut self) -> Result<CallableRef, RuntimeError> {",
    ),
)
require_gated(
    "src/lib.rs",
    ("pub use runtime::{Test262AgentError, Test262AgentSession};",),
)
require_gated(
    "src/heap.rs",
    (
        "pub enum Test262AgentKind {",
        "    StringCodePointRange,",
        "    Test262DetachArrayBuffer,",
        "    Test262EvalScript,",
        "    Test262CreateRealm,",
        "    Test262IsHtmlDda,",
        "    Test262Gc,",
        "    Test262Agent(Test262AgentKind),",
        "    pub(crate) fn set_object_is_html_dda(&mut self, id: ObjectId) -> Result<(), HeapError> {",
    ),
)
require_gated(
    "src/runtime/native_dispatch.rs",
    (
        "            NativeFunctionId::StringCodePointRange => {",
        "            NativeFunctionId::Test262DetachArrayBuffer => {",
        "            NativeFunctionId::Test262EvalScript => {",
        "            NativeFunctionId::Test262CreateRealm => self.call_test262_create_realm(invocation),",
        "            NativeFunctionId::Test262IsHtmlDda => self.call_test262_is_html_dda(invocation),",
        "            NativeFunctionId::Test262Gc => self.call_test262_gc(invocation),",
        "            NativeFunctionId::Test262Agent(kind) => {",
    ),
)
require_gated(
    "src/runtime/intrinsics/array_buffer.rs",
    (
        "    pub(in crate::runtime) fn call_test262_detach_array_buffer(",
        "    pub fn new_detach_array_buffer_function(&mut self) -> Result<CallableRef, RuntimeError> {",
    ),
)
require_gated(
    "src/runtime/intrinsics/string.rs",
    ("    pub(in crate::runtime) fn call_string_code_point_range(",),
)
require_gated(
    "src/value.rs",
    (
        "    pub(crate) fn try_with_exact_capacity(capacity: usize) -> Result<Self, JsStringError> {",
    ),
)

gate = Path("scripts/test-test262.sh").read_text()
if "--features test262-host --bin run-test262" not in gate:
    fail("central Test262 gate must build run-test262 with test262-host")

gc_gate = Path("scripts/test-host-gc-reentrant-oracle.sh").read_text()
if "--features test262-host" not in gc_gate:
    fail("host GC differential must enable test262-host")

parity_gate = Path("scripts/test-parity-slice.sh").read_text()
if parity_gate.count("--features test262-host") < 2:
    fail("parity slice must test and lint the Test262 host feature")

workflow = Path(".github/workflows/ci.yml").read_text()
if "./scripts/check-test262-host-boundary.sh" not in workflow:
    fail("public fast CI must enforce the Test262 host boundary")

print("Test262 host feature boundary passed.")
PY

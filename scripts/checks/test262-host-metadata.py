import json
from pathlib import Path
import sys


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


metadata = json.loads(Path(sys.argv[1]).read_text())
packages = {package["name"]: package for package in metadata["packages"]}

expected_packages = {
    "quickjs-oxide", "quickjs-oxide-web-host",
    "quickjs-oxide-host", "quickjs-oxide-cli",
    "quickjs-oxide-web", "quickjs-oxide-test262",
}
if set(packages) != expected_packages:
    fail("workspace package set drifted from the reviewed engine/app split")
# Candidate A: providers depend on the complete engine, never the reverse.
expected_dependencies = {
    "quickjs-oxide": set(),
    "quickjs-oxide-host": {"quickjs-oxide"},
    "quickjs-oxide-web-host": {"quickjs-oxide"},
    "quickjs-oxide-cli": {"quickjs-oxide", "quickjs-oxide-host"},
    "quickjs-oxide-web": {"quickjs-oxide", "quickjs-oxide-web-host"},
    "quickjs-oxide-test262": {"quickjs-oxide", "quickjs-oxide-host"},
}
for name, expected in expected_dependencies.items():
    actual = {
        dependency["name"] for dependency in packages[name]["dependencies"]
        if dependency.get("kind") != "dev" and dependency.get("path") is not None
    }
    if actual != expected:
        fail(f"{name} production dependencies violate candidate A: {sorted(actual)}")

engine = packages["quickjs-oxide"]
if engine["features"].get("default") != []:
    fail("engine and embedding defaults must exclude Test262")
if engine["features"].get("test262-host") != []:
    fail("engine Test262 feature must be an opt-in internal capability")
runner_package = packages["quickjs-oxide-test262"]
runner = [target for target in runner_package["targets"] if target["name"] == "run-test262"]
if len(runner) != 1:
    fail("runner package must own exactly one run-test262 target")
runner_dependencies = [d for d in runner_package["dependencies"] if d["name"] == "quickjs-oxide"]
if len(runner_dependencies) != 1 or runner_dependencies[0]["features"] != ["test262-host"]:
    fail("runner must explicitly enable the Test262 host")
for package in packages.values():
    if any("custom-build" in t.get("kind", []) for t in package["targets"]):
        fail("workspace must not introduce an unhashed custom build target")
    for dependency in package["dependencies"]:
        if dependency.get("path") is not None and dependency["name"] not in expected_packages:
            fail("unreviewed path dependency is outside engine fingerprint coverage")
for name, expected in {
    "quickjs-oxide": {"checked_string_construction", "rust_only", "unsupported_diagnostics"},
    "quickjs-oxide-cli": {"cli", "oracle"},
}.items():
    targets = [t for t in packages[name]["targets"] if t.get("kind") == ["test"]]
    if {t["name"] for t in targets} != expected or any(t.get("required-features") for t in targets):
        fail(f"{name} must retain its ungated Cargo integration targets")

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


require_gated('apps/cli/tests/oracle/main.rs', ('mod test262_create_realm;', 'mod test262_host_gc;', 'mod test262_is_html_dda;'))
require_gated('src/engine/api/mod.rs', ('pub(crate) mod test262_agent;', 'pub(crate) mod test262_host;'))
require_gated('src/engine/heap/runtime_gc.rs', ('    pub(crate) fn call_test262_gc(',))
require_gated('src/engine/object/storage.rs', ('    pub(crate) fn set_object_is_html_dda(&self, object: &ObjectRef) -> Result<(), RuntimeError> {',))
require_gated('src/engine/api/context/test262.rs', ('    pub fn new_code_point_range_function(&mut self) -> Result<CallableRef, RuntimeError> {', '    pub fn new_test262_gc_function(&mut self) -> Result<CallableRef, RuntimeError> {'))
require_gated('src/engine/api/mod.rs', ('pub use crate::engine::api::test262_agent::{Test262AgentError, Test262AgentSession};',))
require_gated('src/engine/builtins/native.rs', ('pub enum Test262AgentKind {', '    StringCodePointRange,', '    Test262DetachArrayBuffer,', '    Test262EvalScript,', '    Test262CreateRealm,', '    Test262IsHtmlDda,', '    Test262Gc,', '    Test262Agent(Test262AgentKind),'))
require_gated('src/engine/heap/object_storage.rs', ('    pub(crate) fn set_object_is_html_dda(&mut self, id: ObjectId) -> Result<(), HeapError> {',))
require_gated('src/engine/builtins/dispatch.rs', ('            NativeFunctionId::StringCodePointRange => {', '            NativeFunctionId::Test262DetachArrayBuffer => {', '            NativeFunctionId::Test262EvalScript => {', '            NativeFunctionId::Test262CreateRealm => self.call_test262_create_realm(invocation),', '            NativeFunctionId::Test262IsHtmlDda => self.call_test262_is_html_dda(invocation),', '            NativeFunctionId::Test262Gc => self.call_test262_gc(invocation),', '            NativeFunctionId::Test262Agent(kind) => {'))
require_gated('src/engine/builtins/array_buffer.rs', ('    pub(crate) fn call_test262_detach_array_buffer(', '    pub fn new_detach_array_buffer_function(&mut self) -> Result<CallableRef, RuntimeError> {'))
require_gated('src/engine/builtins/string.rs', ('    pub(crate) fn call_string_code_point_range(',))
require_gated('src/engine/value/primitive.rs', ('    pub fn try_with_exact_capacity(capacity: usize) -> Result<Self, JsStringError> {',))

gate = Path("scripts/test262/test-test262.sh").read_text()
if "-p quickjs-oxide-test262 --bin run-test262" not in gate:
    fail("central Test262 gate must build run-test262 with test262-host")
if gate.count("${TEST262_RUNNER+x}") != 1 or "runner_override" in gate:
    fail("central Test262 gate must retire external runner overrides")
if "QUICKJS_OXIDE_TEST262_ENGINE_SEMANTICS_SHA256=$workspace_engine_semantics_sha256" not in gate:
    fail("central Test262 gate must bind the workspace fingerprint at compile time")
if 'runner=$runner_dir/run-test262' not in gate or 'cp -p -- "$built_runner" "$runner"' not in gate:
    fail("central Test262 gate must execute a private authenticated runner copy")
if '--verify-runner-provenance "$workspace_engine_semantics_sha256"' not in gate:
    fail("central Test262 gate must verify the compiled runner fingerprint")
if "run-test262 accepted a stale engine semantics fingerprint" not in gate:
    fail("central Test262 gate must probe that stale fingerprints are rejected")

gc_gate = Path("scripts/quickjs/test-host-gc-reentrant-oracle.sh").read_text()
if "--features test262-host" not in gc_gate:
    fail("host GC differential must enable test262-host")

parity_gate = Path("scripts/checks/test-parity-slice.sh").read_text()
if parity_gate.count("--features test262-host") < 2:
    fail("parity slice must test and lint the Test262 host feature")

workflow = Path(".github/workflows/ci.yml").read_text()
if "./scripts/checks/check-test262-host-boundary.sh" not in workflow:
    fail("public fast CI must enforce the Test262 host boundary")
if "./scripts/test262/test-test262.sh --spec dev-support/test262/current.conf --runner-provenance" not in workflow:
    fail("public fast CI must authenticate the compiled Test262 runner")

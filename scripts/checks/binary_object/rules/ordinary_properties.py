"""Property kernel ownership guards; behavioral coverage remains in Rust/JS."""
import re

FILES = (
    "src/engine/object/ordinary_storage.rs",
    "src/engine/object/ordinary.rs",
    "src/engine/object/internal_methods.rs",
    "src/engine/heap/runtime/mod.rs",
    "src/engine/heap/object_storage.rs",
)


def check(ctx):
    if getattr(ctx, "self_test_marker_authorized", False):
        # The existing codec isolation fixtures deliberately omit the property
        # engine. Dedicated mutation tests exercise these guards on real files.
        return
    sources = []
    for relative in FILES:
        path = ctx.root / relative
        if path.is_symlink() or not path.is_file():
            ctx.fail("ordinary-property-source", f"missing regular source: {relative}")
            return
        sources.append(ctx.rust_code_only(path.read_text()))
    storage, ordinary, dispatch, runtime, heap = sources
    compact = lambda text: re.sub(r"\s+", "", text)
    requirements = [
        (not re.search(r"pub(?:\([^)]*\))?\s+struct\s+OwnSlot", storage), "slot positions must remain private to the storage owner"),
        ("(ObjectKind::Ordinary,ObjectPayload::Ordinary)" in compact(storage), "ordinary eligibility must include the semantic class"),
        (not re.search(r"\.(?:call_internal|internal_set|materialize_auto_init_property)\s*\(", storage), "storage must not execute callbacks or observable internal methods"),
        ("ordinary_set_fast_path_available" not in ordinary + dispatch, "ordinary Set must not pre-scan the prototype chain"),
        ("self.validate_object_and_key(object,key)?" in compact(ordinary) and "self.validate_value_domain(&value," in compact(ordinary) and "self.validate_value_domain(&receiver," in compact(ordinary), "Set must validate object, key, value and receiver domains"),
        ("rejected_object.as_ref().unwrap_or(receiver)" in compact(ordinary), "Proxy forwarding diagnostics must use the rejected target"),
        ("if!failure.published{self.release_atoms(atoms)?;}" in compact(runtime), "only pre-publication failures may roll back replacement Atoms"),
    ]
    body, _, _ = ctx.unique_braced_item(heap, re.compile(r"fn\s+replace_object_slot_with_status\s*\([^{}]*\)\s*->[^{}]*\{"), "ordinary-property-transaction", "slot replacement")
    retained = body.find("retain_edges_transactionally")
    published = body.find("replace_retained_object_slot")
    requirements.append((0 <= retained < published, "replacement edges must be retained before publication"))
    for accepted, message in requirements:
        if not accepted:
            ctx.fail("ordinary-property-contract", message)

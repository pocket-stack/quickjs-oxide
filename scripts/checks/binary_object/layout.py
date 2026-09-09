"""Concrete owners of the runtime, storage and VM evidence after decomposition.

Views combine exact registered files for existing cross-operation checks. Every
registered file must remain connected through conventional Rust module declarations.
Physical source ownership and codec privacy are checked separately.
"""
import re
from pathlib import Path

OWNER_GROUPS = {'src/engine/heap/runtime/mod.rs': ['src/engine/api/runtime.rs',
                                    'src/engine/atom/runtime.rs',
                                    'src/engine/builtins/error/backtrace.rs',
                                    'src/engine/builtins/error/construction.rs',
                                    'src/engine/builtins/function.rs',
                                    'src/engine/builtins/iterator/entry.rs',
                                    'src/engine/builtins/primitive.rs',
                                    'src/engine/code/dynamic_import_policy.rs',
                                    'src/engine/code/dynamic_source.rs',
                                    'src/engine/code/runtime.rs',
                                    'src/engine/heap/ownership.rs',
                                    'src/engine/heap/roots.rs',
                                    'src/engine/heap/runtime_gc.rs',
                                    'src/engine/object/access.rs',
                                    'src/engine/object/allocation.rs',
                                    'src/engine/object/function_initialization.rs',
                                    'src/engine/object/operations.rs',
                                    'src/engine/object/storage.rs',
                                    'src/engine/realm/bindings.rs',
                                    'src/engine/realm/prototypes.rs',
                                    'src/engine/value/conversion.rs',
                                    'src/engine/vm/call.rs',
                                    'src/engine/vm/exception.rs',
                                    'src/engine/vm/frames.rs'],
 'src/engine/heap/mod.rs': ['src/engine/heap/allocation.rs',
                            'src/engine/heap/arena.rs',
                            'src/engine/heap/binding_records.rs',
                            'src/engine/heap/binding_storage.rs',
                            'src/engine/heap/buffer_records.rs',
                            'src/engine/heap/code_records.rs',
                            'src/engine/heap/identity.rs',
                            'src/engine/heap/iteration_records.rs',
                            'src/engine/heap/iterator_records.rs',
                            'src/engine/heap/iterator_storage.rs',
                            'src/engine/heap/module_records.rs',
                            'src/engine/heap/module_storage.rs',
                            'src/engine/heap/object_records.rs',
                            'src/engine/heap/object_storage.rs',
                            'src/engine/heap/promise_records.rs',
                            'src/engine/heap/promise_storage.rs',
                            'src/engine/heap/realm_records.rs',
                            'src/engine/heap/realm_storage.rs',
                            'src/engine/heap/suspension_records.rs',
                            'src/engine/heap/suspension_storage.rs'],
 'src/engine/vm/mod.rs': ['src/engine/vm/protocol.rs',
                          'src/engine/vm/completion.rs',
                          'src/engine/vm/numeric.rs',
                          'src/engine/vm/detached.rs',
                          'src/engine/vm/activation.rs',
                          'src/engine/vm/dispatch.rs',
                          'src/engine/vm/numeric_execution.rs',
                          'src/engine/vm/unwind.rs',
                          'src/engine/vm/frame_execution.rs']}

def validate_link(ctx, relative):
    if ctx.self_test_marker_authorized or not relative.startswith("src/"):
        return
    path = Path(relative)
    components = list(path.parts[1:])
    if components[-1] == "lib.rs":
        return
    if components[-1] == "mod.rs":
        components.pop()
    else:
        components[-1] = Path(components[-1]).stem
    parent = ctx.root / "src/lib.rs"
    directory = ctx.root / "src"
    for name in components:
        if parent.is_symlink() or not parent.is_file():
            ctx.fail("module-route", f"{relative}: missing module parent {parent}")
            return
        code = ctx.rust_code_only(parent.read_text())
        pattern = r"(?m)^(?:pub(?:\([^)]*\))? +)?mod +" + re.escape(name) + r" *;"
        matches = list(re.finditer(pattern, code))
        if len(matches) != 1:
            ctx.fail("module-route", f"{relative}: {parent.relative_to(ctx.root)} must declare {name} once")
            return
        prefix = parent.read_text()[:matches[0].start()].rstrip()
        # Only the existing tests/test-support gates may select these evidence modules.
        attributes = []
        while prefix.endswith("]"):
            start = prefix.rfind("#[")
            if start < 0:
                break
            attributes.append(prefix[start:])
            prefix = prefix[:start].rstrip()
        allowed = {
            "tests": {'#[cfg(test)]'},
            "detached": {'#[cfg(test)]'},
        }.get(name, set())
        if any(" ".join(a.split()) not in allowed for a in attributes):
            ctx.fail("module-route", f"{relative}: unreviewed attributes on module {name}")
        if re.search(r"(?m)^#!\[(?:cfg|cfg_attr)", code):
            ctx.fail("module-route", f"{relative}: parent module must not be conditionally excluded")
        direct = directory / (name + ".rs")
        nested = directory / name / "mod.rs"
        if direct.is_file() and nested.is_file():
            ctx.fail("module-route", f"{relative}: ambiguous module file for {name}")
        parent = direct if direct.is_file() else nested
        directory = directory / name

def owner_sources(ctx, relative):
    pieces = []
    for owner in OWNER_GROUPS.get(relative, []):
        if ctx.self_test_marker_authorized and not (ctx.root / owner).is_file():
            continue
        validate_link(ctx, owner)
        pieces.append(ctx.read_source(owner))
    return "\n".join(pieces)

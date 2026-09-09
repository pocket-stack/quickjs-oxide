#!/usr/bin/env python3
"""Check candidate-A source ownership, module reachability and directory guides."""
from pathlib import Path
import re
import sys

from binary_object.context import ScanContext
from binary_object.rules import source_setup

root = Path(__file__).resolve().parents[2]
src = root / "src"
ctx = ScanContext(root)
source_setup.check(ctx)
errors = []
expected = {"compiler", "code", "value", "object", "atom", "heap", "vm",
            "realm", "builtins", "modules", "jobs", "host", "api"}
if {p.name for p in src.glob("*.rs")} != {"lib.rs"}:
    errors.append("src must contain only lib.rs as a top-level Rust source")
if {p.name for p in src.iterdir() if p.is_dir()} != {"engine", "source", "regexp"}:
    errors.append("src directories must be engine, source and regexp")
if {p.name for p in (src / "engine").iterdir() if p.is_dir()} != expected:
    errors.append("engine directories must match candidate A responsibilities")

# The embedding boundary is explicit; old root aliases and implementation
# modules must not silently become public again.
root_code = ctx.rust_code_only((src / "lib.rs").read_text())
engine_code = ctx.rust_code_only((src / "engine/mod.rs").read_text())
if re.search(r"\bpub\s+use\b", root_code):
    errors.append("lib.rs must not reexport legacy module paths or API items")
if set(re.findall(r"\bpub\s+mod\s+(\w+)", engine_code)) != {"api"}:
    errors.append("engine::api must be the only public engine module")
for source_file in src.rglob("*.rs"):
    if re.search(r"use\s+crate::engine::heap::runtime::\*", ctx.rust_code_only(source_file.read_text())):
        errors.append(f"{source_file.relative_to(root)} must import actual owners, not the runtime facade")

directories = [src] + sorted(p for p in src.rglob("*") if p.is_dir())
for directory in directories:
    readme = directory / "README.md"
    if not readme.is_file() or not readme.read_text().strip():
        errors.append(f"{directory.relative_to(root)} needs a nonempty README.md")
        continue
    for link in re.findall(r"\]\(([^)]+)\)", readme.read_text()):
        if "://" in link or link.startswith("#"):
            continue
        target = link.split("#", 1)[0]
        if target and not (directory / target).exists():
            errors.append(f"{readme.relative_to(root)} has a missing local link: {target}")

pending = [src / "lib.rs"]
seen = set()
while pending:
    path = pending.pop()
    if path in seen:
        continue
    seen.add(path)
    source = path.read_text()
    code = ctx.rust_code_only(source)
    base = path.parent if path.name in {"lib.rs", "mod.rs"} else path.with_suffix("")
    inline = []
    for match in re.finditer(r"\bmod\s+(\w+)\s*([;{])", code):
        name, delimiter = match.groups()
        parents = [n for start, end, n in inline if start < match.start() < end]
        if delimiter == "{":
            depth = 1
            end = match.end()
            while end < len(code) and depth:
                depth += (code[end] == "{") - (code[end] == "}")
                end += 1
            inline.append((match.start(), end, name))
            continue
        stem = base.joinpath(*parents, name)
        candidates = [p for p in [stem.with_suffix(".rs"), stem / "mod.rs"] if p.is_file()]
        if len(candidates) != 1:
            errors.append(f"{path.relative_to(root)}: module {name} needs exactly one conventional source")
        else:
            pending.extend(candidates)
    for match in re.finditer(r'include!\(\s*"([^"]+\.rs)"\s*\)', source):
        if code[match.start():].startswith("include!"):
            included = path.parent / match[1]
            if not included.is_file():
                errors.append(f"{path.relative_to(root)}: missing include {match[1]}")
            else:
                seen.add(included)

all_sources = set(src.rglob("*.rs"))
for path in sorted(all_sources - seen):
    errors.append(f"unreachable Rust source: {path.relative_to(root)}")
if errors:
    print("\n".join("error: " + error for error in errors), file=sys.stderr)
    raise SystemExit(1)
print(f"Source layout passed: {len(all_sources)} reachable Rust files; {len(directories)} directory READMEs.")

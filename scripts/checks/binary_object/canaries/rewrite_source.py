from pathlib import Path
import sys

import re
from mutation_targets import candidates

base_path = Path(sys.argv[1])
if sys.argv[2] == sys.argv[3] and sys.argv[4] == sys.argv[5] and not sys.argv[7]:
    raise SystemExit("full rewrite canary must change its target or add a file")
for before, after in [(sys.argv[2], sys.argv[3]), (sys.argv[4], sys.argv[5])]:
    if not before:
        continue
    # The named file is authoritative when it still contains the target. Search
    # child modules only for code that moved out of that file; unrelated children
    # can legitimately contain the same expression as the named parent.
    targets = candidates(base_path, Path(sys.argv[6]), before)
    matches = []
    for path in targets:
        if not path.is_file():
            continue
        source = path.read_text(encoding="utf-8")
        needle, replacement = before, after
        if path != base_path and (base_path.stem == "ordinary_leaf" or base_path.as_posix().endswith("/vm/mod.rs") and path.name == "tests.rs"):
            # The old inline test module contributed four spaces of indentation.
            needle = re.sub(r"(?m)^ {4}", "", needle)
            replacement = re.sub(r"(?m)^ {4}", "", replacement)
        if needle in source:
            matches.append((path, source, needle, replacement))
    if len(matches) != 1 or matches[0][1].count(matches[0][2]) != 1:
        raise SystemExit(f"full rewrite canary expected one occurrence of {before!r} in {base_path}")
    path, source, needle, replacement = matches[0]
    path.write_text(source.replace(needle, replacement), encoding="utf-8")
case_root = Path(sys.argv[6])
added_relative = sys.argv[7]
if added_relative:
    added_path = case_root / added_relative
    if added_path.exists():
        raise SystemExit(f"full rewrite canary added path already exists: {added_relative!r}")
    added_path.parent.mkdir(parents=True, exist_ok=True)
    added_path.write_text(sys.argv[8], encoding="utf-8")

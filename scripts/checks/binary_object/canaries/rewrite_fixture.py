from pathlib import Path
import sys

path = Path(sys.argv[1])
before = sys.argv[2]
after = sys.argv[3]
from mutation_targets import candidates
if before == after:
    raise SystemExit("rewrite canary must change its target")
root = next(parent for parent in path.parents if (parent / ".boundary-self-test").is_file())
matches = [p for p in candidates(path, root, before) if before in p.read_text(encoding="utf-8")]
if len(matches) != 1:
    raise SystemExit(f"rewrite canary expected one file containing {before!r}")
path = matches[0]
source = path.read_text(encoding="utf-8")
if source.count(before) != 1:
    raise SystemExit(f"rewrite canary expected one occurrence of {before!r}")
path.write_text(source.replace(before, after), encoding="utf-8")

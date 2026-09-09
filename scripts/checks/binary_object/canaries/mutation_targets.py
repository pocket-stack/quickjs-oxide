"""Resolve mutation sites through the same physical ownership map as the scan."""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from binary_object.layout import OWNER_GROUPS


def candidates(base: Path, root: Path, before: str) -> list[Path]:
    if base.is_file() and before in base.read_text(encoding="utf-8"):
        return [base]
    relative = base.relative_to(root).as_posix()
    owners = [root / p for p in OWNER_GROUPS.get(relative, ())]
    child = base.parent if base.name == "mod.rs" else base.with_suffix("")
    if child.is_dir():
        owners.extend(child.rglob("*.rs"))
    return sorted(set(p for p in owners if p.is_file()))

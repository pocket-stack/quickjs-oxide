from pathlib import Path
import hashlib
import os
import sys


root = Path(sys.argv[1])
plan = sys.argv[2]
field = sys.argv[3]
config_path = root / "dev-support/test262/current.conf"
status_path = root / "docs/status.md"
tsv_path = root / "dev-support/test262/generated/test262-class-private-callables-b-global-candidate.tsv"
jsonl_path = root / "dev-support/test262/generated/test262-class-private-callables-b-global-candidate.jsonl"


def replace_once(path: Path, before: str, after: str) -> None:
    source = path.read_text(encoding="utf-8")
    if source.count(before) != 1:
        raise SystemExit(
            f"Stage3I receipt canary expected one occurrence of {before!r} in {path}"
        )
    path.write_text(source.replace(before, after), encoding="utf-8")


if plan == "coherent-config-docs":
    config = config_path.read_text(encoding="utf-8")
    prefix = f"{field}="
    matches = [line for line in config.splitlines() if line.startswith(prefix)]
    if len(matches) != 1:
        raise SystemExit(f"Stage3I receipt canary requires exactly one {field} field")
    current = matches[0][len(prefix):]
    tampered = ("1" if current.startswith("0") else "0") + current[1:]
    replace_once(config_path, f"{field}={current}", f"{field}={tampered}")
    replace_once(status_path, current, tampered)
elif plan == "focused-content":
    replace_once(tsv_path, "# quickjs=2026-06-04", "# quickjs=2026-06-05")
elif plan == "self-consistent-four-file-forgery":
    replace_once(tsv_path, "# quickjs=2026-06-04", "# quickjs=2026-06-05")
    replace_once(
        jsonl_path,
        '\"quickjs\":\"2026-06-04\"',
        '\"quickjs\":\"2026-06-05\"',
    )
    config = config_path.read_text(encoding="utf-8")
    for key, receipt_path in (
        ("focused_tsv_sha256", tsv_path),
        ("focused_jsonl_sha256", jsonl_path),
    ):
        prefix = f"{key}="
        matches = [line for line in config.splitlines() if line.startswith(prefix)]
        if len(matches) != 1:
            raise SystemExit(f"Stage3I receipt canary requires exactly one {key} field")
        current = matches[0][len(prefix):]
        forged = hashlib.sha256(receipt_path.read_bytes()).hexdigest()
        replace_once(config_path, f"{key}={current}", f"{key}={forged}")
        replace_once(status_path, current, forged)
        config = config_path.read_text(encoding="utf-8")
elif plan == "status-html-wrapper":
    receipt_start = "The latest promoted Stage 3I lifecycle receipt"
    receipt_end = "raw-177 coverage, and makes no new conformance claim."
    replace_once(status_path, receipt_start, f"{field}\n\n{receipt_start}")
    replace_once(status_path, receipt_end, f"{receipt_end}\n\n</div>")
elif plan == "focused-hardlink":
    alias_path = root / "tests/stage3i-focused-receipt-hardlink.tsv"
    if alias_path.exists():
        raise SystemExit(f"Stage3I hardlink canary alias already exists: {alias_path}")
    os.link(tsv_path, alias_path)
else:
    raise SystemExit(f"unknown Stage3I receipt canary plan: {plan}")

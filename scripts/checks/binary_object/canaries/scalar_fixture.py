from pathlib import Path
import sys


source_path = Path(sys.argv[1])
target_path = Path(sys.argv[2])
source = source_path.read_text(encoding="utf-8")
test_module = "\n#[cfg(test)]\nmod tests {"
if source.count(test_module) != 1:
    raise SystemExit(
        "error: scalar_script.rs must contain exactly one cfg(test) module boundary"
    )
target_path.write_text(source.split(test_module, 1)[0] + "\n", encoding="utf-8")

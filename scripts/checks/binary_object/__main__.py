"""Internal scan entry; the stable shell command also orchestrates canaries."""
import sys
from pathlib import Path
from .scan import scan

if __name__ == "__main__":
    errors = scan(Path(sys.argv[1]), sys.argv[2] if len(sys.argv) > 2 else "")
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(bool(errors))

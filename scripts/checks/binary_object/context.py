"""Per-scan state; no evidence or caches leak between candidate roots."""
from functools import cache
from types import SimpleNamespace
from .source import SourceTools


class ScanContext(SourceTools, SimpleNamespace):
    def __init__(self, root, self_test_token=""):
        super().__init__(root=root, self_test_token=self_test_token, errors=[])
        # Candidate trees are immutable during a scan. Keep these caches local
        # to this candidate; a later scan must reread even the same root path.
        self.read_source = cache(self.read_source)
        self.rust_code_only = cache(self.rust_code_only)

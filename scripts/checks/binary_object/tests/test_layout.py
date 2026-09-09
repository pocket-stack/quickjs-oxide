from pathlib import Path
import tempfile
import unittest

from binary_object.context import ScanContext
from binary_object.layout import validate_link
from binary_object.rules import source_setup


class LayoutTests(unittest.TestCase):
    def check_route(self, declaration, owner="pub mod engine;"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src/engine/vm").mkdir(parents=True)
            (root / "src/lib.rs").write_text(owner)
            (root / "src/engine/mod.rs").write_text("pub mod vm;")
            (root / "src/engine/vm/mod.rs").write_text(declaration)
            (root / "src/engine/vm/dispatch.rs").write_text("fn execute() {}")
            context = ScanContext(root)
            source_setup.check(context)
            context.self_test_marker_authorized = False
            validate_link(context, "src/engine/vm/dispatch.rs")
            return context.errors

    def test_conventional_owner_route_is_connected(self):
        self.assertEqual(self.check_route("mod dispatch;"), [])

    def test_disconnected_evidence_is_rejected(self):
        self.assertTrue(self.check_route("pub use elsewhere::dispatch;"))
        self.assertTrue(self.check_route("mod dispatch;", owner="pub use elsewhere::engine;"))

    def test_conditional_exclusion_cannot_hide_evidence(self):
        self.assertTrue(self.check_route("#[cfg(any())]\nmod dispatch;"))
        self.assertTrue(self.check_route("#[cfg(test)]\nmod dispatch;"))
        self.assertTrue(self.check_route("#![cfg(not(test))]\nmod dispatch;"))

    def test_duplicate_module_routes_are_rejected(self):
        self.assertTrue(self.check_route("mod dispatch;\nmod dispatch;"))

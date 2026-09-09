from pathlib import Path
import re
import tempfile
import unittest

from binary_object.context import ScanContext
from binary_object.rules import source_setup


class SourceTests(unittest.TestCase):
    def context(self, root):
        context = ScanContext(Path(root))
        source_setup.check(context)
        return context

    def test_mask_preserves_offsets_and_nested_comments(self):
        source = 'fn a() { /* outer /* nested */ end */ let a = r##"}\n//"##; "\\\"{"; }\n'
        masked = self.context(".").rust_code_only(source)
        self.assertEqual(len(source), len(masked))
        self.assertEqual(
            [i for i, value in enumerate(source) if value == "\n"],
            [i for i, value in enumerate(masked) if value == "\n"],
        )
        self.assertEqual(masked.count("{"), 1)
        self.assertEqual(masked.count("}"), 1)
        self.assertIn("let a =", masked)

    def test_new_scan_rereads_changed_source_at_same_path(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "example.rs"
            path.write_text("before", encoding="utf-8")
            first = self.context(directory)
            self.assertEqual(first.read_source("example.rs"), "before")
            path.write_text("after", encoding="utf-8")
            second = self.context(directory)
            self.assertEqual(second.read_source("example.rs"), "after")
            first.fail("first", "isolated diagnostic")
            self.assertEqual(second.errors, [])

    def test_symlink_is_rejected_instead_of_followed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "target.rs").write_text("fn hidden() {}", encoding="utf-8")
            (root / "alias.rs").symlink_to("target.rs")
            context = self.context(root)
            self.assertEqual(context.read_source("alias.rs"), "")
            self.assertTrue(context.errors[0].startswith("missing-source:"))

    def test_missing_closing_brace_has_a_diagnostic(self):
        context = self.context(".")
        self.assertEqual(
            context.unique_braced_item("fn a() {", re.compile(r"fn a\(\) \{"), "shape", "a"),
            ("", -1, -1),
        )
        self.assertEqual(context.errors, ["shape: a has no balanced closing brace"])

    def test_runtime_test_expansion_preserves_gated_declarations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src/engine/heap/runtime/tests").mkdir(parents=True)
            (root / "src/engine/heap/runtime/tests.rs").write_text(
                "mod plain;\n#[cfg(feature = \"host\")] mod gated;\n", encoding="utf-8"
            )
            (root / "src/engine/heap/runtime/tests/plain.rs").write_text(
                "use super::*;\nfn evidence() {}\n", encoding="utf-8"
            )
            context = self.context(root)
            expanded = context.read_source("src/engine/heap/runtime/tests.rs")
            self.assertIn("fn evidence() {}", expanded)
            self.assertIn('#[cfg(feature = "host")] mod gated;', expanded)
            self.assertNotIn("use super::*", expanded)
            self.assertEqual(context.errors, [])


if __name__ == "__main__":
    unittest.main()

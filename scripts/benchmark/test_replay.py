"""Frozen replay rejects changed inputs and never admits a partial score ratio."""
import json
import tempfile
import unittest
from pathlib import Path
from replay import admit, load_workloads, summarize
from run import digest


class ReplayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source.js"
        self.source.write_text("print(42)\n")
        self.receipt = self.root / "receipt.json"
        self.workload = dict(case="sample", path="/old/source.js", sha256=digest(self.source), expected="42\n")

    def test_compile_uses_all_sources_but_original_requires_score_contract(self):
        self.receipt.write_text(json.dumps(dict(workloads=[self.workload])))
        self.assertEqual(len(load_workloads(self.receipt, self.root, "compile")), 1)
        with self.assertRaises(ValueError):
            load_workloads(self.receipt, self.root, "original")
        self.source.write_text("print(41)\n")
        with self.assertRaisesRegex(ValueError, "bytes changed"):
            load_workloads(self.receipt, self.root, "compile")

    def test_compile_output_requires_exact_framing_and_empty_stderr(self):
        stdout, stderr = self.root / "stdout", self.root / "stderr"
        stdout.write_text("compile_ns:42\n")
        stderr.write_text("")
        sample = dict(timed_out=False, exit_code=0, stdout=str(stdout), stderr=str(stderr))
        self.assertEqual(admit(sample, self.workload, "compile"), ("ok", {"compile_ns": 42}))
        stdout.write_text("compile_ns:42\nextra\n")
        self.assertEqual(admit(sample, self.workload, "compile")[0], "invalid-output")
        stderr.write_text("warning\n")
        self.assertEqual(admit(sample, self.workload, "compile")[0], "unexpected-stderr")

    def test_one_failed_round_invalidates_the_pair_without_hiding_good_rounds(self):
        samples = [dict(case="all", engine=name, status="ok", measurements={"Score": value})
                   for name, value in [("old", 10), ("old", 12), ("new", 20), ("new", 22)]]
        rows, ratios = summarize(samples, ["old", "new"])
        self.assertEqual(ratios[0]["ratio"], 21 / 11)
        samples[-1].update(status="timeout", measurements={})
        rows, ratios = summarize(samples, ["old", "new"])
        self.assertEqual(ratios, [])
        self.assertEqual(rows[-1]["metrics"]["Score"]["raw"], [20])
        self.assertFalse(rows[-1]["eligible"])


if __name__ == "__main__":
    unittest.main()

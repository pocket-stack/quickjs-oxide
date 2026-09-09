"""Fast, synthetic tests of result admission, timeout handling and aggregation."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("benchmark_run", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class Results(unittest.TestCase):
    def test_swallowed_failure_is_not_a_score(self):
        for output in ["Richards: Error: failed\n", "Score: 123\n", "Richards: 123\nScore: 0\n", "Richards: 2\nRichards: 3\nScore: 4\n"]:
            with self.assertRaises(ValueError):
                runner.parse_v8(output, ["Richards"])
        self.assertEqual(runner.parse_v8("Richards: 123\n----\nScore: 123\n", ["Richards"])["Score"], 123)

    def test_microbench_requires_every_selected_result(self):
        with self.assertRaises(ValueError):
            runner.parse_microbench(" prop_read 1000 2.3\n", ["prop_read", "prop_write"])
        self.assertEqual(runner.parse_microbench("__oxide_clock__:Date.now\n prop_read 1000 2.3\n total 2.3\n", ["prop_read"])["prop_read"]["ns_per_op"], 2.3)

    def test_microbench_rejects_wrong_clock_even_with_valid_results(self):
        with self.assertRaises(ValueError):
            runner.parse_microbench("__oxide_clock__:performance.now\n prop_read 1000 2.3\n", ["prop_read"])

    def test_clock_adaptation_preserves_every_original_body_byte(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "microbench.js"
            original = b"// synthetic fixture\r\nvar test_list = [empty_loop];\r\nfunction empty_loop(n) { return n; }\r\n"
            source.write_bytes(original)
            workloads, metadata = runner.prepare_microbench(source, ["empty_loop"])
            prepared = Path(workloads[0]["path"])
            self.assertEqual(prepared.read_bytes(), runner.MICROBENCH_CLOCK_PREFIX.encode() + original)
            self.assertEqual(source.read_bytes(), original)
            self.assertNotEqual(metadata["sha256"], metadata["prepared_sha256"])
            prepared.unlink()
            prepared.parent.rmdir()

    def test_one_failed_repetition_disqualifies_comparison(self):
        summary = runner.summarize([
            {"case": "x", "engine": "a", "status": "ok", "measurements": {"Score": 12}},
            {"case": "x", "engine": "a", "status": "timeout"},
        ], "v8-v7")[0]
        self.assertFalse(summary["eligible_for_comparison"])
        self.assertEqual(summary["successful"], 1)
        self.assertEqual(summary["metrics"]["Score"]["raw"], [12])

    def test_timeout_is_retained_with_raw_output(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "sample"
            result = runner.run_sample([sys.executable, "-c", "import time; print('started', flush=True); time.sleep(2)"], directory, prefix, .2)
            self.assertTrue(result["timed_out"])
            self.assertNotEqual(result["exit_code"], 0)
            self.assertIn("started", prefix.with_suffix(".stdout").read_text())


if __name__ == "__main__":
    unittest.main()

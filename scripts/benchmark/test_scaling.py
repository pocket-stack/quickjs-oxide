"""Admission checks for the fixed-work data structure runner."""
import tempfile
import unittest
import shutil
import subprocess
from pathlib import Path

import scaling


class Admission(unittest.TestCase):
    def test_successful_exit_with_wrong_output_is_not_a_measurement(self):
        with tempfile.TemporaryDirectory() as directory:
            stdout = Path(directory) / "sample.stdout"
            stdout.write_text("wrong\n")
            sample = {"exit_code": 0, "timed_out": False, "stdout": str(stdout)}
            self.assertEqual(scaling.admit(sample, b"42\n"), "wrong-output")
            stdout.write_bytes(b"42\n")
            self.assertEqual(scaling.admit(sample, b"42\n"), "ok")

    def test_one_failure_disqualifies_the_entire_group(self):
        samples = [
            {"case": "map-int", "size": 32, "engine": "before", "status": "ok", "process_wall_ns": 100},
            {"case": "map-int", "size": 32, "engine": "before", "status": "timeout", "process_wall_ns": 900},
        ]
        result = scaling.summarize(samples)[0]
        self.assertFalse(result["eligible"])
        self.assertEqual(result["successful_wall_ns"], [100])
        self.assertIsNone(result["median_wall_ns"])


class Workloads(unittest.TestCase):
    def test_import_reexport_workload_checks_the_exported_values(self):
        with tempfile.TemporaryDirectory() as directory:
            workload = scaling.prepare(Path(directory), "module-imports", 4, 16)
            self.assertEqual(workload["expected"], "6\n")

    @unittest.skipUnless(shutil.which("node"), "Node is required for independent workload smoke checks")
    def test_every_workload_has_the_expected_observable_result(self):
        for case in scaling.CASES:
            for size in [4, 8]:
                with self.subTest(case=case, size=size), tempfile.TemporaryDirectory() as directory:
                    workload = scaling.prepare(Path(directory), case, size, 16)
                    result = subprocess.run(["node", workload["path"]], capture_output=True, timeout=10)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.decode(), workload["expected"])

    def test_history_size_is_independent_of_iteration_work(self):
        with tempfile.TemporaryDirectory() as directory:
            workload = scaling.prepare(Path(directory), "map-iterate-churn", 256, 8)
            self.assertEqual(workload["expected"], "9\n")

    def test_capacity_cannot_silently_change_total_operations(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                scaling.prepare(Path(directory), "map-int", 7, 16)


if __name__ == "__main__":
    unittest.main()

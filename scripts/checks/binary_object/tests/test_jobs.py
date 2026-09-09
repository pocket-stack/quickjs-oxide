from pathlib import Path
import os
import subprocess
import tempfile
import unittest


JOBS = Path(__file__).resolve().parents[1] / "canaries/jobs.sh"


class CanaryJobsTests(unittest.TestCase):
    def run_jobs(self, commands, directory, workers="2"):
        return subprocess.run(
            [
                "bash", "-c",
                'set -euo pipefail\n'
                'die() { echo "error: $*" >&2; exit 1; }\n'
                'source "$1"\n' + commands + '\nfinish_canaries\n',
                "canary-jobs-test", str(JOBS), str(directory),
            ],
            env={**os.environ, "QUICKJS_OXIDE_BOUNDARY_JOBS": workers},
            capture_output=True, text=True,
        )

    def test_waits_for_every_successful_job(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_jobs(
                'queue_canary touch "$2/first"\nqueue_canary touch "$2/second"',
                directory,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((Path(directory) / "first").exists())
            self.assertTrue((Path(directory) / "second").exists())
            self.assertIn("checked: 2; all rejected", result.stderr)

    def test_failure_is_not_hidden_by_a_later_success(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_jobs(
                'queue_canary bash -c "exit 7"\nqueue_canary touch "$2/completed"',
                directory,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue((Path(directory) / "completed").exists())
            self.assertIn("1 binary-object canaries failed", result.stderr)

    def test_invalid_worker_limit_is_rejected(self):
        result = self.run_jobs("", ".", workers="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must be a positive integer", result.stderr)

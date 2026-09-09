#!/usr/bin/env python3
"""Build plain and profiling CLIs with commit, compiler and binary provenance."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from run import ROOT, command_output, digest, git_metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plain-target", type=Path, default=ROOT / "target")
    parser.add_argument("--profile-target", type=Path, default=ROOT / "target/profile-feature")
    parser.add_argument("--jobs", type=int, default=2)
    args = parser.parse_args()
    if args.jobs < 1 or args.plain_target.resolve() == args.profile_target.resolve():
        parser.error("jobs must be positive; build directories must differ")
    repo = git_metadata(ROOT)
    if repo["status"]["stdout"]:
        parser.error("commit source changes before measuring; build provenance requires a clean worktree")
    revision = repo["commit"]["stdout"]
    env = {**os.environ, "QUICKJS_OXIDE_BUILD_COMMIT": revision}
    for name, target, features in [("plain", args.plain_target, []), ("profiling", args.profile_target, ["--features", "profiling"])]:
        target = target.resolve()
        command = ["cargo", "build", "--locked", "--release", "-p", "quickjs-oxide-cli", "--no-default-features",
                   "--target-dir", str(target), "--jobs", str(args.jobs), *features]
        subprocess.run(command, cwd=ROOT, env=env, check=True)
        binary = target / "release" / ("qjs.exe" if os.name == "nt" else "qjs")
        manifest = {"schema": "oxide-build-v1", "mode": name, "commit": revision, "binary_sha256": digest(binary),
                    "command": command, "rustc": command_output(["rustc", "-vV"]), "cargo": command_output(["cargo", "-V"]),
                    "cargo_toml_sha256": digest(ROOT / "Cargo.toml"), "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
                    "environment": {key: os.environ.get(key) for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET"]},
                    "commit_environment": revision}
        binary.with_suffix(".build.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(binary)


if __name__ == "__main__":
    main()

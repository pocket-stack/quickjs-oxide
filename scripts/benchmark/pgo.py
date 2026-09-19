#!/usr/bin/env python3
"""Build a profile-guided-optimization CLI: instrument, train, merge, optimize."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from run import ROOT, command_output, digest, git_metadata

PROFILE_GENERATE = "-Cprofile-generate={}"
PROFILE_USE = "-Cprofile-use={}"


def llvm_profdata():
    sysroot = Path(command_output(["rustc", "--print", "sysroot"])["stdout"])
    host = command_output(["rustc", "-vV"])["stdout"]
    triple = next(line.split(":", 1)[1].strip() for line in host.splitlines() if line.startswith("host:"))
    candidate = sysroot / "lib" / "rustlib" / triple / "bin" / "llvm-profdata"
    if candidate.is_file():
        return candidate
    found = command_output(["which", "llvm-profdata"])["stdout"]
    if found:
        return Path(found)
    raise SystemExit("llvm-profdata not found; install the rustup `llvm-tools` component")


def build(target, jobs, rustflags=None):
    command = ["cargo", "build", "--locked", "--release", "-p", "quickjs-oxide-cli", "--no-default-features",
               "--target-dir", str(target.resolve()), "--jobs", str(jobs)]
    env = {**os.environ, "QUICKJS_OXIDE_BUILD_COMMIT": git_metadata(ROOT)["commit"]["stdout"]}
    if rustflags is not None:
        env["RUSTFLAGS"] = rustflags
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    binary = target.resolve() / "release" / ("qjs.exe" if os.name == "nt" else "qjs")
    return binary, command


def run_training(command):
    """Training failures only shrink coverage; raw profiles remain usable."""
    result = subprocess.run(command, cwd=ROOT)
    if result.returncode:
        print(f"warning: training command exited {result.returncode}; keeping its raw profiles", flush=True)


def train_scaling(binary, output, cases, sizes, operations, repeat, timeout):
    command = ["python3", str(ROOT / "scripts" / "benchmark" / "scaling.py"),
               "--engine", f"train={binary}", "--sizes", *map(str, sizes),
               "--operations", str(operations), "--repeat", str(repeat),
               "--timeout", str(timeout), "--output", str(output.resolve())]
    for case in cases or []:
        command += ["--case", case]
    run_training(command)


def train_v8(binary, source, output, cases, repeat, timeout):
    command = ["python3", str(ROOT / "scripts" / "benchmark" / "run.py"), "--suite", "v8-v7",
               "--source", str(source.resolve()), "--engine", f"train={binary}",
               "--repeat", str(repeat), "--timeout", str(timeout),
               "--output", str(output.resolve())]
    for case in cases or []:
        command += ["--case", case]
    run_training(command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generate-target", type=Path, default=ROOT / "target/pgo-generate")
    parser.add_argument("--use-target", type=Path, default=ROOT / "target/pgo-use")
    parser.add_argument("--profile-dir", type=Path, default=ROOT / "target/pgo-profiles")
    parser.add_argument("--v8-source", type=Path, help="external js-engine-benchmark checkout for extra training")
    parser.add_argument("--case", action="append", dest="cases", help="scaling case to train; repeatable, default all")
    parser.add_argument("--sizes", type=int, nargs="+", default=[64, 128])
    parser.add_argument("--operations", type=int, default=32768)
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=60, help="scaling training timeout")
    parser.add_argument("--v8-timeout", type=float, default=300, help="v8-v7 training timeout; instrumented binaries are slow")
    parser.add_argument("--jobs", type=int, default=16)
    parser.add_argument("--skip-training", action="store_true", help="reuse existing raw profiles")
    args = parser.parse_args()
    if args.jobs < 1 or args.operations < 1 or args.repeat < 1 or args.timeout <= 0 or args.v8_timeout <= 0:
        parser.error("jobs, operations, repeat and timeouts must be positive")
    if args.generate_target.resolve() == args.use_target.resolve():
        parser.error("generate and use targets must differ")
    profile_dir = args.profile_dir.resolve()
    if not args.skip_training:
        for stale in profile_dir.rglob("*.profraw"):
            stale.unlink()
    profile_dir.mkdir(parents=True, exist_ok=True)
    # Without a unique runtime template every process overwrites the same
    # default_%m_%c.profraw, so only the last training run remains. %m and %p
    # keep one raw profile per process; merge combines them afterwards.
    os.environ["LLVM_PROFILE_FILE"] = str(profile_dir / "%m_%p.profraw")

    print("building instrumented CLI", flush=True)
    instrumented, _ = build(args.generate_target, args.jobs, PROFILE_GENERATE.format(profile_dir))
    if not args.skip_training:
        raw = profile_dir / "scaling"
        train_scaling(instrumented, raw, args.cases, args.sizes, args.operations, args.repeat, args.timeout)
        if args.v8_source:
            train_v8(instrumented, args.v8_source, profile_dir / "v8-v7", args.cases, args.repeat, args.v8_timeout)

    raw_files = sorted(profile_dir.rglob("*.profraw"))
    if not raw_files:
        parser.error(f"no raw profiles under {profile_dir}; run training first")
    merged = profile_dir / "merged.profdata"
    tool = llvm_profdata()
    subprocess.run([str(tool), "merge", "-o", str(merged), *map(str, raw_files)], cwd=ROOT, check=True)

    print("building optimized CLI", flush=True)
    binary, command = build(args.use_target, args.jobs, PROFILE_USE.format(merged))
    revision = git_metadata(ROOT)["commit"]["stdout"]
    manifest = {"schema": "oxide-build-v1", "mode": "pgo", "vm_configuration": "stack-vm", "features": [],
                "commit": revision, "binary_sha256": digest(binary), "command": command,
                "rustc": command_output(["rustc", "-vV"]), "cargo": command_output(["cargo", "-V"]),
                "cargo_toml_sha256": digest(ROOT / "Cargo.toml"), "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
                "environment": {key: os.environ.get(key) for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET"]},
                "commit_environment": revision,
                "pgo": {"profile_dir": str(profile_dir), "profraw": len(raw_files),
                        "profdata_sha256": digest(merged), "llvm_profdata": str(tool),
                        "training_sizes": args.sizes, "training_operations": args.operations,
                        "training_repeat": args.repeat, "training_cases": args.cases or "all",
                        "v8_source": str(args.v8_source) if args.v8_source else None}}
    binary.with_suffix(".build.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(binary)


if __name__ == "__main__":
    main()

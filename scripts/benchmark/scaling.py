#!/usr/bin/env python3
"""Fixed-work process timings for container, compiler and module scaling."""
from pathlib import Path
import argparse
import json
import re
import statistics

from scaling_workloads import BATCHED_CASES, CASES, prepare
from run import binary_metadata, digest, machine_metadata, run_sample


def admit(sample, expected):
    """A successful process must also produce the independently known result."""
    if sample["timed_out"]:
        return "timeout"
    if sample["exit_code"]:
        return "failed"
    if Path(sample["stdout"]).read_bytes() != expected:
        return "wrong-output"
    return "ok"


def summarize(samples):
    groups = {}
    for sample in samples:
        key = (sample["case"], sample["size"], sample["engine"])
        groups.setdefault(key, []).append(sample)
    results = []
    for (case, size, engine), group in groups.items():
        values = [s["process_wall_ns"] for s in group if s["status"] == "ok"]
        eligible = len(values) == len(group)
        results.append({"case": case, "size": size, "engine": engine,
                        "eligible": eligible, "successful_wall_ns": values,
                        "failures": [s["status"] for s in group if s["status"] != "ok"],
                        "median_wall_ns": statistics.median(values) if eligible else None})
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", action="append", required=True, help="unique name=/absolute/binary")
    parser.add_argument("--case", choices=CASES, action="append")
    parser.add_argument("--sizes", type=int, nargs="+", default=[32, 128, 512, 2048])
    parser.add_argument("--operations", type=int, default=32768)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.repeat < 1 or args.timeout <= 0 or args.operations < 1:
        parser.error("repeat, timeout and operations must be positive")
    if len(set(args.sizes)) != len(args.sizes) or any(s < 1 for s in args.sizes):
        parser.error("sizes must be unique positive integers")
    cases = args.case or list(CASES)
    if any(case in BATCHED_CASES for case in cases) and any(args.operations % s for s in args.sizes):
        parser.error("batched workload sizes must divide operations")
    if len(set(cases)) != len(cases):
        parser.error("cases must be unique")
    engines = {}
    for entry in args.engine:
        name, separator, binary = entry.partition("=")
        if not separator or not re.fullmatch(r"[a-zA-Z0-9_-]+", name) or name in engines:
            parser.error("engines require unique safe names and name=path syntax")
        engines[name] = Path(binary).resolve()
    # Authenticate binaries before creating the output directory or starting samples.
    metadata = {"schema": 1, "metric": "whole-process wall nanoseconds; lower is better",
                "instrumentation": "no profiling flags; verify ordinary builds via receipts",
                "runner_sha256": digest(__file__),
                "generator_sha256": digest(Path(__file__).with_name("scaling_workloads.py")),
                "machine": machine_metadata(),
                "engines": {name: binary_metadata(path) for name, path in engines.items()},
                "repeat": args.repeat, "timeout_seconds": args.timeout}
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "raw").mkdir()
    workloads = [prepare(output / "workloads" / f"{case}-{size}", case, size, args.operations)
                 for case in cases for size in args.sizes]
    metadata["workloads"] = workloads
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    samples = []
    with (output / "samples.jsonl").open("w") as journal:
        for workload in workloads:
            for repetition in range(args.repeat):
                names = list(engines)
                offset = repetition % len(names)
                for name in names[offset:] + names[:offset]:
                    prefix = output / "raw" / f"{workload['case']}-{workload['size']}-{name}-{repetition}"
                    sample = run_sample([str(engines[name]), workload["path"]], output, prefix, args.timeout)
                    sample.update(case=workload["case"], size=workload["size"], engine=name,
                                  repetition=repetition, status=admit(sample, workload["expected"].encode()))
                    samples.append(sample)
                    journal.write(json.dumps(sample) + "\n")
                    journal.flush()
                    print(f"{prefix.name}: {sample['status']}", flush=True)
    summary = summarize(samples)
    (output / "results.json").write_text(json.dumps({"metadata": metadata, "summary": summary}, indent=2) + "\n")
    return 0 if all(row["eligible"] for row in summary) else 1


if __name__ == "__main__":
    raise SystemExit(main())

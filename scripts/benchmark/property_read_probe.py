#!/usr/bin/env python3
"""S0 diagnostic probe for the `a.b` data-property read path.

Generates monomorphic data-property read workloads, measures median
whole-process wall time and ns/op per engine, and can additionally record a
perf profile with a flat symbol report. This is a diagnostic probe, not a
formal score: process startup, compilation and teardown are included.

Output directories must not already exist, protecting prior evidence.

```sh
python3 scripts/benchmark/property_read_probe.py \
  --engine plain=target/plain/release/qjs \
  --engine profiling=target/profile-feature/release/qjs \
  --iterations 20000000 --repeat 5 --perf \
  --output target/property-read-probe
```
"""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

# case -> (source template, expected expression over `n`)
WORKLOADS = {
    "prop_read_int": (
        "let o = {{ a: 1, b: 2, c: 3, d: 4 }};\n"
        "let s = 0;\n"
        "for (let i = 0; i < {n}; i++) {{ s += o.a; }}\n"
        "console.log(s);\n",
        "n",
    ),
    "prop_read_obj": (
        "let o = {{ a: {{ x: 1 }}, b: {{ x: 2 }} }};\n"
        "let s = 0;\n"
        "for (let i = 0; i < {n}; i++) {{ s += o.a.x; }}\n"
        "console.log(s);\n",
        "n",
    ),
    "prop_read_string": (
        "let o = {{ a: \"hello\", b: \"world\" }};\n"
        "let s = 0;\n"
        "for (let i = 0; i < {n}; i++) {{ s += o.a.length; }}\n"
        "console.log(s);\n",
        "5*n",
    ),
}


def parse_engine(value):
    name, _, path = value.partition("=")
    if not name or not path:
        raise argparse.ArgumentTypeError("--engine must be name=path")
    return name, os.path.abspath(path)


def write_workloads(directory, iterations):
    directory.mkdir(parents=True, exist_ok=True)
    rows = []
    for case, (template, expected_expr) in WORKLOADS.items():
        source = template.format(n=iterations)
        path = directory / f"{case}.js"
        path.write_text(source)
        rows.append({
            "case": case,
            "path": str(path),
            "iterations": iterations,
            "expected": eval(expected_expr, {"n": iterations}),  # noqa: S307 (fixed expression)
        })
    return rows


def measure(engine, workload, repeat, timeout):
    samples = []
    output = None
    for _ in range(repeat):
        started = time.perf_counter()
        result = subprocess.run(
            [engine, workload["path"]],
            capture_output=True,
            timeout=timeout,
        )
        elapsed = time.perf_counter() - started
        if result.returncode != 0:
            return None, result.stderr.decode(errors="replace")
        output = result.stdout.decode().strip()
        if output != str(workload["expected"]):
            return None, f"unexpected output {output!r}, expected {workload['expected']!r}"
        samples.append(elapsed)
    samples.sort()
    return {
        "samples_s": samples,
        "median_s": statistics.median(samples),
        "min_s": samples[0],
        "max_s": samples[-1],
        "ns_per_iteration": statistics.median(samples) / workload["iterations"] * 1e9,
    }, None


def perf_profile(binary, perf_bin, workload, output, timeout):
    data = output / f"perf-{workload['case']}.data"
    report = output / f"perf-{workload['case']}.txt"
    record = subprocess.run(
        [perf_bin, "record", "-q", "-g", "-o", str(data), "--", binary, workload["path"]],
        capture_output=True,
        timeout=timeout,
    )
    if record.returncode != 0:
        return {"case": workload["case"], "error": record.stderr.decode(errors="replace")}
    shown = subprocess.run(
        [perf_bin, "report", "-i", str(data), "--stdio", "--no-children"],
        capture_output=True,
        timeout=timeout,
    )
    report.write_text(shown.stdout.decode(errors="replace"))
    return {"case": workload["case"], "data": str(data), "report": str(report)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", action="append", type=parse_engine, required=True,
                        metavar="NAME=PATH", help="engine binary; repeatable")
    parser.add_argument("--iterations", type=int, default=20_000_000)
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=600.0)
    parser.add_argument("--perf", action="store_true", help="record a perf profile per case")
    parser.add_argument("--perf-engine", help="engine name to profile (default: first)")
    parser.add_argument("--perf-bin", default="perf")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.iterations < 1 or args.repeat < 1:
        parser.error("iterations and repeat must be positive")

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    workloads = write_workloads(output / "workloads", args.iterations)

    results = {}
    samples_path = output / "samples.jsonl"
    with samples_path.open("w") as samples_file:
        for name, engine in args.engine:
            if not os.path.isfile(engine):
                parser.error(f"engine binary not found: {engine}")
            for workload in workloads:
                summary, error = measure(engine, workload, args.repeat, args.timeout)
                record = {"engine": name, "case": workload["case"], "error": error, "summary": summary}
                results[(name, workload["case"])] = record
                samples_file.write(json.dumps(record) + "\n")

    perf = []
    if args.perf:
        target = args.perf_engine or args.engine[0][0]
        binary = dict(args.engine)[target]
        for workload in workloads:
            perf.append(perf_profile(binary, args.perf_bin, workload, output, args.timeout))

    metadata = {
        "schema": "oxide-property-read-s0-v1",
        "iterations": args.iterations,
        "repeat": args.repeat,
        "engines": [{"name": name, "path": path} for name, path in args.engine],
        "perf_engine": args.perf_engine if args.perf else None,
    }
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")

    lines = ["# S0 property-read probe", "",
             f"iterations={args.iterations} repeat={args.repeat}", "",
             "| case | engine | median ms | ns/op |", "| --- | --- | ---: | ---: |"]
    for name, _ in args.engine:
        for workload in workloads:
            record = results[(name, workload["case"])]
            if record["summary"] is None:
                lines.append(f"| {workload['case']} | {name} | error | {record['error']} |")
            else:
                summary = record["summary"]
                lines.append(
                    f"| {workload['case']} | {name} | {summary['median_s']*1000:.2f} "
                    f"| {summary['ns_per_iteration']:.2f} |"
                )
    if perf:
        lines += ["", "perf reports:"]
        lines += [f"- {entry.get('report', entry.get('error'))}" for entry in perf]
    (output / "report.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))

    if any(record["error"] for record in results.values()):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

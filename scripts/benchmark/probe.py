#!/usr/bin/env python3
"""Collect profiler artifacts and measure four instrumentation modes on one workload."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics

from run import binary_metadata, machine_metadata, run_sample

WORKLOAD = """globalThis.items = [];
for (let i = 0; i < 1000; i++) items.push({value: i, label: 'item' + i});
globalThis.bytes = new ArrayBuffer(65536);
print(items.length, bytes.byteLength);
"""


def json_reports(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plain", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--repeat", type=int, default=11)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.repeat < 1 or args.timeout <= 0:
        parser.error("repeat and timeout must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    workload = output / "workload.js"
    workload.write_text(WORKLOAD)
    modes = [("plain", args.plain.resolve(), []), ("compiled-off", args.profile.resolve(), []),
             ("dump", args.profile.resolve(), ["-d", "--profile-json"]),
             ("trace", args.profile.resolve(), ["-T", "--profile-json"])]
    samples = []
    for iteration in range(args.repeat):
        order = modes[iteration % len(modes):] + modes[:iteration % len(modes)]
        for name, binary, flags in order:
            prefix = output / f"{name}-{iteration}"
            sample = run_sample([str(binary), *flags, str(workload)], output, prefix, args.timeout)
            sample.update(mode=name, repetition=iteration)
            assert sample["exit_code"] == 0 and not sample["timed_out"], sample
            assert Path(sample["stdout"]).read_bytes() == b"1000 65536\n", sample
            if not flags:
                assert not Path(sample["stderr"]).read_bytes(), sample
            else:
                reports = json_reports(Path(sample["stderr"]))
                assert len(reports) == 1
                report = reports[0]
                assert report["coverage"] == "partial"
                if name == "dump":
                    assert report["phase"] == "after-jobs-before-context-drop"
                    assert report["categories"]["array_buffer_bytes"]["used_bytes"] == 65536
                    assert report["allocator"]["requested_live_bytes"] is None
                else:
                    assert report["finished"] and report["complete_within_scope"]
                    capacity = 0
                    for event in report["events"]:
                        assert event["old_capacity_bytes"] == capacity
                        capacity = event["capacity_bytes"]
                    assert report["events"][0]["kind"] == "A"
                    assert report["events"][-1]["kind"] == "F" and capacity == 0
            samples.append(sample)
    lifecycle = run_sample([str(args.profile.resolve()), "-q", "-d", "--profile-json"], output, output / "lifecycle", args.timeout)
    assert lifecycle["exit_code"] == 0 and not lifecycle["timed_out"]
    records = json_reports(Path(lifecycle["stderr"]))
    lifecycle_data = next(record for record in records if record["schema"] == "oxide-lifecycle-v1")
    assert lifecycle_data["iterations"] == len(lifecycle_data["samples"]) == 100
    assert lifecycle_data["timer"] == "monotonic-wall"
    assert lifecycle_data["minimum_per_phase_ns"] == [min(row[i] for row in lifecycle_data["samples"]) for i in range(4)]
    reference = []
    if args.reference:
        for name, flags in [("dump", ["-d", str(workload)]), ("trace", ["-T", str(workload)]), ("lifecycle", ["-q", "-d"])]:
            sample = run_sample([str(args.reference.resolve()), *flags], output, output / ("quickjs-" + name), args.timeout)
            assert sample["exit_code"] == 0 and not sample["timed_out"], sample
            reference.append(sample)
    groups = {}
    for mode, _, _ in modes:
        values = [sample["process_wall_ns"] for sample in samples if sample["mode"] == mode]
        groups[mode] = {"raw_process_wall_ns": values, "median_ns": statistics.median(values),
                        "minimum_ns": min(values), "maximum_ns": max(values)}
    plain = groups["plain"]["median_ns"]
    for group in groups.values():
        group["median_ratio_to_plain"] = group["median_ns"] / plain
    result = {"schema": "oxide-profile-experiment-v1", "machine": machine_metadata(),
              "engines": {"plain": binary_metadata(args.plain.resolve()), "profile": binary_metadata(args.profile.resolve()),
                          "reference": binary_metadata(args.reference.resolve()) if args.reference else None},
              "workload_sha256": hashlib.sha256(WORKLOAD.encode()).hexdigest(), "workload": str(workload),
              "timer": "Python perf_counter_ns monotonic wall; full process including report formatting/I/O",
              "order": "serial; mode order rotated each repetition", "samples": samples,
              "overhead": groups, "lifecycle": lifecycle_data, "reference": reference}
    (output / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    lines = ["# Profiler experiment", "", "Same workload and stdout in every mode. These full-process timings include startup, teardown and diagnostic output; they do not isolate VM overhead.", "",
             "| Mode | Median ms | Range ms | Ratio to plain |", "| --- | ---: | --- | ---: |"]
    for name, group in groups.items():
        lines.append(f"| {name} | {group['median_ns']/1e6:.3f} | {group['minimum_ns']/1e6:.3f}–{group['maximum_ns']/1e6:.3f} | {group['median_ratio_to_plain']:.3f}× |")
    lines.extend(["", "Lifecycle uses monotonic wall time; reference QuickJS lifecycle uses CPU time, so no cross-clock speedup is calculated.",
                  "", "Allocation coverage is arena backing storage only. Memory totals, strings, shared backing bytes and allocator failures are explicitly unavailable.", ""])
    (output / "report.md").write_text("\n".join(lines))
    print(output / "report.md")


if __name__ == "__main__":
    main()

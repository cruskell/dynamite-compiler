#!/usr/bin/env python3
"""Summarize per-invocation elapsed times; nested timing groups are not additive."""
import argparse
from collections import Counter
import json
from pathlib import Path
import statistics


def summarize(root):
    metadata = json.loads((root / "metadata.json").read_text())
    builds = []
    for build in metadata["builds"]:
        records = [json.loads(p.read_text()) for p in Path(build["records"]).glob("*.json")
                   if not p.name.endswith(".trace.json")]
        compiled = [r for r in records if r["kind"] == "compile" and r["returncode"] == 0
                    and not any(Path(s).name == "foo.c" for s in r["sources"])]
        stages = Counter()
        trace = Counter()
        top = []
        for record in compiled:
            stages.update(record["stages"])
            trace_path = record.get("trace")
            if trace_path and Path(trace_path).is_file():
                events = json.loads(Path(trace_path).read_text())["traceEvents"]
                trace.update({e["name"][6:]: e["dur"] / 1e6 for e in events
                              if e["name"].startswith("Total ") and "dur" in e})
            source = Path(record["cwd"]) / record["sources"][0]
            source = str(source.resolve().relative_to(Path(build["cwd"])))
            top.append({"source": source, "seconds": record["seconds"],
                        "stages": record["stages"], "max_rss_kib": record["max_rss_kib"]})
        builds.append({
            **build,
            "compile_count": len(compiled),
            "compile_worker_seconds": sum(r["seconds"] for r in compiled),
            "compile_cpu_seconds": sum(r["user_seconds"] + r["system_seconds"] for r in compiled),
            "max_compile_rss_kib": max((r["max_rss_kib"] for r in compiled), default=0),
            "dependency_scan_worker_seconds": sum(r["seconds"] for r in records
                                                   if r["kind"] == "preprocess" and r["returncode"] == 0),
            "link_worker_seconds": sum(r["seconds"] for r in records
                                       if r["kind"] == "link_or_assembly" and r["returncode"] == 0),
            "stages": dict(stages), "clang_trace_totals": dict(trace),
            "failed_invocations": [{"args": r["args"], "returncode": r["returncode"]}
                                   for r in records if r["returncode"]],
            "slowest_compilations": sorted(top, key=lambda r: r["seconds"], reverse=True)[:15],
        })
    groups = []
    for project in ("redis", "git"):
        for compiler in dict.fromkeys(build["compiler"] for build in builds):
            runs = [b for b in builds if b["project"] == project and b["compiler"] == compiler]
            if not runs:
                continue
            groups.append({"project": project, "compiler": compiler, "runs": len(runs),
                           "wall_seconds": [r["seconds"] for r in runs],
                           "median_wall_seconds": statistics.median(r["seconds"] for r in runs),
                           "mean_stages": {key: statistics.mean(r["stages"].get(key, 0) for r in runs)
                                           for key in sorted(set().union(*(r["stages"] for r in runs)))},
                           "mean_clang_trace_totals": {key: statistics.mean(r["clang_trace_totals"].get(key, 0) for r in runs)
                                                       for key in sorted(set().union(*(r["clang_trace_totals"] for r in runs)))}})
    return {"groups": groups, "builds": builds}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    summary = summarize(args.directory.resolve())
    (args.directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    for group in summary["groups"]:
        print(f'{group["project"]:5} {group["compiler"]:8} wall={group["wall_seconds"]} median={group["median_wall_seconds"]:.3f}s')
        print("  stages:", json.dumps(group["mean_stages"], sort_keys=True))


if __name__ == "__main__":
    main()

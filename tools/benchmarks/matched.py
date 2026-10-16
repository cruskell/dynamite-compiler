#!/usr/bin/env python3
"""Replay representative preprocessed inputs through both frontends serially."""
import argparse
from collections import defaultdict
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

REPO = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--rounds", type=int, default=7)
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("rounds must be positive")
    root = args.directory.resolve()
    metadata = json.loads((root / "metadata.json").read_text())
    destination = args.output.resolve() if args.output else root / "matched"
    destination.mkdir(exist_ok=False)
    summaries = []
    for project, names in [("redis", ["adlist.c", "server.c"]), ("git", ["abspath.c", "revision.c"])]:
        build = next((b for b in metadata["builds"] if b["project"] == project and b["compiler"] == "dynamite"), None)
        if build is None:
            continue
        records = [json.loads(p.read_text()) for p in Path(build["records"]).glob("*.json")
                   if not p.name.endswith(".trace.json")]
        for name in names:
            record = next(r for r in records if r["kind"] == "compile" and r["returncode"] == 0
                          and any(Path(source).name == name for source in r["sources"]))
            prefix = destination / f"{project}-{name}"
            pp_args = []
            skip = False
            for arg in record["args"]:
                if skip:
                    skip = False
                    continue
                if arg in ("-o", "-MF", "-MT", "-MQ"):
                    skip = True
                elif arg not in ("-c", "-MD", "-MMD", "-MP"):
                    pp_args.append(arg)
            preprocessed = prefix.with_suffix(".i")
            env = dict(os.environ, DYNAMITE_CLANG=args.clang)
            result = subprocess.run([str(REPO / "target/release/dynamite-cc"), *pp_args, "-E", "-P"],
                                    cwd=record["cwd"], env=env, capture_output=True, check=True)
            preprocessed.write_bytes(result.stdout)
            command = [str(REPO / "target/release/examples/profile_frontend"),
                       str(preprocessed), str(args.rounds), str(prefix) + ".dynamite.ll"]
            start = time.perf_counter()
            frontend = subprocess.run(command, env=dict(env, DYNAMITE_TIMINGS="1"), capture_output=True, check=True)
            elapsed = time.perf_counter() - start
            stages = defaultdict(list)
            for line in frontend.stderr.decode().splitlines():
                if line.startswith("dynamite-timing\t"):
                    _, pid, stage, seconds = line.split("\t")
                    stages[stage].append(float(seconds))
            clang_runs = []
            for run in range(args.rounds):
                trace = str(prefix) + f".clang-{run}.trace.json"
                output = str(prefix) + ".clang.ll"
                command = [args.clang, "-x", "cpp-output", "-std=gnu99", "-O0", "-g0", "-S", "-emit-llvm",
                           str(preprocessed), "-o", output, f"-ftime-trace={trace}"]
                start = time.perf_counter()
                subprocess.run(command, capture_output=True, check=True)
                wall = time.perf_counter() - start
                events = json.loads(Path(trace).read_text())["traceEvents"]
                totals = {e["name"][6:]: e["dur"] / 1e6 for e in events
                          if e["name"].startswith("Total ") and "dur" in e}
                clang_runs.append({"wall_seconds": wall, "totals": totals})
            lowering = {}
            ir_counts = {}
            for compiler in ("dynamite", "clang"):
                source_ir = Path(str(prefix) + f".{compiler}.ll")
                lines = source_ir.read_text().splitlines()
                ir_counts[compiler] = {"declarations": sum(line.startswith("declare ") for line in lines),
                                       "definitions": sum(line.startswith("define ") for line in lines)}
                runs = []
                for run in range(args.rounds):
                    trace = str(prefix) + f".{compiler}.lower-{run}.trace.json"
                    command = [args.clang, "-O0", "-g0", "-c", str(source_ir), "-o", str(prefix) + f".{compiler}.o",
                               "-Wno-override-module", f"-ftime-trace={trace}"]
                    start = time.perf_counter()
                    subprocess.run(command, capture_output=True, check=True)
                    wall = time.perf_counter() - start
                    events = json.loads(Path(trace).read_text())["traceEvents"]
                    totals = {e["name"][6:]: e["dur"] / 1e6 for e in events
                              if e["name"].startswith("Total ") and "dur" in e}
                    runs.append({"wall_seconds": wall, "totals": totals})
                lowering[compiler] = runs
            summary = {"project": project, "source": name, "input_bytes": preprocessed.stat().st_size,
                       "rounds": args.rounds, "dynamite_process_seconds": elapsed,
                       "dynamite_stages_median": {k: statistics.median(v) for k, v in stages.items()},
                       "dynamite_stages_all": dict(stages), "clang_runs": clang_runs,
                       "dynamite_ir_bytes": Path(str(prefix) + ".dynamite.ll").stat().st_size,
                       "clang_ir_bytes": Path(str(prefix) + ".clang.ll").stat().st_size,
                       "ir_counts": ir_counts, "lowering_runs": lowering}
            summaries.append(summary)
            (destination / "summary.json").write_text(json.dumps(summaries, indent=2) + "\n")
            print(json.dumps({k: v for k, v in summary.items() if k not in ("dynamite_stages_all", "clang_runs", "lowering_runs")}), flush=True)


if __name__ == "__main__":
    main()

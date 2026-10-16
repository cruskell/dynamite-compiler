#!/usr/bin/env python3
"""Record real compiler invocations and optional Clang/LLVM time traces."""
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import time
import uuid


def main():
    args = sys.argv[1:]
    log = Path(os.environ["BENCH_LOG"])
    log.mkdir(parents=True, exist_ok=True)
    record = log / uuid.uuid4().hex
    compiler = os.environ["BENCH_COMPILER"]
    sources = [arg for arg in args if arg.endswith((".c", ".i"))]
    if any(flag in args for flag in ("-E", "-M", "-MM")):
        kind = "preprocess"
    elif sources and "-c" in args:
        kind = "compile"
    elif sources:
        kind = "compile_link"
    elif any(arg.endswith((".o", ".a", ".s", ".S")) for arg in args):
        kind = "link_or_assembly"
    else:
        kind = "other"
    command = [compiler, *args]
    trace = str(record) + ".trace.json"
    if os.environ.get("BENCH_TRACE") and kind == "compile":
        # Dynamite forwards this option to its Clang IR backend without another process.
        command.append(f"-ftime-trace={trace}")
    env = dict(os.environ, DYNAMITE_TIMINGS="1")
    start = time.perf_counter()
    result = subprocess.run(command, env=env, stderr=subprocess.PIPE)
    elapsed = time.perf_counter() - start
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    stages = {}
    for line in result.stderr.decode(errors="replace").splitlines(keepends=True):
        if line.startswith("dynamite-timing\t"):
            _, pid, stage, seconds = line.strip().split("\t")
            stages[stage] = stages.get(stage, 0) + float(seconds)
        else:
            sys.stderr.write(line)
    record.with_suffix(".json").write_text(json.dumps({
        "cwd": os.getcwd(), "args": args, "compiler": compiler,
        "kind": kind, "sources": sources, "seconds": elapsed,
        "user_seconds": usage.ru_utime, "system_seconds": usage.ru_stime,
        "max_rss_kib": usage.ru_maxrss / (1024 if sys.platform == "darwin" else 1),
        "returncode": result.returncode, "stages": stages,
        "trace": trace if Path(trace).is_file() else None,
    }, indent=2) + "\n")
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())

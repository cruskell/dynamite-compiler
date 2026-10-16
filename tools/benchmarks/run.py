#!/usr/bin/env python3
"""Compare clean upstream builds using the same wrapper, flags and make parallelism."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time

REPO = Path(__file__).resolve().parents[2]


def output(command, cwd=REPO):
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--redis-source", type=Path, required=True)
    parser.add_argument("--git-source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True,
                        help="New directory; existing results are never overwritten")
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--dynamite", type=Path, default=REPO / "target/release/dynamite-cc")
    parser.add_argument("--runs", type=int, default=2)
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.runs < 1 or args.jobs < 1:
        parser.error("runs and jobs must be positive")
    clang = shutil.which(args.clang)
    if not clang:
        parser.error("Clang executable was not found")
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    sources = {"redis": args.redis_source.resolve(), "git": args.git_source.resolve()}
    metadata = {
        "utc": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(), "machine": platform.machine(),
        "clang": output([clang, "--version"]),
        "dynamite_commit": output(["git", "rev-parse", "HEAD"]),
        "dynamite_diff": output(["git", "diff"]),
        "jobs": args.jobs, "runs": args.runs,
        "sources": {name: {"path": str(path), "commit": output(["git", "rev-parse", "HEAD"], path),
                           "status": output(["git", "status", "--porcelain"], path)}
                    for name, path in sources.items()},
        "builds": [],
    }
    (root / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    flags = "-O0 -g0 -std=gnu99"
    redis_flags = flags + " -D__STDC_NO_ATOMICS__=1"
    for project, source in sources.items():
        for run in range(1, args.runs + 1):
            order = ("dynamite", "clang") if run % 2 else ("clang", "dynamite")
            for compiler in order:
                name = f"{project}-{compiler}-{run}"
                work = root / name
                shutil.copytree(source, work)
                records = root / (name + "-records")
                executable = args.dynamite.resolve() if compiler == "dynamite" else clang
                env = dict(os.environ, BENCH_COMPILER=str(executable), BENCH_LOG=str(records),
                           BENCH_TRACE="1", DYNAMITE_CLANG=clang)
                command = ["make", f"-j{args.jobs}", "CC=" + str(REPO / "tools/benchmarks/cc.py")]
                if project == "redis":
                    lua_flags = redis_flags + " -Wall -DLUA_ANSI -DENABLE_CJSON_GLOBAL -DREDIS_STATIC= -DLUA_USE_MKSTEMP"
                    command += ["-C", "src", "MALLOC=libc", "BUILD_TLS=no", "SKIP_VEC_SETS=yes",
                                "OPT=-O0", "OPTIMIZATION=-O0", "DEBUG=", "CFLAGS=" + redis_flags,
                                "LUA_CFLAGS=" + lua_flags, "redis-server", "redis-cli", "redis-benchmark",
                                "redis-check-rdb", "redis-check-aof", "redis-sentinel"]
                else:
                    command += ["NO_RUST=YesPlease", "NO_PRECOMPILED_HEADER=YesPlease",
                                "NO_GETTEXT=YesPlease", "NO_TCLTK=YesPlease", "NO_PERL=YesPlease",
                                "NO_PYTHON=YesPlease", "CFLAGS=" + flags, "all"]
                start = time.perf_counter()
                with (root / (name + ".log")).open("w") as log:
                    result = subprocess.run(command, cwd=work, env=env, stdout=log, stderr=subprocess.STDOUT)
                build = {"project": project, "compiler": compiler, "run": run,
                         "seconds": time.perf_counter() - start, "returncode": result.returncode,
                         "command": command, "cwd": str(work), "records": str(records)}
                metadata["builds"].append(build)
                (root / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
                print(json.dumps(build), flush=True)
                if result.returncode:
                    raise SystemExit(f"Build failed: {root / (name + '.log')}")


if __name__ == "__main__":
    main()

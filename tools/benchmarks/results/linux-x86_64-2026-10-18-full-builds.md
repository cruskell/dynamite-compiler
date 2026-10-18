# Complete Git/Redis build measurements (Linux x86_64, 2026-10-18)

This follows the record-member frontend measurements with complete clean application
builds. The baseline is `7804347` before #216; current main is `6739321`, including
#216 and the CI expansion. Both Dynamite binaries were rebuilt with Rust 1.86,
release opt-level 3 and debuginfo 1. Clang and external LLVM tools are 15.0.7.

## Method

- Intel Core i5-5575R, four logical CPUs, Linux x86_64; no overlapping compiler
  builds, regression tests or smoke tests during the timed runs.
- Pristine Redis 8.10.2 and Git v2.56.0 sources; exact commits are in the snapshot.
- `make -j4`, GNU99, `-O0 -g0`; dependency compilation and linking included.
- Two clean builds per project/compiler, in baseline/current/Clang order and then
  the reverse. Source copies and Rust compiler builds occur outside the timer.
- Warm filesystem caches, no compiler cache, no Clang time tracing.
- Dynamite explicitly uses native preprocessing and **the opt-in LLVM tools backend**
  (`llc`/`opt`/LLD). Its default Clang lowering/linking backend was not measured.
- Redis uses libc, disables TLS/Vector Sets/C11 atomics, sets Lua to `-O0`, and
  builds redis-server, redis-cli, redis-benchmark, redis-check-rdb, redis-check-aof
  and redis-sentinel. External/test modules are excluded. Git disables Rust,
  PCH, optional scripting and localization frontends.

## Whole-build wall time

Each value is the median of two successful clean builds, in seconds.

| Project | Before #216 | Current main | Clang | Reduction vs before | Reduction vs Clang |
|---|---:|---:|---:|---:|---:|
| Redis | 10.906 | 10.731 | 14.008 | 1.61% | 23.40% |
| Git | 38.408 | 38.317 | 40.733 | 0.24% | 5.93% |

The record-member change reduces Redis whole-build time by about 1.6% in this
snapshot. Git's approximately 0.2% difference is too small to treat as a confirmed
improvement from two runs. The earlier 9% Redis-server frontend reduction is a
different boundary/workload and does not imply a 9% full-build reduction.

Raw elapsed samples:

| Project | Compiler | Run 1 (s) | Run 2 (s) |
|---|---|---:|---:|
| redis | baseline | 10.914523 | 10.898025 |
| redis | dynamite | 10.711650 | 10.749889 |
| redis | clang | 13.990622 | 14.026230 |
| git | baseline | 38.397462 | 38.419117 |
| git | dynamite | 38.282219 | 38.352494 |
| git | clang | 40.653023 | 40.812671 |

## Stage totals

These are means of summed successful C-compilation worker seconds across two
runs. Parallel compilation overlaps, and nested timers are not additive. They
must not be interpreted as sequential contributions to the whole-build wall time.
Clang stage traces were disabled, so no matching Clang stage table is inferred.

| Project | Compiler | Preprocess | Parse/lower AST | Generate IR | LLVM lowering |
|---|---|---:|---:|---:|---:|
| redis | baseline | 5.540 | 4.348 | 1.044 | 8.507 |
| redis | dynamite | 5.631 | 3.662 | 1.028 | 8.568 |
| git | baseline | 52.294 | 33.524 | 2.465 | 20.442 |
| git | dynamite | 52.502 | 33.260 | 2.418 | 20.441 |

## Validation and limits

All 12 builds succeeded, with no tracked upstream source changes. Baseline and
current Dynamite used identical successful C compile commands. Git's 568 C
commands also matched Clang exactly. Redis built the same 184 C input files;
two Clang-only successful C11 feature probes add earlier warning/standard flags,
while the final GNU99 flag remains common. Probe overhead is included in whole-build
time and probes are excluded only from the successful C-compilation aggregates.
Capability macros can select different C paths, so this compares complete
application toolchains, not byte-identical preprocessed input.

All 12 runtime smoke checks passed after timing: Git init/add/commit/status,
branch switching, diff, gc and fsck; Redis PING/SET/GET, Lua/cjson/math, SAVE and
redis-check-rdb. These are compiler-output correctness checks, not runtime
performance measurements or complete upstream test suites.

Two samples per variant are a snapshot, not a statistical speedup guarantee.
The [measurement snapshot](linux-x86_64-2026-10-18-full-builds.json) preserves compiler
hashes, exact build commands, sample times, stage totals, source identities and
validation. Full logs/per-invocation records remain in
`temp/full-build-2026-10-18/builds/` in the measurement workspace.

## Reproduction

Build both compiler revisions with the same Rust settings and save the baseline
`dynamite-cc` binary. With pristine source checkouts:

```sh
CARGO_PROFILE_RELEASE_DEBUG=1 cargo +1.86.0 build --release --bins
DYNAMITE_BACKEND=llvm \
DYNAMITE_LLC=/usr/bin/llc-15 DYNAMITE_OPT=/usr/bin/opt-15 \
DYNAMITE_LLD=/path/to/ld.lld-15 \
python3 tools/benchmarks/run.py \
  --redis-source /path/to/redis --git-source /path/to/git \
  --dynamite target/release/dynamite-cc \
  --baseline-dynamite /path/to/baseline/dynamite-cc \
  --clang /usr/bin/clang-15 --no-trace --runs 2 --jobs 4 \
  --output temp/new-full-build-run
python3 tools/benchmarks/summarize.py temp/new-full-build-run
```

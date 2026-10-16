# Native preprocessing and macOS LLVM tools

**Historical load-contended run:** see the [resumed measurements](macos-arm64-2026-10-17-native-pipeline-resumed.md)
for the follow-up against main including #199.

These measurements compare the previous Dynamite pipeline against native
preprocessing and the Clang-free LLVM tools pipeline. The baseline is Dynamite
main `15d0267`, not Clang compiling C directly. The candidate compiler code is
`200792e`. These measurements precede the token-storage changes in PR #199.
PR #198 was subsequently rebased onto main including #199 and #200, so its
current head requires a new performance comparison.

## Method

- Apple M2, macOS ARM64, Rust 1.86 release builds.
- Git v2.56.0 (`a018953688f1b10bddf91bff8747068f5f4746a4`).
- Redis 8.10.2 (`498ecd0d6d007db11ddb3aea9428552598a78622`).
- Four complete rounds with alternating variant order, clean build artifacts,
  warm filesystem caches, no compiler caches, `make -j4`, `-O2
  -fno-omit-frame-pointer`, macOS deployment target 11.0 and SDK 26.5.
- Both Clang-based variants use Apple Clang 21. The Clang-free variant uses
  Homebrew LLVM 20.1.5 opt/llc and `/usr/bin/ld`. This is a comparison of complete
  pipelines, including different downstream LLVM versions.
- Git disables Rust, PCH and compiler-generated header dependencies. Redis uses
  libc allocation, disables Vector Sets/C11 atomics/LTO and builds test modules.
  Neither project's source is patched.

Seven rounds were planned. Measurements were stopped after four balanced rounds
and one additional native/Clang Git sample because host-load variation prevented
reliable wall-clock attribution. All completed samples, including that additional
sample, are retained in the [raw data](macos-arm64-2026-10-17-native-pipeline.json).
The table uses the four complete rounds for each variant. No samples within those
rounds were discarded. Regression tests did not run concurrently with timed builds;
other host workloads were not controlled.

## Observations

| Pipeline | Git median (range), seconds | Redis median (range), seconds |
| --- | ---: | ---: |
| Previous Dynamite: Clang preprocessing/lowering | 45.79 (24.28–51.59) | 29.59 (20.58–42.45) |
| Native preprocessing, Clang lowering | 67.43 (42.26–68.98) | 21.41 (14.97–30.40) |
| Native preprocessing, opt/llc/native linker | 44.45 (27.05–60.42) | 22.49 (19.29–30.56) |

**Performance non-regression is not established.** The mixed native/Clang Git
median is slower; the Clang-free Git median is similar to the baseline, with large
overlapping ranges. Redis medians are lower, but the host variation prevents a
confident speedup claim. Controlled measurements on the planned Ubuntu machine
are still needed before treating performance as verified.

A separate diagnostic sample alternated compilation of every twentieth Git
translation unit (29 files). The preprocessing stage totaled 2.81 seconds for the
baseline and 1.70 seconds for native preprocessing. This sample does not explain
the mixed pipeline's whole-build slowdown and cannot replace whole-build
validation. Per-file phase timings are included in the raw data.

## Functional validation

The following local results apply to the measured compiler before that rebase.
Current-head checks are reported separately in PR #198.

- Native library tests: 48 passed.
- macOS ARM64 integration tests: 2,095 passed; two opt/llc-specific tests executed
  separately and passed.
- macOS x86_64 native regressions: eight passed; the two opt/llc execution tests
  also passed under Rosetta.
- Linux ARM64 integration tests: 2,099 passed.
- Ubuntu CI passes in one job, building and generating IR before Clang/LLVM tools
  are installed, then testing opt/llc without Clang, and finally running all
  execution tests after installing Clang.
- Clang-free Git/Redis clean builds succeeded. The Clang override points to a
  rejecting executable; no invocations were recorded.
- The final Clang-free build also passed Git init/add/commit/fsck/checkout and
  Redis PING/SET/GET/Lua/DEL smoke checks. Git upstream suites `t0000`, `t0001`,
  `t0002`, `t0003`, `t0008` and `t3400` passed (694 passed, eight skipped).
  Redis `unit/keyspace`, `unit/type/string` and `unit/scripting` all passed.
  These are selected upstream suites, not the entire upstream test collections.

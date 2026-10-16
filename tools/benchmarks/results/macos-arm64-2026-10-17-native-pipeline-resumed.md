# Resumed native pipeline measurements

Main subsequently changed in #201; see the [separate latest-main confirmation](macos-arm64-2026-10-17-native-pipeline-post-201.md).

This follow-up supersedes the earlier load-contended measurement for assessing
PR #198. It compares freshly built Rust release binaries for main `824be9a`
and candidate `e21534c`. Both include the token-storage changes from #199.
The baseline is Dynamite using Clang preprocessing/lowering, not Clang compiling
C directly. No compiler source changed during this follow-up.

## Method

- Apple M2, macOS ARM64, Rust 1.86.0, macOS SDK 26.5, deployment target 11.0.
- Pristine Git v2.56.0 and Redis 8.10.2 sources; exact commits and flags are in the raw data.
- `make -j4`, `-O2 -fno-omit-frame-pointer`, clean build artifacts and warm filesystem caches.
- Six rounds per project/variant, using every permutation of the three variants once.
  Three warm-up translation units per variant are excluded. No timed samples are discarded.
- Other compiler tasks were stopped before this follow-up. Regression tests and Rust builds
  did not run concurrently with timed builds. Normal desktop activity was not controlled.
- Git disables Rust, PCH and compiler-generated header dependencies. Redis uses libc
  allocation, disables Vector Sets/C11 atomics/LTO and includes test modules.
- Both Clang-based variants use Apple Clang 21. The Clang-free variant uses
  Homebrew LLVM 20.1.5 opt/llc and the macOS linker. External LLVM versions and
  their target tuning differ, so this measures the deployed pipelines as a whole.

## Results

| Pipeline | Git median (range), s | Redis median (range), s | Git paired change | Redis paired change |
| --- | ---: | ---: | ---: | ---: |
| Main: Clang preprocessing/lowering | 30.72 (22.00–38.42) | 18.26 (12.41–21.99) | +0.0% | +0.0% |
| Native preprocessing, Clang lowering | 28.17 (21.40–34.71) | 14.14 (9.43–18.46) | -5.0% | -18.5% |
| Native preprocessing, opt/llc/native linker | 29.64 (23.18–36.07) | 13.98 (9.72–15.28) | +0.4% | -21.3% |

Paired changes are the median of each round's variant/baseline ratio. They are
computed separately from the time medians; they are not ratios of those independent medians.
Negative changes mean shorter build times.

Native preprocessing with the existing Clang lowering backend was faster in all
six rounds for both projects. The Clang-free pipeline was faster in every Redis
round. Git with that pipeline was close to the baseline: its paired median was
+0.4%, with individual rounds in both directions. The earlier large mixed-pipeline
Git slowdown did not recur.

This supports making native preprocessing the only mode without an observed
build-time regression in the tested workloads. It does not prove exact performance
equivalence for the optional LLVM tools backend: the descriptive 95% bootstrap
percentile interval for its paired Git median ratio is 0.864–1.059. Small regressions
cannot be ruled out on this desktop host; do not claim a precise Git speedup.
Per-round ratios, CPU times and resampling intervals are retained in the raw data.

## Validation

- All 36 clean builds succeeded.
- The Clang-free variant configured a rejecting Clang executable; zero invocations were recorded.
- Its final binaries passed Git init/add/commit/status/fsck/checkout and Redis
  PING/SET/GET/Lua/DEL/shutdown smoke checks after all timed builds finished.
- Compiler sources match the previously validated candidate: strict Clippy, 50
  library tests, seven native macOS regressions, both Clang-free macOS opt/llc
  execution tests and the single Ubuntu CI job passed. The same compiler also
  passed the selected Git and Redis upstream suites recorded in PR #198.

[Raw measurements and metadata](macos-arm64-2026-10-17-native-pipeline-resumed.json).

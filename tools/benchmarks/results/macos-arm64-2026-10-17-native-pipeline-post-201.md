# Native pipeline validation after main #201

Main changed during the six-round experiment: PR #201 added scalar expression
caching and rewritten-path fixes. This separate follow-up rebuilds both variants
against main `de9de24`; candidate `096a261` includes the same changes.
The baseline is Dynamite using Clang preprocessing/lowering, not Clang compiling C.
The earlier six-round results are retained separately and are not pooled with these samples.
The branch subsequently incorporated documentation-only main #202; compiler sources
remain identical to the measured candidate.

## Method

- Rust 1.86 release binaries, Apple M2/macOS ARM64, SDK 26.5 and deployment target 11.0.
- Pristine Git v2.56.0 and Redis 8.10.2, identical source tags and build features to the
  [six-round run](macos-arm64-2026-10-17-native-pipeline-resumed.md).
- `make -j4`, `-O2 -fno-omit-frame-pointer`, clean build artifacts, warm filesystem cache, no compiler caches.
- Three cyclic variant orders: each variant is first, second and third exactly once.
  Three warm-up translation units per variant are excluded; no timed sample is discarded.
- Rust builds and regression tests finished before timed builds. Other compiler tasks
  were stopped; normal desktop background activity was not controlled.
- Clang-based variants use Apple Clang 21; the Clang-free variant uses Homebrew LLVM
  20.1.5 opt/llc and the macOS linker. Backend versions and tuning differ.

## Results

| Pipeline | Git median, s | Redis median, s | Git paired change | Redis paired change |
| --- | ---: | ---: | ---: | ---: |
| Main: Clang preprocessing/lowering | 26.78 | 16.04 | +0.0% | +0.0% |
| Native preprocessing, Clang lowering | 22.54 | 12.08 | -12.0% | -24.8% |
| Native preprocessing, opt/llc/native linker | 24.27 | 13.65 | -9.4% | -19.8% |

Paired changes summarize each round's variant/baseline time ratio; they are not
ratios of the independently computed time medians. Negative changes mean shorter builds.

Native preprocessing was faster in all three Git and Redis rounds, consistent
with every round of the earlier six-round experiment. No native-preprocessing
build-time regression was observed. The Clang-free backend was faster in all
Redis rounds and two Git rounds; the other Git round was +0.7%.

These are observations on this host, not proof of exact equivalence or a precise
portable speedup. The earlier six-round Git Clang-free paired median was +0.4%,
whereas this three-round median is -9.4%; do not interpret the latter as a stable
9.4% optimization of the backend. The broader result is no recurring large slowdown.
Raw ranges, CPU times and individual paired ratios are preserved.

## Validation

- All 18 additional clean builds and final Git/Redis runtime smoke tests passed.
- The rejecting Clang probe recorded zero invocations in the Clang-free variant.
- Before timing: formatting, strict all-target Clippy, 51 library tests, seven
  native macOS regressions and both macOS Clang-free opt/llc execution tests passed.
- Compiler sources did not change while these measurements were collected.

[Raw measurements and metadata](macos-arm64-2026-10-17-native-pipeline-post-201.json).

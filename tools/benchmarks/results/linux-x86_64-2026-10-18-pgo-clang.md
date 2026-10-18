# Mixed-corpus PGO versus Clang

Measured on Linux x86_64, Intel Core i5-5575R (four logical CPUs), with
Clang 15.0.7. Dynamite is the saved Rust 1.86.0 mixed-corpus PGO executable
from the [training comparison](linux-x86_64-2026-10-18-mixed-pgo.md): compiler
commit `7804347c97ee021bfac6c7b1a3e6558ec8fb3f40` plus the preprocessing
copy changes. It predates the newer record-member changes on the rebased PR
base. This measures that saved PGO compiler, not the rebased PR head.

## Method

Each application/compiler pair receives three sequential clean builds with
`make -j4`, `-O2 -g0`, no compiler caches, stage timers or Clang time traces.
Orders are PGO/Clang, Clang/PGO, PGO/Clang. Filesystem caches are not flushed.
Source copies, cleaning and validation are excluded from elapsed time; dependency
compilation, build helpers and linking are included. Both compilers are measured
again together; earlier timing samples are not reused.

Dynamite uses native preprocessing, parsing and LLVM IR generation, then the
same external Clang 15 for IR lowering and linking. Clang compiles C directly.
`DYNAMITE_BACKEND` is unset: this is the default Clang backend, not the opt-in
LLVM tools backend used in the earlier
[Git/Redis comparison](linux-x86_64-2026-10-18-full-builds.md), which also used
`-O0`. Those timings must not be combined with this `-O2` comparison.
Only Dynamite has application-derived PGO; the installed Clang is unchanged.
Equal optimization flags do not imply identical generated code or executable
performance. Runtime performance is not measured here.

Git v2.56.0 and Redis 8.10.2 use fresh pristine source copies and the
[same make arguments](linux-x86_64-2026-10-18-git-redis-copy-pgo.md) as the
PGO experiment, with only `CC` changed between compilers. Git disables Rust,
PCH, gettext and Perl/Python/Tcl/Tk integrations. Redis builds its six native
tools, uses libc allocation, disables TLS/Vector Sets/C11 atomics, and sets
matching optimization flags for bundled Lua. Compiler probes remain in the
build timer and can select different warning options and portable/intrinsic
source paths; this compares application toolchains, not identical preprocessed C.

Curl 8.23.0 uses copies of one tree configured by Dynamite, with static libcurl,
OpenSSL 3.0.2/zlib 1.2.11 enabled, and libpsl/documentation disabled. Each copy
is cleaned and stale dependency files are removed before timing. The configure
step is excluded. Both compilers share its generated configuration; Clang is
not separately configured to enable additional capabilities.

## Clean-build results

| Application | Mixed PGO median | Clang median | Reduction vs Clang |
| --- | ---: | ---: | ---: |
| Curl 8.23.0 | 25.9920 s | 26.5110 s | 1.96% |
| Git v2.56.0 | 49.1027 s | 52.9949 s | 7.34% |
| Redis 8.10.2 | 19.0569 s | 23.1044 s | 17.52% |

| Application | Compiler | Run 1 (s) | Run 2 (s) | Run 3 (s) |
| --- | --- | ---: | ---: | ---: |
| Curl 8.23.0 | Mixed PGO | 25.9920 | 25.9815 | 26.1033 |
| Curl 8.23.0 | Clang | 26.5418 | 26.5110 | 26.3837 |
| Git v2.56.0 | Mixed PGO | 49.3468 | 49.1027 | 49.0024 |
| Git v2.56.0 | Clang | 52.9949 | 53.1249 | 52.4933 |
| Redis 8.10.2 | Mixed PGO | 18.9941 | 19.0569 | 19.2423 |
| Redis 8.10.2 | Clang | 23.1044 | 23.0564 | 23.2718 |

Reduction is `100 * (1 - PGO median / Clang median)`. The PGO compiler was
faster in every paired build in this snapshot. Three repetitions per
configuration do not establish a general speedup across other applications,
toolchains or platforms. All three applications were included in PGO training.

## Validation and artifacts

All 18 builds succeeded. Every curl binary passed HTTP and HTTPS loopback
requests with certificate verification enabled. Every Git binary passed
init/add/commit/show/clean-status checks in a temporary repository. Every Redis
server/client pair passed PING, SET/GET, Lua EVAL and clean shutdown. Git/Redis
tracked source contents were unchanged after all builds. Full upstream test
suites were not run. macOS and AArch64 were not measured.

Compiler hashes, exact make commands, per-run elapsed/resource measurements
and smoke results are preserved in the
[measurement snapshot](linux-x86_64-2026-10-18-pgo-clang.json).
The source archive identities are documented in the preceding
[curl](linux-x86_64-2026-10-18-curl-copy-pgo.md) and
[Git/Redis](linux-x86_64-2026-10-18-git-redis-copy-pgo.md) reports.
Local `temp/pgo-clang/compare.py` reuses the preceding mixed-PGO comparison,
selecting the saved mixed compiler and `/usr/bin/clang-15` instead of two PGO
compilers. `temp/pgo-clang/` retains all build logs, resource measurements,
source copies and smoke artifacts; it is ignored by Git. No compiler code,
normal release binary or training profiles were changed for this comparison.

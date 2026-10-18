# Mixed-corpus PGO versus Clang at -O0

This repeats the [-O2 comparison](linux-x86_64-2026-10-18-pgo-clang.md)
with `-O0 -g0`, using the same saved mixed-corpus PGO executable and installed
Clang 15.0.7. The compiler binary hashes are unchanged. Dynamite uses its
**default external Clang IR lowering/linking backend**. The opt-in LLVM tools
backend used by the older 10.73-second Redis measurement is not selected here.

The host is Linux x86_64 on an Intel Core i5-5575R with four logical CPUs.
Dynamite was built with Rust 1.86.0 and trained on curl/Git/Redis (1,229 C
compilations). Its source is `7804347c97ee021bfac6c7b1a3e6558ec8fb3f40` plus
preprocessing token-copy changes; it predates the newer record-member changes
on the rebased PR base. This measures the saved compiler, not the latest PR head.

## Method

Three fresh builds per compiler/application run sequentially with `make -j4`.
Compiler order is PGO/Clang, Clang/PGO, PGO/Clang. Source copying, cleaning and
smoke validation are outside the timer; bundled dependencies, build helpers and
linking are inside. Filesystem caches are not flushed. Compiler caches, stage
timers and Clang time traces are disabled. The PGO compiler is not retrained.

Git v2.56.0 and Redis 8.10.2 use the same pristine source copies and make
arguments as the preceding -O2 comparison, replacing every `-O2` with `-O0`.
This includes Redis `OPT`, `OPTIMIZATION`, `CFLAGS` and bundled `LUA_CFLAGS`.
Git disables Rust, PCH, localization and scripting frontends. Redis builds its
six native tools with libc allocation and TLS/Vector Sets/C11 atomics disabled.
Compiler probes can select different warning flags or capability-dependent C
paths; these are application toolchain comparisons, not identical preprocessed C.

Curl 8.23.0 shares the same configured tree as before: static libcurl,
OpenSSL/zlib enabled, libpsl/documentation disabled. The original configure step
used Dynamite with -O2 and is not repeated or timed. Each copy is cleaned and
stale dependency files are removed. Timed make overrides `CFLAGS` with
`-O0 -g0 -Werror-implicit-function-declaration -Wno-system-headers`, preserving
all other configured CFLAGS. Both compilers share the generated configuration.

## Clean-build results

| Application | Mixed PGO median | Clang median | Reduction vs Clang |
| --- | ---: | ---: | ---: |
| Curl 8.23.0 | 18.4161 s | 19.8969 s | 7.44% |
| Git v2.56.0 | 31.6302 s | 34.8182 s | 9.16% |
| Redis 8.10.2 | 8.3549 s | 11.7627 s | 28.97% |

| Application | Compiler | Run 1 (s) | Run 2 (s) | Run 3 (s) |
| --- | --- | ---: | ---: | ---: |
| Curl 8.23.0 | Mixed PGO | 18.2300 | 18.4161 | 18.4338 |
| Curl 8.23.0 | Clang | 28.5267 | 18.8372 | 19.8969 |
| Git v2.56.0 | Mixed PGO | 32.6218 | 31.6302 | 31.4783 |
| Git v2.56.0 | Clang | 34.8182 | 34.8878 | 34.5825 |
| Redis 8.10.2 | Mixed PGO | 11.6274 | 8.3549 | 8.3055 |
| Redis 8.10.2 | Clang | 12.4388 | 11.7627 | 11.6918 |

Reduction is `100 * (1 - PGO median / Clang median)`. The PGO compiler was
faster in every paired build in this snapshot. Three repetitions per
configuration do not establish a general speedup across other applications,
toolchains or platforms. All three applications were included in PGO training.

The first Clang curl build took 28.527 seconds, versus about 19-20 seconds in
subsequent runs. The first PGO Redis build took 11.627 seconds, versus about
8.3 seconds in subsequent runs. Their user/system CPU totals vary much less
than wall time, so scheduling or other wall-time effects are not ruled out.
The cause was not isolated. All samples are retained; the median is not a
guarantee that every clean build takes that long.

## Comparison with -O2

| Application | PGO -O0 | PGO -O2 | Clang -O0 | Clang -O2 |
| --- | ---: | ---: | ---: | ---: |
| Curl 8.23.0 | 18.4161 s | 25.9920 s | 19.8969 s | 26.5110 s |
| Git v2.56.0 | 31.6302 s | 49.1027 s | 34.8182 s | 52.9949 s |
| Redis 8.10.2 | 8.3549 s | 19.0569 s | 11.7627 s | 23.1044 s |

These -O2 samples are from the preceding comparison, not interleaved with the
new -O0 builds. Source versions, binaries, backend, make parallelism and optional
features match, but the measurement session differs. Optimization flags can also
change preprocessor capability macros and selected source paths. This is a
whole-build comparison, not isolated optimizer pass timing. Equal flags do not
imply equal executable performance; runtime speed is not measured.

The older Redis result (10.731 seconds, Clang 14.008 seconds) used -O0 with the
LLVM tools backend and a different compiler revision. The new -O0 results use
the Clang backend, so any remaining difference from 10.731 seconds cannot be
attributed solely to the optimization level or solely to PGO.

## Validation and artifacts

All 18 builds succeeded. Every curl binary passed HTTP and HTTPS loopback
requests with certificate verification enabled. Every Git binary passed
init/add/commit/show/clean-status checks in a temporary repository. Every Redis
server/client pair passed PING, SET/GET, Lua EVAL and clean shutdown. Git/Redis
tracked source contents were unchanged after all builds. Full upstream test
suites were not run. macOS and AArch64 were not measured.

The [measurement snapshot](linux-x86_64-2026-10-18-pgo-clang-o0.json) records
compiler hashes, source archive hashes, exact commands, all samples and smoke
results, and the preceding -O2 medians. Local `temp/pgo-clang-o0/` contains the
comparison script, logs, source copies and smoke artifacts; it is ignored by Git.
No compiler source, normal release binary or training profile was changed.

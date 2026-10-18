# Curl token-copy and PGO measurements

Measured on Linux x86_64 with Rust 1.86.0 and Clang 15.0.7, relative to
compiler commit `7804347c97ee021bfac6c7b1a3e6558ec8fb3f40`. The copy changes
borrow cached dependency directives, move expanded macro arguments on their
last use, and consume token-paste inputs. PGO uses the same changed source.

## Clean builds

Curl 8.23.0 and static libcurl, `-O2 -g0`, `make -j4`, native preprocessing
and the Clang IR backend. OpenSSL and zlib are enabled; libpsl and documentation
generation are disabled. All compilers use the same configured source tree.
Configure, cleaning and validation are outside the timer. Each run removes objects
and stale `.deps/*.Po`/`.Plo` files; the latter avoids an upstream make race
between a removed `tool_hugehelp.c` and its absolute dependency entry.

| Compiler | Run 1 | Run 2 | Run 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 26.50 s | 26.38 s | 26.45 s | 26.45 s |
| Token-copy changes | 26.50 s | 26.42 s | 26.48 s | 26.48 s |
| Token-copy changes + PGO | 25.91 s | 26.06 s | 25.94 s | 25.94 s |

Orders were baseline/copy/PGO, PGO/baseline/copy, and copy/PGO/baseline.
Filesystem caches were not cleared; no compiler cache or timing instrumentation
was enabled. Every executable passed `curl --version`. The combined changes
reduce the median by 1.9%; copying changes alone do not measurably improve the
full build in these runs.

## Preprocessing and dependencies

For individual `easy.c`, `http.c`, `multi.c` and `vtls/openssl.c` preprocessing,
copy changes reduce whole-process allocation requests by 3.5–4.3%, with little
timing change. Expanded output matches the baseline after removing line markers.

A single `-M` invocation over those four files reuses cached header directives.
Six alternating-order timing samples per binary gave:

| Compiler | Median dependency scan | Allocation requests |
| --- | ---: | ---: |
| Baseline | 155.81 ms | 381,163 |
| Token-copy changes | 145.84 ms | 320,065 |
| Token-copy changes + PGO | 138.05 ms | 320,065 |

Dependency output is byte-identical. Allocation probes use the existing glibc
`LD_PRELOAD` counter and run separately from timing. Counts include process
initialization; requested bytes are not peak memory. The dependency workload is
a batch and does not represent curl's usual per-file compilation path.

## PGO and validation

`tools/benchmarks/pgo.py` replayed 477 successful curl compilation records with
`-Cprofile-generate`, merged profiles using Rust 1.86.0's `llvm-profdata`, and
rebuilt with `-Cprofile-use`. External Clang is unchanged. Curl was both the
training and evaluation workload; these results do not establish improvements
on Git, Redis or held-out workloads. The normal release binary is not replaced.
Subsequent held-out [Git and Redis measurements](linux-x86_64-2026-10-18-git-redis-copy-pgo.md)
use this same PGO binary without retraining.

The instrumented/trained build, logs, replay commands and profiles are under
`temp/curl-opt/pgo/`; build and preprocessing measurements are under
`temp/curl-opt/`. These local artifacts are ignored by Git.

Validation passed: rustfmt, Clippy with all targets/features and warnings denied,
52 library tests, 2,101 integration tests (two ignored), and one doctest.
Tests used Clang 15 through PATH; the host's default Clang 14 cannot consume the
opaque-pointer IR. Baseline, changed and PGO binaries also compiled and ran a
macro regression using repeated `__COUNTER__`, stringification and token pasting.
macOS and AArch64 were not tested in this measurement.

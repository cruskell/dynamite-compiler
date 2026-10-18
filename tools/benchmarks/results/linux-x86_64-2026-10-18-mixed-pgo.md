# Curl-only versus mixed-corpus PGO

Measured on Linux x86_64 with Rust 1.86.0, four available CPUs, and external
Clang 15.0.7 for LLVM IR lowering and linking. Both compiler binaries contain
the same preprocessing token-copy changes on top of compiler commit
`7804347c97ee021bfac6c7b1a3e6558ec8fb3f40`. Their Rust source changes were
checked against the original curl-only build metadata before training.
These saved binaries predate the subsequent record-member optimization on main;
the numbers are not measurements of those newer compiler changes.

## Training

| Corpus | Curl 8.23.0 | Git v2.56.0 | Redis 8.10.2 | Total |
| --- | ---: | ---: | ---: | ---: |
| Curl-only | 477 | 0 | 0 | 477 |
| Mixed | 477 | 568 | 184 | 1,229 |

Counts are successful C compilation invocations from one build of each included
application, including bundled C dependencies and build helpers. Each invocation
is replayed once; there is no per-project reweighting. The mixed build uses the
same curl records as the original experiment. Missing input files and failed
training commands were checked; all 1,229 invocations completed successfully.

The mixed compiler was produced with the existing replay tool:

```sh
python3 tools/benchmarks/pgo.py \
  --records temp/curl-stages/1-dynamite \
  --records temp/mixed-pgo/git-records \
  --records temp/mixed-pgo/redis-records \
  --output temp/mixed-pgo/pgo --toolchain 1.86.0 --clang clang-15 --jobs 4
```

The output directory was new when this command ran. Instrumented builds,
training, merging and profile-use builds are excluded from evaluation timings.
The original curl-only PGO executable was preserved and measured again alongside
the mixed build; previous timing samples were not reused.

## Clean-build results

Each compiler was measured three times per application with `make -j4` and
`-O2 -g0`, without compiler caches, stage timers or Clang traces. Orders were
curl-only/mixed, mixed/curl-only, curl-only/mixed. Builds ran sequentially.
Filesystem caches were not flushed. Source copying, cleaning and smoke checks
were excluded from the timer.

Git and Redis used fresh pristine source copies and the same GNU99 make commands
as the [previous comparison](linux-x86_64-2026-10-18-git-redis-copy-pgo.md).
Git disables Rust, PCH, gettext and Perl/Python/Tcl/Tk integrations. Redis builds
its six native tools with libc allocation, TLS/Vector Sets/C11 atomics disabled,
and explicit matching Lua flags. Curl uses copies of the same configured tree
as the [curl experiment](linux-x86_64-2026-10-18-curl-copy-pgo.md), with static
libcurl, OpenSSL/zlib enabled and libpsl/documentation disabled. Every curl copy
is cleaned and stale dependency files are removed before timing.

| Application | Curl-only PGO median | Mixed PGO median | Reduction |
| --- | ---: | ---: | ---: |
| Curl 8.23.0 | 25.8992 s | 25.8822 s | 0.07% |
| Git v2.56.0 | 49.4387 s | 49.3617 s | 0.16% |
| Redis 8.10.2 | 19.1529 s | 19.0196 s | 0.70% |

| Application | Compiler | Run 1 | Run 2 | Run 3 |
| --- | --- | ---: | ---: | ---: |
| Curl | Curl-only | 25.8992 | 26.3708 | 25.8910 |
| Curl | Mixed | 26.1164 | 25.8812 | 25.8822 |
| Git | Curl-only | 49.7431 | 49.4387 | 49.4355 |
| Git | Mixed | 49.4339 | 49.1469 | 49.3617 |
| Redis | Curl-only | 19.1498 | 19.1764 | 19.1529 |
| Redis | Mixed | 19.0149 | 19.0196 | 19.0831 |

Additional training does not produce a substantial improvement in these builds.
Curl and Git median differences are smaller than run-to-run variation. Redis is
slightly faster with mixed PGO in all three paired comparisons. These are three
samples per configuration, not evidence of a universal improvement. All evaluated
applications are included in mixed training; no fourth held-out application was
evaluated.

A subsequent [Clang comparison](linux-x86_64-2026-10-18-pgo-clang.md) measures
this same mixed-PGO executable against Clang 15 with matching `-O2` build settings.

## Validation and artifacts

All 18 builds succeeded. Each Git binary passed temporary repository operations;
each Redis server/client pair passed PING, SET/GET, Lua EVAL and clean shutdown.
Each curl binary passed HTTP and HTTPS loopback requests, with certificate
verification enabled. Git and Redis tracked source contents remained unchanged.
Full upstream test suites were not run for this timing comparison.

| Compiler | SHA-256 |
| --- | --- |
| Curl-only PGO | `7f84bbade893ef0d2e5f586f66db679a4a87a87259f96bd303b93dee22d163e8` |
| Mixed PGO | `eeb2b4f70798b22a9c64f6f7eb366939a7b4735cd1010009bba4253f446c4d50` |

`temp/mixed-pgo/` contains collection and training records, raw/merged profiles,
both build logs and exact commands for each evaluation, smoke results, metadata,
the comparison script and summary. The mixed executable is
`temp/mixed-pgo/pgo/dynamite-pgo`. These local artifacts are ignored by Git;
the normal release executable is unchanged.

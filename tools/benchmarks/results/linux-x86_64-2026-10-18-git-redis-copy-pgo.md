# Git and Redis token-copy and PGO measurements

Measured on Linux x86_64 with four available CPUs, using the same three Rust
1.86.0 compiler binaries as the [curl experiment](linux-x86_64-2026-10-18-curl-copy-pgo.md).
The baseline is compiler commit `7804347c97ee021bfac6c7b1a3e6558ec8fb3f40`.
The changed binary reduces preprocessing token copies. The PGO binary includes
those changes and was trained on 477 curl 8.23.0 compilation invocations.
It was not retrained on Git or Redis, so these are held-out application builds.
All three invoke the same external Clang 15.0.7 for IR lowering and linking.

## Conditions

- Git v2.56.0 and Redis 8.10.2, downloaded from their upstream GitHub tag archives.
- `-O2 -g0 -std=gnu99`, `make -j4`, native Dynamite preprocessing.
- A fresh copy of the pristine source repository for every build, including
  bundled dependencies. Source copying and smoke checks are outside the timer.
- No compiler cache, stage timers or Clang time traces. Filesystem caches are
  not cleared; these are clean builds rather than cold-cache builds.
- Three runs per project/compiler, with orders baseline/copy/PGO,
  PGO/baseline/copy, and copy/PGO/baseline. Builds run sequentially.
- Git builds `all` with Rust, precompiled headers, gettext, Perl/Python and Tcl/Tk
  integrations disabled using the existing benchmark's `NO_*` flags.
- Redis builds its six native tools with the libc allocator, TLS and Vector Sets
  disabled, and `__STDC_NO_ATOMICS__=1`. Lua receives explicit matching `-O2 -g0`
  and GNU99 flags; bundled C dependencies use the selected compiler.

The downloaded archives were imported into local Git repositories before timing.
This gives Redis's release-header script repository context, avoiding an expensive
implicit directory comparison outside a repository. An initial archive-only
attempt was discarded. None of the 18 successful builds changes tracked upstream
source contents. Full make commands and logs are in `temp/git-redis-opt/`.

## Results

Times are build wall-clock seconds. Reductions compare each median to its
project's baseline median; positive percentages indicate shorter builds.

| Project | Baseline | Copy changes | Copy + PGO | Copy reduction | Copy + PGO reduction |
| --- | ---: | ---: | ---: | ---: | ---: |
| Git v2.56.0 | 51.54 s | 51.18 s | 49.06 s | 0.7% | 4.8% |
| Redis 8.10.2 | 19.66 s | 19.44 s | 19.04 s | 1.1% | 3.2% |

| Project | Compiler | Run 1 | Run 2 | Run 3 |
| --- | --- | ---: | ---: | ---: |
| Git | Baseline | 51.5429 | 51.0809 | 51.7368 |
| Git | Copy changes | 51.1789 | 51.0761 | 51.2434 |
| Git | Copy + PGO | 49.0618 | 49.0799 | 48.9405 |
| Redis | Baseline | 19.6642 | 19.6834 | 19.6286 |
| Redis | Copy changes | 19.4026 | 19.4391 | 19.4840 |
| Redis | Copy + PGO | 19.0380 | 19.0434 | 19.2455 |

The small copy-only differences should be interpreted with the run-to-run
variation in mind. The PGO binary is faster than the baseline in every measured
build on these two applications. These results do not predict other targets,
optimization levels or runtime performance of the generated programs.

## Validation and provenance

All 18 builds completed and passed smoke validation outside the timer. Each Git
binary performed init/add/commit/show/status operations in a temporary repository.
Each Redis build passed PING, SET, GET, Lua EVAL and clean shutdown using its own
server and client binaries over a local Unix socket. Full upstream test suites
were not run for this timing comparison.

Compiler SHA-256 values:

| Compiler | SHA-256 |
| --- | --- |
| Baseline | `f7eb618308e2c478f93c67ab96487a2b0bc32fa699ce6bc6ab7780b5492a15bc` |
| Copy changes | `cfa32afc68f84a6e489604a0cd35c3cca436be97a11a96b26fa6d2f4996326f2` |
| Copy + PGO | `7f84bbade893ef0d2e5f586f66db679a4a87a87259f96bd303b93dee22d163e8` |

Source archive SHA-256 values:

- Git: `d761232b81394f7d4c3ef1a99fa804ffbe10d278ae1ad302833a0892050ca9fc`
- Redis: `541a374b753a8405683dd88560465791acc98cf09445ef10ec1bd286904273e8`

The comparison script, metadata, exact timings and compiler hashes, source
repositories, build logs, resource measurements and smoke results remain under
`temp/git-redis-opt/`. Those local artifacts are ignored by Git. macOS and
AArch64 were not measured.

# dynamite-compiler

A compiler for a language with syntax similar to the C language, written in Rust

> [!IMPORTANT]
> This project is development phase and is not ready for use.

## Development checks

Run `make lint` (or `cargo fmt --check` and
`cargo clippy --all-targets --all-features -- -D warnings`), followed by
`cargo test` for local validation. GitHub Actions runs the same lint checks with
Rust 1.86 before building and testing in a minimal Rust container; Rust's
`rustfmt` and `clippy` components are required locally.

Lint policy lives in `Cargo.toml`: Clippy correctness, suspicious-code and
performance groups are errors, along with debug/unfinished-code macros,
redundant clones, implicit reference-counted pointer clones and inefficient
string conversions. Other Clippy warnings also fail the lint gate. Rust denies
unused must-use results and unsafe operations without explicit unsafe blocks.
`make ci` includes the lint gate and platform tests; formatting checks do not
modify source files.

The image workflow runs only when `dockerfiles/dev.Dockerfile`, `.dockerignore`,
or `.github/workflows/publish.yaml` changes, or when started manually. Image
changes are tested in a newly built image on pull requests. Publishing runs only
on main after image validation; manual runs on main can also refresh the image.
Local Docker commands may reuse a cached image; refresh it after image changes
with `docker pull ghcr.io/cruskell/dynamite-compiler-dev:latest`.

## Verified project builds

The following upstream tags were clean-built with Dynamite compiler commit
[`f1be756`](https://github.com/cruskell/dynamite-compiler/commit/f1be756843a35d7b28e5ad49b5e10090b6e72fef)
on macOS ARM64 using the LLVM backend. Upstream source files were unchanged.

| Project | Verified tag | Validation |
| --- | --- | --- |
| Git | [`v2.56.0`](https://github.com/git/git/tree/v2.56.0) | Clean build and repository-operation smoke tests. |
| Redis | [`8.10.2`](https://github.com/redis/redis/tree/8.10.2) | Clean build, basic commands and Lua, and all 158 upstream test units plus five additional test groups. |

Git was built with `NO_RUST=YesPlease NO_PRECOMPILED_HEADER=YesPlease`.
Redis used the default macOS libc allocator, with TLS and separately cloned
external modules disabled. Its build probe automatically disabled optional
Vector Sets because C11 `_Atomic` is not supported.

These checks used a `CC` adapter around the Dynamite CLI. Dynamite parsed the
projects' C sources and generated LLVM IR; Clang handled preprocessing,
IR-to-object lowering, and linking. C compilation did not fall back to Clang's
C frontend. The listed platform and build options define the scope of these
checks.

## C build driver

Build the production tools with `cargo build --release --bins`. The
`target/release/dynamite-cc` executable can replace the temporary Python `CC`
adapter in C builds:

```sh
make -j4 CC=/absolute/path/to/target/release/dynamite-cc
```

Building Dynamite requires Rust and its platform linker, but no LLVM/Clang SDK,
C++ bridge, or LLVM development headers and libraries. LLVM IR generation is
implemented directly in Rust. The former `clang-in-process` Cargo feature has
been removed; `--all-features` builds also require no LLVM SDK.

The driver invokes the external Clang executable for LLVM IR lowering and
linking. Preprocessing always uses Dynamite's native Rust implementation.
`DYNAMITE_CLANG` selects that executable (default: `clang` from `PATH`). Removing
the SDK integration does not remove this external toolchain requirement.

On Linux x86_64 GNU systems with Debian/Ubuntu library layouts, an experimental
backend invokes LLVM tools directly instead of Clang for code generation and
linking:

```sh
DYNAMITE_BACKEND=llvm \
DYNAMITE_LLC=llc-15 DYNAMITE_OPT=opt-15 DYNAMITE_LLD=ld.lld-15 \
make -j4 CC=/absolute/path/to/target/release/dynamite-cc
```

`DYNAMITE_LLC`, `DYNAMITE_OPT`, and `DYNAMITE_LLD` default to `llc`, `opt`, and
`ld.lld`; use matching LLVM versions (tested with 15). At `-O0`, `llc` emits
objects directly. Higher optimization levels run `opt` before `llc`. LLD links
PIE/non-PIE executables or shared libraries using the system glibc startup files
and GNU runtime libraries. `DYNAMITE_GCC_RUNTIME` can select the directory
containing `crtbeginS.o`, `crtendS.o`, and libgcc; otherwise the highest installed
numeric version in `/usr/lib/gcc/x86_64-linux-gnu` is selected. No GCC executable
or LLVM SDK is used. OS headers, startup objects, and runtime libraries are still
required.

On native macOS ARM64/x86_64, the same backend uses `opt`/`llc` for Mach-O
objects and Apple's `/usr/bin/ld` for linking. No Clang executable is invoked.
Install matching LLVM tools and select their paths, for example:

```sh
DYNAMITE_BACKEND=llvm \
DYNAMITE_LLC="$(brew --prefix llvm)/bin/llc" \
DYNAMITE_OPT="$(brew --prefix llvm)/bin/opt" \
make -j4 CC=/absolute/path/to/target/release/dynamite-cc
```

`DYNAMITE_LD` overrides the macOS linker. SDK discovery uses `SDKROOT` or
`xcrun`; `-isysroot` overrides the SDK for an invocation.
`MACOSX_DEPLOYMENT_TARGET` controls both header availability and the generated
target/linker deployment version. Mach-O linking supports executables, dylibs
(`-shared`/`-dynamiclib`), bundles (`-bundle`), objects/archives, framework search,
install names, rpaths, and explicit undefined-symbol policies. Unsupported options
fail without falling back to Clang. The C source frontend remains Dynamite.

This backend supports `-c`, `-S`, `-O0` through `-O3`, `-Og`, `-Os`, `-Oz`,
PIC/PIE, `-shared`, `-no-pie`, `-pthread`, `-L`/`-l`, `-Wl,`, and `-Xlinker`.
It rejects unsupported backend options instead of falling back to Clang.
Cross targets, other library layouts, static linking, assembly/bitcode inputs,
`-emit-llvm`, LTO, sanitizers, custom CPU flags, and Clang time traces are not
supported. Debug flags are accepted, but Dynamite does not emit source debug
information. The default IR lowering and linking backend remains Clang.
Preprocessing always uses Dynamite's native implementation.

On Linux x86_64, standalone `-M`/`-MM` dependency scans use Dynamite's Rust
preprocessor by default for supported host options. It scans directives and
includes without expanding unrelated function bodies. Stateful macros such as
`__COUNTER__` and `_Pragma` still execute when they can affect the dependency
result. Independent inputs share immutable header scans and directive tokens;
macro definitions, include guards, counters, and dependency lists reset per input.
Parameter references are indexed when a macro is defined, expanded arguments are
memoized within an invocation, and header/dependency membership uses hash tables.

The same implementation can preprocess C compilation and `-E` without invoking
Clang:

```sh
make -j4 CC=/absolute/path/to/target/release/dynamite-cc
```

This implementation supports native Linux and macOS ARM64/x86_64, GNU/C89 through C17,
object/function/variadic macros, stringification, token pasting, conditional
expressions, quoted/system includes, `include_next`, `pragma once`, and
`-I`/`-isystem`/`-iquote`/`-idirafter`/`-D`/`-U`/`-include`/`-imacros`.
Its compiler headers are embedded in Dynamite; OS libc headers remain external. On macOS, `SDKROOT` or `xcrun --sdk macosx --show-sdk-path` locates the SDK without invoking Clang. `-isysroot`, `-F`, and `-iframework` select SDK/framework paths. `MACOSX_DEPLOYMENT_TARGET` selects availability macros (defaults: ARM64 11.0, x86_64 10.13). Architecture and long-double macros follow the native ABI.
Unsupported native options return an error. It is not a complete replacement
for every Clang/GNU preprocessing extension; cross targets, sysroots, macro
dumps, and date/time predefined macros are outside its current scope.
[macOS measurements](tools/benchmarks/results/macos-arm64-2026-10-17-native-pipeline-post-201.md)
found native preprocessing faster in every Git/Redis round across a six-round
comparison and a separate three-round confirmation after main #201. In the latest
comparison, median paired build time fell by 12.0% for Git and 24.8% for Redis
with Clang lowering. The Clang-free pipeline measured -9.4% and -19.8%, respectively;
the earlier six-round Git result was +0.4%, so do not treat its latest speedup as
precise. No recurring large slowdown was observed on the tested host.
LLVM IR-to-object lowering, assembly, and linking still use external tools;
libc/libm/pthread dependencies of compiled programs are unchanged.

Single-input `-c`/`-S` CLI processes retain AST and token storage until exit to
avoid individual deallocations. Multi-input and library compilation release it.
Library type constructors now use `Rc` for recursive children and parameter
lists, and token spellings use `SmolStr`; `.into()` converts owned values.
`Token.string_bytes` uses copy-on-write `StringBytes` storage: construct it with
`vec![...].into()`, borrow bytes with `.as_slice()`, or consume it with
`.into_vec()`. Clones share literal bytes until mutation. `Token.packing` is
`Option<u8>` for the supported pragma pack limits (1, 2, 4, 8, and 16).
Arithmetic node constructors infer scalar results into the existing `Node.cty`
field, avoiding repeated subtree walks. Library callers that mutate arithmetic
operands must call `refresh_arithmetic_type()` after refreshing changed children.
Nodes built directly without an inferred type still resolve from their operands.

The driver calls Dynamite's frontend directly, then uses the selected external
backend to lower LLVM IR and link. It removes the Python adapter and extra Dynamite process from each
translation unit. File preprocessing also generates `-MD`/`-MMD` dependency
files in the same preprocessing invocation, without a second dependency scan. Clang
never compiles C as a fallback when Dynamite rejects a source file.

Independent C files can be compiled concurrently in a single invocation:

```sh
target/release/dynamite-cc --jobs 4 first.c second.c main.c -O2 -o program
```

Each worker owns its complete frontend state; ASTs and mutable type definitions
are not shared between threads. The default is one worker, so a normal
`make -jN` build does not acquire additional internal parallelism. When passing
multiple files, choose `--jobs` together with the outer build's concurrency.
A multi-file link with `-MD`/`-MMD` writes separate `OUTPUT-0.d`, `OUTPUT-1.d`,
and subsequent files, each targeting the final executable. Compile-only jobs
use the usual object basename for their dependency files.

The driver supports C files, `-c`, `-S`, `-o`, preprocessing and dependency modes,
include paths and forced includes, common Clang optimization/linker flags, and
linking existing objects, archives, and assembly. `DYNAMITE_CLANG` selects the
Clang executable for both preprocessing and backend work. The existing
`dynamite_compiler -o asm|llvm FILE` interface and library APIs remain available.

This is a driver for Dynamite's supported language, not full Clang source
compatibility. Cross-target/ABI changes, C++/Objective-C, precompiled headers,
response files, and C compilation from stdin are rejected. Compile mixed C and
assembly inputs separately before linking. `-g` cannot add C source debug
metadata that the LLVM frontend does not emit.

Set `DYNAMITE_TIMINGS=1` to emit stage elapsed times to stderr. Measurements
separate preprocessing, tokenization, parsing/lowering, output generation,
teardown, IR writes, Clang lowering and linking. See
[the build benchmark tools](tools/benchmarks/README.md) for repeatable Redis/Git
comparisons and interpretation of nested stage and Clang trace totals.

## Measured Linux build times

### Native preprocessing and LLVM tools backend (2026-10-17)

At commit `de9de24` (including [#201](https://github.com/cruskell/dynamite-compiler/pull/201)),
Dynamite and Clang 15.0.7 each built clean upstream Redis 8.10.2 and Git v2.56.0
twice, in alternating order. These are **whole-build wall times**, including
dependencies and linking, on Linux x86_64 (Intel i5-5575R, Rust 1.86 release).
Both used GNU99, `-O0 -g0`, four make workers and the same benchmark wrapper;
Clang time traces were disabled. Filesystem caches were not cleared and no
compiler cache was used.

| Clean build | Dynamite median | Clang median | Dynamite elapsed reduction |
| --- | ---: | ---: | ---: |
| Redis, 184 C compilations | 10.94 s | 14.01 s | 21.9% |
| Git, 568 C compilations | 38.90 s | 40.83 s | 4.7% |

That snapshot explicitly selected native preprocessing and `DYNAMITE_BACKEND=llvm`,
with external LLVM 15 `llc`/`opt`/LLD tools and Clang disabled in its driver.
Preprocessing is now always native; selecting the LLVM tools backend remains
opt-in, and the default IR lowering/linking backend still uses Clang. Clang's reference
build used its complete C toolchain. Compiler capability macros can select
different source paths, so this compares application builds rather than
identical preprocessed inputs. It does not measure generated-program performance.

Sources were unchanged. Redis used libc, with TLS/Vector Sets/C11 atomics
disabled and Lua also at `-O0`; external/test modules were excluded. Git disabled
Rust/PCH and optional scripting/localization frontends. Redis's two C11 feature
probes succeeded only with Clang, adding different earlier standard/warning
flags; the common final `-std=gnu99` overrides those standard flags. Excluding
these probes, both compilers built the same 184 Redis C inputs. Git's 568 C
compile commands matched exactly. All eight builds passed runtime smoke checks:
Redis commands, Lua and RDB persistence checks; Git commits, branches, diffs,
`gc` and `fsck`.

These two-run samples are a snapshot, not a guaranteed speedup. Raw elapsed
samples, source commits, compiler hashes, build commands and validation are in
[the measurement snapshot](tools/benchmarks/results/linux-x86_64-2026-10-17-native-llvm.json).
Reproduce with [the benchmark runner](tools/benchmarks/README.md), setting the
LLVM tools backend and tool paths and passing `--no-trace --runs 2 --jobs 4`.

### Scalar expression type reuse (#201)

A separate alternating before/after run compared `2b6dee5` with the measured
[#201](https://github.com/cruskell/dynamite-compiler/pull/201) head `0455cd1`, using
the same native preprocessing/LLVM tools configuration. Redis's median changed
from 11.02 s to 10.90 s (-1.1%); Git's changed from 38.61 s to 38.63 s (+0.05%,
essentially unchanged). Clang was not rerun in that cohort; the fresh Clang
comparison above is a separate measurement.

Retaining scalar arithmetic result types avoids repeated subtree type queries.
For already-preprocessed scalar addition chains, parsing (including validation
and lowering) changed as follows. Each median uses four alternating samples per
variant, with five frontend iterations per sample and matching optimized harnesses.

| Addition-chain terms | Before parsing | After parsing |
| --- | ---: | ---: |
| 64 | 1.218 ms | 0.243 ms |
| 128 | 4.209 ms | 0.391 ms |
| 256 | 16.373 ms | 0.695 ms |
| 512 | 68.754 ms | 1.336 ms |

This removes quadratic repeated type resolution on these scalar arithmetic
chains; other AST traversals, type queries and backend output construction can
still be quadratic. The synthetic result does not imply the same whole-build
gain. Both cohorts' raw samples are in the snapshot linked above.

### Historical Clang preprocessing/backend measurements (2026-10-16)

The following results used earlier compiler revisions and a Clang-backed
pipeline. They are retained as historical measurements and should not be mixed
with the native preprocessing/LLVM tools results above.

Before declaration pruning, on 2026-10-16, clean upstream Redis 8.10.2 and Git v2.56.0 builds were measured
with the release compiler, Clang 15.0.7, GNU99, -O0/-g0 and four make workers on
Linux x86_64 (Intel i5-5575R, Rust 1.86). Each compiler ran twice with alternating
order; filesystem caches were not cleared. Sources were unchanged. Git disabled
Rust/PCH and optional scripting/localization frontends; Redis used libc, with
TLS/Vector Sets/C11 atomics disabled and Lua also at -O0. External/test modules
were excluded from these timed builds.

| Clean build | Dynamite median | Clang median | Ratio |
| --- | ---: | ---: | ---: |
| Redis, 184 C compilations | 15.95 s | 13.90 s | 1.15x |
| Git, 568 C compilations | 51.68 s | 40.89 s | 1.26x |

Mean Dynamite stage totals below are **worker seconds**, whose parallel work
overlaps; they must not be added to or compared directly with build wall time.

| Stage | Redis | Git |
| --- | ---: | ---: |
| Preprocessing, including Clang invocation | 8.86 | 53.55 |
| Tokenization | 1.79 | 18.54 |
| Parsing, validation and AST lowering | 5.44 | 46.13 |
| LLVM text generation | 1.28 | 4.85 |
| AST/token teardown | 0.64 | 8.39 |
| IR file write | 0.07 | 0.62 |
| Clang IR-to-object invocation | 9.74 | 34.36 |

Clang's C trace combines preprocessing/tokenization/parsing/semantics/IR
construction into Frontend, rather than matching Dynamite's inner timers. Its
Frontend totals are 13.24/106.08 worker seconds for Redis/Git. Dynamite's enclosing
frontend_total is 18.12/131.84, including the preprocessing subprocess and explicit
AST/token teardown; those timer boundaries differ from Clang's Frontend.

The common Clang backend's internal timer excludes driver startup and IR input
parsing. These comparable trace/link totals are also worker seconds:

| Phase | Redis Dynamite | Redis Clang | Git Dynamite | Git Clang |
| --- | ---: | ---: | ---: | ---: |
| Clang internal Backend | 4.04 | 2.88 | 11.25 | 5.96 |
| Link invocations | 0.40 | 0.37 | 3.19 | 3.24 |

Full trace totals, CPU/RSS data, dependency times, exact flags and repeated
samples are in the [measurement snapshot](tools/benchmarks/results/linux-x86_64-2026-10-16.json).
Capability profiles may select different code paths, so four additional serial
comparisons use identical preprocessed inputs. These show the frontend core is
competitive on these samples; extra Clang invocations and IR reading/backend
work explain much of the end-to-end gap, rather than IR text generation alone.

Unused external function declarations were subsequently removed from LLVM output,
including unused built-in libc declarations. References in retained function
bodies, reachable static inline bodies and global initializers preserve the
necessary declarations; ABI diagnostics still run before declaration omission.
Definitions retain their previous emission rules. Native assembly is unchanged.

The following before/after measurements use the same preprocessed bytes and
release binaries on the same host. Frontend runs alternate order; IR lowering
rotates before/after/Clang order, with 15 serial runs each and Clang 15 at -O0/-g0.
Times are median process wall time for Clang IR-to-object invocation, not full
build time. Nine unconditional intrinsic declarations remain in Dynamite IR.

| Input | Declarations before → after | IR bytes before → after | Lowering before → after | Clang IR lowering |
| --- | ---: | ---: | ---: | ---: |
| Redis adlist | 265 → 11 | 49,391 → 40,381 | 29.65 → 29.88 ms | 28.73 ms |
| Redis server | 4,526 → 505 | 1,484,479 → 1,322,124 | 192.34 → 179.75 ms | 140.67 ms |
| Git abspath | 5,744 → 31 | 299,703 → 48,120 | 48.32 → 30.40 ms | 29.48 ms |
| Git revision | 7,188 → 222 | 1,269,466 → 958,482 | 162.31 → 140.71 ms | 105.72 ms |

This reduces lowering time by 37.1% for abspath, 13.3% for revision and 6.5% for
server; adlist shows no gain. Median frontend generation time also decreased on
all four samples. These gains apply to these stages and inputs, not whole builds.
Input/binary hashes, timer boundaries and individual samples are in the
[declaration-pruning snapshot](tools/benchmarks/results/linux-x86_64-2026-10-16-declaration-pruning.json).

Follow-up clean builds with the same flags and two alternating runs each take
median 15.66 s / 14.00 s for Redis and 48.63 s / 41.07 s for Git
(Dynamite / Clang). Each run compiles 184 / 568 C files, with unchanged upstream
sources; basic Redis persistence/Lua/extended-float and Git repository/gc/fsck
checks pass for both compilers. These follow-up builds were not interleaved
with the old compiler, so their difference from the earlier whole-build table
is not a controlled before/after speedup measurement.

A 7,547-sample user-CPU profile of the Git revision frontend attributed at least
25.5% of self samples to allocator/free routines and 7.2% to memory copying.
A subsequent allocation change borrows reserved tokens until AST ownership is
needed, concatenates adjacent strings in the original token buffer, and inspects
call parameter/value-conversion types without unnecessary deep copies.

On the same four preprocessed inputs, 15 alternating serial runs give these
median frontend times, including tokenization, parsing, IR generation and
AST/token teardown, but excluding input/output and process startup:

| Input | Frontend before → after | Reduction | Allocation requests before → after |
| --- | ---: | ---: | ---: |
| Redis adlist | 8.22 → 7.46 ms | 9.2% | 47,896 → 42,755 |
| Redis server | 193.23 → 182.64 ms | 5.5% | 1,629,931 → 1,553,331 |
| Git abspath | 102.10 → 94.32 ms | 7.6% | 678,255 → 601,587 |
| Git revision | 227.04 → 213.67 ms | 5.9% | 1,395,429 → 1,278,665 |

Allocation counts come from three separate Linux/glibc whole-process probe runs
per compiler/input, including startup and file I/O. They count malloc/calloc/
realloc requests, not live objects or peak memory; timing runs use no allocator
instrumentation. Revision's cumulative requested bytes decrease 9.5%, from
220.71 MB to 199.76 MB. In-place concatenation retains the original token buffer
capacity, so it can leave more spare capacity allocated during parsing.
All four outputs have identical LLVM IR after canonicalizing module-level order.
Raw samples, hashes and measurement boundaries are in the
[allocation snapshot](tools/benchmarks/results/linux-x86_64-2026-10-16-allocations.json).

For this change, complete clean builds compare saved baseline/Dynamite/Clang
binaries in that order, then reverse the order on a second run. Flags, source
revisions and four-worker parallelism match the earlier builds. Median wall times:

| Project | Before | After | Reduction | Clang |
| --- | ---: | ---: | ---: | ---: |
| Redis | 15.67 s | 15.45 s | 1.4% | 13.93 s |
| Git | 48.32 s | 46.91 s | 2.9% | 40.85 s |

Both repetitions improve, but two runs are a small sample on this host and flag
set. Every build succeeds with unchanged sources and 184 / 568 C compilations;
Redis persistence/Lua/extended-float and Git repository/gc/fsck smoke checks
pass for all three compilers. Maximum per-compilation process RSS is essentially
unchanged (Redis about 99 MiB, Git 94 MiB), not a peak whole-build measurement.

A further header-processing change replaces a cloned function-signature
comparison with a structural declarator check, stores diagnostic origins only
at contiguous source-run boundaries, and borrows known unescaped marker
filenames. Source lines/columns, UTF-8 boundaries, nested includes and EOF
locations remain covered by regressions. Source-map lookup now counts newlines
within a run when rendering diagnostics, avoiding per-line mapping allocations
during successful compilation. Clang still runs once for preprocessing and once
for IR lowering per C compilation.

On identical preprocessed inputs, 15 alternating serial runs isolate the
declarator change (source mapping is outside these timers):

| Input | Frontend before → after | Reduction | Allocation requests before → after |
| --- | ---: | ---: | ---: |
| Redis adlist | 7.34 → 7.22 ms | 1.6% | 42,755 → 41,328 |
| Redis server | 181.59 → 179.89 ms | 0.9% | 1,553,331 → 1,535,777 |
| Git abspath | 93.88 → 90.79 ms | 3.3% | 601,587 → 559,990 |
| Git revision | 213.32 → 210.05 ms | 1.5% | 1,278,665 → 1,226,882 |

Allocation probes use three separate whole-process runs with the same boundaries
and limitations as the preceding experiment. All four canonical LLVM IR outputs
remain identical. Samples and hashes are in the
[header-bookkeeping snapshot](tools/benchmarks/results/linux-x86_64-2026-10-16-header-bookkeeping.json).

Full clean builds use baseline/Dynamite/Clang, then reverse that order, with the
same four-worker flags and unchanged sources:

| Project | Before median | After median | Clang median |
| --- | ---: | ---: | ---: |
| Redis | 15.46 s | 15.42 s | 13.94 s |
| Git | 46.96 s | 46.52 s | 41.00 s |

Git improves in both repetitions, about 0.9% by median; Redis shows no clear
whole-build gain. These remain small two-run samples on one host/flag set.
The mean sum of source-map construction timers decreases from 0.239 to 0.142
worker seconds for Redis and 1.813 to 1.237 for Git (40% / 32%); these overlapping
worker sums are not build wall times. Replaying Clang's unchanged line markers
for revision reduces origin entries from 35,833 to 1,448, or 859,992 to 34,752
bytes of used origin payload, excluding Vec spare capacity and other state.
Maximum per-compilation RSS remains essentially unchanged. Every build succeeds
with 184 / 568 C compilations, and Redis/Git smoke checks pass for all three
compilers.

## Proposed direction

Dynamite aims to shorten the time from editing C code to seeing results and
understanding failures. The intended experience is to edit a function, observe
its effect quickly, and receive a useful explanation when something goes wrong.
The roadmap below is a proposal, not a description of available features or a
claim of measured performance advantages.

### Fast iteration in existing applications

The initial focus would be frequently edited C components: game logic, image and
audio filters, numerical kernels, and application plugins. Preserving the host
ABI would let projects adopt Dynamite for individual components alongside code
compiled with Clang, rather than requiring a complete toolchain migration.

Development would progress from a single build-and-run command to caching with
dependency tracking, then to limited function hot reloading. The first reload
model would allow function-body changes only when function signatures and data
layouts remain unchanged. State migration after layout changes and replacement
of active function invocations require separate designs and are outside that
initial scope.

### Diagnostics that explain failures

An optional runtime checking mode would aim to report the operation, relevant
values, and source locations behind a failure. Initial candidates include
fixed-size array bounds, integer overflow, and invalid shifts. For example, a
bounds diagnostic could identify the index, valid range, and array declaration.

Existing object-size analysis and checked builtins provide a starting point,
but do not constitute general memory safety. Tracking arbitrary pointer bounds
or detecting use-after-free would require additional metadata and runtime
support. Each checking mode should state its coverage and overhead explicitly.
Structured diagnostics would make the same information usable by the CLI,
editors, and embedding applications.

### Two code generation paths

The native assembly backend would prioritize low compilation latency, with
incremental improvements to instruction selection, register use, and unnecessary
loads, stores, and stack operations. The LLVM IR path would support a workflow
that spends more time optimizing release builds through LLVM tooling.

A small typed intermediate representation should be considered when a concrete
need, such as function caching or register allocation, justifies its complexity.
Profile-guided replacement of hot functions with optimized versions is a possible
later experiment, after reloading and measurement are established.

### Milestones and evidence

1. Establish reproducible benchmarks for cold builds, rebuilds after an edit,
   time to execution, generated-code performance, and peak memory use. Measure
   preprocessing, code generation, assembly, and linking separately as well as
   end to end. Compare equivalent workloads and settings with Clang and, where
   supported, GCC and TCC; no speed advantage is assumed in advance.
2. Make one representative small application work end to end. Prioritize
   compatibility work that enables that application and validates interoperability.
3. Complete the iteration workflow on one initial platform: build and run,
   dependency-aware caching, and restricted function reloading. Expand platform
   coverage after validating the design.
4. Add explanatory runtime checks and structured diagnostics, measuring their
   effect on both iteration latency and execution performance.

The first success criterion is a demonstrably shorter edit-to-result cycle in a
real application. Broad compatibility and general-purpose optimization remain
valuable, but work should be prioritized by how it improves that experience.

## Architecture

This program adopts pipeline architecture and processes source code as input in the flow depicted in the diagram below, resulting in assembly language or LLVM IR output.

```mermaid
flowchart TD
    src([Source Code])
    tokenizer[Tokenizer]
    builder[AST Builder]
    generator[Code Generator]
    assembly([Assembly or LLVM IR])

    src --> tokenizer --> builder --> generator --> assembly
```


## Global array initialization

Assembly and LLVM output support zero-initialized global arrays and brace
initializers with omitted elements. Omitted elements are filled with zero,
including nested arrays. For example, `int values[3] = {7};` initializes the
array to `{7, 0, 0}`.

Array elements can be read and assigned in assembly and LLVM output, including
multidimensional arrays and access through pointers.

## Language behavior and validation

Both output formats support `if`, `while`, `for`, nested `break`, short-circuit
logical operators, conditional expressions, and prefix/postfix updates.
Variables declared in a `for` initializer remain in scope throughout its body.
Compound assignments and prefix updates evaluate the lvalue address once.

Array dimensions follow declaration order. Array parameters decay to pointers,
and brace initializers zero-fill omitted elements in local and global arrays.
Integer arithmetic promotes `char` operands; stores, returns, and fixed function
arguments convert to their declared types. String literals preserve UTF-8 bytes
and support common C escapes, octal escapes, and hexadecimal byte escapes.
Global constant expressions support arithmetic, comparisons, bitwise operators,
and short-circuit evaluation.

The integration suite compiles and executes both output formats with Clang on
macOS and Linux, for x86_64 and AArch64. It includes recursion, nested calls,
control flow, arrays, strings, and scalar conversions. Source diagnostics cover
truncated input, malformed literals, invalid lvalues, initializer sizes, and
function argument counts. This remains a C-like subset, not a full C compiler;
function calls support register and stack arguments.

Native calls support stack arguments using x86_64 and AArch64 calling conventions,
including compact fixed stack arguments on macOS AArch64.

Integer literals support decimal and hexadecimal (`0x` or `0X`) notation.

Comma expressions evaluate operands from left to right and produce the last value.
Commas in declarations, initializers, and argument lists remain separators.

`sizeof` accepts expressions and type names, including pointers and fixed arrays.
Its expression operand is not evaluated. String literals retain their array type
for `sizeof` and address-taking, including the terminating code unit; ordinary
value uses decay to pointers.

Fixed array dimensions accept non-negative integer constant expressions; zero lengths are a GNU extension.

Local and global arrays may omit the outer dimension when a nonempty brace
initializer supplies its size. Array parameters also accept an omitted outer dimension.

`continue` targets the nearest enclosing loop. In a `for` loop it executes the
update expression before testing the condition again.

`do ... while` executes its body before testing the condition. Its `continue`
statements jump to the condition and its `break` statements exit the loop.

`switch` supports integer constant `case` labels, `default`, fall-through, and
nested switches. `break` exits the nearest loop or switch; `continue` targets a loop.

`void` is supported for function results, empty parameter lists, and pointer
pointees. Void functions accept `return;`; objects cannot have void type.

`typedef` introduces scoped type aliases, including pointer and fixed-array aliases.

Explicit casts support integers, pointers, typedef names, and `void`. Integer
casts truncate or sign-extend to the requested type; void casts retain side effects.

Functions may declare `...` after fixed parameters. Variadic calls apply integer
promotions and platform argument placement. Variadic definitions may use their
fixed parameters and retrieve unnamed arguments through the bundled `stdarg.h`.

Parenthesized declarators support function pointers, function-pointer arrays,
callback parameters, unnamed prototype parameters, and indirect calls.
Function prototypes may mention incomplete record parameters and results; their
records must be complete at definitions and value-producing calls.
Function designators convert to compatible function pointers, ignoring top-level
parameter qualifiers while preserving qualifications on pointed-to types.

Structs support tags, anonymous typedefs, aligned members, nested aggregates,
self-referential pointers, member access (`.` and `->`), brace initialization,
and assignment. Struct parameters and results support values and pointers.
Integer bit-fields support
signed and unsigned values, unnamed padding, zero-width separators, initialization,
and assignment. Address-taking and `sizeof` on a bit-field are diagnosed.

GNU `extern inline` definitions provide local bodies for direct calls without
emitting an external definition. Taking their address still refers to the external
function. Reachable bodies are currently emitted as private helpers, so native
output does not promise instruction-level inlining.

File-scope function declarations support `__attribute__((noinline))` and its
underscored spelling. The attribute survives redeclarations and GNU inline body
lowering. LLVM output preserves the prohibition on inlining; native assembly
already emits function calls. Block-scope uses and attributes on objects,
typedefs, or parameters are diagnosed.

LLVM function definitions request unwind tables and retain frame pointers so
runtime stack walkers can traverse generated scalar and aggregate call frames.
This does not add source-level debug information or prevent ordinary inlining;
use `noinline` where individual function frames must remain observable.

## Preprocessing

Sources with directives or predefined macro names are preprocessed using the
native Rust implementation. This supports includes, object-like and function-like macros,
conditional compilation, token pasting, stringification, and predefined macros.
Quoted includes in CLI input are resolved relative to the input file; library
callers may use `generate_with_path` to supply that location. Ordinary source
without directives or predefined macros bypasses preprocessing. Preprocessing
uses C99 mode with Blocks disabled in both bundled-header and system-header modes.
The compiler defines `__DYNAMITE__` and removes Clang/LLVM frontend identity macros.
Host target, ABI and GNU compatibility macros remain available for system headers.
`__has_builtin` reports implemented operations, and `__has_attribute` reports
accepted metadata plus record alignment/packing attributes. `nothrow` and
`__nothrow__` declaration attributes are accepted conservatively without adding
exception-related optimizer assumptions. Unknown queries return
zero, including in ordinary C expressions. Language/extension feature queries
conservatively return zero rather than inheriting Clang capabilities. This does
not disable directly written supported syntax. `-D` and `-U` can explicitly
override the profile; no source-file or project-specific conditions are applied.

Compiler diagnostics retain source file names and original line numbers,
including included files. Columns within macro-expanded lines are approximate.
Preprocessing uses a small bundled include environment: `stdio.h` exposes the
builtin `printf`, `puts`, and `putchar` API, buffer formatting/parsing
(`sprintf`, `snprintf`, `sscanf`), and `EOF`; `stdlib.h` exposes `exit`,
`EXIT_SUCCESS`, `EXIT_FAILURE`, allocation functions (`malloc`, `calloc`,
`realloc`, `free`), and integer/float text conversions; `stdarg.h` exposes variadic
argument access. Host system headers are not searched by default. The CLI's `--system-headers`
option (or `PreprocessorOptions::system_headers`) selects Clang's host include
environment instead of the bundled headers. This preserves the platform's real
API declarations and ABI types; it does not guarantee that every extension used
by a system header is supported. Custom headers must use the compiler's
supported C-like subset.

## Additional language support

Declarations reject incompatible value conversions, invalid function and array
layouts, duplicate initialized objects, and incompatible global redeclarations.
Tentative declarations preserve an existing initializer.

Adjacent string literals concatenate after escape decoding. Character arrays
accept string initializers, infer their outer size, and zero-fill unused elements.
The `u8`, `u`, `U`, and `L` string prefixes are accepted. The `u` prefix stores
UTF-16 code units, while `U` and `L` store 32-bit code units. Universal escapes
are encoded using the selected literal encoding. Adjacent strings with compatible
encodings concatenate; narrow strings adopt the encoding of an adjacent prefixed
string.
Integer literals support octal notation and `U`, `L`, and `LL` suffixes. Integer
types include signed and unsigned `char`, `short`, `int`, `long`, `long long`, and
`_Bool`; both supported targets use 64-bit long and pointer types.

Global pointers accept symbolic addresses and constant element offsets. Explicit
casts to pointer-sized integer types also preserve a single symbolic address
plus constant byte offsets in static initializers. Narrowing address casts and
thread-local addresses are not constant initializers. Enum
constants support explicit constant expressions, automatic increments, and
scoped lookup. Goto targets and labels have function scope; undefined and
duplicate labels are diagnosed.

The CLI accepts repeatable `-I`, `-D`, and `-U` preprocessing options. Library
callers can use `generate_with_options` with `PreprocessorOptions`; existing
generation APIs retain their defaults.
## Compound literals

Compound literals such as `(int[]){1, 2}` and `(struct S){3}` create addressable
objects. Objects created inside a function use automatic storage; file-scope
compound literals use static storage. Postfix subscripts and member access apply
to compound literals directly.
## Designated initializers

Local and global aggregate initializers accept array indices and member names,
including paths such as `.items[2].value = 7`. A following undesignated element
continues with the next subobject, including remaining elements inside a nested
aggregate. Inner braces may be omitted for global and automatic aggregates; scalar expressions fill subobjects in order,
while string literals and compatible record values initialize whole subobjects.
Inferred-bound automatic arrays and compound literals use the highest
initialized outer element to determine their bound, including brace elision.
Their name remains an incomplete array type during initializer parsing;
`sizeof` on that name requires the completed declaration. Omitted elements are initialized
to zero, and the highest initialized index determines an omitted array bound.
## Storage duration and linkage

File-scope `static` objects and functions have internal linkage. `extern` object
declarations do not allocate storage unless accompanied by an initializer.
External array declarations may leave the outer bound unspecified. Compatible
redeclarations retain a known bound, including when a later initializer omits
the bound. Element access and symbolic offsets work before a bound is known;
`sizeof` still requires a complete array type.
Block-scope `static` objects use constant initializers and retain their values
across calls; `extern` declarations refer to externally linked objects or functions.
An object name is in scope after its declarator, including within its own
initializer. Static objects can initialize pointers to themselves or use their
complete type in `sizeof`. Addresses of thread-local objects are not constant
initializers.
## Unions

Union members share their starting address. The largest member and alignment
requirements determine the union size, including trailing padding. Unions
support tags, anonymous typedefs, member access, first-member brace initialization,
and assignment.
## Type qualifiers

`const` and `volatile` qualify object types and pointer declarators. Qualifiers
may appear between fundamental type specifiers, as in `unsigned const char`
or `long volatile unsigned int`; compatible specifier orders name the same type.
Writes to
const objects and implicit conversions that discard pointee qualifiers are
diagnosed. Qualifiers on a pointer distinguish an immutable pointer from a
pointer to an immutable object. LLVM output marks volatile object accesses;
native generation emits each access directly.

`_Alignof(type-name)` evaluates to the alignment of a complete object type.
`_Alignas` applies a requested alignment to file-scope objects, block-scope
objects, and aggregate members. Local objects are limited to 16-byte alignment.

## Floating-point values

`float` and `double` use IEEE binary32 and binary64 storage. Decimal and
hexadecimal literals support fractional parts, exponents, and the `f` suffix.
Arithmetic, comparisons, scalar conversions, global initializers, pointer
access, and increment/decrement support floating values. Mixed expressions use
the usual arithmetic conversions; variadic calls promote `float` to `double`.
Native calls use the platform floating-register and stack conventions.
On macOS AArch64, `long double` is a distinct C type using the platform's
binary64 representation, including `L` literals, arithmetic, and function calls.
On x86_64, the 80-bit format occupies 16 bytes; Linux AArch64 uses 16-byte
binary128 storage. These extended formats support declarations, size/alignment
queries, zero-initialized storage, record copies, and pointer interoperation.
The LLVM backend supports x86_64 80-bit literals, arithmetic, scalar conversions,
scalar function parameters/results, and variadic arguments. Decimal and hexadecimal
constants and constant arithmetic use software floating-point evaluation to retain
x87 precision and exponent range. Native backends diagnose extended value operations;
they still support storage, pointer interoperation, and global constant data.
Linux AArch64 binary128 remains storage-only. Passing records containing extended
formats by value, or mixing extended scalars with by-value aggregate parameters or
results, is diagnosed. Unused scalar function declarations are accepted for
system-header compatibility. Floating environment control and complex
types are not supported.

## Aggregate function values

Structures and unions can be passed and returned by value, including through
function pointers. Each argument has its own copy. Native and LLVM output use
the target calling convention, including register aggregates, AArch64
homogeneous floating aggregates, stack arguments, and indirect large results.

## Variadic argument access

The compiler provides `__builtin_va_list` using the target calling convention.
The bundled `stdarg.h` aliases this type and provides `va_list`, `va_start`, `va_arg`, `va_copy`, and
`va_end`. Variadic definitions can receive promoted scalar values and aggregate
values, including arguments that overflow the register banks. `va_start` must
name the last fixed parameter. Retrieve `float` as `double`, and retrieve narrow
integer types as their promoted integer type.

## Variable length arrays

Block-scope arrays accept integer bounds computed at runtime, including multiple
dimensions, variably modified typedefs, and pointer declarators. Bounds are
captured when the declaration executes; later changes to the source variables
do not change an existing array's size. Array parameters can refer to earlier
named parameters. `sizeof` and pointer arithmetic use the captured dimensions.

Automatic variable arrays use `malloc` and `free` to preserve the existing fixed
frame layout. Normal scope exits, returns, loop jumps, and outward gotos release
their storage. Gotos and switch labels cannot enter the scope of a variably
modified declaration. Variable arrays cannot have static storage, file scope,
struct membership, or an initializer. Static pointers to variable arrays remain
supported at block scope.

## Basic standard headers

The bundled include environment also provides `stddef.h`, `stdint.h`,
`stdbool.h`, `limits.h`, `float.h`, `string.h`, `ctype.h`, and `math.h`. These are small
subsets: fixed-width integer and pointer types, common limits, Boolean macros,
size and offset utilities, character classification and case conversion, and
common string and math function declarations.
`offsetof` is an integer constant expression, including nested member and
constant array-index paths; bit-field offsets are diagnosed.
Math functions use the system C library; link with `-lm` where required.
The headers do not claim complete standard-library coverage.

## Array parameter declarators

Function parameters accept outer array `static` bounds and bracketed `const`
or `volatile` qualifiers. The qualifiers apply to the adjusted pointer, and a
`static` bound states the caller's minimum-size requirement without a runtime
check. Unspecified `[*]` bounds are accepted in function prototypes and rejected
in function definitions.

## Flexible array members

A structure may end with a named incomplete array after another named member.
The array contributes alignment and its starting offset but no elements to the
structure size. Allocate extra storage explicitly before accessing its elements.
Structure assignment and argument passing copy only the fixed-size portion.
Flexible arrays cannot be initialized or measured with `sizeof`, and containing
structures cannot be array elements or members of another structure.

## Anonymous aggregate members

Structures and unions may contain anonymous structure or union members. Their
named members are accessible directly through the containing object, including
nested anonymous members, designated initializers, and `offsetof` paths.
The nested aggregate retains its own layout and ABI classification. Duplicate
visible member names are diagnosed, and containing qualifiers propagate through
member access.

## Restricted pointers

`restrict` qualifies pointers to objects or void, including pointer typedefs
and adjusted array parameters. It expresses the caller's aliasing obligations;
the compiler does not perform alias-based optimization or runtime checks.
Non-pointer and function-pointer applications are diagnosed.

## Inline function declarations

GNU `__inline` and `__inline__` spellings are accepted wherever the standard
`inline` function specifier is accepted. They preserve the existing C99 inline
linkage rules and diagnostics; they do not request GNU89 inline semantics.

File-scope inline function declarations accept `__attribute__((gnu_inline))`
(including `__gnu_inline__`). GNU inline definitions with `extern` supply no
external definition, even when their address is taken. Without `extern` they
supply an external definition; static functions retain internal linkage. The
attribute is inherited by later declarations. An extern GNU inline body may be
followed by an ordinary external definition in the same translation unit.
These rules do not add an inlining optimization. Applying the attribute after
an unannotated definition or in unsupported non-function contexts is diagnosed.

`static inline` and `extern inline` definitions emit internal and external
functions respectively. A plain external `inline` definition supplies no
external definition when every file-scope declaration is inline without extern.
Calls use the separate external definition; no instruction inlining is promised.
A non-inline or extern file-scope declaration makes the definition external.
External inline-only definitions cannot define mutable local static objects or
refer to identifiers with internal linkage. Local const static objects are allowed.

## Static assertions

C11 `_Static_assert(condition, "message");` declarations are supported at file,
block, and struct/union scope. The condition must be an integer constant
expression. A failed assertion reports its message at the declaration's source
location and emits no code. `<assert.h>` provides the `static_assert` macro;
runtime `assert` is not provided yet.

## Automatic storage specifiers

`auto` explicitly declares automatic block-scope objects. `register` is accepted
for automatic objects and function parameters, including adjusted array
parameters. It does not force hardware register allocation. Taking the address
of a register object or its struct/union members is diagnosed. File-scope use,
`auto` parameters, and conflicting automatic storage specifiers are rejected.

## Generic selections

C11 `_Generic` selects an association by the controlling expression's type,
with array/function decay and removal of top-level qualifiers. The controlling
expression and unselected associations are not evaluated; all expressions are
still checked. The selected expression retains its type and value category,
including lvalues and function designators. Duplicate compatible associations,
duplicate defaults, incomplete or variably modified association types, and
missing matches are diagnosed.

Integer type identity follows this compiler's existing width/signedness model:
`long` and `long long` are aliases, as are `char` and `signed char`.

## Declaration annotation attributes

GNU `__attribute__((...))` syntax accepts common diagnostic, availability,
format-checking, and optimization annotations without changing execution.
Both plain and double-underscore attribute names are accepted, including
annotations after enum constant names. These annotations
do not enable additional diagnostics or optimizations. Objective-C bridge and boxing
annotations (`objc_bridge`, `objc_bridge_mutable`, `objc_boxable`), CoreFoundation
ownership annotations (`cf_consumed`, `cf_returns_retained`,
`cf_returns_not_retained`), and Swift importer annotations (`swift_name`,
`swift_private`, `swift_wrapper`) are accepted as metadata for ordinary C. They
do not enable Objective-C syntax, ARC, or Swift import generation. Attributes that require
unsupported layout, linkage, or calling-convention behavior are diagnosed rather
than silently discarded.

## Assembler symbol labels

File-scope function and object declarations may specify an exact external symbol
with `asm("name")`, `__asm("name")`, or `__asm__("name")`. The supported labels
contain letters, digits, underscores, periods, and dollar signs. Labels affect
definitions, references, function pointers, and symbolic initializers consistently
in native and LLVM output, including declarations made after an earlier use.
Conflicting labels are diagnosed. Register-variable bindings remain unsupported;
statement assembly support is described under Register-output extended assembly.

## Pointer nullability annotations

Pointer declarations accept `_Nullable`, `_Nonnull`, `_Null_unspecified`, and
`_Nullable_result`, including pointer typedefs and function pointers. These are
interface annotations; they do not change C type compatibility, pointer layout,
or add runtime checks. Non-pointer applications and conflicting annotations on
the same pointer declarator are diagnosed. Ordinary const/volatile/restrict
qualification remains effective when mixed with these annotations.

Nullability annotations may also precede a pointer typedef, as in
`_Nullable Handle find(void)` where `Handle` is a pointer typedef. Ordinary
`const`, `volatile` and `restrict` qualifiers retain their meaning, and
conflicting annotations before and after the same typedef are diagnosed.
A leading annotation on a non-pointer base type remains invalid.

## Binary16 object layouts

`_Float16` is a distinct type with 2-byte size and alignment. Object declarations,
arrays, record members, pointers, `sizeof`, `_Alignof`, and unevaluated type queries
are supported. Static objects with omitted or empty initializers are zeroed.
Record values can be copied and object representations accessed through character
pointers. Foreign scalar function prototypes retain the binary16 type (`half` in
LLVM IR). Scalar value operations and explicit value initializers, function
definitions with scalar binary16 parameters/results, and passing or returning
records containing binary16 by value, and `va_arg` extraction of binary16 or
records containing it are diagnosed as unsupported.

## Extended integer object layouts

`__int128`, `signed __int128`, `unsigned __int128`, `__int128_t`, and
`__uint128_t` have distinct signed/unsigned types with 16-byte size and alignment.
They can occur in object declarations, pointers, arrays, records, `sizeof`, and
`_Alignof`. Static objects without a value initializer are zero-initialized.
Object representations can be accessed through character pointers, and records
containing these members can be copied. Scalar value operations, explicit value
initializers, and scalar function parameters/results are diagnosed as unsupported;
they are never lowered to truncated 64-bit operations.
The preprocessing profile withholds the host's `__SIZEOF_INT128__` capability
macro while arithmetic is unsupported, so portable sources can select their
ordinary-integer alternative. Explicit storage and size queries remain available.


## Record alignment attributes

Struct and union tags or definitions accept GNU `__attribute__((aligned(N)))`
before the tag or after the definition. Alignment is at least the
natural alignment; size includes the required tail padding. Explicit supported
alignments are powers of two through 4096 bytes; bare `aligned` requests 16 bytes.
Alignments above 16 support static storage, pointers, arrays and nested record
layout; automatic objects and records passed or returned by value remain
diagnosed because their stack ABI is not supported.
Both assembly and LLVM preserve alignment in nested records and arrays.
Postfix `aligned(N)` attributes on ordinary struct and union members raise the
member's alignment and contribute to the record's size, alignment, and ABI
classification. They apply only to the annotated member in a comma-separated
declaration; weaker requests do not lower natural alignment. Bit-field alignment
attributes and alignments above 4096 bytes remain diagnosed as unsupported. GNU packed record support is
described separately below.

## Object alignment attributes

GNU `aligned(N)` attributes on object declarations raise storage alignment without
changing the object's type or size. Prefix attributes and attributes after the
base type apply to every declarator; postfix attributes apply only to that object.
Static and external objects support powers of two through 4096 bytes, including
block-scope static and extern declarations. Redeclarations preserve the greatest
requested alignment. Bare `aligned` requests 16 bytes, and weaker requests do not
lower natural alignment. Automatic objects support alignments through 16 bytes;
larger stack alignment, function alignment and parameter alignment are diagnosed.
These rules apply to both assembly and LLVM output.

## Record packing pragmas

`#pragma pack(n)` caps ordinary struct and union member alignment at 1, 2, 4, 8,
or 16 bytes. `pack()` and `pack(0)` restore the default. `pack(push[, name][, n])`
and `pack(pop[, name][, n])` save and restore settings, including named frames.
Resetting the current setting leaves saved frames intact. Invalid settings and
pops without a matching saved frame are diagnosed.

A record uses the setting at the start of its definition. Packing also caps
member alignment attributes and `_Alignas`; record-level alignment attributes
can still raise the complete record's alignment. Native assembly and LLVM use
the same member offsets, storage size, and target ABI classification. LLVM uses
raw record storage and packed global initializer fragments so pointer relocations
do not introduce padding. Source-derived loads and stores allow unaligned access.

Pragma-packed integer bit-fields follow target layout rules, including fields
that cross their declared storage units and zero-width separators. Accesses use
only occupied bytes, preserve neighboring fields, and retain signed values and
volatile access. Argument/result classification uses the occupied bits rather
than the declared scalar container. GNU packed records with bit-fields remain
unsupported.
On macOS, static pointer initializers at unaligned offsets may require linking
with `-Wl,-no_fixup_chains`, as with the equivalent Clang-generated definitions.

## Static object-size queries

`__builtin_object_size(pointer, mode)` returns an unsigned `size_t` constant
without evaluating the pointer expression, including calls, increments, and
variable-array bounds in casts. The mode must be an integer constant from 0 to 3.
Whole-object and closest-subobject queries support direct object addresses,
fixed arrays, string literals, member accesses, pointer casts, and constant
pointer offsets. Direct conditional pointer alternatives use the maximum or
minimum known bound according to the mode.

Analysis is conservative: pointer-variable assignments, allocation sizes,
interprocedural flow, and other unresolved expressions return `(size_t)-1` for
modes 0/1 or zero for modes 2/3. Pointer/integer round trips do not retain bounds.
The result can be used in constant expressions. Dynamic object-size queries are
not implemented. See the [GCC object-size documentation](https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html)
for the mode definitions.

## Checked memory builtins

`__builtin___memcpy_chk`, `__builtin___memmove_chk`, and
`__builtin___memset_chk` use the corresponding checked C runtime operations.
The final argument supplies the destination's capacity, commonly from
`__builtin_object_size`. Capacity checks are preserved, arguments are evaluated
once, and the result is the destination as `void *`. The operations require a
compatible checked runtime, as provided by the tested macOS and Linux libcs.

## Branch expectation builtin

`__builtin_expect(value, expected)` converts its operands to `long` and returns
`value` as an rvalue. Calls, writes, and volatile reads in either operand retain
their effects and execute once. The prediction is currently ignored by code
generation; no branch-weight metadata is emitted. Constant-valued uses remain
available in constant expressions.

## Predefined function names

Within a function, `__func__` denotes a function-specific static `const char`
array containing the source function name and a terminating zero. Repeated uses
share storage, `sizeof` includes the terminator, and assembler labels do not
change the name. The array can initialize static pointers and is available in
inline definitions. Uses outside a function and writes through its const element
type are diagnosed. Block-static arrays of const elements are treated as const
objects for C11 inline constraints; arrays of mutable pointers remain mutable.

## Checked integer addition

`__builtin_add_overflow(a, b, result)` computes the mathematical sum before
checking the destination's range, writes the converted result even on overflow,
and returns `_Bool`. Signed and unsigned operands and destinations can have
independent widths through 64 bits. Boolean operands are permitted; boolean or
const destinations are rejected. Operand values and the destination address are
evaluated once, including when the destination aliases an operand.

A shared AST lowering uses unsigned low-word arithmetic and a signed high word
so all backends agree on mixed-sign sums and narrowing. It runs after
unevaluated expressions have been discarded, allowing type queries without
performing the write. Volatile destinations receive one volatile store.

## Assembly linkage symbols

The x86_64 backend quotes C linkage names in assembly definitions and operands.
Names such as `flags`, `rax`, or `offset` therefore remain symbols when used in
calls, address calculations, GOT references, and static initializer relocations.

## Checked formatted output

`__builtin___sprintf_chk` and `__builtin___snprintf_chk` call the matching
checked runtime functions with their capacity, flags, format, and variadic
arguments preserved. They return `int` character counts and retain standard
`snprintf` truncation and zero-length query behavior. Calls that violate the
supplied capacity retain the runtime's failure behavior. These builtins require
the corresponding checked libc functions.

## Checked BSD string output

`__builtin___strlcpy_chk` and `__builtin___strlcat_chk` call their matching
checked libc functions, preserving the destination, source, requested size and
object capacity. They return `size_t` and retain BSD truncation and length
semantics, including zero-sized destinations and unterminated bounded inputs.
The runtime rejects requested sizes larger than the supplied object capacity.
These builtins require `__strlcpy_chk` and `__strlcat_chk` from the target libc;
older Linux libcs may not provide these BSD APIs. Tests on those platforms use
a C runtime implementation, while macOS tests use the system libc.

## Byte-swap builtins

`__builtin_bswap16`, `__builtin_bswap32`, and `__builtin_bswap64` convert their
argument to the corresponding unsigned integer width and reverse its bytes.
Arguments are evaluated once. Integer constant operands are folded, including in
static initializers and static assertions. Runtime expressions use shared typed
AST operations, so native assembly and LLVM IR have the same semantics.

## Typedef redeclarations

C11-style same-scope typedef redeclarations are accepted when they name the same
non-variably-modified type. Type qualifiers, array bounds, and the identifier
kind must agree. Function parameter top-level qualifiers do not change function
type compatibility. Inner scopes may still introduce a different typedef with
the same name.

## Alignment of aggregate arguments

AArch64 homogeneous floating aggregates must occupy exactly their floating
members' payload; alignment padding can require integer-register passing instead.
AAPCS64 argument placement distinguishes member-derived alignment from alignment
attributes on the record itself. Native assembly and LLVM share this ABI layout.
Variadic argument cursors use the same stack alignment, and Linux AArch64
general-register arguments preserve required register-pair alignment when
retrieved with `va_arg`.
Interoperability of over-aligned homogeneous floating aggregates in Darwin
variadic calls remains a limitation: the tested Clang 21 caller and `va_arg`
lowering disagree with each other. Naturally aligned HFAs and aligned non-HFA
records are covered by cross-compiler variadic tests.

## Natural-alignment typedef annotations

Postfix `aligned(N)` attributes on complete object typedef declarations are
accepted when the greatest requested alignment equals the underlying type's natural
alignment. This includes scalar, pointer, array, and record aliases. Scalar
alignment reductions are supported as described below; other non-natural requests
remain diagnosed. Type identity and size are unchanged.

## Fixed underlying enum types

Enums accept an explicit underlying type from the existing 8-, 16-, 32-, and
64-bit integers or `_Bool`, for example
`enum Flags : unsigned int { FLAG = 0x80000000U };`. Named fixed enums can be
forward-declared and later defined with the same underlying type. Enumerators
retain that integer type, including width and signedness, and values must fit
its range. Conflicting declarations, noninteger underlying types, and implicit
increments beyond the range are diagnosed. Without an explicit base, ranges
fitting signed int retain the existing model. Extended ranges select unsigned
int or signed/unsigned 64-bit integers, including implicit increments beyond
32-bit limits. After the definition, enumerators use the selected integer type.
Ranges combining negative values with values exceeding signed 64-bit limits are
diagnosed. Enum representation and compatibility use the underlying integer
type in this compiler.

## Empty external declarations

Empty file-scope declarations (extra semicolons) are accepted, including after
macros that expand to nothing.

## Incomplete external records

An `extern` declaration without an initializer may name an incomplete struct or
union at file or block scope. Its address can be taken, stored and passed to
other functions without knowing the record layout. A later definition of the
same tag completes the shared type. Object definitions, record assignments and
`sizeof` still require a complete type. LLVM represents an external record
whose layout remains unknown with a byte declaration and opaque pointers;
this declaration does not allocate storage or assume the object's actual size.

## Attribute version tokens

Ignored declaration metadata can contain underscore-separated version tokens,
including `availability(macosx, introduced=10_0, deprecated=12_3_4)`. The tokenizer
preserves their spelling as opaque version tokens. They are not numeric constants
and cannot be used in C expressions, array bounds, enumerators or alignment
expressions. Availability annotations remain metadata and do not add deployment
target diagnostics or runtime checks.

## Multi-character constants

Ordinary multi-character constants such as `'drag'` have type `int`. Decoded
bytes are packed in source order into a signed 32-bit value, independent of the
target's memory byte order. Constants longer than four bytes retain their final
four bytes. This implementation-defined convention matches Clang on the supported
targets. Byte escape sequences are supported; raw non-ASCII characters and
universal escapes requiring multiple UTF-8 bytes remain unsupported in ordinary
character constants. These constants work in expressions and integer constant
contexts, including enumerators, static initializers and case labels.

## GNU packed records

Struct and union definitions accept `__attribute__((packed))` before the tag or
after the closing brace, including the `__packed__` spelling. Ordinary members
use byte alignment unless an explicit member alignment raises it. An active
`#pragma pack` still caps member alignment. Record-level `aligned` may raise the
aggregate alignment without changing packed member offsets. Nested record types
retain their internal layout, and flexible array offsets follow the packed layout.

Packed storage, unaligned accesses, global initialization and calling conventions
reuse the shared packed-record paths across all three backends. GNU packed
records with bit-fields, packing attributes on individual members or typedefs,
and packing attributes on incomplete tag references remain unsupported and are
diagnosed rather than ignored.

## Floating absolute-value builtins

`__builtin_fabsf` and `__builtin_fabs` convert one argument to `float` or `double`
and return its absolute value with the corresponding type. Each operand is
evaluated once. All backends clear the IEEE sign bit directly, preserving NaN
payloads and producing positive zero from negative zero, without an external
math-library call. Floating static initializers can evaluate these operations.
`__builtin_fabsl` follows the same rules on the supported binary64 long-double
target (macOS AArch64); other long-double ABIs remain unsupported.

## Floating infinity builtins

`__builtin_inff`, `__builtin_inf` and `__builtin_infl` take no arguments and
produce positive infinity with type `float`, `double` or `long double`.
`__builtin_huge_valf`, `__builtin_huge_val` and `__builtin_huge_vall` produce the
same values on the supported IEEE targets. These are typed constants usable in
floating static initializers and type queries; no runtime function is required.
Long-double forms retain the existing binary64-only target restriction.

## Explicit zero-length arrays

`T[0]` is a complete GNU array type with size zero and the element type's
alignment. It is distinct from `T[]`, whose omitted bound still requires
inference or an external declaration. Zero-length arrays can occur in globals,
locals, nested arrays and record members, including before another member.
Empty initializers do not infer an element into an explicitly zero-length array;
excess elements and conflicting nonzero redeclarations are diagnosed.

Zero-length record members contribute alignment but no element storage. Their
layout and by-value calls agree with Clang, including GNU records that contain
no data bytes. Empty records consume no argument or result payload, while their
argument expressions still evaluate normally; `va_arg` for an empty record does
not advance the cursor. Incomplete and flexible arrays retain their existing
completion and placement rules. A constant `static` array-parameter minimum
must still be positive.

## Type queries

`typeof`, `__typeof`, and `__typeof__` accept a parenthesized type name or
expression as a type specifier. They preserve qualifiers, array bounds and
function types without array/function pointer decay. Non-variably-modified
operands are unevaluated. Variably modified types retain captured bounds and
evaluate the required operand and bound expressions in their enclosing context;
`sizeof` and `_Alignof` preserve their own evaluation rules. Bit-field operands
and named declarators in type names are diagnosed.

## Narrow scalar ABI in LLVM output

LLVM function declarations, definitions and call sites carry `signext` or
`zeroext` for integer parameters and results narrower than 32 bits on x86_64 and
macOS ARM64. Linux ARM64 follows AAPCS64 without these extension attributes.
This applies to direct and indirect calls, fixed variadic parameters and scalar
parameters/results alongside records. Aggregate coercion types and LLVM function
types remain unadorned; unnamed variadic arguments retain default promotions.

Character arrays can be initialized by a string literal with optional enclosing
braces and a trailing comma, such as `char name[] = { "value", };`. The array
bound includes the terminating zero when inferred; a fixed bound may omit that
zero when it exactly fits the string. This applies to static and automatic
storage, aggregate members, compound literals, and supported wide strings.

`__builtin_popcount`, `__builtin_popcountl` and `__builtin_popcountll` count the
set bits after conversion to unsigned int, unsigned long and unsigned long long,
respectively, and return an int. Constant operands are folded; other operands
are evaluated once and lowered to shared unsigned integer operations on all
backends. The preprocessing capability queries report these builtins as supported.

Value conversions ignore top-level type qualifiers, including const-qualified
structure and union parameters. Qualifiers on pointed-to objects still constrain
pointer conversions, and const-qualified objects remain nonassignable.
Explicit scalar casts also accept top-level qualifiers on the type name, including
qualified pointer typedefs and qualified void. The result uses the unqualified
type; qualifiers within a pointer type remain part of that type.

Integer bitwise complement uses the promoted operand width in both constant
expressions and native execution. In particular, `~0U` is the unsigned 32-bit
value `4294967295`, including when widened to unsigned long or used in enum
initializers with an explicit unsigned underlying type.

`__builtin_clz`, `__builtin_ctz` and their `l`/`ll` variants return an int count
of leading or trailing zero bits after conversion to the declared unsigned
argument width. Their GNU contract requires a nonzero operand. Constant inputs
are folded, and runtime inputs are evaluated once. Runtime scans remain explicit
AST operations: LLVM uses `llvm.ctlz`/`llvm.cttz`, AArch64 uses `clz` and `rbit`,
and x86_64 uses baseline `bsr`/`bsf` with an explicit zero fallback. No optional
CPU extension or runtime library call is required.

`__builtin_constant_p(expression)` returns an int constant without evaluating its
operand or variable-array bounds. It recognizes numeric/string literals and
foldable integer expressions, conservatively returning zero for object reads,
calls and expressions with side effects. It does not perform data-flow constant
propagation or general floating-expression analysis. Invalid division and shift
operations are not classified as known constants.

## GNU atomic scalar loads and stores

`__atomic_load_n` and `__atomic_store_n` support naturally aligned 1-, 2-, 4-
and 8-byte integer, bool and pointer objects. Arguments, including runtime memory
order expressions, are evaluated once. Loads preserve the unqualified object
type; stores require a writable object and return void. Volatile accesses remain
volatile in LLVM IR.

All valid GNU load/store orders currently use the stronger sequentially consistent
implementation. Native x86_64 uses fenced loads and memory exchanges; AArch64 uses
acquire loads and release stores; LLVM emits `seq_cst` atomic operations. This
preserves synchronization while leaving weaker-order optimization for later.
Invalid constant orders and unsupported object types are diagnosed. Generic buffer forms, compare-exchange and fences remain unsupported.


Atomic read-modify-write operations include `__atomic_exchange_n`, the
`__atomic_fetch_add/sub/and/or/xor/nand` family and the corresponding
`__atomic_add/sub/and/or/xor/nand_fetch` forms. They return the old or new value
as specified and wrap arithmetic at the object width, including signed objects.
Pointer arithmetic operates on raw byte addresses, without element scaling;
boolean objects support exchange but not arithmetic updates.

These operations retain sequentially consistent ordering. Native x86_64 uses
locked compare-exchange loops, AArch64 uses acquire/release exclusive loops, and
LLVM uses atomic read-modify-write instructions. Arguments are captured before
retry loops, so contention does not repeat their side effects.

## Thread-local storage

`_Thread_local` and GNU `__thread` declarations use per-thread storage with the
LLVM backend, including file-scope definitions, external declarations and
block-scope static objects. File-scope declarations may omit `static` or `extern`;
block-scope declarations require one of them. Initial values and mutations are
independent in each thread, and external TLS symbols interoperate with Clang.
Native assembly backends diagnose thread-local declarations because they do not
yet implement TLS addressing and object-file directives.

## NaN builtins

`__builtin_nan` and `__builtin_nanf` construct quiet IEEE NaNs of type `double`
and `float`. Narrow string literals fold into constants suitable for static
initializers. Decimal, octal and hexadecimal payloads retain their low payload
bits; invalid payload spellings use zero, and strings end at the first NUL.
Nonliteral payload expressions are evaluated once and call the target libc's
`nan` or `nanf`, preserving its payload interpretation. Linux runtime calls may
require linking with libm. `__builtin_nanl` follows the same rules on macOS
AArch64, where the supported `long double` ABI is binary64; other long-double
formats remain diagnosed.

## Atomic compare-exchange

`__atomic_compare_exchange_n` supports naturally aligned 1-, 2-, 4- and 8-byte
integer, boolean and pointer objects. It returns `_Bool`, stores the desired
value on success and writes the observed value to the expected-value buffer only
on failure. All six arguments are evaluated once before the atomic operation.
The weak form uses a strong implementation, so it does not fail spuriously.

Both memory orders use sequentially consistent lowering, including runtime order
expressions. Constant orders are checked: failure cannot request release or
acquire-release and cannot be stronger than success. Expected-value buffers must
be writable and have compatible types; their volatile accesses are preserved.
Native backends use locked compare-exchange or an acquire/release exclusive loop;
LLVM emits `cmpxchg` with a conditional failure store. Generic buffer-form
`__atomic_compare_exchange` remains unsupported.

## Function designator value conversions

Function names, including variadic functions, behave as pointer values in
conditions, logical operators, implicit or explicit `_Bool` conversions and
explicit pointer-sized integer casts. Both native assembly and LLVM output
preserve these conversions. Function designators cannot be cast directly to
floating types, matching the restriction on ordinary pointer casts.

## Weak imports

File-scope external function and non-TLS object declarations accept GNU
`__attribute__((weak_import))`, including the `__weak_import__` spelling.
With linker settings that allow unresolved weak references, imported symbols
have null addresses; when a definition is linked,
ordinary calls and object accesses use that definition. Symbol aliases and
static address initializers preserve weak linkage. Redeclarations retain it,
prefix attributes apply to all object declarators, and postfix attributes apply
only to the annotated declarator.

Native output uses Mach-O `.weak_reference` or ELF `.weak`; LLVM uses
`extern_weak` declarations. Definitions of imported symbols in the same
translation unit, internal declarations, TLS imports and block-scope attributes
remain diagnosed. GNU `weak` and `__weak__` are accepted on these external
declarations with the same optional-reference behavior. Weak definitions remain
unsupported.

On macOS, completely undefined weak symbols require an appropriate weak-linked
library or an explicit linker policy such as `-undefined dynamic_lookup`, as with
Clang. Linux permits undefined weak symbols with the default ELF link policy.

## Checked integer subtraction

`__builtin_sub_overflow(a, b, out)` evaluates both integer operands and the output
pointer once, stores the converted difference, and returns `_Bool` indicating
whether the exact mathematical result fits the destination integer type. Signed
and unsigned operands of different widths are supported without first converting
them to a common C arithmetic type. The output must be a writable nonboolean
integer object; volatile stores and unevaluated contexts are preserved.

Shared AST lowering represents the exact difference with an unsigned low word
and a signed high word, correcting sign extension with the low-word borrow.
This uses the same representation as checked addition across all three backends
and does not require 128-bit integer value operations.

## Symbol visibility annotations

GNU `visibility("default")` and `__visibility__("default")` preserve the existing
visibility. File-scope functions and objects also accept `visibility("hidden")`.
Hidden references and definitions emit ELF `.hidden` or Mach-O `.private_extern`
in native output, and LLVM `hidden` visibility. Hidden symbols retain external
linkage within a program but are excluded from dynamic exports. Internal symbols
remain internal. Redeclarations retain hidden visibility; prefix attributes apply
to every declarator, while postfix attributes apply only to that declarator.
Unused hidden external declarations do not force undefined symbols into objects.
Protected/internal visibility modes and hidden annotations on non-symbol
constructs remain diagnosed.

## Vector typedef storage

Postfix GNU `vector_size(N)` attributes on ordinary integer and float scalar
typedef declarators create distinct vector types. Supported sizes are powers of
two through 16 bytes, divisible by the element size. Boolean, pointer, long-double
and storage-only extended scalar element types are not supported. Each typedef
keeps its own attribute, and const qualification is preserved.

Vectors retain their byte size and natural alignment in objects, arrays, packed
or ordinary records, pointers, type queries and implicit static zero storage.
Native assembly reserves the exact bytes; LLVM uses vector storage types.
Explicit vector-containing initializers, vector values, lane access and vector
or vector-containing record arguments/results are diagnosed. General object
vector attributes and vector arithmetic are not implemented, so the compiler
conservatively keeps `__has_attribute(vector_size)` false.

## Common object linkage

GNU `common` and `__common__` attributes on file-scope object declarations mark
uninitialized external definitions as common symbols. Matching symbols from
multiple translation units share storage, and initialized strong definitions
supply the storage when present. Types and requested alignment are retained.
An explicit initializer always produces a strong definition, even if its value
is zero; ordinary objects without this attribute retain their existing linkage.

The attribute persists across redeclarations. Prefix attributes apply to each
object in the declaration; postfix attributes belong to their own declarator.
Extern-only declarations remain references. Static objects and thread-local
objects keep their normal internal or TLS linkage, as with Clang. Functions,
block-scope objects, parameters, typedefs, arguments to the attribute and
combinations with `weak_import` are diagnosed. Both native backends emit the
platform common-symbol directive, and LLVM output uses common global linkage.

## Register-output extended assembly

Block-scope `asm`, `__asm` and `__asm__` statements support `"=r"` and `"+r"` outputs
into writable 32-bit or 64-bit integer or pointer objects, with optional
`volatile` (including its GNU spellings). Each output address is evaluated once
before the assembly executes. Packed members, indirect destinations and volatile
output stores retain their normal behavior.

Read/write `"+r"` assembly receives the object's current value in its output
register and stores the resulting value back. The destination address is still
evaluated once; volatile objects receive one volatile read and one volatile write.
LLVM ties each read/write input to its output register, and native output loads the same
register before executing the template. Empty templates preserve the input value.

Templates number all outputs first, followed by the input registers,
and `%%` for a literal percent sign.
The register width follows the output type. Native x86_64 templates use GNU AT&T
syntax; AArch64 templates use its normal assembler syntax. LLVM output preserves
side effects for volatile assembly and emits conservative condition-code clobbers.
Assembly text remains target-specific and must obey the declared register effects.

Output-free extended assembly is supported and is implicitly volatile. The
`"memory"` clobber prevents LLVM from moving memory accesses across the assembly;
native output preserves source memory access order. This includes compiler
barriers such as `asm volatile("" ::: "memory")`, which emit no machine fence.

Named outputs, non-register input constraints, register clobbers, unsupported
register modifiers, basic assembly and asm-goto are not yet supported and are
diagnosed. Empty output, input, and clobber sections are accepted.
Narrow ASCII templates are supported, including concatenation and escaped
newlines; embedded nulls and wide strings are rejected.

## Memory zeroing builtin

`__builtin_bzero(destination, length)` is available as a direct-call builtin in
native and LLVM output. It calls the target C runtime's `bzero` operation with
`void *` and `size_t` arguments and a void result. Arguments are evaluated once,
only the requested byte range is cleared, and zero lengths leave storage intact.
No prior runtime prototype is required; incompatible declarations or invalid
argument types are diagnosed. The preprocessing capability probe advertises it.

## Prefixed character constants

`L'...'`, `u'...'` and `U'...'` character constants produce the supported targets'
32-bit wchar_t, 16-bit unsigned char16_t and 32-bit unsigned char32_t values.
The wchar_t type is unsigned on Linux AArch64 and signed on the other supported
targets, consistently for character constants, wide strings and the bundled header. UTF-8 source characters, simple escapes, universal character names and
numeric escapes are supported. Numeric escapes may fill the complete destination
code unit; ordinary narrow multi-character constants retain their existing rules.
Prefixed constants require exactly one representable code unit, so multi-character
constants, invalid universal names and supplementary UTF-16 characters are diagnosed.

## Reduced scalar typedef alignment

GNU `aligned(N)` before a typedef's base type or after its declarator can reduce
an ordinary integer, float or pointer alias's object alignment. The byte size and
underlying C type identity are unchanged. Prefix attributes apply to each alias;
postfix attributes stay local, and combined requests use the greatest alignment.
Natural-alignment requests remain supported for other complete object typedefs.
Record typedefs can also raise alignment through 16 bytes, including the default
GNU `aligned` request used in glibc's pthread headers. Typedef alignment leaves
`sizeof` unchanged; array elements whose size is not a multiple of their
alignment are diagnosed.

The requested alignment is preserved in globals, locals, arrays, VLAs and record
members, including initialized storage and indirect unaligned accesses. Native
and LLVM output retain the layout, with conservative LLVM load/store alignment.
Scalar calls keep their normal ABI. Increasing scalar alignment or raising
record typedef alignment above 16 bytes, reducing record or extended
storage-only scalar alignment,
conflicting typedef redeclarations and passing/returning records with
non-natural typedef alignment by value are diagnosed. Pointer-based record interoperation is supported.

## Alias annotations

GNU `may_alias` and `__may_alias__` annotations are accepted and advertised by
the capability probe. They preserve size, alignment and underlying C type identity.
Native output performs the ordinary memory accesses, and LLVM output remains
conservative: no type-based alias-analysis metadata is emitted, including through
annotated typedefs. Alias-enabled accesses therefore remain valid when the
resulting IR is optimized with strict-aliasing enabled. Attribute arguments are
diagnosed rather than silently ignored.

## SIMD preprocessing capabilities

The preprocessing profile withholds host NEON/SVE and x86 MMX/SSE/AVX intrinsic
capability macros while those vector intrinsic families remain unimplemented.
Portable sources can therefore select their scalar alternatives. Target
architecture and ABI macros, ordinary floating-point operations and the supported
storage/type-query subset of vector typedefs remain available. This controls
source feature selection rather than changing the generated code's target CPU.

## Extended assembly register inputs

The register-output assembly subset accepts up to six independent `"r"` inputs
alongside `"=r"` or `"+r"` outputs, with at most seven total numbered operands.
Each output destination is captured once; all results are buffered before
writing output objects. Operands use 32-bit/64-bit integer or
pointer values; arrays and function designators decay to pointers. Each input is
evaluated once, and read/write matching inputs do not change the source operand
numbers. Floating-point, narrow integer and named operands remain unsupported.

AArch64 templates support `%wN` and `%xN` register-width modifiers; x86_64 supports
`%bN`, `%wN`, `%kN` and `%qN`. Unsupported modifiers and operand indices receive
source diagnostics. Native output reserves separate caller-saved input registers;
LLVM uses numbered register constraints and target operand modifiers. Normal GNU
assembly rules still apply: templates must respect their declared register effects
and write their output after reading any input that can share its register.

## Data-prefetch builtin

`__builtin_prefetch(address[, write[, locality]])` evaluates its pointer argument
once and produces a void data-cache hint. The optional integer constant hints
accept write intent 0/1 and locality 0 through 3, defaulting to read intent and
locality 3. Invalid arguments are diagnosed and the capability probe advertises it.

AArch64 output selects PRFM load/store and locality hints. Baseline x86_64 uses
read-prefetch hints for either access intent; LLVM output uses llvm.prefetch.
Hints do not read or modify C object values, and invalid target addresses do not
fault, although computing the address must itself be a valid C expression.

## Integer type identity

Plain `char` and `signed char` have distinct type identities. On the supported
LP64 targets, `long` and `long long` remain distinct 8-byte integer types;
`long long` has the higher conversion rank. Unsigned counterparts follow the
same distinction. `LL`/`ULL` literal suffixes retain that rank, and oversized
unsuffixed decimal constants use unsigned long long as an extension. Type-based
selection and declaration compatibility preserve these identities without
changing their sizes, alignment or ABI. The 64-bit byte-swap builtin returns
unsigned long long, matching Clang's builtin signature.

## Enumeration type identity

Each enumeration tag and anonymous enumeration has a distinct type identity.
Typedefs and fixed-underlying forward declarations retain that identity, while
inner-scope definitions introduce new types. Distinct enums remain incompatible
with each other even when they share an integer representation; each enum is
compatible with its selected underlying integer type. On the supported LP64
ABIs, non-fixed enums whose values fit in unsigned int and are all nonnegative
use unsigned int; negative-valued enums use a signed representation when it fits.
Ordinary enumerator constants whose full enum range fits in int retain int type,
independently of the enum representation. Enum identities
and source qualifiers are lowered to runtime types after semantic validation.

## Type compatibility queries

`__builtin_types_compatible_p(type1, type2)` produces an integer constant
expression describing compatibility without array/function decay or evaluating
VLA/typeof bounds. It ignores outer qualification (including array qualification),
retains pointed-to qualifiers, and supports the compiler's scalar, aggregate,
enum, array, and function types. An omitted array bound is compatible with a known
bound; differing known bounds remain incompatible. Preprocessing advertises this
builtin through `__has_builtin`.

## GNU statement expressions

Within a function, `({ statements; expression; })` evaluates scoped statements
and yields the final expression's value. Nested scopes, records, floating values,
array/function decay, local loops and switches, and variable-length array cleanup
are supported. The result is captured before cleanup and is not assignable.
An empty block or a final non-expression statement yields void. `__extension__`
markers are accepted before declarations, statements, and expressions.
Labels, goto, return, and break/continue that escape the statement expression are
currently diagnosed. GNU statement expressions are not integer constant
expressions and cannot appear at file scope.

## GNU transparent unions

`__attribute__((transparent_union))` is supported on complete unions and union
typedefs with matching scalar calling representations and layout. Fixed function
parameters accept a member value, compatible pointers, or null pointer constants
and use the first member's calling convention. Ordinary union objects and return
values retain their union representation. Function parameter compatibility follows
the member types; distinct union types remain distinct.

Supported members are pointers, 8/16/32/64-bit integers, and float/double. Members
must share a calling representation, size, and alignment; narrow integers must
also share signedness. Boolean, aggregate, vector, mixed pointer/integer, mixed
integer/floating, and layout-changing attributes are diagnosed. Transparent union
conversion does not apply to variadic arguments.

## Aggregate initialization storage

Brace initialization clears record padding and unused bit-field storage before
initializing members. This provides deterministic zero-filled storage for `{0}`
and empty initializers, including nested records, arrays, and compound literals.
Fixed-size aggregates without volatile subobjects use one zero-fill operation
instead of expanding implicit initialization into a store for every element.
LLVM output uses `llvm.memset`; native output uses inline zeroing instructions.
Nonvolatile record assignments and local string initializers retain compact copy
operations rather than byte-wise AST assignments. Record assignment evaluates
each operand once in the existing order and supports self-assignment. String
initialization copies only the available code units (including a terminator when
it fits), clearing any remaining storage in bulk. Native output uses inline
copying; LLVM output uses `llvm.memcpy` with conservative alignment. The intrinsic
supports copying an object onto itself and lets LLVM choose inline instructions
or a runtime call without expanding the copy into a large aggregate SSA value.
Aggregates containing volatile subobjects retain member-wise initialization.
Explicit member writes retain their normal types and volatile access widths.
C does not generally require padding bytes to have a specified value after
subsequent member assignments or record copies.

## Bitwise operand evaluation

The compiler evaluates operands of `&`, `^`, and `|` from left to right in both
native assembly and LLVM output, including nested expressions. Both operands
are always evaluated; these operators do not short-circuit. This is a compiler
evaluation policy: portable C must not rely on a particular order for these
operators, and this policy does not specify the order of other C operators or
function arguments.

## GNU system-header spellings

GNU `__const`, `__volatile`, `__restrict`, and `__signed` spellings, with or
without trailing double underscores, have the same meaning as the ordinary
keywords. `__extension__` is accepted before member type specifiers as well as
ordinary declarations and expressions. Integer typedefs accept `mode(word)`
and `__mode__(__word__)`, preserving signedness and qualifiers while selecting
the supported targets' 64-bit word type. Other machine modes and non-integer
or non-typedef uses remain diagnosed.

The GNU C `__FUNCTION__` and `__PRETTY_FUNCTION__` identifiers use the same
function-name array as `__func__`. GNU `__alignof` and `__alignof__` accept the
same type-name operands as `_Alignof`. Qualified parameter arrays also support
pointer elements, for example `int *items[restrict static 2]`.

## Stack allocation builtin

`__builtin_alloca(size)` allocates 16-byte-aligned stack storage until the current
function returns. The integer byte count is evaluated once and converted to the
target's size type. The LLVM backend emits a dynamic `alloca`; native assembly
backends diagnose this operation. File-scope uses and non-integer sizes are
rejected. The libc `alloca` macro can use this builtin without frontend fallback.

## Memory runtime builtins

`__builtin_memcpy`, `__builtin_memmove`, and `__builtin_memset` use the matching
C runtime functions on all backends. Their pointer results, argument conversions,
and single evaluation of arguments follow ordinary typed calls. These builtins
are advertised by `__has_builtin`, including for Linux CPU-set macros.

The default preprocessing dialect is GNU99, exposing the host's usual libc
extensions such as `mkstemp` when no stricter standard is requested. Explicit
`-std=c99` and other supported `-std` options override this default. This does
not change Dynamite's advertised C99 version or enable unsupported language
capabilities.

Unused `static inline` function bodies are omitted from generated output.
Reachability starts from emitted ordinary definitions and global initializers,
and follows direct calls and function addresses through inline bodies. This
retains transitive helpers and callbacks while avoiding unused header routines'
link dependencies, including in -O0 builds. All bodies are still parsed and
semantically checked; ordinary definitions and global storage remain emitted.

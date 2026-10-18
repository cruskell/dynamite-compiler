# Build-stage benchmarks

Build release binaries, clone pristine Redis 8.10.2 and Git v2.56.0 sources, then:

```sh
cargo build --release --bins
python3 tools/benchmarks/run.py \
  --redis-source /path/to/redis --git-source /path/to/git \
  --output temp/benchmark-run --clang clang-15 --runs 2 --jobs 4
python3 tools/benchmarks/summarize.py temp/benchmark-run
```

The output directory must be new. Sources are copied before each build, compiler
order alternates between repetitions, and no compiler caches are used. The host
filesystem cache is not cleared: these are clean builds, not cold-cache builds.
Both compilers run through the same Python recording wrapper. Toolchain versions,
source commits/status, compiler diff, exact commands, logs, individual invocation
records, Clang/LLVM time traces, and summarized data stay in the output directory.
A failed make stops the run. Individual compiler probes may fail by design; the
summary lists these and excludes unsuccessful invocations and Redis `foo.c`
feature probes from successful C-compilation aggregates.

Flags select -O0, -g0, GNU99 and equal make parallelism. Git disables Rust, PCH,
gettext, Perl/Python and Tcl/Tk frontends. Redis uses libc allocation, disables
TLS/Vector Sets/C11 atomics, builds its six native tools, and excludes external
modules and test modules. Lua flags are overridden explicitly so its dependency
build also uses -O0/-g0 instead of the upstream appended -O2. Archives and other
bundled dependencies still compile through the selected CC. Clang capability
macros can select different portable/intrinsic paths than Dynamite's profile;
this is an application-build comparison, not identical preprocessed input.

`DYNAMITE_TIMINGS=1` emits tab-separated `dynamite-timing`, PID, stage and elapsed
seconds records to stderr. Normal compilation remains quiet. `frontend_total`
contains `preprocess`, tokenization, parsing, generation, teardown and small
bookkeeping costs. `preprocess.*` and `parse.*` are nested measurements; do not
add them to their parent totals. `lower` measures Clang's IR-to-object process,
including startup and IR parsing; `write_ir` and linking are separate.

The wrapper passes `-ftime-trace` to Clang C compilation and Dynamite's existing
Clang backend. For Clang C input, Frontend includes preprocessing, parsing,
semantics and IR construction. For Dynamite the trace concerns IR input, not its
Rust frontend. Clang does not expose matching tokenizer/parser stage timers in
these traces. Backend, OptModule and CodeGenPasses overlap; do not add these
nested trace totals. Sum-of-invocation elapsed times are worker seconds, not
build wall time or CPU time; parallel work overlaps. CPU times and maximum RSS
are recorded separately. Maximum compilation RSS is not peak whole-build RSS.

For a serial frontend comparison on exactly the same preprocessed input:

```sh
cargo build --release --example profile_frontend
python3 tools/benchmarks/matched.py temp/benchmark-run --clang clang-15 --rounds 7
```

This selects Redis adlist/server and Git abspath/revision translation units,
preprocesses with Dynamite's capability profile, and passes identical unmarked
C bytes to each frontend. The Dynamite example runs in-process, excluding file
reads and output writes from its stage timer. Clang runs separately and its trace
reports internal Frontend/Backend/ExecuteCompiler times. These microbenchmark
boundaries differ from end-to-end driver latency: do not compare the in-process
Dynamite timer with Clang process wall time. Both generated IR files are then
lowered separately by the same Clang to expose IR-reading/backend differences.
IR bytes and declaration/definition counts are recorded.

On Linux, a CPU profile of the same in-process workload can be captured with:

```sh
perf record -e cpu-clock:u -F 999 --call-graph dwarf --no-inherit   -o temp/frontend.data -- target/release/examples/profile_frontend   temp/benchmark-run/matched/git-revision.i 40 temp/profile.ll
perf report -i temp/frontend.data --stdio --no-children --call-graph none
```

Use appropriate local perf permissions. Self samples are exclusive; inclusive
call-chain percentages may overlap and depend on unwind coverage. Compiler
version probes piped through `head` may return a closed-pipe exit without failing
make; these are retained in the failed-invocation list, separate from C builds.
Copies preserve symbolic links, and source directories must have no tracked,
untracked or ignored build changes. `--project git` or `--project redis` selects
a single project when needed.

For an optimization comparison, save the previous release `dynamite-cc` binary,
then pass `--baseline-dynamite /path/to/saved/dynamite-cc` to `run.py`. Each
project runs baseline/Dynamite/Clang, then reverses that order on even runs.
The summary includes all three compilers, and metadata records their binary
hashes. Keep builds serial and avoid unrelated CPU-intensive work during timing.

On Linux/glibc, allocation requests can be counted separately from timing:

```sh
clang -shared -fPIC -std=c11 -O2 tools/benchmarks/allocations.c -o temp/allocations.so
LD_PRELOAD="$PWD/temp/allocations.so" target/release/examples/profile_frontend \
  temp/benchmark-run/matched/git-revision.i 1 temp/profile.ll
```

The stderr record contains `allocation-count`, malloc/calloc/realloc calls,
cumulative requested bytes, and non-null free calls, separated by tabs. These
are whole-process counts, including input/output and startup; they are not
live/peak memory, and realloc requests count the entire new size. Aligned
allocation APIs are not intercepted. Counter runs add atomic-operation overhead
and must not supply performance timings. The interposer uses glibc internals
and is an optional Linux profiling tool, not part of the compiler runtime.

## Profile-guided optimization

Replay successful C compilations recorded by `cc.py` to train a PGO build of
`dynamite-cc`. Install `llvm-tools-preview` for the Rust toolchain first:

```sh
rustup component add llvm-tools-preview
python3 tools/benchmarks/pgo.py \
  --records temp/benchmark-run/redis-dynamite-1-records \
  --records temp/benchmark-run/git-dynamite-1-records \
  --output temp/pgo-run --clang clang-15 --jobs 4
```

`--toolchain VERSION` selects a Rustup toolchain. The matching `llvm-profdata`
is located in that toolchain's sysroot. The output directory must be new.
Training sources, generated headers and working directories referenced by the
records must still exist. Use trusted records: their compiler arguments are
replayed as commands, with object outputs redirected and dependency generation
disabled. Commands, build/training logs, raw and merged profiles, toolchain
metadata and both binaries remain in the output directory.

The resulting `temp/pgo-run/dynamite-pgo` is an optional compiler executable;
the normal release binary is not replaced. Only Dynamite is trained: the external
Clang backend is unchanged. Normal `RUSTFLAGS` apply equally to the instrumented
and profile-use builds; do not supply other PGO flags. Encoded Rust flags are
rejected rather than silently overriding these settings.

Compare the regular and PGO binaries with the same flags and clean-build workload.
Keep training and timing separate. Train on several projects and evaluate held-out
inputs before drawing conclusions about general compiler performance. Profiles
are specific to the source and Rust toolchain; regenerate them after changes.
See the [curl-only versus mixed-corpus measurements](results/linux-x86_64-2026-10-18-mixed-pgo.md)
for a three-application comparison using the same compiler source and different
training corpora.

For the experimental LLVM tools backend, set `DYNAMITE_BACKEND=llvm` and select
matching `DYNAMITE_LLC`, `DYNAMITE_OPT`, and `DYNAMITE_LLD` executables. Use
`--no-trace`, since Clang time-trace options are unsupported on that path.
Preprocessing always uses Dynamite's native implementation.

## Record-member scaling

Build and run `cargo run --release --example profile_record_members -- INPUT.i ...`
to measure tokenization, AST construction and teardown for synthetic wide records,
a fully initialized record, and optional already-preprocessed application inputs.
The example prints warm-run medians and excludes file reads and IR generation.
See the [record-member measurements](results/linux-x86_64-2026-10-18-record-members.md)
for the baseline comparison, Clang CLI timings and measurement boundaries.

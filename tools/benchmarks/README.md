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

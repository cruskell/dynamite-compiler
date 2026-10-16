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

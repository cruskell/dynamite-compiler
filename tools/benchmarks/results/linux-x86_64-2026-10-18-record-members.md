# Record-member frontend measurements (Linux x86_64, 2026-10-18)

The baseline is main `7804347` after the responsibility refactor; the candidate
adds the record-member changes in this PR. Both use Rust 1.86 release libraries.
Measurements ran serially on one CPU (Intel Core i5-5575R), with warm filesystem
caches and no overlapping compiler builds or regression tests.

The change replaces repeated duplicate-name scans with a declaration-local
`BTreeSet`: O(m log m) name comparisons in the worst case for m visible names,
instead of repeatedly scanning preceding members. Anonymous-member traversal
borrows definitions and accumulates paths instead of cloning all member vectors
and repeatedly prepending path indices. Indexed initializer type access borrows
the member slice, avoiding a full vector clone for each element. `member_path`
remains a linear search; there is no persistent cache to become stale when a
publicly mutable record definition changes.

## Tokenization, AST construction and teardown

`examples/profile_record_members.rs` excludes input generation, file reads and IR
generation. Each number is the median of nine samples after one warmup; synthetic
samples contain five iterations and application samples one. Values are ms per
iteration. These are frontend timings, not whole-build speedups.

| Input | Before | After | Reduction |
|---|---:|---:|---:|
| wide-128 | 0.469 | 0.158 | 66.3% |
| wide-512 | 6.052 | 0.479 | 92.1% |
| wide-2048 | 87.983 | 1.826 | 97.9% |
| initialized-512 | 354.390 | 290.354 | 18.1% |
| redis-adlist | 3.831 | 3.767 | 1.7% |
| redis-server | 91.168 | 82.981 | 9.0% |
| git-abspath | 54.886 | 54.784 | 0.2% |
| git-revision | 121.099 | 116.224 | 4.0% |

## Clang comparison: separate CLI invocation to LLVM IR

Identical preprocessed Redis/Git bytes were passed to each compiler. One warmup
and seven timed invocations per compiler/input ran serially on the same CPU,
reversing compiler order each round. Timings include process startup, input reads,
frontend work and writing IR to `/dev/null`; Dynamite also runs its normal native
preprocessing entrypoint on the preprocessed input. No assembler, optimizer tool,
object generation or linking is included. These command boundaries differ from
the in-process frontend table above. Small differences should be treated as noise.

| Input | Dynamite before (ms) | Dynamite after (ms) | Clang 15 (ms) |
|---|---:|---:|---:|
| redis-adlist | 11.243 | 11.258 | 34.808 |
| redis-server | 179.790 | 171.602 | 215.171 |
| git-abspath | 99.435 | 99.689 | 130.568 |
| git-revision | 220.796 | 217.644 | 244.571 |

Reproduce the synthetic measurements for each revision:

```sh
cargo +1.86.0 build --release --example profile_record_members
# Pin both revisions to the same available CPU.
taskset -c 0 target/release/examples/profile_record_members /path/to/redis-server.i /path/to/git-revision.i
```

For an older revision without the example, compile the same example source with
`rustc +1.86.0 -O --edition=2024`, pointing `--extern dynamite_compiler` and
`-L dependency` at that revision's release library and dependency directory.
Use the existing `matched.py` benchmark to produce the representative `.i` files.
The CLI commands for each input are:

```sh
taskset -c 0 target/release/dynamite_compiler -o llvm INPUT.i > /dev/null
taskset -c 0 clang-15 -x cpp-output -std=gnu99 -O0 -g0 -S -emit-llvm INPUT.i -o /dev/null
```

Source revisions, input hashes, exact measured CLI commands, all CLI samples and
frontend medians are in the [measurement snapshot](linux-x86_64-2026-10-18-record-members.json).
The unchanged full-build tables in the main README describe their earlier runs;
this experiment did not rerun clean Redis/Git builds or establish a new whole-build speedup.

# Repository Guidelines

These instructions apply to the entire repository. Follow the conventions of the
surrounding code and keep changes focused on the requested behavior.

## Design Philosophy and Programming Paradigm

The principles below reflect the current implementation and guide new work.
They do not imply full C compatibility or permanently freeze the architecture.

- Preserve the compiler pipeline and give each stage a clear responsibility.
  Keep source-language rules in tokenization, parsing, and shared type/AST
  operations; keep target instruction selection, ABI, and object-format details
  in the relevant backend. Share semantic decisions where practical so output
  formats agree on the meaning of a program.
- Favor direct, inspectable implementations. The current design uses recursive
  descent and generation directly from the AST. Introduce an intermediate
  representation, optimization pass, or new abstraction only when the requested
  behavior gives it a concrete purpose; explain the tradeoff when doing so.
- Use an imperative/procedural core for stateful algorithms. Parser cursors,
  scope stacks, offsets, labels, and register allocation state should have clear
  owners and explicit updates. Keep state local to the relevant stage or
  operation rather than introducing shared global mutable state.
- Combine that core with algebraic data modeling: enums describe alternatives,
  structs hold their data, and pattern matching implements operations over them.
  Follow the existing AST representation; stronger invariants may be introduced
  when a task warrants them, without redesigning unrelated node variants.
- Use functional techniques for local transformations: iterator composition,
  expression-oriented code, and side-effect-free helpers where practical.
  Do not force stateful parsing or generation into complicated iterator chains.
  This is a mixed-paradigm codebase, not a requirement for purely functional code.
- Use traits for meaningful behavioral boundaries, such as generators and error
  loggers, and inherent methods for concrete implementation details. The code
  uses encapsulation and polymorphism without requiring an object-oriented class
  hierarchy or a trait for every type.
- Make correctness observable. Validate language behavior by compiling and
  executing small programs, and check representation-specific properties when
  needed. Preserve source locations and useful diagnostics alongside semantics.
- Evolve incrementally: implement the smallest coherent change, review its
  effects across backends, and document limitations. Do not infer unsupported
  language guarantees or project goals from the C-like syntax alone.
- When improving performance, always consider more efficient algorithms and
  data structures, including worst-case complexity and repeated work. Measure
  representative workloads and preserve semantics when choosing a design.

## Project Structure

- This is a Rust 2024 compiler for a C-like language. The pipeline is source text,
  tokenization, AST construction, then assembly or LLVM IR generation.
- Keep token recognition in `src/tokenizer.rs` and token data in `src/token.rs`.
- Keep recursive-descent parsing and name resolution in `src/ast.rs`, AST node
  operations in `src/node.rs`, and type operations in `src/ctype.rs`.
- Keep shared generator interfaces in `src/generator/mod.rs`. Target-specific
  generation belongs in `src/generator/{x86_64,aarch64,llvm}/`.
- Keep command-line handling in `src/main.rs` and library orchestration in
  `src/lib.rs`. Preserve the `generate(code, output_option, is_debug)` interface
  unless the task requires changing it.

## Rust Style

- Use standard `cargo fmt` formatting. There is no repository-specific rustfmt
  configuration; do not introduce a different formatting style.
- Use `snake_case` for functions, variables, and modules, `PascalCase` for types
  and enum variants, and `SCREAMING_SNAKE_CASE` for constants. Assembly mnemonics
  and register variants use their existing uppercase conventions.
- Follow existing vocabulary such as `NodeType`, `Type`, `TokenType`, `gen_*`,
  `new_*`, `attempt_*`, and `expect_*`. Retain existing field names such as `nt`,
  `cty`, `lhs`, and `rhs`; use descriptive names for new concepts.
- Use `use crate::...` for crate modules and `use super::...` for sibling backend
  modules. Group related imports as in the surrounding file and let rustfmt
  format them. Existing backend wildcard imports for instruction and register
  enums are acceptable; do not extend that pattern indiscriminately.
- Represent compiler concepts with enums and structs, and dispatch with `match`.
  Extend existing representations rather than adding parallel string-based ones.
- Prefer small inherent methods and the existing traits at pipeline boundaries.
  Avoid adding abstraction layers or dependencies without a concrete need.
- Use `Self`, field shorthand, and `..Self::default()` or `..Node::default()`
  where they fit the existing construction patterns. Reuse node constructors
  rather than duplicating their initialization logic.
- Use iterator chains for straightforward transformations and collections, and
  explicit loops for parsing or stateful control flow. Favor readability over
  forcing either style everywhere.
- Follow existing ownership boundaries: borrow inputs and diagnostic loggers,
  use `Box<Node>` for recursive children, and use `Vec` for ordered sequences.
  Do not add clones solely to work around a design problem.
- Avoid unrelated renaming, import cleanup, or rewrites of legacy idioms while
  implementing a feature or fixing a bug.

## Parsing, Diagnostics, and Code Generation

- Preserve the recursive-descent precedence hierarchy. Use `attempt_*` helpers
  for optional syntax and `expect_*` helpers for required syntax.
- Preserve source positions in tokens and propagate the relevant token into
  AST nodes so errors can point to the original input.
- Route tokenizer errors through `SyntaxError` and parser/backend diagnostics
  through the existing `ErrorLogger` interface. Keep diagnostic text in English.
- The current error printer uses panic-based diagnostics. Follow that interface
  without expanding ad hoc panics for invalid source. Treat `unwrap()` and
  `unreachable!()` as assertions of established internal invariants, not input
  validation; do not add `todo!()` or `unimplemented!()` as finished behavior.
- Build native assembly through the backend's `Assembly`, `Instruction`, and
  operand helpers. Use raw strings for directives and labels as existing code
  does, rather than bypassing the typed instruction representation.
- Keep LLVM IR construction in the LLVM backend and preserve valid types, labels,
  and block termination when changing control flow.
- For language or AST changes, inspect all three backends. Implement applicable
  behavior consistently, or explicitly report any remaining backend limitation.
- Preserve Linux/macOS distinctions and x86_64/AArch64 calling conventions,
  register use, and stack alignment. Keep platform selection in the existing
  target configuration and generator code.

## Comments and Documentation

- Write all code comments and Markdown documentation in English.
- Keep comments brief and explain invariants, grammar, layout, or non-obvious
  decisions rather than narrating obvious statements.
- Use `///` documentation for public behavior that needs explanation; include
  runnable examples when useful, following `src/trie.rs`.
- Update relevant documentation when externally visible behavior changes.

## Tests and Validation

- Put focused unit tests in the module's `#[cfg(test)] mod tests` and compilation
  regressions in `tests/integration_test.rs`. Reuse existing helpers.
- Use short inline C programs for focused regressions and `tests/c/` fixtures
  for larger programs. Use `#[test_matrix]` for related inputs or output formats.
- Exercise both `asm` and `llvm` for shared behavior where supported. Preserve
  justified architecture guards; do not silently narrow coverage to hide a bug.
- Prefer observable program output or exit status assertions. Inspect generated
  code when the property under test is specifically about code generation.
- Integration tests invoke `clang` to compile generated assembly or IR and run
  the resulting executable. Temporary binaries belong in `tests/temp/` and
  should use the existing helper's unique names and cleanup.
- For Rust changes, run `cargo fmt --check`, `cargo clippy`, and the relevant
  tests; run `cargo test` when feasible. Report failures and unavailable tooling
  accurately rather than claiming unexecuted checks passed.
- The Makefile's `ci` target includes both macOS architectures and Docker-based
  Linux checks. Run applicable platform checks when available; cross-target
  execution requires suitable toolchains and runtimes. Note untested platforms.
- Documentation-only changes need content and diff review, not compiler tests.

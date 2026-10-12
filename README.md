# dynamite-compiler

A compiler for a language with syntax similar to the C language, written in Rust

> [!IMPORTANT]
> This project is development phase and is not ready for use.

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
Its expression operand is not evaluated.

Fixed array dimensions accept positive integer constant expressions.

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
fixed parameters; `va_list` and `va_start`/`va_arg` are not implemented.

Parenthesized declarators support function pointers, function-pointer arrays,
callback parameters, unnamed prototype parameters, and indirect calls.

Structs support tags, anonymous typedefs, aligned members, nested aggregates,
self-referential pointers, member access (`.` and `->`), brace initialization,
and assignment. Struct parameters and results currently require pointers;
passing or returning structs by value is diagnosed. Bitfields are not supported.

## Preprocessing

Sources with directives or predefined macro names are preprocessed using `clang`
from `PATH`. This supports includes, object-like and function-like macros,
conditional compilation, token pasting, stringification, and predefined macros.
Quoted includes in CLI input are resolved relative to the input file; library
callers may use `generate_with_path` to supply that location. Ordinary source
without directives or predefined macros does not invoke Clang.

Compiler diagnostics retain source file names and original line numbers,
including included files. Columns within macro-expanded lines are approximate.
Preprocessing uses a small bundled include environment: `stdio.h` exposes the
builtin `printf`, `puts`, and `putchar` API and `EOF`; `stdlib.h` exposes `exit`,
`EXIT_SUCCESS`, and `EXIT_FAILURE`. Host system headers are not searched. Custom
headers must use the compiler's supported C-like subset.

## Additional language support

Declarations reject incompatible value conversions, invalid function and array
layouts, duplicate initialized objects, and incompatible global redeclarations.
Tentative declarations preserve an existing initializer.

Adjacent string literals concatenate after escape decoding. Character arrays
accept string initializers, infer their outer size, and zero-fill unused elements.
Integer literals support octal notation and `U`, `L`, and `LL` suffixes. Integer
types include signed and unsigned `char`, `short`, `int`, `long`, `long long`, and
`_Bool`; both supported targets use 64-bit long and pointer types.

Global pointers accept symbolic addresses and constant element offsets. Enum
constants support explicit constant expressions, automatic increments, and
scoped lookup. Goto targets and labels have function scope; undefined and
duplicate labels are diagnosed.

The CLI accepts repeatable `-I`, `-D`, and `-U` preprocessing options. Library
callers can use `generate_with_options` with `PreprocessorOptions`; existing
generation APIs retain their defaults.

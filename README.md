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
Its expression operand is not evaluated. String literals retain their array type
for `sizeof` and address-taking, including the terminating code unit; ordinary
value uses decay to pointers.

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
fixed parameters and retrieve unnamed arguments through the bundled `stdarg.h`.

Parenthesized declarators support function pointers, function-pointer arrays,
callback parameters, unnamed prototype parameters, and indirect calls.

Structs support tags, anonymous typedefs, aligned members, nested aggregates,
self-referential pointers, member access (`.` and `->`), brace initialization,
and assignment. Struct parameters and results support values and pointers.
Integer bit-fields support
signed and unsigned values, unnamed padding, zero-width separators, initialization,
and assignment. Address-taking and `sizeof` on a bit-field are diagnosed.

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
`EXIT_SUCCESS`, `EXIT_FAILURE`, allocation functions (`malloc`, `calloc`,
`realloc`, `free`), and integer/float text conversions; `stdarg.h` exposes variadic
argument access. Host system headers are not searched. Custom
headers must use the compiler's supported C-like subset.

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

Global pointers accept symbolic addresses and constant element offsets. Enum
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
continues after the designated outer element. Omitted elements are initialized
to zero, and the highest initialized index determines an omitted array bound.
## Storage duration and linkage

File-scope `static` objects and functions have internal linkage. `extern` object
declarations do not allocate storage unless accompanied by an initializer.
Block-scope `static` objects use constant initializers and retain their values
across calls; `extern` declarations refer to externally linked objects or functions.
## Unions

Union members share their starting address. The largest member and alignment
requirements determine the union size, including trailing padding. Unions
support tags, anonymous typedefs, member access, first-member brace initialization,
and assignment.
## Type qualifiers

`const` and `volatile` qualify object types and pointer declarators. Writes to
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
`long double`, floating environment control, and complex types are not supported.

## Aggregate function values

Structures and unions can be passed and returned by value, including through
function pointers. Each argument has its own copy. Native and LLVM output use
the target calling convention, including register aggregates, AArch64
homogeneous floating aggregates, stack arguments, and indirect large results.

## Variadic argument access

The bundled `stdarg.h` provides `va_list`, `va_start`, `va_arg`, `va_copy`, and
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

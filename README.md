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
without directives or predefined macros does not invoke Clang. Preprocessing
uses C99 mode with Blocks disabled in both bundled-header and system-header modes.
The compiler defines `__DYNAMITE__` and removes Clang/LLVM frontend identity macros.
Host target, ABI and GNU compatibility macros remain available for system headers.
`__has_builtin` reports implemented operations, and `__has_attribute` reports
accepted metadata plus record alignment/packing attributes. Unknown queries return
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
External array declarations may leave the outer bound unspecified. Compatible
redeclarations retain a known bound, including when a later initializer omits
the bound. Element access and symbolic offsets work before a bound is known;
`sizeof` still requires a complete array type.
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
On macOS AArch64, `long double` is a distinct C type using the platform's
binary64 representation, including `L` literals, arithmetic, and function calls.
Other long-double formats are diagnosed as unsupported. Floating environment
control and complex types are not supported.

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
Conflicting labels are diagnosed. This is declaration syntax; inline assembly
statements and register-variable bindings are not supported by this feature.

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

## Record alignment attributes

Struct and union tags or definitions accept GNU `__attribute__((aligned(N)))`
before the tag or after the definition. Alignment is at least the
natural alignment; size includes the required tail padding. Explicit supported
alignments are powers of two through 16 bytes; bare `aligned` requests 16 bytes.
Both assembly and LLVM preserve alignment in nested records and arrays.
Postfix `aligned(N)` attributes on ordinary struct and union members raise the
member's alignment and contribute to the record's size, alignment, and ABI
classification. They apply only to the annotated member in a comma-separated
declaration; weaker requests do not lower natural alignment. Bit-field alignment
attributes, larger alignments, general object declarator alignment attributes,
and packed attributes remain diagnosed as unsupported.

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
accepted when every requested alignment equals the underlying type's natural
alignment. This includes scalar, pointer, array, and record aliases. Each
attribute is checked; typedef attributes that would increase or decrease
alignment remain diagnosed as unsupported. Type identity, size, and layout are
unchanged for accepted annotations.

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
are folded, and runtime inputs are evaluated once using shared integer operations
without target-specific instructions or runtime library calls.

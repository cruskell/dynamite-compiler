use crate::support::{compile_and_get_stdout, compile_and_run, compile_and_run_with_c_flags};
use rand::distributions::{Alphanumeric, DistString};
use std::fs;
use test_case::test_matrix;

#[test_matrix(["asm", "llvm"])]
fn it_preprocesses_macros_and_conditionals(output_option: &str) {
    let code = r#"
#define ADD(a,b) ((a)+(b))
#define JOIN(a,b) a ## b
#define TEXT(x) #x
#define ENABLED 1
#if defined(ENABLED) && ENABLED
int JOIN(val,ue) = ADD(2,5);
#else
#error wrong branch
#endif
#undef ENABLED
#ifdef ENABLED
#error undef failed
#elif 1
int main() { puts(TEXT(hello)); return value; }
#endif
"#;
    let output = compile_and_run(code, output_option);
    assert_eq!(output.status.code(), Some(7), "{:?}", output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello\n");
}

#[test_matrix(["asm", "llvm"])]
fn it_preprocesses_includes(output_option: &str) {
    fs::create_dir_all("tests/temp").unwrap();
    let name = format!(
        "tests/temp/{}.h",
        Alphanumeric.sample_string(&mut rand::thread_rng(), 32)
    );
    fs::write(
        &name,
        "#ifndef DYNAMITE_HEADER\n#define DYNAMITE_HEADER\n#define VALUE 7\n#endif\n",
    )
    .unwrap();
    let code = format!(
        "#include \"{}\"\n#include \"{}\"\nint main() {{ return VALUE; }}",
        std::env::current_dir().unwrap().join(&name).display(),
        std::env::current_dir().unwrap().join(&name).display()
    );
    let output = compile_and_run(&code, output_option);
    fs::remove_file(name).unwrap();
    assert_eq!(output.status.code(), Some(7), "{:?}", output);
}

#[test_matrix(["asm", "llvm"])]
fn bundled_standard_headers(output_option: &str) {
    let code = r#"
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <limits.h>
#include <float.h>
#include <string.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
struct S { char c; int value; };
int main(void) {
    char text[16];
    strcpy(text, "hello");
    memmove(text + 1, text, 5);
    text[6] = 0;
    uint64_t n = UINT64_C(18446744073709551615);
    bool ok = true;
    printf("%lu %lu %d %d %d %d", strlen(text), offsetof(struct S, value),
        n == UINT64_MAX, ok, CHAR_BIT, FLT_MANT_DIG);
    return 0;
}
"#;
    assert_eq!(compile_and_get_stdout(code, output_option), "6 4 1 1 8 24");
}

#[test_matrix(["asm", "llvm"])]
fn bundled_character_classification_header(output_option: &str) {
    let code = "#include <ctype.h>\n#include <ctype.h>\n#include <stdio.h>\nint main(){int (*lower)(int)=tolower;return !isalnum('7')+!isalpha('Z')+!isblank(' ')+!iscntrl('\\n')+!isdigit('2')+!isgraph('!')+!islower('a')+!isprint(' ')+!ispunct('!')+!isspace('\\t')+!isupper('A')+!isxdigit('f')+(lower('A')!='a')+(toupper('z')!='Z')+(tolower(EOF)!=EOF)+(isalpha(EOF)!=0)+(isdigit((unsigned char)255)!=0);}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn compiler_capabilities_select_portable_fallbacks((output_option, optimization): (&str, &str)) {
    let code = r"
#if defined(__clang__) || __has_builtin(__builtin_rotateleft32)
#error Unsupported frontend advertised
#endif
#if __has_attribute(vector_size)
typedef int Vector __attribute__((vector_size(16)));
#else
typedef int Vector;
#endif
unsigned rotate(unsigned value,int n){return (value<<n)|(value>>(32-n));}
unsigned swap(unsigned value){
#if __has_builtin(__builtin_bswap32)
    return __builtin_bswap32(value);
#else
#error Implemented builtin not advertised
#endif
}
int main(){Vector value=7;return value-7+(rotate(0x12345678u,8)!=0x34567812u)
    +(swap(0x12345678u)!=0x78563412u)+(__has_builtin(unknown)!=0)
    +(__has_builtin(__builtin_expect)!=1);}
";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn pragma_pack_expands_from_macros_and_restores_layout(output_option: &str) {
    let code = r#"
#define PACK _Pragma("pack(push,1)")
#define RESTORE _Pragma("pack(pop)")
PACK
struct Packed{char c;long x;};
RESTORE
struct Restored{char c;long x;};
_Static_assert(sizeof(struct Packed)==9,"packed size");
_Static_assert(__builtin_offsetof(struct Packed,x)==1,"packed offset");
_Static_assert(sizeof(struct Restored)==16,"restored size");
int main(){struct Packed p={1,7};return p.x!=7;}
"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

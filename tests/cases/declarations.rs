use crate::support::{
    assert_frontend_error, compile_and_get_stdout, compile_and_run, compile_and_run_with_c,
    compile_and_run_with_c_flags,
};
use test_case::test_matrix;

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn nested_function_declarators_keep_the_named_functions_parameters(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
int values[3]={7,11,13};
int (*array_result(double x))[3]{return &values;}
int (*(*choose(int index))(double))[3]{return index==2?array_result:0;}
int (*(*choose(int index))(double))[3];
int main(){return (*choose(2)(3.5))[1]-11;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn array_parameter_declarators(output_option: &str) {
    let code = "int sum(int n,int a[*]);int sum(int n,int a[const static n]){return a[0]+a[n-1]+sizeof(a);}int matrix(int n,int a[*][*]);int matrix(int n,int a[volatile static 2][n]){return a[1][n-1];}int main(){int a[3]={2,3,5};int b[2][3]={{0},{0,0,7}};return sum(3,a)+matrix(3,b)-22;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn unused_static_inline_bodies_do_not_require_external_symbols(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
int missing(void) __attribute__((visibility("hidden")));
static inline int dead_recursive(int n){return n?dead_recursive(n-1):missing();}
static inline int dead_a(void);
static inline int dead_b(void){return dead_a();}
static inline int dead_a(void){return dead_b();}
static inline int leaf(int x){return x+1;}
static inline int bridge(int x){return leaf(x);}
static inline int addressed(int x){return x+2;}
struct Holder{int (*function)(int);};struct Holder callbacks[1]={{addressed}};
int main(){return bridge(3)+callbacks[0].function(5)-11;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
    let output = dynamite_compiler::generate(code, output_option, false);
    assert!(
        !output.contains("dead_recursive")
            && !output.contains("dead_a")
            && !output.contains("dead_b")
            && !output.contains("missing")
    );
}

#[test]
fn llvm_omits_unreferenced_function_declarations() {
    let code = r#"
int unused(void);
int unused_alias(void) __asm__("unused_link_name");
int unused_weak(void) __attribute__((weak_import));
inline int unused_inline(void){return unused();}
static inline int unused_static_inline(void){return unused();}
int exported(void){return 7;}
static int retained(void){return 8;}
int main(){return 0;}
"#;
    let output = dynamite_compiler::generate(code, "llvm", false);
    assert!(!output.contains("unused"));
    for name in ["malloc", "free", "printf", "puts"] {
        assert!(!output.contains(&format!("@{name}(")));
    }
    assert!(output.contains("define i32 @exported("));
    assert!(output.contains("define internal i32 @retained("));
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn external_declarations_follow_emitted_references(output_option: &str, optimization: &str) {
    let code = r#"
int direct(int);
int addressed(int);
int renamed(int) __asm__("external_alias");
int optional(void) __attribute__((weak_import));
int from_retained(void);
inline int supplied_inline(int x){return x+100;}
static inline int leaf(int x){return direct(x);}
static inline int bridge(int x){return leaf(x);}
int retained(void){return from_retained();}
union Callback{int (*function)(int);long raw;};
struct Holder{union Callback callback;};
struct Holder callbacks[1]={{{.function=addressed}}};
int (*alias_pointer)(int)=renamed;
int main(){
    int (*local)(int)=direct;
    char *p=malloc(1);p[0]=7;int value=p[0];free(p);
    return bridge(1)+local(2)+callbacks[0].callback.function(3)
        +alias_pointer(4)+supplied_inline(5)+value-67+(optional!=0);
}
"#;
    let companion = r#"
int direct(int x){return x+1;}
int addressed(int x){return x+2;}
int renamed(int) __asm__("external_alias");
int renamed(int x){return x+3;}
int from_retained(void){return 0;}
int supplied_inline(int x){return x+38;}
"#;
    let mut flags = vec![optimization];
    if cfg!(target_os = "macos") {
        flags.push("-Wl,-undefined,dynamic_lookup");
    }
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &flags)
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "static inline int add(int x){return x+1;}int main(){int (*p)(int)=add;return p(6)-7;}",
    "extern inline int add(int x){return x+1;}int main(){return add(6)-7;}",
    "static inline int add(int a,int b,int c,int d,int e,int f,int g){return a+b+c+d+e+f+g;}int main(){int (*p)(int,int,int,int,int,int,int)=add;return 1+p(1,2,3,4,5,6,7)-29;}",
    "static inline int add(int x){return x+1;}int main(){long extra=2;int (*p)(int)=add;return extra+p(6)-9;}",
    "inline int add(int x){return x+1;}int add(int x);int main(){return add(6)-7;}",
    "int add(int x);inline int add(int x){return x+1;}int main(){return add(6)-7;}",
    "const inline static int add(int x){static int count;return x+ ++count;}int main(){return add(6)-7;}",
    "extern inline int add(int x){return x+1;}int main(){inline int add(int x);return add(6)-7;}",
], ["asm", "llvm"])]
fn inline_function_definitions(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([false, true], ["asm", "llvm"], ["-O0", "-O2"])]
fn gnu_inline_spellings_preserve_standard_inline_linkage(
    trailing_underscores: bool,
    output_option: &str,
    optimization: &str,
) {
    let keyword = if trailing_underscores {
        "__inline__"
    } else {
        "__inline"
    };
    let code = format!(
        r"
int __inline__count;
struct Pair{{long x;long y;}};
static {keyword} __attribute__((always_inline)) struct Pair make(long x){{
    __inline__count++;struct Pair p={{x,x+1}};return p;
}}
{keyword} int external(void){{return 3;}}
extern {keyword} int defined(void){{return 9;}}
int {keyword} postfix(void){{return 5;}}
extern int postfix(void);
int main(){{
    {keyword} int external(void);
    struct Pair (*p)(long)=make;struct Pair value=p(7);
    return external()-7+defined()-9+postfix()-5+__inline__count-1
        +(value.x!=7)+(value.y!=8);
}}
"
    );
    assert_eq!(
        compile_and_run_with_c_flags(
            &code,
            output_option,
            Some("int external(void){return 7;}"),
            &[optimization]
        )
        .status
        .code(),
        Some(0)
    );
}

#[test_matrix([
    "INLINE int object;",
    "INLINE int f(void){static int x;return x;}",
    "int main(){INLINE int local;return 0;}",
], [false, true], ["asm", "llvm"])]
fn invalid_gnu_inline_spellings_are_diagnosed(
    code: &str,
    trailing_underscores: bool,
    output_option: &str,
) {
    let keyword = if trailing_underscores {
        "__inline__"
    } else {
        "__inline"
    };
    let code = code.replace("INLINE", keyword);
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(&code, output_option, false))
            .is_err()
    );
}

#[test_matrix([false, true], ["-O0", "-O2"])]
fn noinline_functions_remain_in_runtime_backtraces(aggregate: bool, optimization: &str) {
    let code = if aggregate {
        "struct Pair{long a,b;};int capture(void);struct Pair compiler_frame(struct Pair) __attribute__((__noinline__));struct Pair compiler_frame(struct Pair p){p.a+=capture();return p;}int main(){struct Pair p={2,9};p=compiler_frame(p);return p.a-2;}"
    } else {
        "int capture(void);__attribute__((noinline)) int compiler_frame(void);int compiler_frame(void){return capture()+1;}int main(){return compiler_frame()-1;}"
    };
    let companion = String::from(
        r#"
#include <execinfo.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
int capture(void){void *frames[32];int count=backtrace(frames,32);char **names=backtrace_symbols(frames,count);int found=0;
if(names){for(int i=0;i<count;i++){fprintf(stderr,"%s\n",names[i]);if(strstr(names[i],"compiler_frame"))found=1;}free(names);}return found?0:100;}
"#,
    );
    let generated = dynamite_compiler::generate(code, "llvm", false);
    assert!(
        generated.lines().any(|line| line.starts_with("define ")
            && line.contains("@compiler_frame(")
            && line.contains(" noinline")),
        "{generated}"
    );
    let mut flags = vec![optimization, "-fno-omit-frame-pointer"];
    if cfg!(target_os = "linux") {
        flags.push("-rdynamic");
    }
    let result = compile_and_run_with_c_flags(code, "llvm", Some(&companion), &flags);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn noinline_attributes_preserve_external_declarations_and_redeclarations() {
    let code = "int external(void) __attribute__((noinline));struct Pair{long a,b;};struct Pair aggregate(struct Pair) __attribute__((__noinline__));int later(void){struct Pair p={1,2};return external()+aggregate(p).a;}int later(void) __attribute__((noinline));int main(){return 0;}";
    let generated = dynamite_compiler::generate(code, "llvm", false);
    for name in ["external", "aggregate", "later"] {
        assert!(
            generated
                .lines()
                .any(|line| line.contains(&format!("@{name}(")) && line.contains(" noinline")),
            "{generated}"
        );
    }
}

#[test_matrix(["asm", "llvm"])]
fn noinline_attributes_follow_gnu_inline_local_bodies(output_option: &str) {
    let code = "extern inline __attribute__((gnu_inline,noinline)) int value(void){return 7;}int main(){return value()-7;}";
    let generated = dynamite_compiler::generate(code, output_option, false);
    if output_option == "llvm" {
        assert!(
            generated
                .lines()
                .any(|line| line.contains("@.inline.value(") && line.contains(" noinline")),
            "{generated}"
        );
    }
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int value __attribute__((noinline));",
    "int value(void) __attribute__((noinline(1)));",
    "typedef int value __attribute__((noinline));",
    "int value(int p __attribute__((noinline)));",
    "int main(){extern int value(void) __attribute__((noinline));return 0;}",
])]
fn invalid_noinline_attributes_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn gnu_inline_direct_calls_use_local_bodies((output_option, optimization): (&str, &str)) {
    let code = r"
#define HEADER extern inline __attribute__((gnu_inline))
int missing(void);
HEADER int unused(void){return missing();}
static int calls;
HEADER int next(int n){calls++;return n+1;}
HEADER int nested(int n){return next(n)+1;}
HEADER int recurse(int n){return n?recurse(n-1)+1:0;}
HEADER int variadic(int n,...){return n+1;}
struct Pair{long a,b;};
HEADER struct Pair record(struct Pair p){p.a++;return p;}
int main(){struct Pair p={7,9};p=record(p);
    return nested(40)!=42 || calls!=1 || recurse(5)!=5 || variadic(41,3)!=42 || p.a!=8 || p.b!=9;}
";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn gnu_inline_addresses_keep_external_identity(output_option: &str, optimization: &str) {
    let code = r"
extern inline __attribute__((gnu_inline)) int value(void){return 3;}
int compare(int (*p)(void));
int main(){int (*p)(void)=value;return value()!=3 || p()!=7 || compare(value);}
";
    let companion =
        "int value(void){return 7;}int compare(int (*p)(void)){return p!=value || p()!=7;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn gnu_inline_attributes_preserve_definition_and_reference_linkage(
    output_option: &str,
    optimization: &str,
) {
    let code = r"
#if !__has_attribute(gnu_inline) || !__has_attribute(__gnu_inline__)
#error GNU inline attribute missing
#endif
extern int only(void);
extern __inline __attribute__((__gnu_inline__)) int only(void){return 3;}
extern int only(void);
__attribute__((gnu_inline)) inline int emitted(void){return 9;}
extern inline int replaced(void) __attribute__((gnu_inline));
extern inline __attribute__((gnu_inline)) int replaced(void){return 1;}
int replaced(void){return 11;}
extern inline __attribute__((gnu_inline)) int inherited(int);
inline int inherited(int x){return x+13;}
static int internal=5;
extern inline __attribute__((gnu_inline)) int private_body(void){static int x;return ++x+internal;}
inline int standard(void){return 1;}
struct Pair{long x;long y;};
__attribute__((gnu_inline)) inline struct Pair record(long n){struct Pair p={n,n+1};return p;}
int compare(int (*f)(void),int expected);
int check_emitted(void);
int main(){
    struct Pair p=record(7);
    return compare(only,7)+compare(standard,17)+compare(private_body,21)
        +check_emitted()+(emitted()!=9)+(replaced()!=11)+(inherited(2)!=15)
        +(p.x!=7)+(p.y!=8);
}
";
    let companion = r"
int only(void){return 7;}
int standard(void){return 17;}
int private_body(void){return 21;}
int emitted(void);int inherited(int);int replaced(void);
int compare(int (*f)(void),int expected){return f()!=expected;}
int check_emitted(void){return emitted()!=9 || inherited(2)!=15 || replaced()!=11;}
";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int x __attribute__((gnu_inline));",
    "__attribute__((gnu_inline)) int f(void);",
    "inline int f(void) __attribute__((gnu_inline(1)));",
    "inline int (*p)(void) __attribute__((gnu_inline));",
    "inline int f(int x __attribute__((gnu_inline)));",
    "inline int f(void){return 0;}inline int f(void) __attribute__((gnu_inline));",
    "extern inline __attribute__((gnu_inline)) int f(void){return 1;}extern inline __attribute__((gnu_inline)) int f(void){return 2;}",
    "inline int f(void){return 1;}int f(void){return 2;}",
    "inline struct S{int x __attribute__((gnu_inline));} f(void);",
])]
fn invalid_gnu_inline_attributes_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn inline_uses_external_definition(output_option: &str) {
    let code = "inline int value(void){static const int local=1;return local;}int main(){return value()-7;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some("int value(void){return 7;}"))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "inline int object;int main(){return 0;}",
    "inline int mutable(void){static int x;return 0;}int main(){return 0;}",
    "static int internal;inline int reference(void){return internal;}int main(){return 0;}",
    "static int hidden(void){return 1;}inline int caller(void){return hidden();}int main(){return 0;}",
    "int main(){inline int local;return 0;}",
])]
fn rejects_invalid_inline(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn inline_external_aggregate(output_option: &str) {
    let code = "struct Pair{long a;long b;};inline struct Pair value(struct Pair p){p.a=1;return p;}int main(){struct Pair p={3,4};p=value(p);return p.a+p.b-11;}";
    let companion = "struct Pair{long a;long b;};struct Pair value(struct Pair p){p.a=7;return p;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn static_assert_declarations(output_option: &str) {
    let code = "#include <assert.h>\n#include <stddef.h>\nenum {N=4};static_assert(sizeof(long)==8,\"long size\");struct S{int x;_Static_assert(N==4,\"enum\");long y;char tail[];_Static_assert(sizeof(int)==4,\"after tail\");};_Static_assert(offsetof(struct S,y)==8,\"member offset\");int main(){int x=3;static_assert(N==4?1:0,\"conditional\");{_Static_assert((int)3.5==3,\"cast\");}return x-3;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "_Static_assert(0,\"failed\");int main(){return 0;}",
    "int n;_Static_assert(n,\"variable\");int main(){return 0;}",
    "_Static_assert(1.0,\"floating\");int main(){return 0;}",
    "_Static_assert((int)((double)1+1),\"nonconstant cast\");int main(){return 0;}",
    "_Static_assert(1,1);int main(){return 0;}",
    "int main(){int n=3;int a[n];_Static_assert(sizeof(a)>0,\"vla\");return 0;}",
])]
fn rejects_invalid_static_assert(code: &str) {
    assert_frontend_error(code);
}

#[test]
fn static_assert_reports_message() {
    let result = std::panic::catch_unwind(|| {
        dynamite_compiler::generate("_Static_assert(0,\"layout \" \"mismatch\");", "llvm", false)
    })
    .unwrap_err();
    let message = result.downcast_ref::<String>().unwrap();
    assert!(message.contains("static assertion failed: layout mismatch"));
}

#[test_matrix(["asm", "llvm"])]
fn bundled_allocation_and_numeric_conversion_declarations(output_option: &str) {
    let code = "#include <stdlib.h>\n#include <stdlib.h>\nint main(){char *p=calloc(4,sizeof(char));int bad=p[0]+p[1]+p[2]+p[3];p[0]=7;void *(*resize)(void*,size_t)=realloc;p=resize(p,8);bad+=(p[0]!=7);p[7]=11;bad+=(p[7]!=11);free(p);char *end;double d=strtod(\"1.25x\",&end);bad+=(d!=1.25)+(end[0]!='x');float f=strtof(\"-1.5y\",&end);bad+=(f!=-1.5)+(end[0]!='y');long signed_value=strtol(\"-17z\",&end,10);bad+=(signed_value!=-17)+(end[0]!='z');unsigned long unsigned_value=strtoul(\"ff!\",&end,16);bad+=(unsigned_value!=255)+(end[0]!='!');long long wide_signed=strtoll(\"-123\",&end,10);bad+=(wide_signed!=-123)+end[0];unsigned long long wide_unsigned=strtoull(\"18446744073709551615\",&end,10);bad+=(wide_unsigned!=18446744073709551615ULL)+end[0];return bad+(atoi(\"-23\")!=-23)+(atol(\"123\")!=123)+(atoll(\"456\")!=456)+(atof(\"2.5\")!=2.5);}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn bundled_buffer_formatting_and_scanning_declarations(output_option: &str) {
    let code = r#"
#include <stdio.h>
#include <stdio.h>
#include <string.h>
int main(){
    char buffer[32];float f=1.5;
    int (*format)(char*,const char*,...)=sprintf;
    int length=format(buffer,"%d %.1f %s",17,f,"ok");
    int bad=(length!=9)+(strcmp(buffer,"17 1.5 ok")!=0);
    int n=0;double d=0;char word[4];
    int count=sscanf(buffer,"%d %lf %3s",&n,&d,word);
    bad+=(count!=3)+(n!=17)+(d!=1.5)+(strcmp(word,"ok")!=0);
    char small[3];length=snprintf(small,sizeof(small),"%s","abcd");
    bad+=(length!=4)+(small[0]!='a')+(small[1]!='b')+small[2];
    bad+=(snprintf(NULL,0,"%d",123)!=3);
    return bad;
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn declaration_metadata_attributes_preserve_execution(output_option: &str) {
    let code = r#"__attribute__((unused)) static int calls;int add(int n __attribute__((unused))) __attribute__((__warn_unused_result__, availability(macos,introduced=10.10)));__attribute__((noinline)) int add(int n){calls++;return n+1;}typedef int scalar __attribute__((deprecated("message with ) parentheses")));struct __attribute__((deprecated)) Pair{int a;int b;};enum __attribute__((flag_enum,enum_extensibility(open))) Flags{Flag=1};int main(){__attribute__((unused)) static int local=Flag;scalar n=3;int (*p)(int) __attribute__((unused))=add;struct Pair pair={1,2};int result=p(n);return result+calls+pair.a+pair.b+local-9;}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn common_attribute_keeps_declarator_ownership_and_redeclarations(output_option: &str) {
    let code = "__attribute__((common)) int first,second;int third __attribute__((common)),strong;extern int imported __attribute__((common));int initialized __attribute__((common))=3;int zero __attribute__((common))=0;static int local __attribute__((common));int use(void){return first+second+third+strong+imported+initialized+local;}";
    let generated = dynamite_compiler::generate(code, output_option, false);
    if output_option == "llvm" {
        for name in ["first", "second", "third"] {
            assert!(
                generated.contains(&format!("@{name} = common global")),
                "{generated}"
            );
        }
        assert!(generated.contains("@strong = global"), "{generated}");
        assert!(generated.contains("@zero = global i32 0"), "{generated}");
        assert!(
            generated.contains("@initialized = global i32 3"),
            "{generated}"
        );
        assert!(
            generated.contains("@imported = external global"),
            "{generated}"
        );
        assert!(
            generated.contains("@local = internal global"),
            "{generated}"
        );
    } else {
        let generated = generated.replace('"', "");
        let prefix = if cfg!(target_os = "macos") { "_" } else { "" };
        for name in ["first", "second", "third"] {
            assert!(
                generated.contains(&format!(".comm {prefix}{name},4,")),
                "{generated}"
            );
        }
        for name in ["strong", "initialized", "zero", "imported", "local"] {
            assert!(
                !generated.contains(&format!(".comm {prefix}{name},")),
                "{generated}"
            );
        }
    }
}

#[test]
fn common_thread_local_objects_keep_thread_local_linkage() {
    let generated = dynamite_compiler::generate(
        "_Thread_local int object __attribute__((common));int read(void){return object;}",
        "llvm",
        false,
    );
    assert!(
        generated.contains("@object = thread_local global"),
        "{generated}"
    );
    assert!(!generated.contains("common thread_local"), "{generated}");
}

#[test_matrix([
    "int f(void) __attribute__((common));",
    "struct S{int member __attribute__((common));}object;",
    "enum E{ITEM __attribute__((common))}object;",
    "int main(){int object __attribute__((common));return 0;}",
    "void f(int object __attribute__((common)));",
    "typedef int T __attribute__((common));",
    "__attribute__((common)) typedef int T;",
    "__attribute__((common)) struct S{int n;};",
    "int object __attribute__((common(1)));",
    "extern int object __attribute__((common,weak_import));",
    "extern int object __attribute__((weak_import));extern int object __attribute__((common));",
], ["asm", "llvm"])]
fn invalid_common_attributes_have_source_diagnostics(code: &str, output_option: &str) {
    let panic =
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .unwrap_err();
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains('^'), "{message}");
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn default_visibility_annotations_preserve_linkage(output_option: &str, optimization: &str) {
    let code = r#"
extern __attribute__((__visibility__("default"))) int imported;
static __attribute__((visibility("default"))) int internal_object=3;
static int local(int n) __attribute__((visibility("de" "fault")));
static int local(int n){return n+internal_object;}
__attribute__((visibility("default"))) int exported=7;
int api(int n) __attribute__((unused,visibility("default")));
int api(int n){return local(n)+imported+exported;}
int check(void);int main(void){return check();}
"#;
    let companion = "int imported=11;extern int exported;int api(int);int check(void){return exported!=7 || api(5)!=26;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn hidden_visibility_preserves_linkage_and_unused_declarations(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
extern int unused_object __attribute__((visibility("hidden")));
int unused_function(void) __attribute__((visibility("hidden")));
__attribute__((visibility("hidden"))) extern int imported;
int foreign(int) __attribute__((visibility("hidden")));
__attribute__((visibility("hidden"))) int first=3,second=5;
int local(int) __attribute__((visibility("hidden")));
int local(int x){return x+first+second+imported;}
extern int first;int local(int);
struct Pair{int x;int y;};
struct Pair pair(struct Pair x) __attribute__((visibility("hidden")));
struct Pair pair(struct Pair x){x.x+=1;return x;}
static int internal __attribute__((visibility("hidden")))=7;
int renamed(void) __asm__("hidden_alias") __attribute__((visibility("hidden")));
int renamed(void){return internal;}
int (*saved)(int)=foreign;
int check(void);int main(){struct Pair p={7,9};p=pair(p);return local(2)!=21 || saved(3)!=10 || renamed()!=7 || p.x!=8 || p.y!=9 || check();}
"#;
    let companion = "int imported=11;int foreign(int x){return x+7;}extern int first,second;int local(int);int check(void){return first!=3 || second!=5 || local(1)!=20;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
    let output = dynamite_compiler::generate(code, output_option, false);
    if output_option == "llvm" {
        assert!(
            output.contains("define hidden i32 @local(")
                && output.contains("@first = hidden global")
        );
    } else {
        let directive = if cfg!(target_os = "macos") {
            ".private_extern"
        } else {
            ".hidden"
        };
        assert!(output.contains(directive));
        assert!(
            !output
                .lines()
                .any(|line| line.contains(directive) && line.contains("unused_"))
        );
    }
}

#[test_matrix([
    "typedef int Alias __attribute__((visibility(\"hidden\")));",
    "int f(int x __attribute__((visibility(\"hidden\"))));",
    "struct S{int x __attribute__((visibility(\"hidden\")));};",
    "int main(){int x __attribute__((visibility(\"hidden\")));return 0;}",
])]
fn hidden_visibility_on_non_symbols_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test]
fn default_visibility_annotations_preserve_internal_llvm_symbols() {
    let ir = dynamite_compiler::generate(
        "static int value __attribute__((visibility(\"default\")))=3;static int local(void) __attribute__((visibility(\"default\")));static int local(void){return value;}int main(){return local()-3;}",
        "llvm",
        false,
    );
    assert!(
        ir.lines()
            .any(|line| line.starts_with("@value = internal global"))
    );
    assert!(
        ir.lines()
            .any(|line| line.starts_with("define internal i32 @local("))
    );
}

#[test_matrix([
    "int x __attribute__((visibility(\"protected\")));",
    "int x __attribute__((visibility(\"internal\")));",
    "int x __attribute__((visibility()));",
    "int x __attribute__((visibility(1)));",
    "int x __attribute__((visibility(L\"default\")));",
    "int x __attribute__((visibility(\"default\",\"hidden\")));",
])]
fn unsupported_visibility_annotations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn weak_import_declarations_preserve_optional_linkage(output_option: &str, optimization: &str) {
    let code = r#"
extern int absent(void) __attribute__((weak_import));
extern int missing __attribute__((weak_import));
__attribute__((weak_import)) extern int missing_first, missing_second;
extern int present(int) __attribute__((__weak_import__));
extern int present(int);
extern int available __attribute__((weak_import));
extern int available;
extern signed char narrow(signed char) __attribute__((weak_import));
struct Pair{long x;long y;};
extern struct Pair pair(struct Pair) __attribute__((weak_import));
extern int renamed(void) __asm__("weak_present_alias") __attribute__((weak_import));
extern int absent_alias(void) __asm__("weak_absent_alias") __attribute__((weak_import));
int (*saved_absent)(void)=absent;
int (*saved_present)(int)=present;
int *saved_missing=&missing,*saved_available=&available;
int main(void){
    if(absent || &missing || &missing_first || &missing_second || absent_alias)return 1;
    if(saved_absent || saved_missing || !present || !&available)return 2;
    if(present(3)!=7 || available!=11 || saved_present(5)!=9 || *saved_available!=11)return 3;
    if(narrow(-3)!=-7 || renamed()!=17)return 4;
    struct Pair value={19,23};value=pair(value);return value.x!=23 || value.y!=19;
}"#;
    let companion = "int available=11;int present(int n){return n+4;}signed char narrow(signed char n){return n-4;}int provided_alias(void) __asm__(\"weak_present_alias\");int provided_alias(void){return 17;}struct Pair{long x;long y;};struct Pair pair(struct Pair p){struct Pair r={p.y,p.x};return r;}";
    let mut flags = vec![optimization];
    if cfg!(target_os = "macos") {
        flags.push("-Wl,-undefined,dynamic_lookup");
    }
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &flags)
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn gnu_weak_declarations_preserve_optional_linkage(output_option: &str, optimization: &str) {
    let code = "extern int absent(void) __attribute__((__weak__));extern int missing __attribute__((weak));extern int present(void) __attribute__((weak));int main(){return absent!=0 || &missing!=0 || present()!=7;}";
    let companion = "int present(void){return 7;}";
    let mut flags = vec![optimization];
    if cfg!(target_os = "macos") {
        flags.push("-Wl,-undefined,dynamic_lookup");
    }
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &flags)
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn weak_import_declarations_can_remain_unused(output_option: &str) {
    let code = "extern int missing __attribute__((weak_import));int absent(void) __attribute__((weak_import));int main(){return 0;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn weak_import_attributes_do_not_leak_between_declarators() {
    let ir = dynamite_compiler::generate(
        "extern int first __attribute__((weak_import)),second;int main(){return &first!=0;}",
        "llvm",
        false,
    );
    assert!(
        ir.lines()
            .any(|line| line.starts_with("@first = extern_weak global"))
    );
    assert!(
        ir.lines()
            .any(|line| line.starts_with("@second = external global"))
    );
}

#[test_matrix([
    "int value __attribute__((weak_import));",
    "extern int value __attribute__((weak_import))=1;",
    "extern _Thread_local int value __attribute__((weak_import));",
    "static int f(void) __attribute__((weak_import));",
    "int f(void) __attribute__((weak_import)){return 0;}",
    "int f(void){return 0;}int f(void) __attribute__((weak_import));",
    "int f(void) __attribute__((weak_import));int f(void){return 0;}",
    "int value;extern int value __attribute__((weak_import));",
    "extern int value __attribute__((weak_import));int value;",
    "extern int invalid_argument __attribute__((weak_import(1)));",
    "struct S{int member __attribute__((weak_import));};",
    "int f(int parameter __attribute__((weak_import)));",
    "int main(){extern int object __attribute__((weak_import));return 0;}",
    "typedef int Alias __attribute__((weak_import));",
])]
fn invalid_weak_import_declarations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([
    "__thread int f(void);",
    "typedef __thread int T;",
    "__thread int x; int x;",
    "int main(){__thread int x;return 0;}",
    "int main(){static __thread int f(void);return 0;}",
])]
fn invalid_thread_local_declarations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["static __thread int x;", "extern _Thread_local int x;"])]
fn native_thread_local_storage_is_diagnosed(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "asm", false)).is_err());
}

#[test_matrix([
    "struct __attribute__((packed(1))) S{int x;};",
    "int value __attribute__((aligned(8192)));",
    "int value __attribute__((vector_size(16)));",
    "int value __attribute__((unknown_attribute));",
    "int value __attribute__((deprecated(\"unterminated\");",
    "int value __attribute__((unused);",
])]
fn unsupported_or_malformed_attributes_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn assembler_labels_preserve_linkage_and_symbolic_addresses(output_option: &str) {
    let code = r#"
int exported __asm__("shared_object")=17;
int actual(int x) __asm__("shared_function");
int actual(int x){return x+exported;}
int *global_pointer=&exported;
int (*global_function)(int)=actual;
struct Handles{int *p;int (*f)(int);};
struct Handles handles={&exported,actual};
union Link{int *p;long value;};union Link link={&exported};
extern int foreign_data __asm__("external_object");
int foreign(int);int before(){return foreign(2);}
int foreign(int) __asm__("external_function");
int companion(void);
int main(){return (*global_pointer!=17)+(global_function(1)!=18)
    +(handles.f(2)!=19)+(*handles.p!=17)+(*link.p!=17)
    +(foreign_data!=3)+(before()!=5)+(companion()!=35);}
"#;
    let companion = r#"
extern int data __asm__("shared_object");
int function(int) __asm__("shared_function");
int partner_data __asm__("external_object")=3;
int partner_function(int n) __asm__("external_function");
int partner_function(int n){return n+3;}
int companion(void){return data+function(1);}
"#;
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn incomplete_array_redeclarations_preserve_the_composite_bound(output_option: &str) {
    let code = "extern int first[];int first[3]={1,2,3};extern int first[];extern int second[4];int second[]={7};int main(){extern int first[];return sizeof(first)+sizeof(second)+first[2]+second[0]+second[3]-38;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "extern int a[];char a[3];",
    "extern int a[3];extern int a[4];",
    "extern int a[2];int a[]={1,2,3};",
    "extern const int a[];extern int a[3];",
    "extern int a[];int main(){return sizeof(a);}",
    "static int a[];",
])]
fn invalid_incomplete_array_declarations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn extended_integer_static_storage_is_zeroed(output_option: &str) {
    let code = "__uint128_t words[2];union U{__int128 wide;char bytes[16];};union U data;int target=17;union P{__uint128_t wide;int *p;};union P pointer={.p=&target};struct Wrapper{char tag;union P payload;char end;};struct Wrapper wrapper={3,{.p=&target},5};int main(){unsigned char *bytes=(unsigned char*)words;int i=0;while(i<32){if(bytes[i])return 1;i++;}i=0;while(i<16){if(data.bytes[i])return 2;i++;}return sizeof(words)+_Alignof(__int128_t)+sizeof(wrapper)-96+(*pointer.p-17)+(wrapper.tag-3)+(wrapper.end-5)+(*wrapper.payload.p-17);}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn empty_file_scope_declarations_are_accepted(output_option: &str) {
    let code = r#"
#define OMIT(argument)
;;;OMIT(ignored);
typedef int Number;;;
int value=7;;;
_Static_assert(sizeof(Number)==4,"int width");;;
int read_value(void);;;
int read_value(void){OMIT(statement);return value;};;;
int main(){return read_value()-7;};;;
"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "1;int main(){return 0;}",
    "return 1;",
    "int value=;",
    "struct S{int x;};}",
])]
fn nondeclarations_at_file_scope_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn predefined_function_names_have_const_array_storage(output_option: &str, optimization: &str) {
    let code = r#"
#include <string.h>
int check_name(const char *,const char *,unsigned long);
const char *first(void){return __func__;}
static const char *hidden(void){return __func__;}
const char *renamed(void) __asm__("dynamite_renamed_name");
const char *renamed(void){return __func__;}
inline const char *inline_only(void){return __func__;}
inline const char *inline_array(void){static const char text[]="ok";return text;}
inline const char *inline_pointer_array(void){static const char *const text[1]={"ok"};return text[0];}
inline const char *inline_defined(void){return __func__;}
extern const char *inline_defined(void);
int saved(void){
    static const char *pointer=__func__;
    const char (*whole)[sizeof(__func__)]=&__func__;
    return pointer!=__func__ || __func__[0]!='s' || sizeof(__func__)!=6
        || check_name(*whole,"saved",sizeof(__func__))
        || __builtin_object_size(__func__,0)!=6
        || _Generic(__func__,const char*:0,default:1);
}
int main(){
    return saved() || first()!=first() || check_name(first(),"first",6)
        || check_name(hidden(),"hidden",7) || check_name(renamed(),"renamed",8)
        || check_name(inline_defined(),"inline_defined",15)
        || check_name(__func__,"main",sizeof(__func__))
        || _Generic(&__func__,const char(*)[5]:0,default:1);
}
"#;
    let companion = r#"
#include <string.h>
int check_name(const char *name,const char *expected,unsigned long size){
    return strcmp(name,expected)!=0 || strlen(name)+1!=size;
}
"#;
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"])]
fn gnu_predefined_function_names(output_option: &str) {
    let code = "int named(void){return sizeof(__FUNCTION__)!=6 || sizeof(__PRETTY_FUNCTION__)!=6 || __PRETTY_FUNCTION__[0]!='n' || __FUNCTION__[4]!='d';}int main(){return named();}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "const char *name=__func__;",
    "const char *f(){return __func__;}const char *name=__func__;",
    "int main(){__func__[0]='x';}",
    "int main(){char *p=__func__;}",
    "inline int f(){static int values[2];return values[0];}",
    "inline const char *f(){static const char *values[1]={\"x\"};return values[0];}",
])]
fn invalid_predefined_function_names_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn assembler_register_and_operator_names_remain_linkage_symbols(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
long flags=3,offset[2]={5,7},rax=11,xmm0=13,byte=17;
long *pointers[5]={&flags,offset+1,&rax,&xmm0,&byte};
extern long rip;extern long rcx(long);
long *imported=&rip;long (*external_function)(long)=rcx;
long rdx(long x){return x+flags+rax;}
long and(long x){return rdx(x)+byte;}
long (*function)(long)=and;
long alias(long) __asm__("r8");
long alias(long x){return x+23;}
int main(){
    return flags!=3 || offset[1]!=7 || rax!=11 || xmm0!=13 || byte!=17 || rip!=19
        || *pointers[0]!=3 || *pointers[1]!=7 || *pointers[2]!=11 || *pointers[3]!=13 || *pointers[4]!=17
        || *imported!=19 || rdx(2)!=16 || and(2)!=33 || function(2)!=33
        || rcx(2)!=21 || external_function(2)!=21 || alias(2)!=25;
}
"#;
    let companion = "long rip=19;long rcx(long x){return x+rip;}";
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "int main(){return 10_0;}",
    "int value=10_0;",
    "int values[10_0];",
    "enum E{VALUE=10_0};",
    "struct S{int value __attribute__((aligned(1_0)));};",
])]
fn attribute_versions_are_not_c_numeric_constants(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn nothrow_declarations_preserve_calls_and_effects(output_option: &str, optimization: &str) {
    let code = r#"
#if !__has_attribute(nothrow) || !__has_attribute(__nothrow__)
#error nothrow must be advertised
#endif
int foreign(int) __attribute__((__nothrow__));
static int calls;
int add(int) __attribute__((nothrow));
__attribute__((__nothrow__,noinline)) int add(int x){calls++;return foreign(x)+1;}
int main(void){int (*p)(int)=add;int a=p(3);int b=p(5);return a+b+calls-22;}
"#;
    let companion = "int foreign(int x){return 2*x+1;}";
    let output =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix(["asm", "llvm"])]
fn gnu_extension_markers_preserve_declarations_and_evaluation(output_option: &str) {
    let code = r#"
__extension__ typedef unsigned long long Word;
__extension__ static Word value=7;
#define ONCE(x) __extension__ ({int captured=(x);captured*2;})
int main(void){__extension__ Word local=value;int n=0;int result=ONCE(++n);
    return local!=7 || result!=2 || n!=1 || __extension__ (n+2)!=3;}
"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn prototype_fast_path_keeps_existing_definitions_and_unevaluated_bounds(output_option: &str) {
    let code = "int calls; int f(int a[++calls]); int g(int x); int g(int x){return x+1;} int g(int x); int main(){printf(\"%d,%d\",calls,g(41));return 0;}";
    assert_eq!(compile_and_get_stdout(code, output_option), "0,42");
}

use crate::support::{
    assert_frontend_error, compile_and_get_stdout, compile_and_run, compile_and_run_with_c,
    compile_and_run_with_c_flags,
};
use test_case::test_matrix;

#[test_matrix([
    ("int a[3]; int main() { return a[0] + a[1] + a[2]; }", 0),
    ("int a[3] = {7}; int main() { return a[0] + a[1] + a[2]; }", 7),
    ("char a[3] = {7}; int main() { return a[0] + a[1] + a[2]; }", 7),
    ("int a[2][3] = {{7}, {}}; int main() { return a[0][0] + a[0][1] + a[1][2]; }", 7),
    ("int a[3]; int main() { a[1] = 9; return a[0] + a[1] + a[2]; }", 9),
    ("char a[3]; int main() { a[1] = 9; return a[0] + a[1] + a[2]; }", 9),
    ("int main() { int a[3]; a[0] = 2; a[1] = 5; a[2] = 9; return a[0] + a[1] + a[2]; }", 16),
    ("int a[3] = {2, 5, 9}; int main() { int *p = a; return *(p + 2) + *(p - 0); }", 11),
    ("int a[2][3]; int main() { a[1][2] = 13; return a[1][2] + a[0][2]; }", 13),
    ("int a[3] = {2, 5, 9}; int main() { int *p = &a[2]; return p[-1]; }", 5),

], ["asm", "llvm"])]
fn it_reads_and_writes_array_elements(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix(["asm", "llvm"])]
fn it_preserves_string_bytes(output_option: &str) {
    let code = r#"char *message = "日本語\t\"\\\x0A"; int main() { printf("%s", message); puts("done"); return 0; }"#;
    assert_eq!(
        compile_and_get_stdout(code, output_option),
        "日本語\t\"\\\ndone\n"
    );
}

#[test_matrix(["asm", "llvm"])]
fn it_supports_hexadecimal_integers(output_option: &str) {
    let output = compile_and_run(
        "int n = 0X10; int main() { int a[0x2] = {0xf, 0xA}; return n + a[0] + a[1]; }",
        output_option,
    );
    assert_eq!(output.status.code(), Some(41), "{:?}", output);
    let output = compile_and_run(
        "int main() { int n = 0xFFFFFFFF; return n == -1; }",
        output_option,
    );
    assert_eq!(output.status.code(), Some(1), "{:?}", output);
}

#[test_matrix([
    ("int main() { int x = 0; return (x = 3, x += 4, x); }", 7),
    ("int main() { int x = 0; int y = 2; for (x = 0, y = 1; x < 3; x++, y += 2) {} return y; }", 7),
    ("int add(int a, int b) { return a + b; } int main() { int a[2] = {2, 3}; return add((a[0] = 4, a[0]), a[1]); }", 7),
    ("int main() { int x = 0; return 1 ? (x = 2, x + 5) : 0; }", 7),
], ["asm", "llvm"])]
fn it_supports_comma_expressions(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("int main() { int a[1 + 2]; return sizeof(a); }", 12),
    ("int a[sizeof(int)][2 * 3]; int main() { return sizeof(a); }", 96),
    ("int main() { return sizeof(char[1 ? 7 : 2]); }", 7),
], ["asm", "llvm"])]
fn it_supports_constant_array_dimensions(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("int n; void set(void) { n = 7; return; } int main(void) { set(); return n; }", 7),
    ("int n; void set() { n = 7; } int main() { set(); return n; }", 7),
    ("int main() { int n = 7; void *p = &n; return sizeof(p); }", 8),
    ("int n; void set() { n = 7; } void call() { return set(); } int main() { call(); return n; }", 7),
    ("int n; void a() { n = 7; } void b() { n = 3; } int main() { 1 ? a() : b(); return n; }", 7),
], ["asm", "llvm"])]
fn it_supports_void(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("int main() { return (char)300 + 3; }", 47),
    ("int n = (char)255; int main() { return n + 8; }", 7),
    ("typedef char Byte; int main() { int n = 7; void *p = &n; return *(int*)p + (Byte)256; }", 7),
    ("int main() { int n = 0; (void)n++; return n; }", 1),
    ("int main() { return (int)(char)255 + 8; }", 7),
    ("int main() { return (int)(void*)(char)-1 == -1; }", 1),
    ("int main() { return (int)(void*)0xFFFFFFFF == -1; }", 1),
], ["asm", "llvm"])]
fn it_supports_casts(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix(["asm", "llvm"])]
fn it_concatenates_adjacent_strings(output_option: &str) {
    let code =
        r#"int main(){char *p="\x41" /* gap */ "B" "\0" "C";return p[0]+p[1]+p[2]+p[3]-198;}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn it_supports_octal_integers(output_option: &str) {
    assert_eq!(
        compile_and_run("int main(){return 010+077-71;}", output_option)
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([


    "int main(){return sizeof(short)+sizeof(long)+sizeof(unsigned char)+sizeof(_Bool)-12;}",
    "int main(){unsigned int x=4294967295U;return (x/3==1431655765U && x>1)-1;}",
    "int main(){long x=4294967296L;return (x+3==4294967299L)-1;}",
    "short g=-3; unsigned short h=65535; int main(){return g+h-65532;}",
    "_Bool f(int x){return x;} int main(){_Bool x=9;return x+f(-3)-2;}",
    "int main(){unsigned long x=18446744073709551615UL;return (x/3==6148914691236517205UL)-1;}",
    "int main(){long x=4294967296L;switch(x){case 4294967296L:return 0;default:return 9;}}",
    "int main(){unsigned int x=4294967295U;return (x>>31)-1;}",
    "int main(){int x;_Bool b=&x;return b-1;}",
    "int main(){unsigned int x=4294967295U;return (x+1==0)-1;}",
    "int main(){_Bool b=1;int old=b++;return old+b-2;}",
    "int main(){_Bool b=1;int old=b--;return old+b-1;}",
    "int main(){long x=(long)(0?1U:-1);return (x==4294967295L)-1;}",
], ["asm", "llvm"])]
fn it_supports_extended_integers(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){return ((int[]){7,9})[1]-9;}",
    "struct S{int x;char y;}; int main(){return ((struct S){7,9}).y-9;}",
    "int main(){int *p=&(int){7};*p=9;return *p-9;}",
    "int *p=(int[]){7,9};int main(){return p[1]-9;}",
    "int f(int x,int y){return x+y;}int main(){return f(3,((int[]){7,9})[1])-12;}",
], ["asm", "llvm"])]
fn it_supports_compound_literals(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int f(){static int x=3;return x++;}int main(){return f()+f()-7;}",
    "static int x=3;static int f(){return x;}int main(){return f()-3;}",
    "extern int x;int x=7;extern int x;int main(){extern int x;return x-7;}",
    "int f(){static char s[]=\"abc\";return s[2];}int main(){return f()-99;}",
    "int f(){return 7;}int main(){extern int f();return f()-7;}",
], ["asm", "llvm"])]
fn it_supports_storage_classes(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test]
fn it_preserves_volatile_accesses_in_llvm() {
    let ir =
        dynamite_compiler::generate("int main(){volatile int x=7;x++;return x;}", "llvm", false);
    assert!(ir.matches("load volatile i32").count() >= 2);
    assert!(ir.matches("store volatile i32").count() >= 2);
    let ir = dynamite_compiler::generate(
        "typedef volatile int V;int main(){const V x=7;return x;}",
        "llvm",
        false,
    );
    assert!(ir.contains("load volatile i32"));
}

#[test_matrix([
    "typedef int A[3];const A a={[2]=7};int main(){return a[0]+a[2]-7;}",
    "struct S{unsigned long x;};const struct S s={.x=4294967296UL};int main(){return (s.x==4294967296UL)-1;}",
    "int main(){const unsigned long *p=(const unsigned long[]){[1]=4294967296UL};return (p[1]==4294967296UL)-1;}",
    "int f(){static const unsigned short x=65535;return x;}int main(){return f()-65535;}",
], ["asm", "llvm"])]
fn it_combines_qualified_extended_features(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){double x=1.5;float y=2.25f;return (int)(x+y)-3;}",
    "double x=1.5+2.25;float y=0x1.8p1f;int main(){return (x==3.75 && y==3.0f)-1;}",
    "double f(float x,int y){return x+y;}int main(){return (int)f(1.25f,3)-4;}",
    "int main(){double x=-0.0;if(x)return 1;return (x==0.0)-1;}",
    "int main(){double x=2.5;double old=x++;return (x==3.5 && old==2.5)-1;}",
    "int main(){double x=1.0;x+=2.5;return (x==3.5)-1;}",
    "int main(){float x=.5e1F;return (int)(x*2.0f/4.0f)-2;}",
    "int x=(int)3.75;double y=7;int main(){return x+(int)y-10;}",
], ["asm", "llvm"])]
fn it_supports_floating_point(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn arithmetic_type_annotations_preserve_long_expression_values(output_option: &str) {
    let output_option = output_option.to_owned();
    // The recursive backends need a larger stack for deep ASTs in debug builds.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let expression = std::iter::repeat_n("x", 512).collect::<Vec<_>>().join("+");
            let code = format!(
                "long f(long x){{return {expression};}}int main(void){{return f(7)!=3584;}}"
            );
            assert_eq!(
                compile_and_run(&code, &output_option).status.code(),
                Some(0)
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test_matrix([
    "int main(void){unsigned long u=4294967296UL;int x=-1;double y=.5;return ((u+x)*2+y!=8589934590.5);}",
    "int f(int n){int a[2][n];int (*p)[n]=a;unsigned long x=(p+1-p)+4294967295U;return x!=4294967296UL;}int main(void){return f(3);}",
    "struct S{unsigned x:3;};int main(void){struct S s={2};double x=(s.x+1)*.5;return x!=1.5;}"
], ["asm", "llvm"])]
fn arithmetic_types_follow_promotions_and_lowering(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "float x=16777217-16777216.0f;int main(){return (x==0.0f)-1;}",
    "int main(){long x=9007199791611905L;float f=x;return (f==9007200328482816.0f)-1;}",
    "int main(){double x=-0.0;return (1.0/x<0.0)-1;}",
    "int main(){double x=1.5;while(x){x-=.5;}return (_Bool)x;}",
    "int main(){float a[2]={1.5f,2.5f};float *p=a;float old=p[0]++;return (old==1.5f && a[0]==2.5f && p[1]==2.5f)-1;}",
], ["asm", "llvm"])]
fn it_rounds_and_updates_floating_values(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "struct S{int a;int b;};struct S f(struct S x){x.a+=2;return x;}int main(){struct S s={3,5};struct S t=f(s);return s.a+t.a+t.b-13;}",
    "struct S{long a;long b;long c;};struct S f(struct S x){x.c+=7;return x;}int main(){struct S s={1,2,3};struct S t=f(s);return t.a+t.b+t.c+s.c-16;}",
    "struct S{char a[3];};struct S f(struct S x){x.a[2]+=2;return x;}int main(){struct S s={{1,2,3}};return f(s).a[2]-5;}",
    "struct S{double a;long b;};struct S f(struct S x){x.a+=1.5;x.b+=2;return x;}int main(){struct S s={2.5,3};struct S t=f(s);return (t.a==4.0 && t.b==5)-1;}",
    "struct S{double a[4];};struct S f(struct S x){x.a[3]+=2;return x;}int main(){struct S s={{1,2,3,4}};return (f(s).a[3]==6.0)-1;}",
    "union U{long x;double y;};union U f(union U x){x.x+=2;return x;}int main(){union U u={7};return f(u).x-9;}",
    "struct S{long a;long b;};long f(long a,long b,long c,long d,long e,struct S s,long z){return a+b+c+d+e+s.a+s.b+z;}int main(){struct S s={6,7};return f(1,2,3,4,5,s,8)-36;}",
], ["asm", "llvm"])]
fn it_supports_aggregate_values(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "struct S{int a;int b;int c;};struct S g={1,2,3};struct S f(){return g;}int main(){struct S (*p)()=f;return p().c-3;}",
    "struct S{long a;long b;long c;};struct S f(int n,struct S s){if(n)return f(n-1,s);return s;}int main(){struct S s={1,2,3};return f(3,s).c-3;}",
], ["asm", "llvm"])]
fn it_returns_global_and_recursive_aggregates(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn it_starts_after_named_stack_and_floating_parameters(output_option: &str) {
    let code = "#include <stdarg.h>\ndouble local(int a,int b,int c,int d,int e,int f,int g,int h,int i,double x,...){va_list ap;va_start(ap,x);int y=va_arg(ap,int);double z=va_arg(ap,double);va_end(ap);return i+x+y+z;}int main(){return (local(1,2,3,4,5,6,7,8,9,1.5,3,2.5)==16.0)-1;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){int n=3;int a[n];for(int i=0;i<n;i++)a[i]=i+2;return a[2]+sizeof(a)-16;}",
    "int main(){int n=2;int m=3;int a[n][m];a[1][2]=7;return a[1][2]+sizeof(a)+sizeof(a[0])-43;}",
    "int main(){int n=2;int a[n++];n=8;a[1]=3;return sizeof(a)+n+a[1]-19;}",
    "int main(){int n=3;int s=sizeof(int[n++]);return s+n-16;}",
    "int f(int n){int a[n];a[n-1]=7;return a[n-1];}int main(){return f(3)-7;}",
    "int main(){int s=0;for(int i=1;i<5;i++){int a[i];a[0]=i;if(i==2)continue;s+=a[0];if(i==3)break;}return s-4;}",
    "int main(){int n=3;{int a[n];a[0]=7;goto out;}out:return 0;}",
    "int main(){int n=3;typedef int A[n];n=8;A a;a[2]=7;return sizeof(a)+a[2]-19;}",
], ["asm", "llvm"])]
fn it_supports_variable_length_arrays(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int f(struct S{int x;} *p){struct S s={7};return s.x;}int main(){return f(0)-7;}",
    "int f(enum E{A=7} e){enum E x=A;return x;}int main(){return f(0)-7;}",
    "struct S{int x;};int f(struct S{long y;} *p){struct S s={7};return sizeof(s)+s.y-15;}int main(){struct S s;return f(0)+sizeof(s)-4;}",
], ["asm", "llvm"])]
fn it_preserves_function_parameter_tag_scopes(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn qualified_arrays_of_pointers_in_parameters(output_option: &str) {
    let code = "int sum(int *values[__restrict]);int sum(int *values[restrict static 2]){return *values[0]+*values[1];}int main(){int a=3,b=4;int *p[2]={&a,&b};return sum(p)-7;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int a[static 3];",
    "int main(){int a[const 3];return 0;}",
    "int f(int a[*]){return 0;}",
    "int star(int a[static *]);",
    "int bound(int a[static]);",
    "int f(int a[const 3]){a=0;return 0;}",
])]
fn rejects_invalid_array_parameter_modifiers(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn constant_offsetof(output_option: &str) {
    let code = "#include <stddef.h>\nstruct S{char c;int values[3];};enum{OFFSET=offsetof(struct S,values[2])};unsigned long global=offsetof(struct S,values);int a[offsetof(struct S,values)];int main(){return OFFSET!=12 || global!=4 || sizeof(a)!=16;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn rejects_bitfield_offsetof(output_option: &str) {
    let code =
        "#include <stddef.h>\nstruct S{int bits:3;};int main(){return offsetof(struct S,bits);}";
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"])]
fn anonymous_aggregate_members(output_option: &str) {
    let code = "#include <stddef.h>\nstruct S{int n;union{int x;struct{int y;int z;};};};struct S g={.n=1,.y=2,.z=3};enum{OFF=offsetof(struct S,z)};int main(){struct S s={.n=4,.y=5,.z=6};struct S *p=&s;p->x=7;return g.n!=1 || g.y!=2 || g.z!=3 || s.n!=4 || s.y!=7 || s.z!=6 || OFF!=8;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "struct Duplicate{int x;union{int x;};};int main(){return 0;}",
    "struct Nested{struct{int x;};union{int x;};};int main(){return 0;}",
    "struct Qualified{struct{int x;};};int main(){const struct Qualified s={{1}};s.x=2;return 0;}",
])]
fn rejects_invalid_anonymous_aggregate_members(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn glibc_integer_word_mode(output_option: &str) {
    let code = "typedef int word __attribute__((__mode__(__word__)));typedef unsigned int uword __attribute__((mode(word)));struct Counter{__extension__ unsigned long long value;};int main(){word x=-1;uword y=0xffffffffffffffffUL;struct Counter c={7};return sizeof(word)!=8 || sizeof(uword)!=8 || x>=0 || y!=0xffffffffffffffffUL || c.value!=7;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "typedef float invalid __attribute__((mode(word)));",
    "typedef int invalid __attribute__((mode(byte)));",
    "int invalid __attribute__((mode(word)));",
])]
fn rejects_unsupported_integer_modes(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn call_type_queries_preserve_decay_and_argument_promotions(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
typedef int Vector __attribute__((vector_size(16)));
int accepts(void *p){return p!=0;}
int increment(int x){return x+1;}
int invoke(int callback(int),const int values[2]){return callback(values[1]);}
int fixed(signed char n,float x){return n+(int)x;}
int variadic(int count,...){
    __builtin_va_list ap;__builtin_va_start(ap,count);
    int n=__builtin_va_arg(ap,int);double x=__builtin_va_arg(ap,double);
    __builtin_va_end(ap);return n+(int)x;
}
int main(){
    int values[2]={3,7};Vector vectors[1];long double storage[1];
    return invoke(increment,values)!=8 || !accepts(vectors) || !accepts(storage)
        || fixed(257,3.5)!=4 || variadic(2,(signed char)-1,(float)3.5)!=2;
}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn restrict_qualified_pointers(output_option: &str) {
    let code = "typedef int *P;int sum(int n,int a[restrict static n],const int *restrict b){P restrict p=a;int result=0;for(int i=0;i<n;i++){p[i]+=b[i];result+=p[i];}return result;}int main(){int a[3]={1,2,3};int b[3]={4,5,6};return sum(3,a,b)-21;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "restrict int x;int main(){return 0;}",
    "int (*restrict function)(void);int main(){return 0;}",
    "struct S{int restrict x;};int main(){return 0;}",
])]
fn rejects_invalid_restrict_types(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([false, true], ["-O0", "-O2"])]
fn llvm_functions_keep_unwindable_frames(aggregate: bool, optimization: &str) {
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
int capture(void){void *frames[32];int count=backtrace(frames,32);char **names=backtrace_symbols(frames,count);int found=0,main_found=0;
if(names){for(int i=0;i<count;i++){fprintf(stderr,"%s\n",names[i]);if(strstr(names[i],"compiler_frame"))found=1;if(strstr(names[i]," main + ")||strstr(names[i],"(main+"))main_found=1;}free(names);}return found&&main_found?0:100;}
"#,
    );
    let generated = dynamite_compiler::generate(code, "llvm", false);
    for line in generated.lines().filter(|line| line.starts_with("define ")) {
        assert!(
            line.contains("uwtable") && line.contains("\"frame-pointer\"=\"all\""),
            "{generated}"
        );
    }
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

#[test_matrix(["asm", "llvm"])]
fn automatic_storage_specifiers(output_option: &str) {
    let code = "int increment(register int x){return ++x;}int main(){auto int a=2,b=3;register int c=increment(a);const register int d=4;int register e=5;auto int *p=&a;register struct S{int x;long y;}s={6,7};for(register int i=0;i<2;i++){b++;}{auto int c=9;int *q=&c;b+=*q;}return *p+b+c+d+e+s.x+s.y+sizeof(c)-45;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn register_array_parameter(output_option: &str) {
    let code = "int sum(register int n,register int a[static n]){int result=0;for(auto int i=0;i<n;i++){result+=a[i];}return result;}int main(){int a[3]={1,2,3};return sum(3,a)-6;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "auto int global;int main(){return 0;}",
    "register int global;int main(){return 0;}",
    "int f(auto int x){return x;}int main(){return 0;}",
    "int f(register void);int main(){return 0;}",
    "int main(){register int f(void);return 0;}",
    "int f(register int x){return *&x;}int main(){return 0;}",
    "int main(){register int x;int *p=&x;return 0;}",
    "int main(){register struct S{int x;}s;int *p=&s.x;return 0;}",
    "int main(){auto register int x;return 0;}",
    "int main(){register register int x;return 0;}",
    "int main(){auto int f(void);return 0;}",
])]
fn rejects_invalid_automatic_storage(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn pointer_arithmetic_preserves_integer_index_values(output_option: &str) {
    let code = "int values[256]; unsigned char bytes[40001]; int main(){unsigned char u=255; signed char n=-128; unsigned short wide=40000; _Bool one=1; int *p=values+255; values[u]=7; p[n]=9; bytes[wide]=11; return values[255]+values[127]+bytes[40000]+(p-u==values)+(p+one==values+256)+(p+n==values+127)+(values+u==p)-31;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn string_literals_retain_array_type(output_option: &str) {
    let code = r#"char *global="hello"+2;char (*array_pointer)[4]=&"abc";int main(){char *selected=1?"x":"longer";return sizeof("")+sizeof("ab")+sizeof("a\0b")+sizeof("a" "bc")+sizeof(u8"é")+sizeof(u"😀")+sizeof(U"😀")+sizeof(L"ab")+sizeof(*array_pointer)+global[0]+(*array_pointer)[2]+selected[0]-372;}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn prefixed_string_literal_code_units(output_option: &str) {
    let code = r#"#include <stddef.h>
int main(){unsigned short *u=u"😀";unsigned int *v=U"😀";wchar_t *w=L"ab";return (u[0]!=55357)+(u[1]!=56832)+u[2]+(v[0]!=128512)+v[1]+(w[0]!=97)+(w[1]!=98)+w[2]+("a\0b"[2]!=98);}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn addresses_of_imported_functions_and_objects(output_option: &str) {
    let code = "extern int external_value;int external_add(int x);static int local_add(int x){return x+1;}int main(){void *(*allocate)(unsigned long)=malloc;void (*release)(void*)=free;int *p=allocate(sizeof(int));*p=7;int (*f)(int)=external_add;int (*g)(int)=local_add;int *v=&external_value;int result=f(*p)+g(*v);release(p);return result-42;}";
    let companion = "int external_value=17;int external_add(int x){return x+17;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn c_bridge_metadata_preserves_types_and_calls(output_option: &str, optimization: &str) {
    let code = r#"
typedef const __attribute__((objc_bridge(id))) void *Object;
typedef const struct __attribute__((__objc_bridge__(Example))) Opaque *Handle;
typedef struct __attribute__((objc_bridge_mutable(MutableExample))) Opaque *MutableHandle;
typedef __attribute__((objc_bridge(id))) Object Property __attribute__((swift_wrapper(struct)));
struct __attribute__((objc_boxable)) Pair{long x;long y;};
Object create(void) __attribute__((cf_returns_retained,swift_name("makeObject()")));
Object borrow(Object object) __attribute__((__cf_returns_not_retained__,__swift_private__));
void consume(Object __attribute__((cf_consumed)) object);
int calls;
Object create(void){calls++;return (Object)&calls;}
Object borrow(Object object){calls++;return object;}
struct Pair echo(struct Pair);
int main(){
    Property object=create();
    if(borrow(object)!=object)return 1;
    consume(object);
    struct Pair p={7,9};p=echo(p);
    return calls-3+(sizeof(Handle)!=8)+(sizeof(MutableHandle)!=8)
        +(sizeof(struct Pair)!=16)+(p.x!=9)+(p.y!=7)
        +_Generic(object,Object:0,default:1);
}"#;
    let companion = r"
extern int calls;
void consume(const void *object){if(object==&calls)calls++;}
struct Pair{long x;long y;};
struct Pair echo(struct Pair p){struct Pair result={p.y,p.x};return result;}
";
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0));
}

#[test_matrix([
    "int f(const int *p){return *p;}int use(int (*p)(int*)){return 0;}int main(){return use(f);}",
    "int f(int **p){return 0;}int use(int (*p)(const int**)){return 0;}int main(){return use(f);}",
    "int f(int a,int b){return a+b;}int use(int (*p)(int)){return 0;}int main(){return use(f);}",
    "int f(int a,...){return a;}int use(int (*p)(int)){return 0;}int main(){return use(f);}",
    "double f(int a){return a;}int use(int (*p)(int)){return 0;}int main(){return use(f);}",
])]
fn incompatible_function_designators_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn function_designators_convert_to_pointer_values((output_option, optimization): (&str, &str)) {
    let code = r"
int function(void){return 7;}int another(void){return 9;}int variadic(int n,...){return n;}
int accept(_Bool value){return value;}
int main(void){
    _Bool present=function,other=variadic;
    if(!function || !variadic || !present || !other || function==0 || 0==function)return 1;
    if(!(function && variadic) || !(0 || function))return 2;
    if(accept(function)!=1 || accept(variadic)!=1)return 3;
    int (*pointer)(void)=function;
    if((unsigned long)function!=(unsigned long)pointer || !(unsigned long)variadic)return 4;
    if((function?11:13)!=11 || (variadic?17:19)!=17)return 5;
    int (*choice)(void)=1?function:another;
    if(choice()!=7 || (0?function:another)()!=9)return 6;
    return pointer()!=7 || !(_Bool)function || !(_Bool)variadic;
}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int f(void){return 0;}int main(){double x=(double)f;return 0;}",
    "int f(int n,...){return n;}int main(){float x=(float)f;return 0;}",
])]
fn floating_function_designator_casts_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O3"])]
fn may_alias_annotations_preserve_cross_type_memory_access(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
#if !__has_attribute(may_alias) || !__has_attribute(__may_alias__)
#error missing alias metadata
#endif
typedef int Alias __attribute__((may_alias));
typedef __attribute__((__may_alias__)) unsigned int Prefix;
union U{float f;int i;};
int overwrite(float *p,Alias *q){*p=1.0f;*q=0;return *p==0.0f;}
int exercise(void);int reference(float*,Alias*);
int main(){
 union U a,b;
 if(overwrite(&a.f,(Alias*)&a.i)!=1 || exercise()!=1)return 1;
 if(reference(&b.f,(Alias*)&b.i)!=1)return 2;
 return _Generic((Alias*)0,int*:0,default:1) || sizeof(Prefix)!=4;
}
"#;
    let companion = r#"
typedef int Alias __attribute__((may_alias));union U{float f;int i;};
int overwrite(float*,Alias*);
int exercise(void){union U u;return overwrite(&u.f,(Alias*)&u.i);}
int reference(float *p,Alias *q){*p=1.0f;*q=0;return *p==0.0f;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(
            code,
            output_option,
            Some(companion),
            &[optimization, "-fstrict-aliasing"]
        )
        .status
        .code(),
        Some(0)
    );
}

#[test_matrix([
    "typedef int T __attribute__((may_alias(1)));",
    "typedef __attribute__((__may_alias__())) int T;",
], ["asm", "llvm"])]
fn may_alias_arguments_have_source_diagnostics(code: &str, output_option: &str) {
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
fn prefixed_character_constants_preserve_values_and_types(output_option: &str, optimization: &str) {
    let code = r#"
#include <stddef.h>
int wide=L'\0';unsigned short utf16=u'\u03a9';unsigned int utf32=U'\U0001f600';
long reference_wide(void);unsigned long reference_utf32(void);
int main(){
 if(wide || L'A'!=65 || L'\n'!=10 || L'Ω'!=937 || L'\U0001f600'!=128512)return 1;
 if(utf16!=937 || u'\xffff'!=65535 || u'\377'!=255 || u'Ω'!=937)return 2;
 if(utf32!=128512 || U'\xffffffff'!=4294967295U || U'😀'!=128512)return 3;
 if(L'\xffffffff'!=-1 || (long)L'\xffffffff'!=reference_wide() || U'\xffffffff'!=reference_utf32())return 4;
 if(sizeof(L'A')!=4 || sizeof(u'A')!=2 || sizeof(U'A')!=4)return 5;
 if(_Generic(L'A',wchar_t:0,default:1) || _Generic(u'A',unsigned short:0,default:1) || _Generic(U'A',unsigned int:0,default:1))return 6;
 switch(L'A'){case L'\x41':return 0;default:return 7;}
}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(r"long reference_wide(void){return L'\xffffffff';}unsigned long reference_utf32(void){return U'\xffffffff';}"), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn common_objects_coalesce_and_yield_to_strong_definitions(
    output_option: &str,
    optimization: &str,
) {
    let declarations = r#"
struct S{long n;unsigned flag:3;};
#pragma pack(push,1)
struct P{char tag;long n;};
#pragma pack(pop)
int shared __attribute__((common));
int array[3] __attribute__((__common__,aligned(64)));
struct S record __attribute__((common));struct P packed __attribute__((common));
int (*callback)(int) __attribute__((common));
"#;
    let code = format!(
        r#"{declarations}
int shared;extern int array[3];
int overridden __attribute__((common));
extern int imported __attribute__((common));
int initialized __attribute__((common))=11;
int later __attribute__((common));int later=13;
int earlier=17;extern int earlier __attribute__((common));
static int local __attribute__((common));
int check(int*,int*,struct S*,struct P*,int(**)(int));
int add(int n){{return n+2;}}
int main(void){{
 if(shared || array[0] || record.n || packed.n || callback || local)return 1;
 if(overridden!=7 || imported!=9 || initialized!=11 || later!=13 || earlier!=17)return 2;
 if((unsigned long)array%64)return 3;
 shared=3;array[2]=5;record.n=19;record.flag=6;packed.tag=1;packed.n=23;callback=add;
 if(check(&shared,array,&record,&packed,&callback))return 4;
 return shared!=29 || callback(5)!=7;
}}
"#
    );
    let companion = format!(
        r#"{declarations}
int overridden=7,imported=9;
int initialized __attribute__((common));int later __attribute__((common));int earlier __attribute__((common));
int check(int *s,int *a,struct S *r,struct P *p,int (**fn)(int)){{
if(s!=&shared || a!=array || r!=&record || p!=&packed || fn!=&callback)return 1;
if(shared!=3 || array[2]!=5 || record.n!=19 || record.flag!=6 || packed.tag!=1 || packed.n!=23 || callback(3)!=5)return 2;
shared=29;return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[cfg(target_os = "linux")]
#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn hidden_symbols_are_absent_from_dynamic_exports(output_option: &str, optimization: &str) {
    let code = "int secret __attribute__((visibility(\"hidden\")))=7;int private_api(void) __attribute__((visibility(\"hidden\")));int private_api(void){return secret;}int public_api(void){return private_api();}int probe(void);int main(){return public_api()!=7 || probe();}";
    let companion = "#define _GNU_SOURCE\n#include <dlfcn.h>\nint probe(void){return dlsym(RTLD_DEFAULT,\"secret\")!=0 || dlsym(RTLD_DEFAULT,\"private_api\")!=0 || dlsym(RTLD_DEFAULT,\"public_api\")==0;}";
    assert_eq!(
        compile_and_run_with_c_flags(
            code,
            output_option,
            Some(companion),
            &[optimization, "-rdynamic", "-ldl"]
        )
        .status
        .code(),
        Some(0)
    );
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix(["asm", "llvm"])]
fn binary64_long_double_preserves_language_type(output_option: &str) {
    let code = r#"
long double value=1.25L;
double long other=0x1.8p1l;
long double add(long double x,float y){return x+y;}
int main(){
    long double x=add(value,2.5f);
    double y=x;
    float z=x;
    x=y;
    x++;
    long double array[2]={value,other};
    return (sizeof(x)!=8)+(_Alignof(long double)!=8)+(x!=4.75L)+(z!=3.75f)
        +(array[1]!=3.0L)+_Generic(x,long double:0,double:1,default:2)
        +_Generic(value+1.0,long double:0,default:1)
        +_Generic(y,double:0,long double:1,default:2);
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn extended_long_double_storage_matches_clang(output_option: &str, optimization: &str) {
    let code = r"
typedef long double Extended;
Extended unused_api(Extended);
struct Packet{char tag;Extended value;char end;};
Extended zeros[2]={};struct Packet storage={};
void fill(Extended *);int check(struct Packet *,Extended *);
int main(){struct Packet local;local.tag=7;local.end=9;fill(&local.value);storage=local;
return check(&storage,zeros)+(sizeof(Extended)!=16)+(_Alignof(Extended)!=16)
+(sizeof(local)!=48)+((char*)&local.value-(char*)&local!=16)
+((char*)&local.end-(char*)&local!=32);}
";
    let companion = "struct Packet{char tag;long double value;char end;};void fill(long double *p){*p=1.25L;}int check(struct Packet *p,long double *z){return p->tag!=7 || p->value!=1.25L || p->end!=9 || z[0]!=0 || z[1]!=0;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[cfg(not(any(
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_os = "macos")
)))]
#[test_matrix([
    "long double value=1.0L;",
    "int main(){return 1.0L==1.0;}",
    "long double f(long double x){return x;}",
    "long double f(void);int main(){f();return 0;}",
    "struct S{long double x;};struct S f(struct S);",
    "int main(){long double x;long double y=x;return 0;}",
])]
fn unsupported_long_double_formats_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[cfg(target_arch = "x86_64")]
#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn x87_global_storage_preserves_constant_bits(output_option: &str, optimization: &str) {
    let code = "long double exact=18446744073709551615.0L;union Value{long double value;unsigned char bytes[16];};union Value payload={.value=0x1.fffffffffffffffep63L};long double nan=__builtin_nanl(\"0x123456789abcdef\");int check(long double *,union Value *,long double *);int main(){return check(&exact,&payload,&nan);}";
    let companion = "union Value{long double value;unsigned char bytes[16];};int check(long double *a,union Value *b,long double *c){unsigned long bits=0;unsigned char *out=(unsigned char*)&bits;unsigned char *in=(unsigned char*)c;for(int i=0;i<8;i++){out[i]=in[i];}return *a!=18446744073709551615.0L || b->value!=*a || bits!=0xc123456789abcdefUL;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[cfg(target_arch = "x86_64")]
#[test]
fn native_x87_values_have_a_source_diagnostic() {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(
            "long double f(long double x){return x+1.0L;}",
            "asm",
            false
        ))
        .is_err()
    );
}

#[test_matrix([
    "_Float16 x=1;",
    "int main(){_Float16 x;x;return 0;}",
    "int main(){_Float16 x;x=x;return 0;}",
    "int main(){_Float16 x;return (int)x;}",
    "double x=(double)(_Float16)1;",
    "_Float16 f(void){}",
    "void f(_Float16 x){}",
    "_Float16 f(void);int main(){f();return 0;}",
    "struct S{_Float16 x;};void f(struct S);",
    "struct S{_Float16 x[2];};struct S f(void);",
    "union U{_Float16 x;int y;};void f(union U);",
    "struct S;void f(struct S);struct S{_Float16 x;};",
    "struct S;struct S (*p)(void);struct S{_Float16 x;};int main(){p();return 0;}",
    "struct S{_Float16 x;};void f(int,...);int main(){struct S s;f(0,s);return 0;}",
    "struct S{_Float16 x;};void f(int n,...){__builtin_va_list ap;__builtin_va_start(ap,n);__builtin_va_arg(ap,struct S);}",
])]
fn unsupported_float16_values_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([
    "__int128 value=1;",
    "__int128 f(__int128 x){return x;}",
    "__int128 f(void);",
    "int main(){__int128 x;return (int)x;}",
    "int main(){__int128 x;x=x+1;return 0;}",
    "int main(){__int128 x;if(x)return 1;return 0;}",
    "int main(){__int128 x;x;return 0;}",
    "int main(){__int128 x;x=x;return 0;}",
])]
fn unsupported_extended_integer_values_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn qualified_scalar_casts((output_option, optimization): (&str, &str)) {
    let code = r#"
typedef char *String;
typedef int *Pointer;
int global = (const int)7;
int main() {
    char text[] = "abc";
    const void *key = text;
    String p = (const String)key;
    String q = (volatile String)key;
    int n = 9;
    Pointer ptr = &n;
    const Pointer *saved = &ptr;
    Pointer copy = (const Pointer)*saved;
    Pointer restricted = (Pointer restrict)ptr;
    if(p[1] != 'b' || q[2] != 'c' || *copy != 9 || *restricted != 9) return 1;
    if((const unsigned short)65537 != 1 || (volatile double)3 != 3.0) return 2;
    if(global != 7 || _Generic((const String)key, char*:0, default:1)) return 3;
    (const void)n++;
    struct S{int x;};const struct S s={3};(const void)s;
    if(n != 10 || (const int)3.9 != 3) return 4;
    return 0;
}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){return (int * const)1.5 != 0;}",
    "int main(){int x;return (const double)&x;}",
    "struct S{int x;};int main(){return (const struct S)1;}"
])]
fn invalid_qualified_scalar_casts(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn typed_integer_complement_constants((output_option, optimization): (&str, &str)) {
    let code = r#"
enum E : unsigned int { LIMIT=~0U, PREVIOUS=~1U };
enum Wide : unsigned long { WIDE_LIMIT=~0UL };
_Static_assert(~0U == 4294967295U, "unsigned complement");
_Static_assert((~0U >> 31) == 1, "logical shift");
_Static_assert(~(unsigned char)0 == -1, "integer promotion");
_Static_assert(~(unsigned short)0 == -1, "short promotion");
_Static_assert(~0UL == 18446744073709551615UL, "long width");
unsigned long wide = ~0U;
unsigned int table[2] = { ~0U, ~1U };
int array[~0U == 4294967295U ? 2 : 0];
int main(){
    unsigned int zero=0;
    unsigned long runtime=~zero;
    if(wide!=runtime || wide!=4294967295UL) return 1;
    if(LIMIT!=table[0] || PREVIOUS!=table[1] || PREVIOUS!=4294967294U) return 2;
    if(WIDE_LIMIT!=~0UL || sizeof(array)!=8) return 3;
    if((~0U > 0U)!=1 || (~~0U)!=0) return 4;
    return 0;
}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn integer_type_identity_and_long_long_rank_match_clang(output_option: &str, optimization: &str) {
    let code = r"
signed char narrow=-7;
long long large=-9223372036854775807LL;
unsigned long long maximum=18446744073709551615ULL;
struct Values{char a;signed char b;long c;long long d;unsigned long long e;};
struct Values storage={3,-5,7,-11LL,18446744073709551615ULL};
long long check(signed char,long,long long,unsigned long long,struct Values*);
int variadic(int,...);
int main(){
    char plain=1;signed char signed_byte=-1;
    long normal=2;long long wide=3;unsigned long unormal=4;unsigned long long uwide=5;
    int bad=_Generic(plain,char:0,signed char:1,default:2)
        +_Generic(signed_byte,signed char:0,char:1,default:2)
        +_Generic(normal,long:0,long long:1,default:2)
        +_Generic(wide,long long:0,long:1,default:2)
        +_Generic(unormal,unsigned long:0,unsigned long long:1,default:2)
        +_Generic(uwide,unsigned long long:0,unsigned long:1,default:2)
        +_Generic(1LL,long long:0,long:1,default:2)
        +_Generic(1ULL,unsigned long long:0,unsigned long:1,default:2)
        +_Generic(1LU,unsigned long:0,unsigned long long:1,default:2)
        +_Generic(wide+normal,long long:0,default:1)
        +_Generic(wide+unormal,unsigned long long:0,unsigned long:1,default:2)
        +_Generic(uwide+normal,unsigned long long:0,default:1)
        +_Generic(signed_byte+plain,int:0,default:1)
        +_Generic(18446744073709551615,unsigned long long:0,default:1)
        +_Generic(0xffffffffffffffff,unsigned long:0,default:1)
        +_Generic(0xffffffffffffffffLL,unsigned long long:0,default:1)
        +_Generic(__builtin_bswap64(1),unsigned long long:0,unsigned long:1,default:2);
    if(bad)return 1;
    if(sizeof(wide)!=8 || _Alignof(long long)!=8 || sizeof(signed_byte)!=1)return 2;
    if(check(narrow,normal,large,maximum,&storage)!=17LL)return 3;
    if(variadic(0,-13LL,18446744073709551615ULL)!=0)return 4;
    if((signed char)255!=-1 || (double)wide!=3.0)return 5;
    return 0;
}
";
    let companion = r"
#include <stdarg.h>
struct Values{char a;signed char b;long c;long long d;unsigned long long e;};
long long check(signed char a,long b,long long c,unsigned long long d,struct Values *v){
    if(a!=-7 || b!=2 || c!=-9223372036854775807LL || d!=18446744073709551615ULL)return 0;
    if(v->a!=3 || v->b!=-5 || v->c!=7 || v->d!=-11LL || v->e!=d)return 0;
    return 17LL;
}
int variadic(int n,...){va_list ap;va_start(ap,n);long long a=va_arg(ap,long long);unsigned long long b=va_arg(ap,unsigned long long);va_end(ap);return a!=-13LL || b!=18446744073709551615ULL;}
";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "typedef char T;typedef signed char T;",
    "typedef long T;typedef long long T;",
    "typedef unsigned long T;typedef unsigned long long T;",
    "void f(char);void f(signed char);",
    "void f(long);void f(long long);",
    "int main(){char x;signed char *p=&x;return 0;}",
    "int main(){long x;long long *p=&x;return 0;}",
])]
fn incompatible_integer_type_identities_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([
    "int main(){return __builtin_types_compatible_p();}",
    "int main(){return __builtin_types_compatible_p(int);}",
    "int main(){return __builtin_types_compatible_p(int,int,int);}",
    "int main(){return __builtin_types_compatible_p(1,int);}",
    "int main(){return __builtin_types_compatible_p(int value,int);}",
    "int main(){return __builtin_types_compatible_p(void[2],int);}",
])]
fn invalid_types_compatible_queries_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn pragma_pack_preserves_storage_and_clang_calls(output_option: &str, optimization: &str) {
    let shared = r#"
#include <stdarg.h>
#pragma pack(push,one,1)
struct P1{char c;int x;short y;};
union U1{long number;int *pointer;};
struct B1{unsigned char a:3;unsigned char b:4;};
#pragma pack(push,two,2)
struct P2{char c;int x;short y;};
#pragma pack(push,four,4)
struct P4{char c;long x;};
struct Capped{char c;long x __attribute__((aligned(16)));};
struct CappedAlignas{char c;_Alignas(16) long x;};
struct Raised{char c;long x;} __attribute__((aligned(16)));
struct B4{char c;unsigned int a:3;unsigned int b:4;};
struct Hfa{double x;double y;};
#pragma pack(pop,four)
struct Parent{char c;struct P4 child;struct P1 array[2];union U1 choice;};
#pragma pack(pop,one)
struct Natural{char c;long x;};
struct AtStart{char c;
#pragma pack(push,1)
long x;};
#pragma pack(pop)
#pragma pack(push,8)
struct P8{char c;long x;};
#pragma pack(16)
struct P16{char c;_Alignas(16) long x;};
#pragma pack(0)
struct Reset{char c;long x;};
#pragma pack(pop)
#define DEFINE_VAR(name) int name(int n,...){va_list ap;va_start(ap,n);struct P1 p=va_arg(ap,struct P1);struct P4 q=va_arg(ap,struct P4);struct Parent parent=va_arg(ap,struct Parent);struct Hfa h=va_arg(ap,struct Hfa);double d=va_arg(ap,double);va_end(ap);return n!=1 || p.x!=2 || q.x!=5 || parent.child.x!=3 || h.x!=2.5 || d!=7.5;}

"#;
    let code = [shared,r#"
DEFINE_VAR(local_var)
int target[2]={37,41};
struct Hfa global_hfa={-2.5,3.5};struct P1 zeros;
_Alignas(8) struct Parent global={1,{2,3},{{4,5,6},{7,8,9}},{.pointer=&target[1]}};
union U1 union_array[2]={{.pointer=&target[0]},{.number=19}};
struct B1 byte_bits={5,9};struct B4 word_bits={1,5,9};
struct P1 local_p1(struct P1 value){value.x+=2;value.y++;return value;}
struct P4 local_p4(struct P4 value){value.x+=3;return value;}
struct Hfa local_hfa(struct Hfa value){value.x+=value.y;return value;}
struct P1 foreign_p1(struct P1);struct P4 foreign_p4(struct P4);struct Hfa foreign_hfa(struct Hfa);
int foreign_var(int,...);int call_local_var(struct P1,struct P4,struct Parent,struct Hfa);
int mutate_copy(struct Parent);
int check(struct Parent *,union U1 *,struct B1 *,struct B4 *,struct P1,struct P4);
int main(){
    struct Parent local=global;struct P1 p={1,2,3};struct P4 q={4,5};struct Hfa h={2.5,3.5};
    volatile struct P1 v={1,2,3};v.x++;v.y+=2;local.array[1].x++;
    if(foreign_var(1,p,q,local,h,7.5) || call_local_var(p,q,local,h))return 2;
    for(int i=0;i<4;i++)if(mutate_copy(local)!=99 || local.child.x!=3)return 3;
    p=foreign_p1(p);q=foreign_p4(q);h=foreign_hfa(h);
    return check(&local,union_array,&byte_bits,&word_bits,p,q)
        || v.x!=3 || v.y!=5 || h.x!=6.0 || h.y!=3.5
        || global_hfa.x!=-2.5 || global_hfa.y!=3.5 || zeros.x!=0 || zeros.y!=0
        || sizeof(struct P1)!=7 || _Alignof(struct P1)!=1 || __builtin_offsetof(struct P1,x)!=1
        || sizeof(struct P2)!=8 || _Alignof(struct P2)!=2 || __builtin_offsetof(struct P2,x)!=2
        || sizeof(struct P4)!=12 || _Alignof(struct P4)!=4 || __builtin_offsetof(struct P4,x)!=4
        || sizeof(struct Capped)!=12 || __builtin_offsetof(struct Capped,x)!=4
        || sizeof(struct CappedAlignas)!=12 || __builtin_offsetof(struct CappedAlignas,x)!=4
        || sizeof(struct Raised)!=16 || _Alignof(struct Raised)!=16
        || sizeof(struct Natural)!=16 || sizeof(struct AtStart)!=16 || sizeof(struct Reset)!=16
        || sizeof(struct P8)!=16 || sizeof(struct P16)!=32 || sizeof(struct B1)!=1 || sizeof(struct B4)!=4;
}
"#].concat();
    let companion = [
        shared,
        r#"
DEFINE_VAR(foreign_var)
int local_var(int,...);
int call_local_var(struct P1 p,struct P4 q,struct Parent parent,struct Hfa h){return local_var(1,p,q,parent,h,7.5);}
int mutate_copy(struct Parent parent){parent.child.x=99;return parent.child.x;}
struct P1 local_p1(struct P1);struct P4 local_p4(struct P4);struct Hfa local_hfa(struct Hfa);
struct P1 foreign_p1(struct P1 p){return local_p1(p);}
struct P4 foreign_p4(struct P4 p){return local_p4(p);}
struct Hfa foreign_hfa(struct Hfa p){return local_hfa(p);}
int check(struct Parent *p,union U1 *unions,struct B1 *b,struct B4 *w,struct P1 x,struct P4 y){
    return p->c!=1 || p->child.c!=2 || p->child.x!=3 || p->array[1].x!=9
        || *p->choice.pointer!=41 || *unions[0].pointer!=37 || unions[1].number!=19
        || b->a!=5 || b->b!=9 || w->a!=5 || w->b!=9 || x.x!=4 || x.y!=4 || y.x!=8
        || sizeof(*p)!=36 || _Alignof(struct Parent)!=2;
}
"#,
    ]
    .concat();
    // Mach-O chained fixups cannot encode every unaligned static pointer.
    // This linker option also permits the same packed definitions from Clang.
    let mut flags = vec![optimization];
    if cfg!(target_os = "macos") {
        flags.push("-Wl,-no_fixup_chains");
    }
    let result = compile_and_run_with_c_flags(&code, output_option, Some(&companion), &flags);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "int main(){char a[2];__builtin___memcpy_chk(a,a,1);}",
    "int main(){char a[2];__builtin___memmove_chk(a,a,1,2,3);}",
    "int main(){char a[2];__builtin___memset_chk(1,0,1,2);}",
    "int main(){char a[2];__builtin___memcpy_chk(a,1,1,2);}",
    "int main(){char a[2];__builtin___memset_chk(a,a,1,2);}",
    "int main(){const char a[2]={0};__builtin___memset_chk(a,0,1,2);}",
    "int main(){char a[2];__builtin___memcpy_chk(a,a,a,2);}",
    "int __memcpy_chk(int);int main(){char a[2];__builtin___memcpy_chk(a,a,1,2);}",
    "int main(){void *p=__builtin___memset_chk;}",
    "typedef int __memcpy_chk;int main(){char a[2];__builtin___memcpy_chk(a,a,1,2);}",
])]
fn invalid_checked_memory_operands_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn branch_expectation_preserves_long_values_and_effects(output_option: &str, optimization: &str) {
    let shared = r#"
#define DEFINE_EXPECTATIONS(name) \
int name(int value,unsigned int narrow,unsigned long wide,void *pointer){ \
    return __builtin_expect(value,0)!=(long)value \
        || __builtin_expect(narrow,1)!=(long)narrow \
        || __builtin_expect(wide,0)!=(long)wide \
        || __builtin_expect(!!pointer,1)!=(long)(pointer!=0) \
        || __builtin_expect(2.75,0)!=2 \
        || _Generic(__builtin_expect(value,0),long:0,default:1); \
}
"#;
    let code = [
        shared,
        r#"
DEFINE_EXPECTATIONS(local_expectations)
int foreign_expectations(int,unsigned int,unsigned long,void *);
long global=__builtin_expect(7,0);
_Static_assert(__builtin_expect(7,1)==7,"constant prediction");
int values,hints;
int next_value(){values++;return 7;}
long next_hint(){hints++;return 0;}
double float_hint(){hints++;return 0.5;}
int main(){
    long first=__builtin_expect(next_value(),next_hint());
    long second=__builtin_expect(next_value(),0);
    long third=__builtin_expect(next_value(),float_hint());
    long x=7;long hint=8;long previous=__builtin_expect(x++,hint++);
    volatile int prediction=1;long fourth=__builtin_expect(11,prediction);
    return first!=7 || second!=7 || third!=7 || fourth!=11 || global!=7
        || values!=3 || hints!=2 || previous!=7 || x!=8 || hint!=9
        || local_expectations(-3,0xffffffffu,0xfffffffffffffffful,&x)
        || foreign_expectations(-3,0xffffffffu,0xfffffffffffffffful,&x);
}
"#,
    ]
    .concat();
    let companion = [shared, "DEFINE_EXPECTATIONS(foreign_expectations)"].concat();
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "int main(){return __builtin_expect(1);}",
    "int main(){return __builtin_expect(1,0,1);}",
    "int main(){int x;return __builtin_expect(&x,0);}",
    "int main(){int x;return __builtin_expect(1,&x);}",
    "void f(void);int main(){return __builtin_expect(f(),0);}",
    "int main(){long x=1;__builtin_expect(x,0)=2;}",
    "int main(){long x=1;long *p=&__builtin_expect(x,0);}",
])]
fn invalid_branch_expectation_operands_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn multi_character_constants_match_clang_integer_values(output_option: &str, optimization: &str) {
    let code = r#"
int reference(int);
int values[9]={'ab','abc','drag','A\0B','\377\377','\xff\xff\xff\xff','\377ABC','abcdef','\\u'};
enum Codes{DRAG='drag',NEGATIVE='\377ABC'};
_Static_assert('ABCD'==0x41424344,"byte order");
_Static_assert('\xff\xff\xff\xff'==-1,"signed int");
int main(){for(int i=0;i<9;i++)if(values[i]!=reference(i))return i+1;
if(DRAG!=values[2] || NEGATIVE!=values[6] || sizeof('ab')!=sizeof(int) || 'Q'!=81)return 10;
switch(DRAG){case 'drag':break;default:return 11;}
return _Generic('ab',int:0,default:1);}
"#;
    let companion = r#"
int reference(int index){static int values[9]={'ab','abc','drag','A\0B','\377\377','\xff\xff\xff\xff','\377ABC','abcdef','\\u'};return values[index];}
"#;
    let result = compile_and_run_with_c_flags(
        code,
        output_option,
        Some(companion),
        &[optimization, "-Wno-multichar", "-Wno-four-char-constants"],
    );
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "struct __attribute__((packed)) S;",
    "struct S{int x;} __attribute__((packed(1)));",
    "struct S{unsigned int field:3;} __attribute__((packed));",
    "int object __attribute__((packed));",
    "typedef int Scalar __attribute__((packed));",
    "struct S{int member __attribute__((packed));};",
])]
fn unsupported_gnu_packing_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn parenthesized_type_keyword_strings_remain_expressions(
    (output_option, optimization): (&str, &str),
) {
    let keywords = [
        "char",
        "short",
        "int",
        "long",
        "signed",
        "unsigned",
        "__int128",
        "__int128_t",
        "__uint128_t",
        "float",
        "double",
        "void",
        "struct",
        "union",
        "enum",
        "const",
        "volatile",
        "restrict",
        "typeof",
        "_Bool",
        "bool",
        "Number",
    ];
    let mut code = String::from(
        "typedef int Number;struct Entry{const char *name;};int first(const char *s){return s[0];}int main(){",
    );
    for keyword in keywords {
        code.push_str(&format!(
            "{{struct Entry e={{(\"{keyword}\")}};const char *p=((\"{keyword}\"));if(e.name[0]!={first}||first((\"{keyword}\"))!={first}||p[0]!={first}||sizeof((\"{keyword}\"))!={size})return 1;}}",
            first = keyword.as_bytes()[0], size = keyword.len()+1,
        ));
    }
    code.push_str("return 0;}");
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn compact_aggregate_memory_operations_preserve_values_and_evaluation(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
struct S { unsigned char bytes[4097]; };
struct S first, second;
int check_zero(const unsigned char*,unsigned long);
int trace;
struct S *destination(void){trace=trace*10+1;return &second;}
struct S *source(void){trace=trace*10+2;return &first;}
struct __attribute__((packed)) P {char tag;long value;char tail[4];};
int main(void){
    for(int i=0;i<4097;i++)first.bytes[i]=(unsigned char)(i*17+3);
    *destination()=*source();
    if(trace!=12)return 1;
    second=second;
    struct S third=second;
    for(int i=0;i<4097;i++)if(first.bytes[i]!=third.bytes[i])return 2;
    if((second=first).bytes[13]!=first.bytes[13])return 3;
    struct S zero={0};third=zero;
    if(check_zero(third.bytes,sizeof(third)))return 6;
    struct P p={1,42,{3,4,5,6}},q={0};q=p;
    if(q.tag!=1 || q.value!=42 || q.tail[3]!=6)return 4;
    char large[16384]="x\0y";
    for(int i=0;i<16384;i++)if(large[i]!=(i==0?'x':i==2?'y':0))return 5;
    char truncated[3]="abc",inferred[]="xy";
    unsigned short u16[100]=u"\U0001F600";
    unsigned int u32[100]=U"\U0001F600";
    int wide[13]=L"x";
    volatile char v[5]="ab";
    return truncated[2]!='c' || sizeof(inferred)!=3 || inferred[2]!=0
        || u16[0]!=0xd83d || u16[1]!=0xde00 || u16[99]!=0
        || u32[0]!=0x1f600 || u32[99]!=0 || wide[0]!='x' || wide[12]!=0
        || v[0]!='a' || v[1]!='b' || v[4]!=0;
}
"#;
    let output = compile_and_run_with_c_flags(
        code,
        output_option,
        Some(
            "int check_zero(const unsigned char*p,unsigned long n){for(unsigned long i=0;i<n;i++)if(p[i])return 1;return 0;}",
        ),
        &[optimization],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn compact_aggregate_memory_operations_bound_llvm_size_and_keep_volatile_accesses() {
    let ir = dynamite_compiler::generate(
        "struct S{char bytes[4097];};void copy(struct S*d,struct S*s){*d=*s;}void consume(char*);void fill(void){char a[16384]=\"x\";consume(a);}",
        "llvm",
        false,
    );
    assert_eq!(
        ir.matches("call void @llvm.memcpy.p0.p0.i64").count(),
        2,
        "{ir}"
    );
    assert!(ir.contains("i64 4097, i1 false"), "{ir}");
    assert!(!ir.contains("load [4097 x i8]"), "{ir}");
    assert!(ir.contains("i64 16384, i1 false"), "{ir}");
    assert!(
        ir.len() < 10000,
        "compact operations expanded to {} bytes",
        ir.len()
    );
    let ir = dynamite_compiler::generate(
        "struct V{volatile int x;};void copy(struct V*d,struct V*s){*d=*s;}void fill(void){volatile char a[3]=\"ab\";}",
        "llvm",
        false,
    );
    assert!(!ir.contains("load [4 x i8]"), "{ir}");
    assert_eq!(ir.matches("store volatile i8").count(), 3, "{ir}");
}

#[test_matrix(["and", "xor", "or"], [("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn bitwise_operands_evaluate_left_to_right(
    operator: &str,
    (output_option, optimization): (&str, &str),
) {
    let (operator, combine): (&str, fn(u64, u64) -> u64) = match operator {
        "and" => ("&", |a, b| a & b),
        "xor" => ("^", |a, b| a ^ b),
        "or" => ("|", |a, b| a | b),
        _ => unreachable!(),
    };
    let expected = combine(combine(15, 7), 3);
    let mixed = combine(u64::MAX, 1 << 40);
    let zero = combine(0, 3);
    let nested = combine(3, combine(5, 9));
    let code = format!(
        r#"
int trace;
unsigned long mark(int n,unsigned long value){{trace=trace*10+n;return value;}}
int narrow(int n,int value){{trace=trace*10+n;return value;}}
int main(void){{
    unsigned long result=mark(1,15) {operator} mark(2,7) {operator} mark(3,3);
    if(trace!=123 || result!={expected}UL)return 1;
    trace=0;
    result=narrow(1,-1) {operator} mark(2,1099511627776UL);
    if(trace!=12 || result!={mixed}UL)return 2;
    trace=0;
    result=mark(1,1099511627776UL) {operator} narrow(2,-1);
    if(trace!=12 || result!={mixed}UL)return 3;
    trace=0;
    result=mark(1,0) {operator} mark(2,3);
    if(trace!=12 || result!={zero}UL)return 4;
    trace=0;
    result=mark(1,3) {operator} (mark(2,5) {operator} mark(3,9));
    if(trace!=123 || result!={nested}UL)return 4;
    return 0;
}}
"#
    );
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

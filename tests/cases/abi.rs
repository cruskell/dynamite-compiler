use crate::support::{
    assert_frontend_error, compile_and_get_stdout, compile_and_run, compile_and_run_with_c,
    compile_and_run_with_c_flags,
};
use test_case::test_matrix;

#[test_matrix([
    ("int sum(int a,int b,int c,int d,int e,int f,int g,int h,int i,int j) { return a+b+c+d+e+f+g+h+i+j; } int main() { return sum(1,2,3,4,5,6,7,8,9,10); }", 55),
    ("int sum(int a,int b,int c,int d,int e,int f,int g,int h,char i,char j,int k) { return i+j+k; } int main() { return sum(1,2,3,4,5,6,7,8,-3,2,8); }", 7),
    ("int f(int a,int b,int c,int d,int e,int f,int g) { return g; } int main() { return 2 + f(1,2,3,4,5,6,f(1,2,3,4,5,6,5)); }", 7),
], ["asm", "llvm"])]
fn it_supports_stack_arguments(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix(["asm", "llvm"])]
fn it_supports_variadic_declarations(output_option: &str) {
    let code = "int snprintf(char *s, int size, char *format, ...); int main() { char s[40]; char c = 7; snprintf(s, 40, \"%d %d %d %d %d %d %d %d\", c, 2, 3, 4, 5, 6, 7, 8); puts(s); return 0; }";
    assert_eq!(
        compile_and_get_stdout(code, output_option),
        "7 2 3 4 5 6 7 8\n"
    );
}
#[test_matrix(["asm", "llvm"])]
fn it_defines_variadic_functions(output_option: &str) {
    let output = compile_and_run(
        "int first(int n, ...) { return n; } int main() { return first(7, 1, 2); }",
        output_option,
    );
    assert_eq!(output.status.code(), Some(7), "{:?}", output);
}

#[test_matrix([
    ("int add(int a, int b) { return a + b; } int main() { int (*f)(int, int) = add; return f(2,5); }", 7),
    ("int add(int a, int b) { return a + b; } int call(int (*f)(int, int), int n) { return (*f)(n, 5); } int main() { return call(&add, 2); }", 7),
    ("int f(int n) { return n+2; } int g(int n) { return n+3; } int main() { int (*fs[2])(int) = {f,g}; return fs[1](4); }", 7),
    ("int f(int a,int b,int c,int d,int e,int f,int g,int h,int i) { return i; } int main() { int (*p)(int,int,int,int,int,int,int,int,int) = f; return 2 + p(1,2,3,4,5,6,7,8,5); }", 7),
    ("int f(int n) { return n + 2; } int (*p)(int) = &f; int main() { return p(5); }", 7),
    ("typedef int (*Callback)(int); int f(int n) { return n + 2; } int main(void) { Callback p = (int (*)(int))f; return p(5); }", 7),
    ("int n; void set(void) { n = 7; } int main(void) { void (*p)(void) = set; p(); return n; }", 7),
    ("int f(int n) { return n+2; } int (*get(void))(int) { return f; } int main() { return get()(5); }", 7),
    ("int main() { return sizeof(int (*)(int)) + sizeof(int (*)[3]); }", 16),
    ("typedef void Nothing; int main(Nothing) { return 7; }", 7),
    ("int f(int n) { return n + 2; } int f(int); int main() { return f(5); }", 7),
], ["asm", "llvm"])]
fn it_supports_function_pointers(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix(["asm", "llvm"])]
fn it_interoperates_with_clang_bit_fields(output_option: &str) {
    let layout =
        "struct S{char c; unsigned a:3; signed b:5; int:0; unsigned long d:40; char tail;};";
    let code = format!(
        "{layout} int check(struct S *s);int main(){{struct S s={{2,7,-3,1099511627775UL,9}};return check(&s);}}"
    );
    let companion = format!(
        "{layout} int check(struct S *s){{if(s->c!=2 || s->a!=7 || s->b!=-3 || s->d!=1099511627775UL || s->tail!=9)return 1;s->a=3;s->b=-2;return 0;}}"
    );
    assert_eq!(
        compile_and_run_with_c(&code, output_option, Some(&companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){double n=0.0/0.0;return (n!=n && !(n<n) && !(n==n) && !(n<=n))-1;}",
    "int main(){double z=-0.0;return (_Bool)z;}",
    "double f(double a,double b,double c,double d,double e,double f,double g,double h,double i){return a+b+c+d+e+f+g+h+i;}int main(){return (int)f(1,2,3,4,5,6,7,8,9)-45;}",
    "double f(int a,float b,int c,double d,int e,float f,int g,double h,int i,float j,int k,double l,int m,float n){return a+b+c+d+e+f+g+h+i+j+k+l+m+n;}int main(){return (int)f(1,2,3,4,5,6,7,8,9,10,11,12,13,14)-105;}",
    "int main(){unsigned long x=18446744073709549568UL;double y=x;return ((unsigned long)y==x)-1;}",
    "int main(){unsigned long x=18014398509481987UL;double y=x;return (y==18014398509481988.0)-1;}",
    "double f(double x){return x+1.5;}int main(){double (*p)(double)=f;return (p(2.5)==4.0)-1;}",
    "struct S{float x;double y;};struct S s={1.5f,2.25};int main(){return (s.x+s.y==3.75)-1;}",
], ["asm", "llvm"])]
fn it_supports_floating_abi_and_edges(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}
#[test_matrix(["asm", "llvm"])]
fn it_promotes_variadic_floats(output_option: &str) {
    let output = compile_and_run(
        "int main(){float x=1.5f;printf(\"%.2f %.2f %.2f\\n\",x,2.25,3.0);return 0;}",
        output_option,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1.50 2.25 3.00\n");
}

#[test_matrix(["asm", "llvm"])]
fn it_interoperates_with_clang_floating_point(output_option: &str) {
    let code = "double foreign(float x,double y,long z);float local(double x,int y,float z){return x+y+z;}int main(){return (foreign(1.5f,2.25,3)==10.75)-1;}";
    let companion = "float local(double,int,float);double foreign(float x,double y,long z){return x+y+z+local(1.5,1,1.5f);}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    ("typedef struct{int a;int b;} S;", "{3,5}", "s.a+=1;", "s.b+=2;", "a.a==3 && a.b==5 && b.a==4 && b.b==7"),
    ("typedef struct{long a;long b;long c;} S;", "{1,2,3}", "s.c+=4;", "s.a+=2;", "a.c==3 && b.a==3 && b.b==2 && b.c==7"),
    ("typedef struct{char a[3];} S;", "{{1,2,3}}", "s.a[0]+=1;", "s.a[2]+=2;", "a.a[0]==1 && b.a[0]==2 && b.a[2]==5"),
    ("typedef struct{double a;long b;} S;", "{2.5,3}", "s.a+=1.5;", "s.b+=2;", "a.a==2.5 && b.a==4.0 && b.b==5"),
    ("typedef struct{float a[4];} S;", "{{1,2,3,4}}", "s.a[1]+=2;", "s.a[3]+=1;", "a.a[1]==2 && b.a[1]==4 && b.a[3]==5"),
    ("typedef struct{double a[4];} S;", "{{1,2,3,4}}", "s.a[1]+=2;", "s.a[3]+=1;", "a.a[1]==2 && b.a[1]==4 && b.a[3]==5"),
    ("typedef union{double x;double y;} S;", "{2.5}", "s.x+=1;", "s.x+=2;", "a.x==2.5 && b.x==5.5"),
    ("typedef struct{unsigned a:3;int b;} S;", "{5,9}", "s.a+=1;", "s.b+=2;", "a.a==5 && a.b==9 && b.a==6 && b.b==11"),
], ["asm", "llvm"])]
fn it_interoperates_with_clang_aggregate_values(
    case: (&str, &str, &str, &str, &str),
    output_option: &str,
) {
    let (layout, initializer, local_update, foreign_update, comparison) = case;
    let code = format!(
        "{layout} S foreign(S s);int check(S a,S b);S local(S s){{{local_update}return s;}}int main(){{S s={initializer};S t=foreign(s);return check(s,t);}}"
    );
    let companion = format!(
        "{layout} S local(S s);S foreign(S s){{s=local(s);{foreign_update}return s;}}int check(S a,S b){{return !({comparison});}}"
    );
    assert_eq!(
        compile_and_run_with_c(&code, output_option, Some(&companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn it_interoperates_with_clang_aggregate_register_spills(output_option: &str) {
    let layout = "typedef struct{double a[4];} S;";
    let code = format!(
        "{layout} double foreign(double a,double b,double c,double d,double e,double f,S s,int z);int main(){{S s={{{{1,2,3,4}}}};return (foreign(1,2,3,4,5,6,s,7)==38.0)-1;}}"
    );
    let companion = format!(
        "{layout} double foreign(double a,double b,double c,double d,double e,double f,S s,int z){{return a+b+c+d+e+f+s.a[0]+s.a[1]+s.a[2]+s.a[3]+z;}}"
    );
    assert_eq!(
        compile_and_run_with_c(&code, output_option, Some(&companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "#include <stdarg.h>\nlong sum(int n,...){va_list ap;va_start(ap,n);long s=0;for(int i=0;i<n;i++)s+=va_arg(ap,int);va_end(ap);return s;}int main(){return sum(10,1,2,3,4,5,6,7,8,9,10)-55;}",
    "#include <stdarg.h>\ndouble sum(int n,...){va_list ap;va_start(ap,n);double s=0;for(int i=0;i<n;i++)s+=va_arg(ap,double);va_end(ap);return s;}int main(){return (sum(10,1.0,2.0,3.0,4.0,5.0,6.0,7.0,8.0,9.0,10.0)==55.0)-1;}",
    "#include <stdarg.h>\nint sum(int n,...){va_list ap;va_list cp;va_start(ap,n);int a=va_arg(ap,int);va_copy(cp,ap);int b=va_arg(ap,int);int c=va_arg(cp,int);va_end(ap);va_end(cp);return a+b+c;}int main(){return sum(2,3,5)-13;}",
    "#include <stdarg.h>\nstruct S{double a;long b;};long sum(int n,...){va_list ap;va_start(ap,n);struct S s=va_arg(ap,struct S);va_end(ap);return (long)s.a+s.b;}int main(){struct S s={2.0,3};return sum(1,s)-5;}",
    "#include <stdarg.h>\nstruct S{long a[3];};long sum(int n,...){va_list ap;va_start(ap,n);struct S s=va_arg(ap,struct S);va_end(ap);return s.a[0]+s.a[2];}int main(){struct S s={{2,3,5}};return sum(1,s)-7;}",
], ["asm", "llvm"])]
fn it_receives_variadic_arguments(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn it_interoperates_with_clang_variadic_receivers(output_option: &str) {
    let code = "#include <stdarg.h>\nstruct S{double a;long b;};struct B{long a[3];};double local(int n,...){va_list ap;va_start(ap,n);double s=0;for(int i=0;i<n;i++){s+=va_arg(ap,int);s+=va_arg(ap,double);}struct S a=va_arg(ap,struct S);struct B b=va_arg(ap,struct B);char *p=va_arg(ap,char*);va_end(ap);return s+a.a+a.b+b.a[2]+*p;}double foreign();int main(){return (foreign()==107.0)-1;}";
    let companion = "struct S{double a;long b;};struct B{long a[3];};double local(int,...);double foreign(){struct S a={2.0,3};struct B b={{1,2,5}};return local(10,1,1.0,2,2.0,3,3.0,4,4.0,5,5.0,6,6.0,7,7.0,8,8.0,9,9.0,10,10.0,a,b,\"A\")-78;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn it_receives_variadic_hfa_after_register_spill(output_option: &str) {
    let code = "#include <stdarg.h>\nstruct S{double a[4];};double local(int n,...){va_list ap;va_start(ap,n);double s=0;for(int i=0;i<6;i++)s+=va_arg(ap,double);struct S a=va_arg(ap,struct S);double b=va_arg(ap,double);va_end(ap);return s+a.a[0]+a.a[3]+b;}double foreign();int main(){return (foreign()==33.0)-1;}";
    let companion = "struct S{double a[4];};double local(int,...);double foreign(){struct S a={{1,2,3,4}};return local(0,1.0,2.0,3.0,4.0,5.0,6.0,a,7.0);}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn it_forwards_va_list(output_option: &str) {
    let code = "#include <stdarg.h>\nint take(va_list ap){return va_arg(ap,int)+va_arg(ap,int);}int local(int n,...){va_list ap;va_start(ap,n);int s=take(ap);va_end(ap);return s;}int main(){return local(2,4,7)-11;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn it_interoperates_with_clang_variable_array_parameters(output_option: &str) {
    let code = "int foreign(int n,int m,int a[n][m]);int local(int n,int m,int a[n][m]){return a[n-1][m-1]+sizeof(*a);}int main(){int n=2;int m=3;int a[n][m];a[1][2]=7;return foreign(n,m,a)-21;}";
    let companion = "int local(int n,int m,int a[n][m]);int foreign(int n,int m,int a[n][m]){a[0][1]=2;return local(n,m,a)+a[0][1];}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn flexible_array_clang_interoperability(output_option: &str) {
    let code = "struct S{int n;int values[];};int foreign(struct S *p);int main(){struct S *p=malloc(sizeof(struct S)+3*sizeof(int));p->n=3;p->values[0]=2;p->values[1]=3;p->values[2]=5;int r=foreign(p);free(p);return r-10;}";
    let companion = "struct S{int n;int values[];};int foreign(struct S *p){int sum=0;for(int i=0;i<p->n;i++)sum+=p->values[i];return sum;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn flexible_array_by_value_clang_interoperability(output_option: &str) {
    let code = "struct S{double value;int tail[];};struct S foreign(struct S s);struct S local(struct S s){s.value+=1.0;return s;}int main(){struct S s={2.0};s=foreign(s);return s.value!=6.0;}";
    let companion = "struct S{double value;int tail[];};struct S local(struct S s);struct S foreign(struct S s){s=local(s);s.value*=2.0;return s;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn anonymous_aggregate_clang_interoperability(output_option: &str) {
    let code = "struct S{int n;union{double x;long y;};};struct S foreign(struct S s);struct S local(struct S s){s.x+=s.n;return s;}int main(){struct S s={.n=2,.x=3.5};s=foreign(s);return s.n!=4 || s.x!=5.5;}";
    let companion = "struct S{int n;union{double x;long y;};};struct S local(struct S s);struct S foreign(struct S s){s=local(s);s.n*=2;return s;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn simd_capability_profile_selects_portable_scalar_code(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
#if defined(__ARM_NEON) || defined(__ARM_NEON__) || defined(__ARM_FEATURE_SVE) || defined(__SSE2__) || defined(__AVX2__) || defined(__AVX512F__)
#error unimplemented vector intrinsic path selected
#else
int dot(int *a,int *b,int n){int result=0;for(int i=0;i<n;i++)result+=a[i]*b[i];return result;}
#endif
#if !defined(__aarch64__) && !defined(__x86_64__)
#error target architecture must remain available
#endif
int main(){int a[4]={1,2,3,4},b[4]={5,6,7,8};return dot(a,b,4)!=70 || sizeof(void*)!=8;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn reduced_typedef_alignment_preserves_scalar_call_abi(output_option: &str, optimization: &str) {
    let declarations = "typedef long T __attribute__((aligned(1)));";
    let signature = "int a,int b,int c,int d,int e,int f,int g,int h,char tag,T x,T y";
    let code = format!(
        "{declarations}T foreign({signature});T own({signature}){{return a+b+c+d+e+f+g+h+tag+x+y;}}int main(){{return foreign(1,2,3,4,5,6,7,8,9,10,11)!=66;}}"
    );
    let companion = format!(
        "{declarations}T own({signature});T foreign({signature}){{return own(a,b,c,d,e,f,g,h,tag,x,y);}}"
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn integer128_capability_selects_portable_arithmetic((output_option, optimization): (&str, &str)) {
    let code = r#"
#ifdef __SIZEOF_INT128__
#error Arithmetic for this extension is not supported
#else
unsigned long multiply_low(unsigned long a,unsigned long b){return a*b;}
#endif
__uint128_t storage;
int main(){return multiply_low(7,9)!=63 || sizeof(storage)!=16 || _Alignof(__int128)!=16;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["_Thread_local", "__thread"], ["-O0", "-O2"])]
fn thread_local_storage_is_private_and_interoperates_with_clang(keyword: &str, optimization: &str) {
    let code = format!(
        r"
{keyword} int shared = 7;
extern {keyword} int foreign;
{keyword} static int private = 11;
int check(int *, int *, int *);
int generated(void){{
    static {keyword} int block = 13;
    if(shared!=7 || foreign!=17 || private!=11 || block!=13)return 1;
    shared++;foreign++;private++;block++;
    return check(&shared,&private,&block);
}}
int run(int(*)(void));
int main(void){{int result=run(generated);if(result)return result;return generated();}}
"
    );
    let companion = r"
#include <pthread.h>
extern _Thread_local int shared;
_Thread_local int foreign=17;
int check(int *s,int *p,int *b){return s!=&shared || shared!=8 || foreign!=18 || *p!=12 || *b!=14;}
static void *worker(void *p){int (*f)(void)=p;return (void *)(long)f();}
int run(int(*f)(void)){pthread_t threads[2];for(int i=0;i<2;i++)if(pthread_create(&threads[i],0,worker,f))return 2;for(int i=0;i<2;i++){void *r;pthread_join(threads[i],&r);if(r)return 3;}return shared!=7 || foreign!=17;}
";
    let result =
        compile_and_run_with_c_flags(&code, "llvm", Some(companion), &[optimization, "-pthread"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
}

#[test_matrix(["asm", "llvm"])]
fn pointer_nullability_preserves_types_and_execution(output_option: &str) {
    let code = "typedef int *Handle;int read_value(int * _Nonnull p){return *p;}struct API{int (* _Nullable callback)(int*);};int main(){int n=7;Handle _Nullable p=&n;int *const _Null_unspecified same=p;struct API api={read_value};int result=api.callback(same);int * _Nullable_result empty=0;return result-7+(empty!=0);}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int _Nullable value;",
    "int * _Nonnull _Nullable p;",
    "int main(){int x=1;int * _Nonnull const p=&x;p=0;return 0;}",
])]
fn invalid_nullability_and_const_pointer_writes_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix(["asm", "llvm"])]
fn binary64_long_double_interoperates_with_clang(output_option: &str) {
    let code = r#"
#include <stdio.h>
struct Mixed{double x;long double y;};
struct Mixed foreign(struct Mixed value,long double delta);
long double local(long double x,double y){return x+y;}
struct Mixed local_record(struct Mixed x){x.y+=2.0L;return x;}
int main(){
    struct Mixed x={1.25,2.5L};
    struct Mixed y=foreign(x,3.0L);
    char text[32];
    sprintf(text,"%.2Lf",y.y);
    return (y.x!=1.25)+(y.y!=8.75L)+(text[0]!='8')+(text[2]!='7')+(text[3]!='5');
}"#;
    let companion = r#"
struct Mixed{double x;long double y;};
long double local(long double,double);
struct Mixed local_record(struct Mixed);
struct Mixed foreign(struct Mixed value,long double delta){
    value=local_record(value);
    value.y=local(value.y,delta+value.x);
    return value;
}"#;
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[cfg(target_arch = "x86_64")]
#[test_matrix(["-O0", "-O2"])]
fn x87_long_double_values_interoperate_with_clang(optimization: &str) {
    let code = r#"
unsigned long implicit=18446744073709551615.0L;
unsigned long explicit=(unsigned long)18446744073709551615.0L;
_Bool tiny=0x1p-2000L;
int conditional=0x1p-2000L?7:9;
float rounded=0x1.000001000000001p0L;
long double exact=18446744073709551615.0L;
long double hex=0x1.fffffffffffffffep63L;
long double computed=(18446744073709551615.0L-1.0L)/2.0L;
int comparison=18446744073709551615.0L>18446744073709551614.0L;
long double foreign(long double,int,double);
int verify(int,...);int check_global(long double *);int call_receiver(void);
long double receiver(long double first,int count,...){__builtin_va_list ap;__builtin_va_start(ap,count);long double result=first;for(int i=0;i<count;i++){result+=__builtin_va_arg(ap,long double);}__builtin_va_end(ap);return result;}
long double local(long double x,long double y){return x+y;}
long double promote(unsigned long x){return x;}
int main(){
unsigned long n=18446744073709551615UL;
long double x=promote(n);x-=1.0L;x++;x--;
long double negative=-1.25L;
return implicit!=n || explicit!=n || !tiny || conditional!=7 || rounded!=0x1.000002p0f || call_receiver() || exact!=promote(n) || hex!=exact || (unsigned long)x!=n-1
 || computed!=9223372036854775807.0L || !comparison
 || local(1.25L,2.5L)!=3.75L || foreign(1.25L,2,3.5)!=6.75L
 || __builtin_fabsl(negative)!=1.25L || !(__builtin_nanl("")!=0.0L)
 || !(__builtin_infl()>exact) || !(_Bool)x || (_Bool)0.0L
 || verify(3,exact,computed,local(1.0L,2.0L)) || check_global(&exact);
}
"#;
    let companion = r#"
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
long double receiver(long double,int,...);
int call_receiver(void){return receiver(1.25L,2,2.5L,3.75L)!=7.5L;}
long double local(long double,long double);
long double foreign(long double x,int y,double z){return local(x,(long double)y+z);}
int verify(int count,...){va_list ap;va_start(ap,count);long double a=va_arg(ap,long double),b=va_arg(ap,long double),c=va_arg(ap,long double);va_end(ap);return a!=18446744073709551615.0L || b!=9223372036854775807.0L || c!=3.0L;}
int check_global(long double *p){char text[64];snprintf(text,sizeof(text),"%.0Lf",*p);return strcmp(text,"18446744073709551615")!=0;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, "llvm", Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[cfg(target_arch = "x86_64")]
#[test_matrix([
    "struct S{int x;};long double f(struct S);int main(){return 0;}",
    "struct S{int x;};void f(int,...);int main(){struct S s={};f(0,s,1.0L);return 0;}",
])]
fn x87_aggregate_abi_combinations_are_diagnosed(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "llvm", false)).is_err());
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn float16_objects_preserve_layout_and_pointer_interop(output_option: &str, optimization: &str) {
    let code = r"
typedef _Float16 Half;
Half unused_api(Half);
struct Packet{char tag;Half value;char end;};
Half zeros[3]={};
struct Packet storage={};
void fill(Half *value);
int check(struct Packet *value,Half *array);
int main(){
    Half local;
    struct Packet packet;
    packet.tag=7;packet.end=9;
    fill(&local);
    unsigned char *from=(unsigned char*)&local;
    unsigned char *to=(unsigned char*)&packet.value;
    to[0]=from[0];to[1]=from[1];
    storage=packet;
    unsigned char *bytes=(unsigned char*)zeros;
    int i=0;while(i<6){if(bytes[i])return 1;i++;}
    return check(&storage,zeros)+(sizeof(Half)!=2)+(_Alignof(Half)!=2)
        +(sizeof(zeros)!=6)+(sizeof(packet)!=6)
        +((char*)&packet.value-(char*)&packet!=2)
        +((char*)&packet.end-(char*)&packet!=4)
        +((char*)&zeros[1]-(char*)zeros!=2)
        +_Generic(local,Half:0,float:1,default:2);
}";
    let companion = r"
struct Packet{char tag;_Float16 value;char end;};
void fill(_Float16 *value){*value=1.5f;}
int check(struct Packet *value,_Float16 *array){
    return value->tag!=7 || value->end!=9 || value->value!=1.5f || array[2]!=0;
}";
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn extended_integer_object_layout_interoperates_with_clang(output_option: &str) {
    let code = r#"
struct Wide{char tag;__uint128_t payload;int tail;};
struct Wide storage;
__int128_t values[2];
int check(struct Wide value,void *array);
int main(){
    struct Wide local;
    local.tag=7;local.tail=9;
    unsigned char *bytes=(unsigned char*)&local.payload;
    bytes[0]=3;bytes[15]=5;
    storage=local;
    unsigned char *next=(unsigned char*)&values[1];
    next[15]=11;
    return check(storage,values)+(sizeof(storage)!=48)+(_Alignof(struct Wide)!=16)
        +(sizeof(values)!=32)+(sizeof(unsigned __int128)!=16)
        +_Generic(storage.payload,unsigned __int128:0,__int128:1,default:2)
        +_Generic(values[0],signed __int128:0,unsigned __int128:1,default:2);
}"#;
    let companion = r#"
struct Wide{char tag;__uint128_t payload;int tail;};
int check(struct Wide value,void *array){
    unsigned char *bytes=(unsigned char*)&value.payload;
    unsigned char *other=array;
    return value.tag!=7 || value.tail!=9 || bytes[0]!=3 || bytes[15]!=5 || other[31]!=11;
}"#;
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "struct __attribute__((aligned(64))) S{int x;};int main(){struct S s;return 0;}",
    "struct __attribute__((aligned(64))) S{int x;};struct S f(struct S);",
    "struct __attribute__((aligned(64))) S{int x;};struct S f(void);",
    "struct S;struct S f(void);struct __attribute__((aligned(64))) S{int x;};",
    "struct __attribute__((aligned(64))) S{int x;};void f(int,...);struct S s;int main(){f(0,s);return 0;}"
])]
fn unsupported_extended_record_stack_abi_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn narrow_scalar_abi_interoperates_with_clang_in_both_directions(
    output_option: &str,
    optimization: &str,
) {
    let code = r"
struct Box{long value;};
struct Tiny{signed char value;};
int foreign(signed char,unsigned char,short,unsigned short,_Bool);
int foreign_variadic(signed char,int,...);
int foreign_mixed(signed char,struct Box,...);
int callbacks(int (*)(signed char,unsigned char,short,unsigned short,_Bool),signed char (*)(signed char),unsigned short (*)(unsigned short),signed char (*)(struct Box,signed char),struct Box (*)(signed char),struct Tiny (*)(struct Tiny),_Bool (*)(_Bool));
int own(signed char a,unsigned char b,short c,unsigned short d,_Bool e){return a!=-7 || b!=250 || c!=-12345 || d!=60000 || e!=1;}
signed char byte(signed char value){return value;}
unsigned short word(unsigned short value){return value;}
signed char mixed(struct Box value,signed char extra){return value.value+extra;}
struct Box record(signed char value){struct Box result={value};return result;}
struct Tiny tiny(struct Tiny value){return value;}
_Bool truth(_Bool value){return value;}
int main(){
    signed char a=-7;unsigned char b=250;short c=-12345;unsigned short d=60000;
    if(foreign(a,b,c,d,1))return 1;
    int (*indirect)(signed char,unsigned char,short,unsigned short,_Bool)=foreign;
    if(indirect(a,b,c,d,1))return 2;
    if(callbacks(own,byte,word,mixed,record,tiny,truth))return 3;
    if(foreign_variadic(a,0,(signed char)-9,(unsigned short)60000))return 4;
    struct Box box={11};
    if(foreign_mixed(a,box,(signed char)-9))return 5;
    return 0;
}
";
    let companion = r"
#include <stdarg.h>
struct Box{long value;};
struct Tiny{signed char value;};
int foreign(signed char a,unsigned char b,short c,unsigned short d,_Bool e){return a!=-7 || b!=250 || c!=-12345 || d!=60000 || e!=1;}
int foreign_variadic(signed char a,int n,...){va_list ap;va_start(ap,n);int b=va_arg(ap,int);int c=va_arg(ap,int);va_end(ap);return a!=-7 || b!=-9 || c!=60000;}
int foreign_mixed(signed char a,struct Box box,...){va_list ap;va_start(ap,box);int b=va_arg(ap,int);va_end(ap);return a!=-7 || box.value!=11 || b!=-9;}
int callbacks(int (*check)(signed char,unsigned char,short,unsigned short,_Bool),signed char (*byte)(signed char),unsigned short (*word)(unsigned short),signed char (*mixed)(struct Box,signed char),struct Box (*record)(signed char),struct Tiny (*tiny)(struct Tiny),_Bool (*truth)(_Bool)){
    struct Box box={11};struct Box result=record(-7);struct Tiny small={-13};
    return check(-7,250,-12345,60000,1) || byte(-7)!=-7 || word(60000)!=60000 || mixed(box,-18)!=-7 || result.value!=-7 || tiny(small).value!=-13 || truth(0)!=0 || truth(1)!=1;
}
";
    if output_option == "llvm" {
        let ir = dynamite_compiler::generate(code, output_option, false);
        if cfg!(any(
            target_arch = "x86_64",
            all(target_arch = "aarch64", target_os = "macos")
        )) {
            assert!(ir.contains("define signext i8 @byte(i8 signext noundef"));
            assert!(ir.contains("define zeroext i8 @truth(i8 zeroext noundef"));
            assert!(ir.contains("define signext i8 @mixed("));
        } else {
            assert!(ir.contains("define i8 @byte(i8 noundef"));
            assert!(!ir.contains("signext") && !ir.contains("zeroext"));
        }
    }
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn fixed_underlying_enums_interoperate_with_clang(output_option: &str) {
    let code = r#"
enum Byte:unsigned char{START=250};
enum Adjustment:signed short{DELTA=-2};
struct Packet{char tag;enum Byte value;enum Adjustment adjustment;unsigned int flags;};
struct Packet packet={7,START,DELTA,0x80000000U};
enum Byte foreign(enum Byte value,enum Adjustment adjustment,double extra);
int check(struct Packet value);
enum Byte local(enum Byte value,enum Adjustment adjustment){return value+adjustment;}
int main(){return (foreign(START,DELTA,3.0)!=251)+check(packet)+(sizeof(packet)!=8);}
"#;
    let companion = r#"
enum Byte:unsigned char{START=250};
enum Adjustment:signed short{DELTA=-2};
struct Packet{char tag;enum Byte value;enum Adjustment adjustment;unsigned int flags;};
enum Byte local(enum Byte,enum Adjustment);
enum Byte foreign(enum Byte value,enum Adjustment adjustment,double extra){return local(value,adjustment)+(int)extra;}
int check(struct Packet value){return value.tag!=7 || value.value!=250 || value.adjustment!=-2 || value.flags!=0x80000000U;}
"#;
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn aligned_variadic_arguments_match_clang(output_option: &str, optimization: &str) {
    let shared = r#"
#include <stdarg.h>
struct RecordPadding{double x;} __attribute__((aligned(16)));
struct MemberPadding{_Alignas(16) double x;};
struct HfaMember{_Alignas(16) double x;double y;};
struct HfaRecord{double x;double y;} __attribute__((aligned(16)));
#define DEFINE_BASIC(name) \
int name(int n,...){ \
    va_list ap;va_start(ap,n); \
    struct MemberPadding member=va_arg(ap,struct MemberPadding); \
    int prefix=va_arg(ap,int); \
    struct RecordPadding record=va_arg(ap,struct RecordPadding); \
    int middle=va_arg(ap,int); \
    struct HfaMember hm=va_arg(ap,struct HfaMember); \
    double scalar=va_arg(ap,double); \
    struct HfaRecord hr=va_arg(ap,struct HfaRecord); \
    int tail=va_arg(ap,int);va_end(ap); \
    return n!=1 || member.x!=2.5 || prefix!=12 || record.x!=3.5 || middle!=13 \
        || hm.x!=4.5 || hm.y!=5.5 || scalar!=8.5 || hr.x!=6.5 || hr.y!=7.5 || tail!=14; \
}
#define DEFINE_GP(name) \
int name(int a,int b,int c,int d,int e,int f,int g,int h,...){ \
    va_list ap;va_start(ap,h);int prefix=va_arg(ap,int); \
    struct RecordPadding record=va_arg(ap,struct RecordPadding); \
    int middle=va_arg(ap,int);int next=va_arg(ap,int); \
    struct MemberPadding member=va_arg(ap,struct MemberPadding); \
    int tail=va_arg(ap,int);va_end(ap); \
    return prefix!=12 || record.x!=3.5 || middle!=13 || next!=14 || member.x!=2.5 || tail!=15; \
}
#define DEFINE_FP(name) \
int name(double a,double b,double c,double d,double e,double f,double g,double h,int n,...){ \
    va_list ap;va_start(ap,n);double prefix=va_arg(ap,double); \
    struct HfaRecord hr=va_arg(ap,struct HfaRecord); \
    double middle=va_arg(ap,double);double next=va_arg(ap,double); \
    struct HfaMember hm=va_arg(ap,struct HfaMember); \
    int tail=va_arg(ap,int);va_end(ap); \
    return prefix!=9.5 || hr.x!=6.5 || hr.y!=7.5 || middle!=10.5 || next!=11.5 \
        || hm.x!=4.5 || hm.y!=5.5 || tail!=16; \
}
"#;
    // Clang on Darwin AArch64 disagrees with its own va_arg lowering for
    // over-aligned HFAs. Keep those HFAs naturally aligned in this interop test;
    // the non-HFA records above still exercise sixteen-byte variadic alignment.
    let shared = if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        shared
            .replace(
                "struct HfaMember{_Alignas(16) double x;double y;};",
                "struct HfaMember{double x;double y;};",
            )
            .replace(
                "struct HfaRecord{double x;double y;} __attribute__((aligned(16)));",
                "struct HfaRecord{double x;double y;};",
            )
    } else {
        shared.to_owned()
    };
    let code = [
        shared.as_str(),
        r#"
DEFINE_BASIC(local_basic)
DEFINE_GP(local_gp)
DEFINE_FP(local_fp)
int foreign_basic(int n,...);
int foreign_gp(int,int,int,int,int,int,int,int,...);
int foreign_fp(double,double,double,double,double,double,double,double,int,...);
int call_local_basic(void);int call_local_gp(void);int call_local_fp(void);
int main(){
    struct MemberPadding member={2.5};struct RecordPadding record={3.5};
    struct HfaMember hm={4.5,5.5};struct HfaRecord hr={6.5,7.5};
    if(foreign_basic(1,member,12,record,13,hm,8.5,hr,14))return 1;
    if(call_local_basic())return 2;
    if(foreign_gp(1,2,3,4,5,6,7,8,12,record,13,14,member,15))return 3;
    if(call_local_gp())return 4;
    if(foreign_fp(1,2,3,4,5,6,7,8,1,9.5,hr,10.5,11.5,hm,16))return 5;
    if(call_local_fp())return 6;
    return 0;
}
"#,
    ]
    .concat();
    let companion = [
        shared.as_str(),
        r#"
DEFINE_BASIC(foreign_basic)
DEFINE_GP(foreign_gp)
DEFINE_FP(foreign_fp)
int local_basic(int n,...);
int local_gp(int,int,int,int,int,int,int,int,...);
int local_fp(double,double,double,double,double,double,double,double,int,...);
int call_local_basic(void){
    struct MemberPadding member={2.5};struct RecordPadding record={3.5};
    struct HfaMember hm={4.5,5.5};struct HfaRecord hr={6.5,7.5};
    return local_basic(1,member,12,record,13,hm,8.5,hr,14);
}
int call_local_gp(void){
    struct MemberPadding member={2.5};struct RecordPadding record={3.5};
    return local_gp(1,2,3,4,5,6,7,8,12,record,13,14,member,15);
}
int call_local_fp(void){
    struct HfaMember hm={4.5,5.5};struct HfaRecord hr={6.5,7.5};
    return local_fp(1,2,3,4,5,6,7,8,1,9.5,hr,10.5,11.5,hm,16);
}
"#,
    ]
    .concat();
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "failed case: {:?}", result);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn over_aligned_hfa_variadic_round_trip((output_option, optimization): (&str, &str)) {
    let code = r#"
#include <stdarg.h>
struct Member{_Alignas(16) double x;double y;};
struct Wide{_Alignas(16) double x;double y;double z;double w;};
struct Record{double x;double y;} __attribute__((aligned(16)));
int check(int n,...){
    va_list ap;va_start(ap,n);
    int prefix=va_arg(ap,int);
    struct Member m=va_arg(ap,struct Member);
    int middle=va_arg(ap,int);
    struct Wide wide=va_arg(ap,struct Wide);
    struct Record r=va_arg(ap,struct Record);
    int tail=va_arg(ap,int);va_end(ap);
    return n!=1 || prefix!=2 || m.x!=3.5 || m.y!=4.5 || middle!=5
        || wide.x!=1.5 || wide.y!=2.5 || wide.z!=3.5 || wide.w!=4.5
        || r.x!=6.5 || r.y!=7.5 || tail!=8;
}
int main(){struct Member m={3.5,4.5};struct Wide w={1.5,2.5,3.5,4.5};
    struct Record r={6.5,7.5};return check(1,2,m,5,w,r,8);}
"#;
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn leading_nullability_on_pointer_typedefs_preserves_types(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
typedef int *Pointer;typedef int (*Callback)(int);
_Nullable Pointer nullable;
const _Nullable Pointer constant=0;
_Nullable const Pointer also_constant=0;
_Nonnull Pointer identity(_Nullable Pointer value){return value;}
_Null_unspecified Callback callback;
_Nullable_result Pointer maybe(_Nonnull Pointer value){return value;}
int increment(int n){return n+1;}
int main(){
    int n=7;nullable=&n;callback=increment;
    _Nullable Pointer local=identity(nullable);
    _Nonnull Pointer _Nonnull repeated=&n;
    if(local!=&n || repeated!=&n || maybe(&n)!=&n || callback(*local)!=8)return 1;
    if(constant || also_constant)return 2;
    return _Generic(nullable,int*:0,default:1) || _Generic(callback,int(*)(int):0,default:1);
}
"#;
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "_Nullable int object;",
    "_Nonnull int *object;",
    "typedef int *Pointer;_Nullable Pointer _Nonnull object;",
    "typedef int *Pointer;_Nonnull const _Nullable Pointer object;",
    "typedef int *Pointer;int main(){const _Nullable Pointer p=0;p=0;return 0;}",
])]
fn invalid_leading_nullability_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn availability_attributes_accept_underscore_versions(output_option: &str) {
    let code = r#"
int available(void) __attribute__((availability(macosx,introduced=10_0,deprecated=12_3_4,message="use newer API")));
int available(void){return 7;}
typedef int Word __attribute__((availability(ios,introduced=3_0)));
struct Record{Word value;} __attribute__((availability(macos,introduced=11_2)));
int main(){struct Record r={available()};return r.value!=7 || sizeof(Word)!=4;}
"#;
    let result = compile_and_run(code, output_option);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn nonnegative_enum_abi_matches_clang(output_option: &str, optimization: &str) {
    let definitions = r#"
enum Flags{ZERO=0,SEVEN=7};enum Negative{MINUS=-1,PLUS=3};enum Fixed:int{FIXED=2};
enum Large{HIGH=0xffffffffU};
"#;
    let code = [definitions,r#"
_Static_assert(__builtin_types_compatible_p(enum Flags,unsigned int),"unsigned enum representation");
_Static_assert(!__builtin_types_compatible_p(enum Flags,int),"distinct signed representation");
_Static_assert(__builtin_types_compatible_p(enum Negative,int),"negative enum representation");
_Static_assert(__builtin_types_compatible_p(enum Fixed,int),"fixed representation");
_Static_assert(_Generic(SEVEN,int:1,default:0),"ordinary enumerator type");
_Static_assert(_Generic(HIGH,unsigned int:1,default:0),"extended enumerator type");
extern enum Flags shared;unsigned int shared=SEVEN;
int accept_unsigned(unsigned value){return value==7;}
int accept_enum(enum Flags value){return value==7;}
int (*as_enum)(enum Flags)=accept_unsigned;
int (*as_unsigned)(unsigned)=accept_enum;
int foreign(enum Flags,int(*)(unsigned));
int reference(void);
int main(void){enum Flags all=(enum Flags)-1;enum Negative negative=MINUS;
    if(all<0 || all!=~0U || negative>=0 || shared!=7)return 1;
    if(_Generic(+all,unsigned int:0,default:1))return 2;
    return as_enum(SEVEN)!=1 || as_unsigned(SEVEN)!=1 || foreign(SEVEN,accept_enum)!=1 || reference()!=1;}
"#].concat();
    let companion = [
        definitions,
        r#"
int foreign(enum Flags value,int(*call)(unsigned)){return call(value);}
int reference(void){return __builtin_types_compatible_p(enum Flags,unsigned int)
    && !__builtin_types_compatible_p(enum Flags,int)
    && _Generic(SEVEN,int:1,default:0)
    && _Generic(+(enum Flags)0,unsigned int:1,default:0);}
"#,
    ]
    .concat();
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn transparent_union_pointers_interoperate_with_clang(output_option: &str, optimization: &str) {
    let definitions = r#"
typedef union __attribute__((transparent_union)) Handle{int *integer;char *text;void *any;}Handle;
typedef union __attribute__((transparent_union)) Other{int *integer;char *text;void *any;}Other;
typedef int HandleFunction(Handle);typedef int IntegerFunction(int*);
typedef int TextFunction(char*);typedef int FloatFunction(float*);typedef int OtherFunction(Other);
"#;
    let code=[definitions,r#"
#if !__has_attribute(transparent_union) || !__has_attribute(__transparent_union__)
#error missing transparent union attribute
#endif
_Static_assert(!__builtin_types_compatible_p(Handle,void*),"union identity");
_Static_assert(__builtin_types_compatible_p(HandleFunction,IntegerFunction),"member function compatibility");
_Static_assert(__builtin_types_compatible_p(HandleFunction,TextFunction),"other member function compatibility");
_Static_assert(!__builtin_types_compatible_p(HandleFunction,FloatFunction),"nonmember function type");
_Static_assert(!__builtin_types_compatible_p(HandleFunction,OtherFunction),"distinct union parameters");
int calls;int object=7;int *next(void){calls++;return &object;}
void *identity(Handle h){Handle local=h;return local.any;}
Handle roundtrip(Handle h){return h;}
int read_handle(Handle h){return h.any!=0;}
void *foreign(Handle);
Handle foreign_roundtrip(Handle);
int foreign_object(Handle*);
int invoke(int(*)(int*),int*);
int addressable(Handle h){return foreign_object(&h);}
int main(void){char text[2]="a";void *any=&object;Handle stored={.integer=&object};
    if(identity(next())!=&object || calls!=1 || identity(text)!=text || identity(any)!=any)return 1;
    if(identity(0)!=0 || identity(0UL)!=0 || identity(1-1)!=0 || identity((void*)0)!=0)return 2;
    if(identity(stored)!=&object || foreign(text)!=text || foreign_roundtrip(&object).any!=&object)return 3;
    if(roundtrip(text).any!=text || !addressable(&object))return 4;
    int (*callback)(int*)=read_handle;
    return !callback(&object) || !invoke(read_handle,&object);}
"#].concat();
    let companion = [
        definitions,
        r#"
extern int object;
void *foreign(Handle h){return h.any;}
Handle foreign_roundtrip(Handle h){return h;}
int foreign_object(Handle *h){return h->any==&object;}
int invoke(int(*call)(int*),int *p){return call(p);}
"#,
    ]
    .concat();
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    ("signed char","-7"), ("unsigned char","250"),
    ("signed short","-20000"), ("unsigned short","60000"),
    ("int","-123456"), ("unsigned int","4000000000U"),
    ("long","-5000000000L"), ("long long","-6000000000LL"),
    ("unsigned long long","18446744073709551615ULL"),
    ("float","1.5f"), ("double","2.25"),
], ["asm", "llvm"], ["-O0", "-O2"])]
fn transparent_union_scalar_abi_matches_clang(
    case: (&str, &str),
    output_option: &str,
    optimization: &str,
) {
    let (ty, value) = case;
    let definition =
        format!("typedef union U{{{ty} first;{ty} second;}}U __attribute__((transparent_union));");
    let code = format!(
        r#"{definition}
{ty} ours(U value){{return value.first;}}
{ty} foreign(U);
int main(void){{{ty} value={value};U explicit={{.second=value}};
    {ty} (*callback)({ty})=ours;
    return ours(value)!=value || ours(explicit)!=value || foreign(value)!=value || callback(value)!=value;}}
"#
    );
    let companion = format!("{definition}\n{ty} foreign(U value){{return value.first;}}");
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn transparent_union_callback_types_can_refer_to_the_union(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
union Recursive;
typedef union Recursive{int (*call)(union Recursive);}Recursive __attribute__((transparent_union));
int terminal(Recursive value){return value.call!=0;}
int invoke(Recursive value){return value.call(terminal);}
int main(void){return invoke(terminal)!=1;}
"#;
    let output = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn transparent_union_attribute_forms_and_variadic_calls(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
typedef __attribute__((transparent_union)) union Prefix{int *p;char *q;}Prefix;
union Trailing{int *p;char *q;} __attribute__((__transparent_union__));
union Original{const int *p;const char *q;};
typedef union Original Alias __attribute__((transparent_union));
int prefix(Prefix value){return value.p!=0;}
int trailing(union Trailing value){return value.p!=0;}
int alias(Alias value){return value.p!=0;}
int sum(Prefix first,...){__builtin_va_list ap;__builtin_va_start(ap,first);
    int n=__builtin_va_arg(ap,int);__builtin_va_end(ap);return (first.p!=0)+n;}
int main(void){int n=3;const int immutable=7;
    return !prefix(&n) || !trailing(&n) || !alias(&immutable) || sum(&n,7)!=8;}
"#;
    let output = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

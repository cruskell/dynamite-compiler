use crate::support::{
    assert_frontend_error, compile_and_run, compile_and_run_with_c, compile_and_run_with_c_flags,
};
use test_case::test_matrix;

const CHECKED_BSD_STRING_RUNTIME: &str = r#"
#include <stddef.h>
#include <stdlib.h>
#include <signal.h>
#ifndef __APPLE__
size_t strlcpy(char *dst,const char *src,size_t n){
    size_t len=0;while(src[len])len++;
    if(n){size_t copied=len<n-1?len:n-1;for(size_t i=0;i<copied;i++)dst[i]=src[i];dst[copied]=0;}
    return len;
}
size_t strlcat(char *dst,const char *src,size_t n){
    size_t used=0;while(used<n && dst[used])used++;
    size_t len=0;while(src[len])len++;
    if(used<n)strlcpy(dst+used,src,n-used);
    return used+len;
}
size_t __strlcpy_chk(char *dst,const char *src,size_t n,size_t capacity){
    if(n>capacity)abort();return strlcpy(dst,src,n);
}
size_t __strlcat_chk(char *dst,const char *src,size_t n,size_t capacity){
    if(n>capacity)abort();return strlcat(dst,src,n);
}
#endif
static volatile unsigned char *watched;
static void on_failure(int number){
    (void)number;for(int i=0;i<8;i++)if(watched[i]!=55)_Exit(74);_Exit(73);
}
void install_failure_handler(char *p){watched=(unsigned char*)p;signal(SIGABRT,on_failure);signal(SIGTRAP,on_failure);signal(SIGILL,on_failure);}
unsigned long excessive_size(void){return 5;}
"#;

#[test_matrix([
    "#include <stdarg.h>\nint f(int n){va_list ap;va_start(ap,n);return 0;}",
    "#include <stdarg.h>\nint f(int a,int b,...){va_list ap;va_start(ap,a);return 0;}",
    "#include <stdarg.h>\nint f(int n,...){va_list ap;va_start(ap,n);return (int)va_arg(ap,float);}",
])]
fn it_diagnoses_invalid_variadic_builtins(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "asm", false)).is_err());
}

#[test_matrix(["-O0", "-O2"])]
fn builtin_alloca_preserves_function_lifetime_and_expression_values(optimization: &str) {
    let code = r"
int check(int a,int *p,int b){return a!=3 || *p!=7 || b!=9;}
int run(int n){int *saved[4];int i=0;while(i<4){saved[i]=__builtin_alloca(n*sizeof(int));saved[i][0]=i+7;i++;}
int total=0;for(i=0;i<4;i++){total+=saved[i][0];if((unsigned long)saved[i]%16)return 1;}
int *p=__builtin_alloca(sizeof(int));*p=7;
return total!=34 || check(3,p,9) || (5+(unsigned long)__builtin_alloca(8)-5)%16;}
int main(){return run(3)+run(5);}
";
    assert_eq!(
        compile_and_run_with_c_flags(code, "llvm", None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn memory_builtins_call_the_runtime_once(output_option: &str, optimization: &str) {
    let code = "int calls;int size(void){calls++;return 4;}int main(){char a[8]={1,2,3,4};char b[8]={};void *p=__builtin_memcpy(b,a,size());if(p!=b || calls!=1 || b[3]!=4)return 1;p=__builtin_memmove(b+1,b,4);if(p!=b+1 || b[1]!=1 || b[4]!=4)return 2;p=__builtin_memset(b,257,size());return p!=b || calls!=2 || b[0]!=1 || b[3]!=1 || b[4]!=4;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){char a[4];__builtin_memset(a,0);return 0;}",
    "int main(){char a[4];__builtin_memcpy(a,1,4);return 0;}",
    "int main(){char a[4];__builtin_memmove(a,a,4,4);return 0;}",
])]
fn invalid_memory_builtins_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn builtin_va_list_uses_the_target_layout(output_option: &str) {
    let code = "typedef __builtin_va_list system_list;int sum(int n,...){system_list a,b;__builtin_va_start(a,n);__builtin_va_copy(b,a);int x=__builtin_va_arg(a,int);int first=__builtin_va_arg(b,int);double y=first+__builtin_va_arg(b,double);__builtin_va_end(a);__builtin_va_end(b);return x+(int)y;}int main(){return sum(2,7,3.0)-17;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
    let expected = if cfg!(target_arch = "x86_64") {
        24
    } else if cfg!(target_os = "macos") {
        8
    } else {
        32
    };
    let code = format!(
        "int main(){{return sizeof(__builtin_va_list)-{expected}+_Alignof(__builtin_va_list)-8;}}"
    );
    assert_eq!(compile_and_run(&code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn builtin_va_list_interoperates_with_system_c(output_option: &str) {
    let code = "int consume(__builtin_va_list);int relay(int n,...){__builtin_va_list ap;__builtin_va_start(ap,n);int result=consume(ap);__builtin_va_end(ap);return result;}int main(){return relay(0,9,2.5)-11;}";
    let companion = "#include <stdarg.h>\nint consume(va_list ap){int a=va_arg(ap,int);double b=va_arg(ap,double);return a+(int)b;}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn builtin_prefetch_preserves_address_side_effects((output_option, optimization): (&str, &str)) {
    let mut code = String::from(
        r#"
#if !__has_builtin(__builtin_prefetch)
#error missing prefetch builtin
#endif
char buffer[64];int calls;const void *destination(void){calls++;return buffer;}
_Static_assert(_Generic(__builtin_prefetch((void*)0),int:0,default:1),"void hint result");
int main(){for(int i=0;i<64;i++)buffer[i]=37;__builtin_prefetch(destination());__builtin_prefetch(destination(),1);
"#,
    );
    for write in 0..=1 {
        for locality in 0..=3 {
            code.push_str(&format!(
                "__builtin_prefetch(destination(),{write},{locality});"
            ));
        }
    }
    code.push_str("__builtin_prefetch((void*)1);if(calls!=10)return 1;for(int i=0;i<64;i++)if(buffer[i]!=37)return 2;return 0;}");
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn builtin_prefetch_emits_a_target_hint(output_option: &str) {
    let generated = dynamite_compiler::generate(
        "int value;int main(){__builtin_prefetch(&value,1,2);return 0;}",
        output_option,
        false,
    );
    let expected = if output_option == "llvm" {
        "call void @llvm.prefetch.p0"
    } else if cfg!(target_arch = "aarch64") {
        "prfm pstl2keep"
    } else {
        "prefetcht1"
    };
    assert!(generated.contains(expected), "{generated}");
}

#[test_matrix([
    "int main(){__builtin_prefetch();}",
    "int main(){__builtin_prefetch(1);}",
    "int main(){__builtin_prefetch((void*)0,2);}",
    "int main(){__builtin_prefetch((void*)0,-1);}",
    "int main(){__builtin_prefetch((void*)0,0,4);}",
    "int main(){__builtin_prefetch((void*)0,0,-1);}",
    "int main(){int hint=0;__builtin_prefetch((void*)0,hint);}",
    "int main(){__builtin_prefetch((void*)0,0.0);}",
    "int main(){__builtin_prefetch((void*)0,0,0,0);}",
], ["asm", "llvm"])]
fn invalid_builtin_prefetch_calls_have_source_diagnostics(code: &str, output_option: &str) {
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

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn builtin_bzero_clears_only_the_requested_bytes((output_option, optimization): (&str, &str)) {
    let code = r#"
#if !__has_builtin(__builtin_bzero)
#error missing zeroing builtin
#endif
char buffer[64];int pointers,lengths;
void *destination(void){pointers++;return buffer+7;}
unsigned long length(void){lengths++;return 17;}
_Static_assert(_Generic(__builtin_bzero((void*)0,0),int:0,default:1),"void builtin result");
int main(void){
for(int i=0;i<64;i++)buffer[i]=90;
__builtin_bzero(destination(),length());
if(pointers!=1 || lengths!=1)return 1;
__builtin_bzero(buffer,0);
for(int i=0;i<64;i++)if(buffer[i]!=(i>=7 && i<24?0:90))return 2;
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
    "int main(){__builtin_bzero();}",
    "int main(){char a[3];__builtin_bzero(a,1,2);}",
    "int main(){__builtin_bzero(3,1);}",
    "int main(){const char a[3]={0};__builtin_bzero(a,1);}",
    "int main(){char a[3];__builtin_bzero(a,a);}",
    "int main(){char a[3];int n=__builtin_bzero(a,1);}",
    "void (*pointer)(void*,unsigned long)=__builtin_bzero;",
    "int bzero;int main(){char a[3];__builtin_bzero(a,1);}",
    "int bzero(void*,unsigned long);int main(){char a[3];__builtin_bzero(a,1);}",
], ["asm", "llvm"])]
fn invalid_builtin_bzero_calls_have_source_diagnostics(code: &str, output_option: &str) {
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
fn population_count_builtins_match_clang(output_option: &str, optimization: &str) {
    let code = r#"
int reference32(unsigned int);
int reference64(unsigned long);
int reference_ll(unsigned long long);
_Static_assert(__builtin_popcount(0) == 0, "zero");
_Static_assert(__builtin_popcount(-1) == 32, "int width");
_Static_assert(__builtin_popcount(0x100000001UL) == 1, "narrow input");
_Static_assert(__builtin_popcountl(-1L) == 64, "long width");
_Static_assert(__builtin_popcountll(-1LL) == 64, "long long width");
int global = __builtin_popcountll(0x8100000000000001ULL);
int main() {
    unsigned long x = 0x8123456789abcdefUL;
    int i;
    for(i = 0; i < 300; ++i) {
        if(__builtin_popcount(x) != reference32(x)) return 1;
        if(__builtin_popcountl(x) != reference64(x)) return 2;
        if(__builtin_popcountll(x) != reference_ll(x)) return 3;
        x = x * 6364136223846793005UL + 1442695040888963407UL;
    }
    volatile unsigned long observed = 0x8000000000000001UL;
    if(__builtin_popcountll(observed++) != 2 || observed != 0x8000000000000002UL) return 4;
    i = 0;
    if((0 && __builtin_popcount(i++)) || (1 ? 0 : __builtin_popcount(i++)) || i) return 5;
    if(sizeof(__builtin_popcount(i++)) != sizeof(int) || i) return 6;
    if(_Generic(__builtin_popcountl(x), int:0, default:1)) return 7;
    if(global != 3 || __builtin_popcount((unsigned char)255) != 8) return 8;
    return 0;
}
"#;
    let companion = "int reference32(unsigned int x){return __builtin_popcount(x);} int reference64(unsigned long x){return __builtin_popcountl(x);} int reference_ll(unsigned long long x){return __builtin_popcountll(x);}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){return __builtin_popcount();}",
    "int main(){return __builtin_popcount(1,2);}",
    "int main(){int x;return __builtin_popcount(&x);}",
    "struct S{int n;};int main(){struct S x={1};return __builtin_popcountll(x);}"
])]
fn invalid_population_count_builtins(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn integer_bit_scan_builtins_match_clang(output_option: &str, optimization: &str) {
    let code = r#"
int reference32(unsigned int,int);
int reference64(unsigned long,int);
int reference_ll(unsigned long long,int);
_Static_assert(__builtin_clz(1)==31,"int leading");
_Static_assert(__builtin_clzl(1UL)==63,"long leading");
_Static_assert(__builtin_clzll(1ULL)==63,"long long leading");
_Static_assert(__builtin_ctz(0x80000000U)==31,"int trailing");
_Static_assert(__builtin_ctzl(0x8000000000000000UL)==63,"long trailing");
_Static_assert(__builtin_ctzll(0x8000000000000000ULL)==63,"long long trailing");
_Static_assert(__builtin_clz(0x100000001UL)==31,"input conversion");
int global=__builtin_ctzll(0x100000000ULL);
int main(){
    int i;
    for(i=0;i<64;++i){
        unsigned long bit=1UL<<i;
        if(__builtin_clzl(bit)!=63-i || __builtin_clzll(bit)!=63-i) return 1;
        if(__builtin_ctzl(bit)!=i || __builtin_ctzll(bit)!=i) return 2;
        if(i<32 && (__builtin_clz(bit)!=31-i || __builtin_ctz(bit)!=i)) return 3;
    }
    unsigned long x=0x8123456789abcdefUL;
    for(i=0;i<300;++i){
        if((unsigned int)x && (__builtin_clz(x)!=reference32(x,1) || __builtin_ctz(x)!=reference32(x,0))) return 4;
        if(x && (__builtin_clzl(x)!=reference64(x,1) || __builtin_ctzl(x)!=reference64(x,0))) return 5;
        if(x && (__builtin_clzll(x)!=reference_ll(x,1) || __builtin_ctzll(x)!=reference_ll(x,0))) return 6;
        x=x*6364136223846793005UL+1442695040888963407UL;
    }
    volatile unsigned long observed=1;
    if(__builtin_clzll(observed++)!=63 || observed!=2) return 7;
    if(__builtin_ctzll(observed++)!=1 || observed!=3) return 8;
    i=1;
    if((0 && __builtin_clz(i++)) || (1?0:__builtin_ctz(i++)) || sizeof(__builtin_clzll(i++))!=4 || i!=1) return 9;
    if(_Generic(__builtin_clzll(x),int:0,default:1) || global!=32 || __builtin_clz(-1)!=0) return 10;
    volatile unsigned long zero=0;
    if(__builtin_clz(zero)!=32 || __builtin_ctz(zero)!=32) return 11;
    if(__builtin_clzl(zero)!=64 || __builtin_ctzl(zero)!=64) return 12;
    if(__builtin_clzll(zero)!=64 || __builtin_ctzll(zero)!=64) return 13;
    return 0;
}
"#;
    let companion = "int reference32(unsigned int x,int leading){return leading?__builtin_clz(x):__builtin_ctz(x);}int reference64(unsigned long x,int leading){return leading?__builtin_clzl(x):__builtin_ctzl(x);}int reference_ll(unsigned long long x,int leading){return leading?__builtin_clzll(x):__builtin_ctzll(x);}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn runtime_bit_scans_use_width_correct_llvm_intrinsics() {
    let ir = dynamite_compiler::generate(
        "int leading(unsigned long x){return __builtin_clzll(x);}int trailing(unsigned int x){return __builtin_ctz(x);}",
        "llvm",
        false,
    );
    assert!(ir.contains("call i64 @llvm.ctlz.i64(i64"), "{ir}");
    assert!(ir.contains("call i32 @llvm.cttz.i32(i32"), "{ir}");
    assert!(ir.contains("trunc i64"), "{ir}");
    assert_eq!(ir.matches("i1 false)").count(), 2, "{ir}");
}

#[test_matrix([
    "int main(){return __builtin_clz();}",
    "int main(){return __builtin_ctzll(1,2);}",
    "int main(){int x;return __builtin_clzl(&x);}",
    "struct S{int x;};int main(){struct S s={1};return __builtin_ctz(s);}"
])]
fn invalid_integer_bit_scan_builtins(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn compile_time_constant_queries_do_not_evaluate_operands(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
_Static_assert(__builtin_constant_p(7),"literal");
_Static_assert(__builtin_constant_p(2+3*4),"integer arithmetic");
_Static_assert(__builtin_constant_p(3.5),"floating literal");
_Static_assert(__builtin_constant_p("text"),"string literal");
_Static_assert(__builtin_constant_p((int)3.5),"literal cast");
_Static_assert(!__builtin_constant_p(1/0),"invalid division");
_Static_assert(!__builtin_constant_p(1<<32),"invalid shift");
_Static_assert(!__builtin_constant_p((-9223372036854775807L-1)/-1),"invalid quotient");
int global=__builtin_constant_p(1+2);
int calls;
int side(){calls++;return 3;}
int runtime(int x){calls++;return x+1;}
#define SELECT(x) (__builtin_constant_p(x)?(x)+1:runtime(x))
int main(){
    int n=2;
    volatile int v=7;
    if(__builtin_constant_p(n) || __builtin_constant_p(v) || __builtin_constant_p(side())) return 1;
    if(__builtin_constant_p(n++) || __builtin_constant_p((n=9)) || __builtin_constant_p((side(),3))) return 2;
    if(n!=2 || calls) return 3;
    if(__builtin_constant_p(sizeof(int[n++]))) return 4;
    if(n!=2) return 5;
    int a[n];
    if(__builtin_constant_p(sizeof(a)) || __builtin_constant_p(a)) return 6;
    if(SELECT(7)!=8 || calls) return 7;
    if(SELECT(n++)!=3 || calls!=1 || n!=3) return 8;
    if(!global || _Generic(__builtin_constant_p(n),int:0,default:1)) return 9;
    if(!__builtin_constant_p((unsigned long)~0U) || !__builtin_constant_p(1?7:8)) return 10;
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
    "int main(){return __builtin_constant_p();}",
    "int main(){return __builtin_constant_p(1,2);}",
    "int main(){return __builtin_constant_p(unknown);}"
])]
fn invalid_compile_time_constant_queries(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn atomic_compare_exchange_matches_clang_for_scalar_widths(
    output_option: &str,
    optimization: &str,
) {
    let types = [
        "signed char",
        "unsigned char",
        "short",
        "unsigned short",
        "int",
        "unsigned int",
        "long",
        "unsigned long",
        "_Bool",
    ];
    let mut code = String::from("int check(void);int main(void){return check();}");
    let mut companion = String::new();
    let mut cases = String::new();
    for (index, ty) in types.iter().enumerate() {
        code.push_str(&format!("int cas{index}({ty} *p,{ty} *e,{ty} v){{return __atomic_compare_exchange_n(p,e,v,0,5,5);}}"));
        companion.push_str(&format!("int cas{index}({ty}*,{ty}*,{ty});"));
        cases.push_str(&format!(
            r"
{{ {ty} a=({ty})seed,b=a,e=({ty})(seed+(i%2)),f=e,v=({ty})(seed>>13);
int x=cas{index}(&a,&e,v),y=__atomic_compare_exchange_n(&b,&f,v,0,5,5);
if(x!=y || a!=b || e!=f)return {index}+1;
if(!cas{index}(&a,&a,({ty})42) || a!=({ty})42)return {index}+11;
}}
"
        ));
    }
    companion.push_str(&format!("int check(void){{unsigned long seed=0x8123456789abcdefUL;for(int i=0;i<80;i++){{{cases}seed=seed*6364136223846793005UL+1442695040888963407UL;}}return 0;}}"));
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn atomic_compare_exchange_preserves_pointers_and_argument_effects(
    (output_option, optimization): (&str, &str),
) {
    let code = r"
int object=7,expected=0,calls;
int *target(void){calls++;return &object;}int *expectation(void){calls++;expected=0;return &expected;}
int desired(void){calls++;return 9;}int weak(void){calls++;return 1;}
int success(void){calls++;expected=7;return 5;}int failure(void){calls++;return 2;}
int function(void){return 13;}
int main(void){
    if(!__atomic_compare_exchange_n(target(),expectation(),desired(),weak(),success(),failure()) || calls!=6 || object!=9 || expected!=7)return 1;
    if(__atomic_compare_exchange_n(&object,&expected,11,1,5,5) || object!=9 || expected!=9)return 2;
    int array[2]={3,5};int *p=0,*e=0;
    if(!__atomic_compare_exchange_n(&p,&e,array,0,5,5) || p!=array || e)return 3;
    if(__atomic_compare_exchange_n(&p,&e,0,0,5,5) || e!=array || p!=array)return 4;
    if(!__atomic_compare_exchange_n(&p,&e,array+1,0,5,5) || *p!=5 || e!=array)return 5;
    int (*fn)(void)=0,(*ef)(void)=0;
    if(!__atomic_compare_exchange_n(&fn,&ef,function,0,5,5) || fn()!=13 || ef)return 6;
    if(__atomic_compare_exchange_n(&fn,&ef,0,0,5,5) || ef!=function)return 7;
    if(!__atomic_compare_exchange_n(&fn,&ef,0,0,5,5) || fn || ef!=function)return 8;
    if(_Generic(__atomic_compare_exchange_n(&object,&expected,17,0,5,5),_Bool:0,default:1) || object!=9)return 9;
    volatile unsigned short v=65535,e16=65535;
    if(!__atomic_compare_exchange_n(&v,&e16,1,0,5,5) || v!=1 || e16!=65535)return 10;
    if(__atomic_compare_exchange_n(&v,&e16,2,0,5,5) || e16!=1 || v!=1)return 11;
    return 0;
}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn atomic_compare_exchange_retains_updates_under_contention(
    output_option: &str,
    optimization: &str,
) {
    let code = r"
void *increment(void *p){unsigned long *counter=p;for(int i=0;i<4000;i++){unsigned long expected=__atomic_load_n(counter,0);while(!__atomic_compare_exchange_n(counter,&expected,expected+1,1,5,0)){} }return 0;}
int run(void*(*)(void*));int main(void){return run(increment);}
";
    let companion = r"
#include <pthread.h>
static void *other(void *p){for(int i=0;i<4000;i++)__atomic_fetch_add((unsigned long*)p,1,5);return 0;}
int run(void*(*increment)(void*)){unsigned long count=0;pthread_t threads[4];for(int i=0;i<4;i++)if(pthread_create(&threads[i],0,i<2?increment:other,&count))return 2;for(int i=0;i<4;i++)pthread_join(threads[i],0);return count!=16000;}
";
    assert_eq!(
        compile_and_run_with_c_flags(
            code,
            output_option,
            Some(companion),
            &[optimization, "-pthread"]
        )
        .status
        .code(),
        Some(0)
    );
}

#[test]
fn atomic_compare_exchange_preserves_volatile_expected_accesses_in_llvm() {
    let ir = dynamite_compiler::generate(
        "volatile int object,expected;int main(){return __atomic_compare_exchange_n(&object,&expected,3,0,5,5);}",
        "llvm",
        false,
    );
    assert!(ir.contains("cmpxchg volatile ptr"));
    assert!(ir.contains("load volatile i32"));
    assert!(ir.contains("store volatile i32"));
    assert!(ir.contains("seq_cst seq_cst, align 4"));
}

#[test_matrix([
    "int main(){int x,e;return __atomic_compare_exchange_n(&x,&e,1,0,0,2);}",
    "int main(){int x,e;return __atomic_compare_exchange_n(&x,&e,1,0,3,1);}",
    "int main(){int x,e;return __atomic_compare_exchange_n(&x,&e,1,0,5,3);}",
    "int main(){int x,e;return __atomic_compare_exchange_n(&x,&e,1,0,5,4);}",
    "int main(){int x,e;return __atomic_compare_exchange_n(&x,&e,1,0,6,0);}",
    "int main(){const int x=0;int e;return __atomic_compare_exchange_n(&x,&e,1,0,5,5);}",
    "int main(){int x;const int e=0;return __atomic_compare_exchange_n(&x,&e,1,0,5,5);}",
    "int main(){int x;long e;return __atomic_compare_exchange_n(&x,&e,1,0,5,5);}",
    "int main(){float x,e;return __atomic_compare_exchange_n(&x,&e,1,0,5,5);}",
    "int main(){int x,e;return __atomic_compare_exchange_n(&x,&e,1,0,5);}",
])]
fn atomic_compare_exchange_rejects_invalid_arguments(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn atomic_load_store_preserve_values_and_publication(output_option: &str, optimization: &str) {
    let code = r#"
int run_publication(void*(*)(void*));
struct State { int ready; int data; int error; };
void *publish(void *raw){
    struct State *s=(struct State*)raw;
    int i;
    for(i=1;i<=10000;++i){
        while(__atomic_load_n(&s->ready,2)){}
        s->data=i;
        __atomic_store_n(&s->ready,1,3);
    }
    return 0;
}
int plus_three(int n){return n+3;}
int calls;
int order(){calls++;return 5;}
int *address(int *p){calls++;return p;}
int value(){calls++;return 19;}
int main(){
    signed char byte=-7;unsigned char high=250;
    short word=-12345;unsigned short wide_word=60000;
    int integer=-1234567;unsigned int unsigned_integer=4000000000U;
    long large=-123456789012345L;unsigned long high_large=0xfedcba9876543210UL;
    _Bool truth=0;
    int target=31;int *pointer=0;
    int (*callback)(int)=plus_three;
    if(__atomic_load_n(&byte,0)!=-7 || __atomic_load_n(&high,1)!=250) return 1;
    if(__atomic_load_n(&word,2)!=-12345 || __atomic_load_n(&wide_word,5)!=60000) return 2;
    if(__atomic_load_n(&integer,5)!=-1234567 || __atomic_load_n(&unsigned_integer,0)!=4000000000U) return 3;
    if(__atomic_load_n(&large,0)!=-123456789012345L || __atomic_load_n(&high_large,5)!=0xfedcba9876543210UL) return 4;
    __atomic_store_n(&byte,-9,0);__atomic_store_n(&high,251,3);
    __atomic_store_n(&word,-23456,5);__atomic_store_n(&wide_word,65000,0);
    __atomic_store_n(&integer,27,3);__atomic_store_n(&unsigned_integer,4100000000U,5);
    __atomic_store_n(&large,-9876543210123L,0);__atomic_store_n(&high_large,0x8000000000000001UL,3);
    __atomic_store_n(&truth,9,5);__atomic_store_n(&pointer,&target,3);
    if(byte!=-9 || high!=251 || word!=-23456 || wide_word!=65000 || integer!=27 || unsigned_integer!=4100000000U) return 5;
    if(large!=-9876543210123L || high_large!=0x8000000000000001UL || !__atomic_load_n(&truth,2) || *__atomic_load_n(&pointer,2)!=31) return 6;
    __atomic_store_n(address(&integer),value(),order());
    if(calls!=3 || __atomic_load_n(address(&integer),order())!=19 || calls!=5) return 7;
    if(__atomic_load_n(&pointer,2)[0]!=31 || __atomic_load_n(&callback,2)(7)!=10) return 8;
    volatile unsigned char observed=0;
    __atomic_store_n(&observed,201,5);
    if(__atomic_load_n(&observed,5)!=201) return 8;
    if(_Generic(__atomic_load_n(&high,0),unsigned char:0,default:1)) return 9;
    if(__builtin_constant_p(__atomic_load_n(&integer,0))) return 10;
    return run_publication(publish);
}
"#;
    let companion = r#"
#include <pthread.h>
struct State {int ready;int data;int error;};
static void *consume(void *raw){struct State*s=raw;for(int i=1;i<=10000;++i){while(!__atomic_load_n(&s->ready,__ATOMIC_ACQUIRE)){}if(s->data!=i)s->error=11;__atomic_store_n(&s->ready,0,__ATOMIC_RELEASE);}return 0;}
int run_publication(void*(*publish)(void*)){struct State s={0};pthread_t a,b;if(pthread_create(&a,0,publish,&s))return 12;if(pthread_create(&b,0,consume,&s))return 13;pthread_join(a,0);pthread_join(b,0);return s.error;}
"#;
    if output_option == "llvm" {
        let ir = dynamite_compiler::generate(code, output_option, false);
        assert!(ir.contains("load atomic volatile i8"));
        assert!(ir.contains("store atomic volatile i8"));
        assert!(ir.contains("seq_cst"));
    }
    assert_eq!(
        compile_and_run_with_c_flags(
            code,
            output_option,
            Some(companion),
            &[optimization, "-pthread"]
        )
        .status
        .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){return __atomic_load_n(1,0);}",
    "int main(){double x=1;return __atomic_load_n(&x,0);}",
    "int main(){__int128 x;__atomic_store_n(&x,1,0);return 0;}",
    "int main(){const int x=1;__atomic_store_n(&x,2,0);return 0;}",
    "int main(){int x;__atomic_store_n(&x,1,2);return 0;}",
    "int main(){int x;return __atomic_load_n(&x,3);}",
    "int main(){int x;return __atomic_load_n(&x,6);}",
    "int main(){int x;return __atomic_store_n(&x,1,0);}",
    "int main(){int x;__atomic_store_n(&x,1);return 0;}"
])]
fn invalid_atomic_load_store_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn atomic_read_modify_write_matches_clang(output_option: &str, optimization: &str) {
    let dispatch = r#"
unsigned long execute(int op,unsigned long *p,unsigned long n){
    switch(op){
    case 0:return __atomic_fetch_add(p,n,5);
    case 1:return __atomic_add_fetch(p,n,5);
    case 2:return __atomic_fetch_sub(p,n,5);
    case 3:return __atomic_sub_fetch(p,n,5);
    case 4:return __atomic_fetch_and(p,n,5);
    case 5:return __atomic_and_fetch(p,n,5);
    case 6:return __atomic_fetch_or(p,n,5);
    case 7:return __atomic_or_fetch(p,n,5);
    case 8:return __atomic_fetch_xor(p,n,5);
    case 9:return __atomic_xor_fetch(p,n,5);
    case 10:return __atomic_fetch_nand(p,n,5);
    case 11:return __atomic_nand_fetch(p,n,5);
    case 12:return __atomic_exchange_n(p,n,5);
    }
    return 0;
}
"#;
    let code = [dispatch, r#"
unsigned long reference(int,unsigned long*,unsigned long);
int run_workers(void*(*)(void*));
int plus_one(int n){return n+1;}
int plus_two(int n){return n+2;}
struct Count {unsigned long value;};
void *increment(void *raw){struct Count *count=(struct Count*)raw;int i;for(i=0;i<10000;++i)__atomic_fetch_add(&count->value,1UL,0);return 0;}
int calls;
int *address(int *p){calls++;return p;}
int value(){calls++;return 3;}
int order(){calls++;return 5;}
int main(){
    unsigned long seed=0x8123456789abcdefUL;
    int op,i;
    for(op=0;op<13;++op)for(i=0;i<40;++i){
        unsigned long x=seed,y=seed;
        seed=seed*6364136223846793005UL+1442695040888963407UL;
        unsigned long a=execute(op,&x,seed),b=reference(op,&y,seed);
        if(a!=b || x!=y)return 1;
    }
    signed char byte=127;
    if(__atomic_add_fetch(&byte,1,0)!=-128 || byte!=-128)return 2;
    short word=32767;
    if(__atomic_sub_fetch(&word,-1,0)!=-32768 || word!=-32768)return 3;
    int integer=2147483647;
    if(__atomic_add_fetch(&integer,1,0)!=(-2147483647-1))return 4;
    long large=9223372036854775807L;
    if(__atomic_add_fetch(&large,1L,0)!=(-9223372036854775807L-1))return 5;
    volatile unsigned char observed=255;
    if(__atomic_fetch_add(&observed,1,0)!=255 || observed!=0)return 6;
    unsigned short mask=0xff00;
    if(__atomic_nand_fetch(&mask,0x0f0f,0)!=0xf0ff || mask!=0xf0ff)return 7;
    if(__atomic_exchange_n(&byte,-9,0)!=-128 || byte!=-9)return 8;
    _Bool flag=0;
    if(__atomic_exchange_n(&flag,1,0) || !flag || !__atomic_exchange_n(&flag,0,0) || flag)return 9;
    int array[2];int *pointer=array;
    if(__atomic_fetch_add(&pointer,1L,0)!=array || (unsigned long)pointer-(unsigned long)array!=1)return 10;
    if(__atomic_sub_fetch(&pointer,1L,0)!=array)return 11;
    if(__atomic_exchange_n(&pointer,&array[1],0)!=array || pointer!=&array[1])return 12;
    integer=7;
    if(__atomic_add_fetch(address(&integer),value(),order())!=10 || calls!=3)return 13;
    if(_Generic(__atomic_fetch_add(&observed,0,0),unsigned char:0,default:1))return 14;
    __atomic_store_n(&pointer,0,0);
    if(__atomic_load_n(&pointer,0) || __atomic_exchange_n(&pointer,array,0))return 16;
    int (*callback)(int)=plus_one;
    if(__atomic_exchange_n(&callback,plus_two,0)(5)!=6 || callback(5)!=7)return 17;
    __atomic_store_n(&callback,plus_one,0);
    if(__atomic_load_n(&callback,0)(5)!=6)return 18;
    return run_workers(increment);
}
"#].concat();
    let companion = [dispatch.replace("execute(", "reference("), r#"
#include <pthread.h>
struct Count{unsigned long value;};
static void *increment(void *raw){struct Count *c=raw;for(int i=0;i<10000;++i)__atomic_fetch_add(&c->value,1UL,__ATOMIC_RELAXED);return 0;}
int run_workers(void*(*generated)(void*)){struct Count c={0};pthread_t t[4];for(int i=0;i<4;++i)if(pthread_create(&t[i],0,i<2?generated:increment,&c))return 15;for(int i=0;i<4;++i)pthread_join(t[i],0);return __atomic_load_n(&c.value,__ATOMIC_SEQ_CST)!=40000;}
"#.into()].concat();
    if output_option == "llvm" {
        let ir = dynamite_compiler::generate(&code, output_option, false);
        assert!(ir.contains("atomicrmw volatile add"));
        assert!(ir.contains("atomicrmw nand"));
        assert!(ir.contains("ptrtoint"));
    }
    assert_eq!(
        compile_and_run_with_c_flags(
            &code,
            output_option,
            Some(&companion),
            &[optimization, "-pthread"]
        )
        .status
        .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){_Bool x=0;return __atomic_fetch_add(&x,1,0);}",
    "int main(){const int x=1;return __atomic_exchange_n(&x,2,0);}",
    "int main(){int x;return __atomic_or_fetch(&x,1,7);}",
    "int main(){double x;return __atomic_fetch_xor(&x,1,0);}",
    "int main(){int x;return __atomic_add_fetch(&x,1);}"
])]
fn invalid_atomic_read_modify_write_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn byte_swap_builtins_preserve_widths_constants_and_single_evaluation(output_option: &str) {
    let code = r#"
_Static_assert(__builtin_bswap16(0x1234)==0x3412,"constant swap");
unsigned int constant=__builtin_bswap32(0x12345678);
unsigned long wide_constant=__builtin_bswap64(0x0123456789abcdefULL);
int calls;
unsigned long next(){calls++;return 0x8123456789abcdefULL;}
unsigned int swap(unsigned int value){return __builtin_bswap32(value);}
int main(){
    unsigned short narrow=0x1234;
    unsigned int word=0x89abcdef;
    unsigned long wide=__builtin_bswap64(next());
    unsigned short result=__builtin_bswap16(narrow++);
    volatile unsigned int source=0x12345678;
    return (constant!=0x78563412)+(wide_constant!=0xefcdab8967452301ULL)
        +(wide!=0xefcdab8967452381ULL)+(calls!=1)+(narrow!=0x1235)+(result!=0x3412)
        +(swap(word)!=0xefcdab89)+(__builtin_bswap32(source)!=constant)
        +(__builtin_bswap16(-2)!=0xfeff)
        +_Generic(__builtin_bswap16(1),unsigned short:0,default:1)
        +_Generic(__builtin_bswap32(1),unsigned int:0,default:1)
        +_Generic(__builtin_bswap64(1),unsigned long long:0,default:1);
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){return __builtin_bswap16();}",
    "int main(){return __builtin_bswap32(1,2);}",
    "int main(){int x;return __builtin_bswap64(&x);}",
    "struct S{int x;};int main(){struct S x;return __builtin_bswap32(x);}",
    "int x;int swapped=__builtin_bswap32(x);",
])]
fn invalid_byte_swap_operands_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn byte_swap_results_match_clang_for_runtime_values(output_option: &str) {
    let code = r#"
unsigned long reference64(unsigned long value);
unsigned int reference32(unsigned int value);
unsigned short reference16(unsigned short value);
int main(){
    unsigned long value=0x76543210fedcba98ULL;
    int i=0;
    while(i<200){
        if(__builtin_bswap64(value)!=reference64(value))return 1;
        if(__builtin_bswap32(value)!=reference32(value))return 2;
        if(__builtin_bswap16(value)!=reference16(value))return 3;
        value=value*6364136223846793005ULL+1;
        i++;
    }
    float fraction=1.75f;
    return __builtin_bswap32(fraction)!=0x01000000;
}"#;
    let companion = "unsigned long reference64(unsigned long x){return __builtin_bswap64(x);}unsigned int reference32(unsigned int x){return __builtin_bswap32(x);}unsigned short reference16(unsigned short x){return __builtin_bswap16(x);}";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn builtin_types_compatible_queries_match_clang(output_option: &str, optimization: &str) {
    let definitions = r"
typedef const int Const;typedef int Plain;
typedef int Fn(int);typedef int QualifiedFn(const int);
typedef const int QualifiedResult(void);typedef int PlainResult(void);
typedef int Adjusted(const int);typedef int Adjusted(int);
typedef int OtherFn(long);typedef int VariadicFn(int,...);
typedef int ArrayFn(int[3]);typedef int PointerFn(int*);
enum First:int{FIRST=3};enum Second:int{SECOND=5};
struct Record{long value;};struct Other{long value;};struct Opaque;
union Mixed{int x;float y;};
";
    let pairs = [
        ("int", "const volatile int", 1),
        ("Const", "Plain", 1),
        ("char", "signed char", 0),
        ("char", "unsigned char", 0),
        ("_Bool", "unsigned char", 0),
        ("long", "long long", 0),
        ("unsigned long", "unsigned long long", 0),
        ("int*const", "int*", 1),
        ("const int*", "int*", 0),
        ("int*restrict", "int*", 1),
        ("int[]", "int[3]", 1),
        ("int[2]", "int[3]", 0),
        ("const int[2]", "int[2]", 1),
        ("const int[2][3]", "int[2][3]", 1),
        ("int*const[2]", "int*[2]", 1),
        ("const int*[2]", "int*[2]", 0),
        ("int(*)[2]", "const int(*)[2]", 0),
        ("int(*)[]", "int(*)[2]", 1),
        ("int[2][3]", "int[][3]", 1),
        ("int[2][3]", "int[][4]", 0),
        ("Fn", "QualifiedFn", 1),
        ("QualifiedResult", "PlainResult", 0),
        ("Adjusted", "Fn", 1),
        ("Fn", "OtherFn", 0),
        ("Fn", "VariadicFn", 0),
        ("ArrayFn", "PointerFn", 1),
        ("Fn*", "QualifiedFn*", 1),
        ("enum First", "enum First", 1),
        ("enum First", "enum Second", 0),
        ("enum First", "int", 1),
        ("struct Record", "struct Record", 1),
        ("struct Record", "struct Other", 0),
        ("struct Opaque", "struct Opaque", 1),
        ("union Mixed", "struct Record", 0),
        ("void", "void", 1),
        ("void", "int", 0),
        ("__int128", "unsigned __int128", 0),
    ];
    let mut code = String::from(
        "#if !__has_builtin(__builtin_types_compatible_p)\n#error missing type query\n#endif\n",
    );
    code.push_str(definitions);
    let mut companion = String::from(definitions);
    for (index, (left, right, expected)) in pairs.iter().enumerate() {
        code.push_str(&format!("_Static_assert(__builtin_types_compatible_p({left},{right})=={expected},\"type pair\");int reference{index}(void);\n"));
        companion.push_str(&format!(
            "int reference{index}(void){{return __builtin_types_compatible_p({left},{right});}}\n"
        ));
    }
    code.push_str("int main(){int bad=0;");
    for (index, (left, right, _)) in pairs.iter().enumerate() {
        code.push_str(&format!(
            "bad+=(__builtin_types_compatible_p({left},{right})!=reference{index}());"
        ));
    }
    code.push_str("return bad;}");
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn builtin_types_compatible_queries_do_not_evaluate_bounds(output_option: &str) {
    let code = r#"
int calls;int side_effect(void){calls++;return 1;}
extern int values[];int values[3]={3,5,7};extern int values[];
int main(){int n=2;int array[4];
    _Static_assert(__builtin_types_compatible_p(int[n++],int[7]),"VLA bounds ignored");
    if(!__builtin_types_compatible_p(__typeof__(side_effect()),int))return 1;
    if(__builtin_types_compatible_p(__typeof__(array),int*))return 2;
    if(!__builtin_types_compatible_p(__typeof__(array),int[4]))return 3;
    int (*pointer)[]=&values;
    return n!=2 || calls!=0 || sizeof(values)!=12 || (*pointer)[2]!=7
        || _Generic(__builtin_types_compatible_p(int,int),int:0,default:1);
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn object_size_bounds_match_clang(output_option: &str, optimization: &str) {
    let shared = r#"
struct Buffers{char first[10];int count;char last[10];};
#define DEFINE_SIZES(name) \
int name(int choice){ \
    char small[10];char large[20];int grid[3][4];struct Buffers object; \
    return __builtin_object_size(small,0)!=10 \
        || __builtin_object_size(small+3,1)!=7 \
        || __builtin_object_size(2+small,2)!=8 \
        || __builtin_object_size((small+6)-2,3)!=6 \
        || __builtin_object_size(small+10,0)!=0 \
        || __builtin_object_size(small+11,0)!=0 \
        || __builtin_object_size(&object.first[1],0)!=sizeof(object)-1 \
        || __builtin_object_size(&object.first[1],1)!=9 \
        || __builtin_object_size(&object.count,0)!=sizeof(object)-__builtin_offsetof(struct Buffers,count) \
        || __builtin_object_size(&object.count,1)!=sizeof(int) \
        || __builtin_object_size(&object.last[2],0)!=sizeof(object)-__builtin_offsetof(struct Buffers,last)-2 \
        || __builtin_object_size(&object.last[2],3)!=8 \
        || __builtin_object_size(grid[1],0)!=32 \
        || __builtin_object_size(grid[1],1)!=16 \
        || __builtin_object_size(&grid[1],1)!=32 \
        || __builtin_object_size(&grid[1][1],1)!=12 \
        || __builtin_object_size("abc",0)!=4 \
        || __builtin_object_size(u"abc",0)!=8 \
        || __builtin_object_size((char*)small,0)!=10 \
        || __builtin_object_size(choice?(char*)small:(char*)large,0)!=20 \
        || __builtin_object_size(choice?(char*)small:(char*)large,2)!=10; \
}
"#;
    let code = [
        shared,
        r#"
char global[5];
_Static_assert(__builtin_object_size(global,0)==5,"global bound");
DEFINE_SIZES(local_sizes)
int foreign_sizes(int);
int main(){return local_sizes(0) || local_sizes(1) || foreign_sizes(0) || foreign_sizes(1)
    || _Generic(__builtin_object_size(global,0),unsigned long:0,default:1);}
"#,
    ]
    .concat();
    let companion = [shared, "DEFINE_SIZES(foreign_sizes)"].concat();
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"])]
fn object_size_uses_unknown_bounds_without_evaluating_effects(output_option: &str) {
    let code = r#"
char storage[20];int calls;
char *next(){calls++;return storage;}
unsigned long unknown(char *pointer,int n){
    char dynamic[n];
    return __builtin_object_size(pointer,0)!=(unsigned long)-1
        || __builtin_object_size(pointer,1)!=(unsigned long)-1
        || __builtin_object_size(pointer,2)!=0 || __builtin_object_size(pointer,3)!=0
        || __builtin_object_size(dynamic,0)!=(unsigned long)-1;
}
int main(){
    char buffer[10];char *pointer=buffer;int n=2;
    unsigned long a=__builtin_object_size(next(),0);
    unsigned long b=__builtin_object_size(pointer++,1);
    unsigned long c=__builtin_object_size((char(*)[n++])buffer,0);
    volatile int choice=1;
    unsigned long d=__builtin_object_size(choice?buffer:storage,0);
    volatile char bytes[3];
    return a!=(unsigned long)-1 || b!=(unsigned long)-1 || c!=(unsigned long)-1
        || d!=(unsigned long)-1 || pointer!=buffer || calls!=0 || n!=2
        || __builtin_object_size((char*)(int)(unsigned long)buffer,0)!=(unsigned long)-1
        || __builtin_object_size((void*)0,0)!=(unsigned long)-1
        || __builtin_object_size(bytes,0)!=3 || __builtin_object_size(&bytes,0)!=3
        || unknown(buffer,4);
}
"#;
    let result = compile_and_run(code, output_option);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "int main(){char a[2];return __builtin_object_size(a);}",
    "int main(){char a[2];return __builtin_object_size(a,0,1);}",
    "int main(){char a[2];int n=0;return __builtin_object_size(a,n);}",
    "int main(){char a[2];return __builtin_object_size(a,4);}",
    "int main(){char a[2];return __builtin_object_size(a,-1);}",
    "int main(){char a[2];return __builtin_object_size(a,0.0);}",
    "struct S{int x;};int main(){struct S s;return __builtin_object_size(s,0);}",
])]
fn invalid_object_size_queries_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn checked_memory_builtins_preserve_results_and_single_evaluation(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
char destination[16];const char source[]="abcdef";
int destinations,sources,counts,bounds,fills;
char *next_destination(){destinations++;return destination;}
const char *next_source(){sources++;return source;}
unsigned long count(unsigned long n){counts++;return n;}
unsigned long bound(unsigned long n){bounds++;return n;}
int fill(){fills++;return 256+'X';}
int main(){
    void *copied=__builtin___memcpy_chk(next_destination(),next_source(),count(7),bound(__builtin_object_size(destination,0)));
    void *moved=__builtin___memmove_chk(destination+1,destination,count(6),bound(15));
    void *set=__builtin___memset_chk(destination+2,fill(),count(2),bound(14));
    if(__builtin___memcpy_chk(destination,source,0,0)!=destination)return 1;
    if(__builtin___memmove_chk(destination,destination,0,0)!=destination)return 2;
    if(__builtin___memset_chk(destination,0,0,0)!=destination)return 3;
    if(__builtin___memcpy_chk(destination+8,source,7,(unsigned long)-1)!=destination+8)return 4;
    return copied!=destination || moved!=destination+1 || set!=destination+2
        || destination[0]!='a' || destination[1]!='a' || destination[2]!='X' || destination[3]!='X'
        || destination[4]!='d' || destination[6]!='f' || destination[7]!=0 || destination[14]!=0
        || destinations!=1 || sources!=1 || counts!=3 || bounds!=3 || fills!=1
        || __builtin_object_size(next_destination(),0)!=(unsigned long)-1 || destinations!=1
        || _Generic(__builtin___memcpy_chk(destination,source,0,16),void*:0,default:1);
}
"#;
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["__builtin___memcpy_chk", "__builtin___memmove_chk", "__builtin___memset_chk"], ["asm", "llvm"], ["-O0", "-O2"])]
fn checked_memory_builtins_stop_before_overflow(
    builtin: &str,
    output_option: &str,
    optimization: &str,
) {
    let second = if builtin == "__builtin___memset_chk" {
        "0"
    } else {
        "source"
    };
    let code = format!(
        r#"
void install_failure_handler(char *);unsigned long read_count(void);
int main(){{char destination[4]={{55,55,55,55}};char source[8]={{0}};
    install_failure_handler(destination);{builtin}(destination,{second},read_count(),__builtin_object_size(destination,0));return 99;}}
"#
    );
    let companion = r#"
#include <signal.h>
#include <stdlib.h>
static volatile unsigned char *watched;
// Checked libc operations may abort or issue a hardware trap.
static void on_failure(int number){
    (void)number;
    _Exit(watched[0]==55 && watched[1]==55 && watched[2]==55 && watched[3]==55 ? 73 : 74);
}
void install_failure_handler(char *value){watched=(unsigned char *)value;signal(SIGABRT,on_failure);signal(SIGTRAP,on_failure);signal(SIGILL,on_failure);}
unsigned long read_count(void){return 5;}
"#;
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(73), "{:?}", result);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn checked_integer_addition_matches_clang(output_option: &str, optimization: &str) {
    assert_checked_integer_overflow_matches_clang(
        "__builtin_add_overflow",
        output_option,
        optimization,
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn checked_integer_subtraction_matches_clang(output_option: &str, optimization: &str) {
    assert_checked_integer_overflow_matches_clang(
        "__builtin_sub_overflow",
        output_option,
        optimization,
    );
}

fn assert_checked_integer_overflow_matches_clang(
    builtin: &str,
    output_option: &str,
    optimization: &str,
) {
    let mut code = format!(
        "int global;\n_Static_assert(sizeof({builtin}(1,2,&global))==sizeof(_Bool),\"result size\");\n_Static_assert(_Generic({builtin}(1,2,&global),_Bool:1,default:0),\"result type\");\n",
    );
    let mut companion = String::new();
    let mut calls = String::new();
    let targets = [
        ("i8", "signed char"),
        ("u8", "unsigned char"),
        ("i16", "short"),
        ("u16", "unsigned short"),
        ("i32", "int"),
        ("u32", "unsigned int"),
        ("i64", "long"),
        ("u64", "unsigned long"),
    ];
    let sources = [
        ("ss", "long", "long"),
        ("su", "long", "unsigned long"),
        ("us", "unsigned long", "long"),
        ("uu", "unsigned long", "unsigned long"),
    ];
    let mut cases = Vec::new();
    for (target_tag, target) in targets {
        for (source_tag, left, right) in sources {
            cases.push((format!("{target_tag}_{source_tag}"), target, left, right));
        }
    }
    cases.extend([
        ("narrow8".into(), "int", "signed char", "unsigned char"),
        (
            "narrow16".into(),
            "unsigned long",
            "unsigned short",
            "short",
        ),
        ("narrow32".into(), "long", "int", "unsigned int"),
        ("boolean".into(), "unsigned char", "_Bool", "_Bool"),
    ]);
    for (index, (tag, target, left, right)) in cases.iter().enumerate() {
        code.push_str(&format!(r#"
int reference_{tag}(unsigned long,unsigned long,{target} *);
int check_{tag}(unsigned long a,unsigned long b){{
    {target} actual=3,expected=4;
    int got={builtin}(({left})a,({right})b,&actual);
    int want=reference_{tag}(a,b,&expected);
    if(got!=want || actual!=expected){{
        printf("{tag}: %lu %lu flags %d %d values %lu %lu\n",a,b,got,want,(unsigned long)actual,(unsigned long)expected);return 1;
    }}
    return 0;
}}
"#));
        companion.push_str(&format!("int reference_{tag}(unsigned long a,unsigned long b,{target} *out){{return {builtin}(({left})a,({right})b,out);}}\n"));
        calls.push_str(&format!("if(check_{tag}(a,b))return {};\n", index + 1));
    }
    code.push_str(&format!(r#"
int check_all(unsigned long a,unsigned long b){{{calls}return 0;}}
unsigned long state=42;
unsigned long next(){{state=state*6364136223846793005ul+1;return state;}}
int main(){{
    unsigned long edges[]={{0,1,2,127,128,255,256,2147483647,0x80000000u,0x7ffffffffffffffful,0x8000000000000000ul,0xfffffffffffffffful}};
    for(unsigned long i=0;i<sizeof(edges)/sizeof(edges[0]);i++){{
        for(unsigned long j=0;j<sizeof(edges)/sizeof(edges[0]);j++){{
            int result=check_all(edges[i],edges[j]);if(result)return result;
        }}
    }}
    for(int i=0;i<100;i++){{unsigned long a=next(),b=next();int result=check_all(a,b);if(result)return result;}}
    return global!=0;
}}
"#));
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"])]
fn checked_integer_subtraction_preserves_effects_and_aliases(output_option: &str) {
    let code = r#"
int left_calls,right_calls,pointer_calls;volatile unsigned char output;
long left(void){left_calls++;return -1;}unsigned long right(void){right_calls++;return 0xffffffffffffffffUL;}
volatile unsigned char *destination(void){pointer_calls++;return &output;}
_Bool bounded(unsigned long a,unsigned long b,unsigned char *out){return __builtin_sub_overflow(a,b,out);}
int main(void){
    _Bool overflow=__builtin_sub_overflow(left(),right(),destination());
    if(!overflow || output || left_calls!=1 || right_calls!=1 || pointer_calls!=1)return 1;
    long value=7;if(__builtin_sub_overflow(value,9,&value) || value!=-2)return 2;
    unsigned long wide=0;if(!__builtin_sub_overflow(0,1,&wide) || wide!=0xffffffffffffffffUL)return 3;
    unsigned char array[1]={0};if(!__builtin_sub_overflow(3,9,array) || array[0]!=250)return 4;
    int inner=0,outer=0;if(__builtin_sub_overflow(__builtin_sub_overflow(1,2,&inner),4,&outer) || inner!=-1 || outer!=-4)return 5;
    if(!bounded(1,2,array) || array[0]!=255)return 6;
    if(sizeof(__builtin_sub_overflow(left(),right(),destination()))!=sizeof(_Bool) || left_calls!=1 || right_calls!=1 || pointer_calls!=1)return 7;
    int untouched=19;if(0 && __builtin_sub_overflow(1,2,&untouched))return 8;
    return untouched!=19;
}"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn checked_integer_subtraction_preserves_volatile_result_store() {
    let ir = dynamite_compiler::generate(
        "_Bool difference(long a,unsigned long b,volatile int *out){return __builtin_sub_overflow(a,b,out);}",
        "llvm",
        false,
    );
    assert_eq!(ir.matches("store volatile i32").count(), 1);
    assert_eq!(ir.matches("load volatile").count(), 0);
}

#[test_matrix([
    "int main(){int r;return __builtin_sub_overflow(1,2);}",
    "int main(){int r;return __builtin_sub_overflow(1,2,&r,3);}",
    "int main(){int r;return __builtin_sub_overflow(1.0,2,&r);}",
    "int main(){int r;return __builtin_sub_overflow(&r,2,&r);}",
    "int main(){_Bool r;return __builtin_sub_overflow(1,2,&r);}",
    "int main(){const int r=0;return __builtin_sub_overflow(1,2,&r);}",
    "int main(){return __builtin_sub_overflow(1,2,0);}",
    "int main(){double r;return __builtin_sub_overflow(1,2,&r);}",
])]
fn checked_integer_subtraction_rejects_invalid_arguments(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn checked_integer_addition_preserves_effects_and_aliases(output_option: &str) {
    let code = r#"
int left_calls,right_calls,pointer_calls;
volatile int output;
long left(){left_calls++;return -1;}
unsigned long right(){right_calls++;return 0;}
volatile int *destination(){pointer_calls++;return &output;}
_Bool bounded(unsigned long a,unsigned long b,unsigned char *out){return __builtin_add_overflow(a,b,out);}
int main(){
    _Bool overflow=__builtin_add_overflow(left(),right(),destination());
    long value=7;_Bool alias=__builtin_add_overflow(value,2,&value);
    unsigned long wide=5;_Bool wrapped=__builtin_add_overflow(0xfffffffffffffffful,1,&wide);
    unsigned char array[1]={0};_Bool narrow=__builtin_add_overflow(250,10,array);
    int prediction_result=0;long carried=__builtin_expect(9,__builtin_add_overflow(1,2,&prediction_result));
    int inner=0,outer=0;_Bool nested=__builtin_add_overflow(__builtin_add_overflow(1,2,&inner),4,&outer);
    unsigned char returned;_Bool returned_flag=bounded(250,10,&returned);
    return carried!=9 || prediction_result!=3 || nested || inner!=3 || outer!=4 || !returned_flag || returned!=4
        || overflow || output!=-1 || left_calls!=1 || right_calls!=1 || pointer_calls!=1
        || alias || value!=9 || !wrapped || wide!=0 || !narrow || array[0]!=4;
}
"#;
    let result = compile_and_run(code, output_option);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test]
fn checked_integer_addition_emits_one_volatile_destination_store() {
    let code = "_Bool add(long a,unsigned long b,volatile int *out){return __builtin_add_overflow(a,b,out);}";
    let ir = dynamite_compiler::generate(code, "llvm", false);
    assert_eq!(ir.matches("store volatile i32").count(), 1, "{ir}");
    assert!(!ir.contains("load volatile i32"), "{ir}");
}

#[test_matrix([
    "int main(){int r;return __builtin_add_overflow(1,2);}",
    "int main(){int r;return __builtin_add_overflow(1,2,&r,3);}",
    "int main(){int r;return __builtin_add_overflow(1.0,2,&r);}",
    "int main(){int r;return __builtin_add_overflow(&r,2,&r);}",
    "int main(){_Bool r;return __builtin_add_overflow(1,2,&r);}",
    "int main(){const int r=0;return __builtin_add_overflow(1,2,&r);}",
    "int main(){return __builtin_add_overflow(1,2,0);}",
    "int main(){double r;return __builtin_add_overflow(1,2,&r);}",
])]
fn invalid_checked_integer_addition_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")]) ]
fn checked_formatted_output_preserves_results_and_effects(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
#include <string.h>
char output[32];int destinations,sizes,flags,bounds,formats,texts,integers,floats;
char *destination(){destinations++;return output;}
unsigned long size(){sizes++;return 32;}
int flag(){flags++;return 0;}
unsigned long bound(unsigned long n){bounds++;return n;}
const char *format(){formats++;return "%s:%d:%.1f";}
const char *text(){texts++;return "ok";}
int integer(){integers++;return 7;}
double floating(){floats++;return 2.5;}
int main(){
    int n=__builtin___snprintf_chk(destination(),size(),flag(),bound(__builtin_object_size(output,1)),format(),text(),integer(),floating());
    if(n!=8 || strcmp(output,"ok:7:2.5"))return 1;
    if(destinations!=1 || sizes!=1 || flags!=1 || bounds!=1 || formats!=1 || texts!=1 || integers!=1 || floats!=1)return 2;
    n=__builtin___sprintf_chk(output,1,32,"%s/%ld/%.1f","done",-9l,1.5);
    if(n!=11 || strcmp(output,"done/-9/1.5"))return 3;
    char tiny[4];n=__builtin___snprintf_chk(tiny,4,0,__builtin_object_size(tiny,1),"%s","abcdef");
    if(n!=6 || strcmp(tiny,"abc"))return 4;
    if(__builtin___snprintf_chk((char*)0,0,0,0,"%s","hello")!=5)return 5;
    return _Generic(__builtin___snprintf_chk(output,32,0,32,"x"),int:0,default:1);
}
"#;
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([false,true], ["asm", "llvm"], ["-O0", "-O2"])]
fn checked_formatted_output_rejects_capacity_violations(
    bounded: bool,
    output_option: &str,
    optimization: &str,
) {
    let call = if bounded {
        "__builtin___snprintf_chk(storage,read_size(),0,4,\"%s\",\"x\")"
    } else {
        "__builtin___sprintf_chk(storage,0,4,\"%s\",\"abcdefgh\")"
    };
    let code = format!(
        r#"
void install_failure_handler(char *);unsigned long read_size(void);
int main(){{char storage[12]={{0,0,0,0,55,55,55,55,0,0,0,0}};
    install_failure_handler(storage+4);{call};return 99;}}
"#
    );
    let companion = r#"
#include <signal.h>
#include <stdlib.h>
static volatile unsigned char *watched;
static void on_failure(int number){
    (void)number;_Exit(watched[0]==55 && watched[1]==55 && watched[2]==55 && watched[3]==55 ? 73 : 74);
}
void install_failure_handler(char *value){watched=(unsigned char*)value;signal(SIGABRT,on_failure);signal(SIGTRAP,on_failure);signal(SIGILL,on_failure);}
unsigned long read_size(void){return 5;}
"#;
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(73), "{:?}", result);
}

#[test_matrix([
    "int main(){char a[4];return __builtin___sprintf_chk(a,0,4);}",
    "int main(){char a[4];return __builtin___snprintf_chk(a,4,0,4);}",
    "int main(){const char a[4]={0};return __builtin___sprintf_chk(a,0,4,\"x\");}",
    "int main(){char a[4];return __builtin___snprintf_chk(a,a,0,4,\"x\");}",
    "int main(){char a[4];return __builtin___snprintf_chk(a,4,a,4,\"x\");}",
    "int main(){char a[4];return __builtin___sprintf_chk(a,0,4,1);}",
    "int main(){void *p=__builtin___snprintf_chk;}",
])]
fn invalid_checked_formatted_output_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn checked_bsd_string_output_preserves_results_and_effects(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
#include <string.h>
char buffer[8];int destinations,sources,sizes,capacities;
char *destination(){destinations++;return buffer;}
const char *source(){sources++;return "abcdefghi";}
unsigned long size(){sizes++;return 8;}
unsigned long capacity(){capacities++;return 8;}
int main(){
    unsigned long n=__builtin___strlcpy_chk(destination(),source(),size(),capacity());
    if(n!=9 || strcmp(buffer,"abcdefg"))return 1;
    if(destinations!=1 || sources!=1 || sizes!=1 || capacities!=1)return 2;
    if(__builtin___strlcpy_chk(buffer,"ab",8,__builtin_object_size(buffer,1))!=2)return 3;
    if(__builtin___strlcat_chk(buffer,"cdefghij",8,8)!=10 || strcmp(buffer,"abcdefg"))return 4;
    if(__builtin___strlcpy_chk((char*)0,"hello",0,0)!=5)return 5;
    if(__builtin___strlcat_chk((char*)0,"hello",0,0)!=5)return 6;
    char raw[4]={'a','b','c','d'};
    if(__builtin___strlcat_chk(raw,"xy",4,4)!=6 || memcmp(raw,"abcd",4))return 7;
    if(__builtin___strlcpy_chk(buffer,"hi",8,(unsigned long)-1)!=2 || strcmp(buffer,"hi"))return 8;
    buffer[0]=0;destinations=0;sources=0;sizes=0;capacities=0;
    if(__builtin___strlcat_chk(destination(),source(),size(),capacity())!=9 || strcmp(buffer,"abcdefg"))return 9;
    if(destinations!=1 || sources!=1 || sizes!=1 || capacities!=1)return 10;
    return _Generic(__builtin___strlcat_chk(buffer,"",8,8),unsigned long:0,default:1);
}
"#;
    let result = compile_and_run_with_c_flags(
        code,
        output_option,
        Some(CHECKED_BSD_STRING_RUNTIME),
        &[optimization],
    );
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["strlcpy", "strlcat"], ["asm", "llvm"], ["-O0", "-O2"])]
fn checked_bsd_string_output_rejects_capacity_violations(
    operation: &str,
    output_option: &str,
    optimization: &str,
) {
    let code = format!(
        r#"
void install_failure_handler(char *);unsigned long excessive_size(void);
int main(){{char storage[8]={{55,55,55,55,55,55,55,55}};
install_failure_handler(storage);__builtin___{operation}_chk(storage,"",excessive_size(),4);return 99;}}
"#
    );
    let result = compile_and_run_with_c_flags(
        &code,
        output_option,
        Some(CHECKED_BSD_STRING_RUNTIME),
        &[optimization],
    );
    assert_eq!(result.status.code(), Some(73), "{:?}", result);
}

#[test_matrix([
    "int main(){char a[4];return __builtin___strlcpy_chk(a,\"x\",4);}",
    "int main(){char a[4];return __builtin___strlcat_chk(a,\"x\",4,4,4);}",
    "int main(){const char a[4]={0};return __builtin___strlcpy_chk(a,\"x\",4,4);}",
    "int main(){char a[4];return __builtin___strlcat_chk(a,1,4,4);}",
    "int main(){char a[4];return __builtin___strlcpy_chk(a,\"x\",a,4);}",
    "int main(){char a[4];return __builtin___strlcat_chk(a,\"x\",4,a);}",
    "int main(){void *p=__builtin___strlcpy_chk;}",
])]
fn invalid_checked_bsd_string_output_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn floating_absolute_builtins_preserve_ieee_bits_and_effects(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
unsigned int reference32(unsigned int);unsigned long reference64(unsigned long);
union Bits32{unsigned int bits;float value;};union Bits64{unsigned long bits;double value;};
unsigned int absolute32(unsigned int bits){union Bits32 v;v.bits=bits;v.value=__builtin_fabsf(v.value);return v.bits;}
unsigned long absolute64(unsigned long bits){union Bits64 v;v.bits=bits;v.value=__builtin_fabs(v.value);return v.bits;}
float initial32=__builtin_fabsf(-2.25);double initial64=__builtin_fabs(-3.5);
int calls;float next32(){calls++;return -2.5f;}double next64(){calls++;return -7.25;}
int main(){
unsigned int bits32[10]={0,0x80000000u,0x3f800000u,0xbf800000u,0x7f800000u,0xff800000u,0x7fc12345u,0xffc12345u,0x7f812345u,0xff812345u};
unsigned long bits64[10]={0,0x8000000000000000ul,0x3ff0000000000000ul,0xbff0000000000000ul,0x7ff0000000000000ul,0xfff0000000000000ul,0x7ff8123456789abcul,0xfff8123456789abcul,0x7ff0123456789abcul,0xfff0123456789abcul};
for(int i=0;i<10;i++)if(absolute32(bits32[i])!=reference32(bits32[i]) || absolute64(bits64[i])!=reference64(bits64[i]))return i+1;
if(initial32!=2.25f || initial64!=3.5)return 11;
if(__builtin_fabsf(next32())!=2.5f || __builtin_fabs(next64())!=7.25 || calls!=2)return 12;
if(sizeof(__builtin_fabs(next64()))!=sizeof(double) || calls!=2)return 13;
if(__builtin_fabs(-7)!=7.0 || __builtin_fabsf(-3.25)!=3.25f)return 14;
return _Generic(__builtin_fabsf(1),float:0,default:1) || _Generic(__builtin_fabs(1),double:0,default:1);
}
"#;
    let companion = r#"
#include <string.h>
unsigned int reference32(unsigned int bits){float value;memcpy(&value,&bits,4);value=__builtin_fabsf(value);memcpy(&bits,&value,4);return bits;}
unsigned long reference64(unsigned long bits){double value;memcpy(&value,&bits,8);value=__builtin_fabs(value);memcpy(&bits,&value,8);return bits;}
"#;
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test]
fn floating_absolute_builtins_read_volatile_operands_once() {
    let code = "float single(volatile float *p){return __builtin_fabsf(*p);}double wide(volatile double *p){return __builtin_fabs(*p);}";
    let ir = dynamite_compiler::generate(code, "llvm", false);
    assert_eq!(ir.matches("load volatile float").count(), 1, "{ir}");
    assert_eq!(ir.matches("load volatile double").count(), 1, "{ir}");
    assert!(!ir.contains("@fabs"), "{ir}");
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn floating_absolute_builtins_preserve_binary64_long_double(
    (output_option, optimization): (&str, &str),
) {
    let code = "long double initial=__builtin_fabsl(-2.5L);long double absolute(long double v){return __builtin_fabsl(v);}int main(){return initial!=2.5L || absolute(-3.25L)!=3.25L || _Generic(__builtin_fabsl(1),long double:0,default:1);}";
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "int main(){return __builtin_fabs();}",
    "int main(){return __builtin_fabsf(1,2);}",
    "int main(){int value;return __builtin_fabs(&value);}",
    "int main(){void *p=__builtin_fabs;}",
    "void action(){}int main(){return __builtin_fabsf(action());}",
])]
fn invalid_floating_absolute_builtins_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[cfg(not(any(
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_os = "macos")
)))]
#[test_matrix(["asm", "llvm"])]
fn floating_absolute_builtins_reject_unsupported_long_double(output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(
            "double absolute(double value){return __builtin_fabsl(value);}",
            output_option,
            false,
        ))
        .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn floating_nan_builtins_preserve_payloads_and_runtime_calls(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
union D{double f;unsigned long u;}; union F{float f;unsigned int u;};
double global=__builtin_nan(("0x" "123"));float small=__builtin_nanf("0123");
unsigned long reference64(int);unsigned int reference32(int);int check_runtime(double,float);
int calls;const char *payload(void){calls++;return "0x321";}
int main(void){
    union D d={global};union F f={small};
    if(d.u!=0x7ff8000000000123UL || f.u!=0x7fc00053U)return 1;
    double values[]={__builtin_nan(""),__builtin_nan("123"),__builtin_nan("0123"),__builtin_nan("0x123"),__builtin_nan("bad"),__builtin_nan("-1"),__builtin_nan("18446744073709551616"),__builtin_nan("0xffffffffffffffffffffffff"),__builtin_nan("123x"),__builtin_nan(" 12"),__builtin_nan("12\0tail")};
    float floats[]={__builtin_nanf(""),__builtin_nanf("123"),__builtin_nanf("0123"),__builtin_nanf("0x123"),__builtin_nanf("bad"),__builtin_nanf("-1"),__builtin_nanf("18446744073709551616"),__builtin_nanf("0xffffffffffffffffffffffff"),__builtin_nanf("123x"),__builtin_nanf(" 12"),__builtin_nanf("12\0tail")};
    for(int i=0;i<11;++i){d.f=values[i];f.f=floats[i];if(d.u!=reference64(i) || f.u!=reference32(i))return 2;if(d.f==d.f || f.f==f.f)return 3;}
    if(_Generic(__builtin_nan(""),double:0,default:1) || _Generic(__builtin_nanf(""),float:0,default:1))return 4;
    if(check_runtime(__builtin_nan(payload()),__builtin_nanf(payload())) || calls!=2)return 5;
    const char *dynamic="0x321";union D dynamic_value={__builtin_nan(dynamic)};if(dynamic_value.f==dynamic_value.f)return 6;return 0;
}"#;
    let companion = r#"
#include <math.h>
#include <string.h>
#define VALUES(B) B(""),B("123"),B("0123"),B("0x123"),B("bad"),B("-1"),B("18446744073709551616"),B("0xffffffffffffffffffffffff"),B("123x"),B(" 12"),B("12\0tail")
unsigned long reference64(int i){double values[]={VALUES(__builtin_nan)};unsigned long bits;memcpy(&bits,&values[i],8);return bits;}
unsigned int reference32(int i){float values[]={VALUES(__builtin_nanf)};unsigned int bits;memcpy(&bits,&values[i],4);return bits;}
int check_runtime(double d,float f){return !isnan(d) || !isnan(f);}
"#;
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization, "-lm"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn floating_nan_builtins_support_binary64_long_double((output_option, optimization): (&str, &str)) {
    let code = r#"
long double stored=__builtin_nanl("0x123");
const char *payload(void){return "0x321";}
int main(void){union Bits{long double f;unsigned long u;};union Bits v={stored};if(v.u!=0x7ff8000000000123UL)return 1;v.f=__builtin_nanl(payload());return v.f==v.f || _Generic(__builtin_nanl(""),long double:0,default:1);}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){__builtin_nan();return 0;}",
    "int main(){double (*p)(const char *)=__builtin_nan;return 0;}",
    "int main(){__builtin_nanf(1,2);return 0;}",
    "int main(){int n;__builtin_nan(&n);return 0;}",
    "int main(){__builtin_nan(L\"123\");return 0;}",
])]
fn invalid_floating_nan_builtins_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[cfg(not(any(
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_os = "macos")
)))]
#[test_matrix(["asm", "llvm"])]
fn floating_nan_builtins_reject_unsupported_long_double(output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(
            "int main(){return __builtin_nanl(\"\")!=0;}",
            output_option,
            false,
        ))
        .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn floating_infinity_builtins_preserve_bits_types_and_classifiers(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
unsigned int reference32(int);unsigned long reference64(int);
union Bits32{unsigned int bits;float value;};union Bits64{unsigned long bits;double value;};
float initial32=__builtin_inff();double initial64=__builtin_huge_val();double zero=1.0/__builtin_inf();
int finite32(float x){return x==x && __builtin_fabsf(x)!=__builtin_inff();}
int finite64(double x){return x==x && __builtin_fabs(x)!=__builtin_inf();}
int main(){
union Bits32 a,b;a.value=__builtin_inff();b.value=__builtin_huge_valf();
union Bits64 c,d;c.value=__builtin_inf();d.value=__builtin_huge_val();
if(a.bits!=reference32(0) || b.bits!=reference32(1) || c.bits!=reference64(0) || d.bits!=reference64(1))return 1;
if(initial32!=a.value || initial64!=c.value || zero!=0.0 || !(a.value>3e38f) || !(c.value>1e300))return 2;
if(finite32(a.value) || finite32(-a.value) || finite64(c.value) || finite64(-c.value))return 3;
if(!finite32(0.0f) || !finite32(-2.5f) || !finite64(0.0) || !finite64(-3.25))return 4;
a.bits=0x7fc12345u;c.bits=0x7ff8123456789abcul;if(finite32(a.value) || finite64(c.value))return 5;
return _Generic(__builtin_inff(),float:0,default:1) || _Generic(__builtin_inf(),double:0,default:1) || _Generic(__builtin_huge_valf(),float:0,default:1) || _Generic(__builtin_huge_val(),double:0,default:1);
}
"#;
    let companion = r#"
#include <string.h>
unsigned int reference32(int which){float value=which?__builtin_huge_valf():__builtin_inff();unsigned int bits;memcpy(&bits,&value,4);return bits;}
unsigned long reference64(int which){double value=which?__builtin_huge_val():__builtin_inf();unsigned long bits;memcpy(&bits,&value,8);return bits;}
"#;
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn floating_infinity_builtins_preserve_binary64_long_double(
    (output_option, optimization): (&str, &str),
) {
    let code = "long double infinity=__builtin_infl();long double huge=__builtin_huge_vall();int main(){return infinity!=huge || !(infinity>1e300L) || __builtin_fabsl(-infinity)!=infinity || _Generic(__builtin_infl(),long double:0,default:1) || _Generic(__builtin_huge_vall(),long double:0,default:1);}";
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "int main(){return __builtin_inff(1);}",
    "int main(){return __builtin_inf(1,2);}",
    "int main(){return __builtin_huge_valf(1);}",
    "int main(){return __builtin_huge_val(1);}",
    "int main(){void *p=__builtin_inf;}",
])]
fn invalid_floating_infinity_builtins_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[cfg(not(any(
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_os = "macos")
)))]
#[test_matrix(["__builtin_infl", "__builtin_huge_vall"], ["asm", "llvm"])]
fn floating_infinity_builtins_reject_unsupported_long_double(name: &str, output_option: &str) {
    let code = format!("double value={name}();");
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(&code, output_option, false))
            .is_err()
    );
}

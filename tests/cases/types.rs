use crate::support::{
    assert_frontend_error, compile_and_run, compile_and_run_with_c, compile_and_run_with_c_flags,
};
use test_case::test_matrix;

#[test_matrix([
    ("int main() { return sizeof(int) + sizeof(char) + sizeof(int*); }", 13),
    ("int main() { return sizeof(int[2][3]); }", 24),
    ("int main() { int x = 7; return sizeof(x++) + x; }", 11),
    ("int n = sizeof(int); int main() { return n; }", 4),
], ["asm", "llvm"])]
fn it_supports_sizeof_types(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("typedef int Number; Number f(Number x) { return x; } int main() { Number x = 7; return f(x); }", 7),
    ("typedef int *Pointer, Values[2]; int main() { Values a = {2, 5}; Pointer p = a; return p[0] + p[1]; }", 7),
    ("typedef int T; int main() { T x = 7; { typedef char T; T c = 1; x += sizeof(T); } return x + sizeof(T); }", 12),
    ("typedef void Nothing; Nothing f(void) { return; } int main() { f(); return 7; }", 7),
], ["asm", "llvm"])]
fn it_supports_typedef(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("struct S { char a; int b; char c; }; int main() { struct S s = {1, 2, 3}; return sizeof(s) + s.a + s.b + s.c; }", 18),
    ("struct S { int a; int b; }; struct S s = {7}; int main() { return s.a + s.b; }", 7),
    ("struct S { int a[2]; }; int main() { struct S s = {{2,5}}; struct S *p = &s; return p->a[0] + p->a[1]; }", 7),
    ("struct S { int a; }; int main() { struct S a = {7}; struct S b = a; a.a = 2; return b.a; }", 7),
    ("struct S { int a; }; int main() { struct S a = {7}; struct S b; struct S c; c = b = a; return c.a; }", 7),
    ("struct Link { int n; struct Link *next; }; int main() { struct Link a = {7,0}; struct Link b = {2,&a}; return b.next->n; }", 7),
    ("typedef struct { char a; int b; } Pair; int main() { Pair p = {2,5}; return p.a + p.b; }", 7),
    ("struct S { int a; }; int main() { struct S a[2] = {{3},{7}}; struct S *p = a; return (p + 1)->a; }", 7),
    ("struct S { int n; }; int main() { struct S a = {7}; struct S b = {3}; struct S c = 1 ? a : b; return c.n; }", 7),
    ("struct S { int n; }; int main() { struct S a = {7}; struct S b = {3}; return (1 ? a : b).n; }", 7),
    ("struct S { int (*f)(int); }; int add(int n) { return n + 2; } int main() { struct S s = {add}; return s.f(5); }", 7),
    ("struct Inner { int n; }; struct Outer { char c; struct Inner s; }; struct Outer x = {2,{5}}; int main() { return x.c + x.s.n; }", 7),
], ["asm", "llvm"])]
fn it_supports_structs(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    "enum E { A=3,B,C=A+7, }; int main(){enum E x=B;return x+C-14;}",
    "typedef enum { A=-2,B, } E; int main(){E x=B;enum Inner { A=7 };return x+A-6;}",
], ["asm", "llvm"])]
fn it_supports_enumerations(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "union U{int x;char bytes[4];};int main(){union U u={258};return u.bytes[0]+u.bytes[1]-3;}",
    "union U{int x;char bytes[4];};union U u={258};int main(){return u.bytes[0]+u.bytes[1]-3;}",
    "union U{char a[9];int x;};struct S{char c;union U u;char d;};int main(){return sizeof(union U)+sizeof(struct S)-32;}",
    "int x=7;union U{int *p;char bytes[8];};union U u={&x};int main(){return *u.p-7;}",
], ["asm", "llvm"])]
fn it_supports_unions(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){const int x=7;const int *p=&x;return *p-7;}",
    "int main(){volatile int x=7;volatile int *p=&x;*p=9;return x-9;}",
    "int main(){int x=7;int *const p=&x;return *p-7;}",
    "int main(){const char s[]=\"abc\";return s[2]-99;}",
    "typedef int A[2];int main(){const A a={7};return a[0]-7;}",
    "int main(){const char *s=\"ok\";puts(s);return 0;}",
], ["asm", "llvm"])]
fn it_supports_type_qualifiers(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn volatile_array_and_pointer_updates_preserve_object_qualifiers(output_option: &str) {
    let code = "typedef volatile int V;V g=3;int main(){V a[2]={4,5};V *volatile p=a;int x=*p++;int y=(*p)++;--g;return x+y+g-11;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
    if output_option == "llvm" {
        let ir = dynamite_compiler::generate(code, "llvm", false);
        assert!(ir.matches("load volatile i32").count() >= 4, "{ir}");
        assert!(ir.contains("load volatile ptr"), "{ir}");
        assert!(ir.contains("store volatile ptr"), "{ir}");
    }
}

#[test_matrix([
    "struct S{unsigned a:3; signed b:5; int c;};int main(){struct S s={7,-3,9};return s.a+s.b+s.c-13;}",
    "struct S{unsigned a:3;unsigned b:5;};struct S s={.b=17,.a=5};int main(){return s.a+s.b-22;}",
    "struct S{unsigned a:3;unsigned b:5;};int main(){struct S s={7,17};int x=s.a++;int y=++s.b;return x+s.a+y+s.b-43;}",
    "struct S{signed a:3;unsigned b:5;};int main(){struct S s={3,17};int x=(s.a=7);s.a+=2;return x+s.a+s.b-17;}",
    "struct S{unsigned a:3;int:0;unsigned b:5;};int main(){struct S s={7,17};return sizeof(s)+s.a+s.b-32;}",
    "union U{unsigned a:3;unsigned b:5;};union U u={7};int main(){return u.a+u.b-14;}",
    "struct S{unsigned a:3;unsigned b:5;};int main(){struct S s[2]={{1,2},{3,4}};int i=0;s[i++].a+=2;return i+s[0].a+s[0].b-6;}",
    "struct S{unsigned long a:40;unsigned long b:24;};struct S s={1099511627775UL,16777215};int main(){return (s.a==1099511627775UL && s.b==16777215)-1;}",
    "struct S{unsigned a:3;int *p;};int x=7;struct S s={5,&x};int main(){return s.a+*s.p-12;}",
], ["asm", "llvm"])]
fn it_supports_bit_fields(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "struct S{unsigned :2;unsigned a:3;char c;};int main(){struct S s={7,9};s.a=2;return s.a+s.c-11;}",
    "struct S{unsigned long a:64;};struct S s={18446744073709551615UL};int main(){s.a++;return (s.a==0)-1;}",
    "struct S{_Bool b:1;unsigned a:3;};int main(){struct S s={9,7};s.b--;return s.b+s.a-7;}",
], ["asm", "llvm"])]
fn it_supports_bit_field_boundaries(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn flexible_array_members(output_option: &str) {
    let code = "struct S{char tag;double data[];};struct S global={7};struct S echo(struct S s){return s;}int main(){struct S *p=malloc(sizeof(struct S)+3*sizeof(double));p->tag=4;p->data[0]=1.5;p->data[2]=2.5;struct S s=echo(*p);int ok=sizeof(struct S)==8 && (char *)p->data-(char *)p==8 && p->data[0]+p->data[2]==4.0 && s.tag==4 && global.tag==7;free(p);return !ok;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "struct Empty{int values[];};int main(){return 0;}",
    "union U{int n;int values[];};int main(){return 0;}",
    "struct Last{int n;int values[];int other;};int main(){return 0;}",
    "struct Comma{int n;int values[],other;};int main(){return 0;}",
    "struct Array{int n;int values[];};struct Array objects[2];int main(){return 0;}",
    "struct Inner{int n;int values[];};struct Outer{struct Inner inner;};int main(){return 0;}",
    "struct Init{int n;int values[];};struct Init x={1,{2}};int main(){return 0;}",
    "struct Local{int n;int values[];};int main(){struct Local x={1,{2}};return 0;}",
    "struct Size{int n;int values[];};int main(){struct Size x;return sizeof(x.values);}",
])]
fn rejects_invalid_flexible_array_members(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn gnu_qualifier_spellings(output_option: &str) {
    for restrict in ["__restrict", "__restrict__"] {
        let code = format!(
            "typedef __signed__ char S;int sum(__const int *{restrict} p){{return *p;}}int main(){{__volatile__ int x=7;S s=-1;return sum((__const__ int *)&x)-7+(s>=0)+(__alignof__(S)!=1);}}"
        );
        assert_eq!(compile_and_run(&code, output_option).status.code(), Some(0));
    }
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn typeof_preserves_arrays_functions_qualifiers_and_unevaluated_values(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
int values[3]={2,4,6};
__typeof__(values) copy={1,3,5};
__typeof("abc") text="abc";
int add(int x){return x+7;}
typedef typeof(add) Function;
Function *function=add;
struct Pair{long x;int y;};
int calls;
int side(void){calls++;return 2;}
int main(){
    const int original=7;
    typeof(original) qualified=9;
    __typeof__(values) *array=&copy;
    __typeof__(side()) local=3;
    typeof((calls++,local)) integer=5;
    typeof(struct Pair) pair={11,13};
    typeof(void) *nothing=0;
    __typeof__(int (*)(int)) pointer=function;
    typeof(*(int*)0) from_null=0;
    return calls+local-3+integer-5+qualified-9
        +(sizeof(copy)!=12)+(sizeof(text)!=4)+(text[2]!='c')
        +((*array)[2]!=5)+(pointer(2)!=9)+(pair.x!=11)+(pair.y!=13)
        +_Generic(&qualified,const int*:0,default:1)+(nothing!=0)+from_null;
}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn typeof_variable_modified_types_capture_bounds_and_side_effects(
    (output_option, optimization): (&str, &str),
) {
    let code = r"
int main(){
    int n=3;
    int original[n];
    n=7;
    typeof(original) copy;
    if(sizeof(copy)!=12)return 1;
    int calls=0;
    __typeof__(*(calls++,&original)) other;
    if(calls!=1 || sizeof(other)!=12)return 2;
    int bound=2;
    typedef __typeof__(int[++bound]) Array;
    Array first;Array second;
    if(bound!=3 || sizeof(first)!=12 || sizeof(second)!=12)return 3;
    int k=1;
    if(0){sizeof(typeof(int[++k]));}
    int ignored=0 && sizeof(typeof(int[++k]));
    unsigned long alignment=_Alignof(typeof(int[++k]));
    if(k!=1 || ignored || alignment!=4)return 4;
    unsigned long size=sizeof(typeof(int[++k]));
    if(k!=2 || size!=8)return 5;
    int *p=original;
    __typeof__((int(*)[n++])p++) pointer=(void*)original;
    if(n!=8 || p!=original+1 || sizeof(*pointer)!=28)return 6;
    first[2]=7;second[2]=9;copy[2]=11;other[2]=13;
    return first[2]+second[2]+copy[2]+other[2]-40;
}
";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "typeof() x;",
    "typeof(int named) x;",
    "int main(){const int x=1;typeof(x) y=2;y=3;return y;}",
    "struct S{int x:3;};int main(){struct S s;typeof(s.x) x;return 0;}",
    "typeof(void) object;",
    "extern int values[];typeof(values) object;",
])]
fn invalid_typeof_specifiers_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn generic_selection_values(output_option: &str) {
    let code = "#define KIND(x) _Generic((x),int:1,unsigned int:2,double:3,default:4)\nenum{N=_Generic(0,int:7,default:1)};int global=_Generic(0,unsigned int:3,int:9);int main(){int n=0;const int x=2;int a[2];const int ca[2];int *const p=a;return KIND(++n)+KIND(1U)+KIND(1.0)+KIND(x)+_Generic(a,int*:5,default:0)+_Generic(ca,const int*:6,default:0)+_Generic(p,int*:7,default:0)+_Generic((void)0,default:8)+N+global+n-49;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn generic_selection_preserves_expressions(output_option: &str) {
    let code = "int add(int x){return x+1;}struct S{int x;};int main(){int a=1,b=2;struct S s={3};_Generic(a,int:a,default:b)=4;_Generic(s,struct S:s,default:s).x=5;int n=2;int value=_Generic(n,int:7,default:sizeof(int[++n]));int ignored=_Generic(sizeof(int[++n]),unsigned long:1,default:0);return _Generic(add,int(*)(int):add,default:add)(a)+s.x+n+value+ignored-20;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){return _Generic(0,int:1,int:2);}",
    "typedef int I;int main(){return _Generic(0,int:1,I:2);}",
    "int main(){return _Generic(0,default:1,default:2);}",
    "int main(){return _Generic(0,double:1);}",
    "int main(){return _Generic(0,void:1,default:2);}",
    "struct S;int main(){return _Generic(0,struct S:1,default:2);}",
    "int main(){int n=3;return _Generic(0,int[n]:1,default:2);}",
    "int main(){return _Generic(0,int:1,default:1.0%2);}",
    "int main(){return _Generic(1.0%2,int:1,default:2);}",
    "int main(){const int x=1;_Generic(x,int:x,default:x)=2;return 0;}",
    "int main(){register int x=1;int *p=&_Generic(x,int:x,default:x);return 0;}",
])]
fn rejects_invalid_generic_selection(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn generic_qualifiers_and_nested_selection(output_option: &str) {
    let code = "_Static_assert(_Generic(0,int:1,default:0),\"selected constant\");int main(){const int x=3;int n=0;int result=_Generic(x,const int:99,int:_Generic(1.0,float:88,double:++n));return result+n-2;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn generic_selection_array_bounds(output_option: &str) {
    let code = "int first(int *a){return a[0];}int main(){int n=2;int (*p)(int*)=first;int value=_Generic(p,int(*)(int a[++n]):1,default:0);int size=_Generic(n,int:sizeof(int[++n]),default:0);return n+value+size-16;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn aggregate_copy_preserves_source_qualifiers(output_option: &str) {
    let code = "struct S{int x;long y;};union U{long x;char bytes[8];};struct S copy(const struct S *p){struct S out;out=*p;return out;}int main(){const struct S source={7,11};const struct S *p=&source;struct S result=copy(p);volatile struct S observed={3,5};result=observed;const union U original={13};union U target;target=original;return result.x+result.y+target.x-21;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
    if output_option == "llvm" {
        let ir = dynamite_compiler::generate(code, output_option, false);
        assert!(ir.contains("load volatile i8"));
    }
}

#[test_matrix([
    "struct S{int x;};int main(){struct S a={1};const struct S b={2};b=a;return 0;}",
    "struct S{int x;};int main(){const struct S a={1};struct S *p=&a;return p->x;}",
    "struct A{int x;};struct B{int x;};int main(){struct A a;struct B b={1};a=b;return 0;}",
])]
fn aggregate_copy_rejects_invalid_qualifier_conversions(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn compatible_function_designators_ignore_parameter_qualifiers(
    output_option: &str,
    optimization: &str,
) {
    let code = r"
typedef int (*Callback)(int *,const int *);
typedef int (*Variadic)(const int *,...);
int combine(int *restrict a,const int *restrict b){return *a+*b;}
int constant(const int value){return value+1;}
int variable(const int *restrict value,...){return *value;}
Callback select(void){return combine;}
int invoke(Callback f,int *a,const int *b){return f(a,b);}
int external_invoke(Callback f,int *a,const int *b);
int main(void){
    int a=17,b=25;
    Callback initial=combine,assigned;assigned=combine;
    int (*scalar)(int)=constant;
    Variadic variadic=variable;
    if(initial(&a,&b)!=42 || assigned(&a,&b)!=42)return 1;
    if(select()(&a,&b)!=42 || invoke(combine,&a,&b)!=42)return 2;
    if(external_invoke(combine,&a,&b)!=42)return 3;
    return scalar(41)!=42 || variadic(&a,3)!=17;
}";
    let companion =
        "int external_invoke(int (*f)(int*,const int*),int *a,const int *b){return f(a,b);}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    ("unsigned int",1,4), ("unsigned int",2,4), ("unsigned long",1,8),
    ("double",1,8), ("double",4,8), ("void *",1,8),
], ["asm", "llvm"], ["-O0", "-O2"])]
fn reduced_typedef_alignment_matches_clang(
    case: (&str, usize, usize),
    output_option: &str,
    optimization: &str,
) {
    let (element, alignment, size) = case;
    let declarations = format!(
        r#"
typedef __attribute__((aligned({alignment}))) {element} T;
typedef T Alias;typedef const T Const;
typedef __attribute__((aligned(1))) int Mixed __attribute__((aligned(2)));
typedef {element} Base;typedef Base Suffix __attribute__((aligned({alignment}))), Ordinary;
struct S{{char tag;T value;char tail;}};
"#
    );
    let code = format!(
        r#"{declarations}
T scalar=(T)5,array[2]={{(T)11,(T)13}};struct S global={{1,(T)7,2}};
int inspect(struct S*,T*,T*);
int main(void){{
 if(sizeof(T)!={size} || _Alignof(T)!={alignment} || _Alignof(Alias)!={alignment} || _Alignof(Const)!={alignment})return 1;
 if(_Alignof(Suffix)!={alignment} || _Alignof(Ordinary)!={size} || _Alignof(Mixed)!=2)return 2;
 if(sizeof(struct S)!={size}+2*{alignment} || _Alignof(struct S)!={alignment} || __builtin_offsetof(struct S,value)!={alignment})return 3;
 if(_Generic((T*)0,{element}*:0,default:1))return 4;
 struct S local={{3,(T)17,4}};if(local.value!=(T)17)return 5;
 unsigned char bytes[64];T *unaligned=(T*)(bytes+1);*unaligned=(T)21;if(*unaligned!=(T)21)return 6;
 int n=2;T variable[n];variable[1]=(T)23;if(sizeof(variable)!=2*{size} || variable[1]!=(T)23)return 7;
 if(inspect(&global,&scalar,array))return 8;
 return scalar!=(T)29 || global.value!=(T)31;
}}
"#
    );
    let companion = format!(
        r#"{declarations}
int inspect(struct S *s,T *scalar,T *array){{
 if((char*)&s->value-(char*)s!=__builtin_offsetof(struct S,value) || (char*)&array[1]-(char*)array!=sizeof(T))return 1;
 if(s->tag!=1 || s->value!=(T)7 || s->tail!=2 || *scalar!=(T)5 || array[1]!=(T)13)return 2;
 *scalar=(T)29;s->value=(T)31;return 0;
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

#[test_matrix([
    "typedef int T __attribute__((aligned(1)));typedef int T;",
    "typedef long T __attribute__((aligned(16)));",
    "typedef struct S{int n;}T __attribute__((aligned(1)));",
    "typedef __int128 T __attribute__((aligned(1)));",
    "typedef int T __attribute__((aligned(1)));struct S{T n;};void f(struct S);",
    "typedef int T __attribute__((aligned(1)));struct S;void f(struct S);struct S{T n;};",
    "typedef int T __attribute__((aligned(1)));struct S{T n;};int f(int,...);int main(){struct S s;return f(0,s);}",
], ["asm", "llvm"])]
fn unsupported_reduced_typedef_alignment_has_source_diagnostics(code: &str, output_option: &str) {
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

#[test_matrix([
    ("unsigned char",1), ("short",4), ("int",16),
    ("float",8), ("unsigned long long",16), ("double",16),
], ["asm", "llvm"], ["-O0", "-O2"])]
fn vector_typedef_storage_matches_clang(
    case: (&str, usize),
    output_option: &str,
    optimization: &str,
) {
    let (element, bytes) = case;
    let declarations = format!(
        r"
typedef {element} V __attribute__((vector_size({bytes})));
typedef const {element} CV __attribute__((__vector_size__({bytes})));
struct Plain{{char prefix;V value;char tail;}};
#pragma pack(push,1)
struct Packed{{char prefix;V value;char tail;}};
#pragma pack(pop)
"
    );
    let code = format!(
        r#"{declarations}
V global, array[2];struct Plain plain;struct Packed packed;
int inspect(V*,V*,V*,struct Plain*,struct Packed*);
int main(void){{
    V local;static V block;int n=2;V variable[n];
    if(sizeof(V)!={bytes} || _Alignof(V)!={bytes} || sizeof(global)!={bytes})return 1;
    if(sizeof(array)!=2*{bytes} || sizeof(variable)!=2*{bytes} || (char*)&array[1]-(char*)&array[0]!={bytes})return 2;
    if(__builtin_offsetof(struct Plain,value)!={bytes} || sizeof(struct Plain)!=3*{bytes})return 3;
    if(__builtin_offsetof(struct Packed,value)!=1 || sizeof(struct Packed)!={bytes}+2)return 4;
    if(_Generic((CV*)0,const V*:0,default:1))return 5;
    if(inspect(&global,&local,array,&plain,&packed))return 6;
    if((unsigned long)&block%{bytes})return 7;
    return *(unsigned char*)&local!=0x3c || *(unsigned char*)&global!=0x5a;
}}
"#
    );
    let companion = format!(
        r#"{declarations}
#include <string.h>
int inspect(V *g,V *local,V *array,struct Plain *plain,struct Packed *packed){{
if((unsigned long)g%_Alignof(V) || (unsigned long)local%_Alignof(V) || (char*)&array[1]-(char*)&array[0]!=sizeof(V))return 1;
if((char*)&plain->value-(char*)plain!=__builtin_offsetof(struct Plain,value) || (char*)&packed->value-(char*)packed!=1)return 2;
for(unsigned long i=0;i<sizeof(V);i++)if(((unsigned char*)g)[i] || ((unsigned char*)array)[i])return 3;
memset(g,0x5a,sizeof(V));memset(local,0x3c,sizeof(V));return 0;
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

#[test_matrix(["asm", "llvm"])]
fn vector_typedef_attributes_remain_local_and_type_queries_are_unevaluated(output_option: &str) {
    let code = r#"
typedef int V __attribute__((vector_size(16))), Scalar;
typedef int V __attribute__((vector_size(16)));
V object;
_Static_assert(sizeof(object)==16,"vector size");
_Static_assert(sizeof(_Generic(object,V:1,default:0))==4,"generic type");
int main(){typedef short Local __attribute__((vector_size(sizeof(int))));Local local;return sizeof(Scalar)!=4 || sizeof(Local)!=4 || _Generic(object,V:0,default:1) || _Generic((V*)0,int(*)[4]:1,default:0);}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "typedef int V __attribute__((vector_size(16)));V a,b;int main(){a=b;return 0;}",
    "typedef int V __attribute__((vector_size(16)));V a,b;int main(){a+b;return 1;}",
    "typedef int V __attribute__((vector_size(16)));V a;int main(){return a[0];}",
    "typedef int V __attribute__((vector_size(16)));V a={0};",
    "typedef int V __attribute__((vector_size(16)));int main(){V a={0};return 0;}",
    "typedef int V __attribute__((vector_size(16)));V f(void);",
    "typedef int V __attribute__((vector_size(16)));void f(V);",
    "typedef int V __attribute__((vector_size(16)));struct S{V value;};void f(struct S);",
    "typedef int V __attribute__((vector_size(16)));struct S;void f(struct S);struct S{V value;};",
    "typedef int V __attribute__((vector_size(16)));struct S{V value;};int f(int,...);int main(){struct S s;return f(0,s);}",
    "typedef int V __attribute__((vector_size(3)));",
    "typedef int V __attribute__((vector_size(32)));",
    "typedef double V __attribute__((vector_size(4)));",
    "typedef _Bool V __attribute__((vector_size(16)));",
    "typedef int *P __attribute__((vector_size(16)));",
    "typedef int V __attribute__((vector_size(16),vector_size(8)));",
    "int object __attribute__((vector_size(16)));",
], ["asm", "llvm"])]
fn unsupported_vector_operations_have_source_diagnostics(code: &str, output_option: &str) {
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
fn gnu_object_alignment_preserves_storage_and_type(output_option: &str, optimization: &str) {
    let code = r#"
__attribute__((aligned(64))) int prefix = 3, prefix_second = 4;
static __attribute__((__aligned__(128))) long cache[3] = {5, 6, 7};
int __attribute__((aligned(256))) after_type = 8, after_second = 9;
char suffix __attribute__((aligned(4096))) = 10, ordinary = 11;
extern int redeclared __attribute__((aligned(64)));
int redeclared = 12;
long weaker __attribute__((aligned(1))) = 13;
char bare __attribute__((aligned)) = 14;
struct Tagged { int n; };
struct Tagged __attribute__((aligned(64))) tagged = {21};
int block_redeclared = 15;
int check(void *, unsigned long);
int main(void) {
    static __attribute__((aligned(128))) int block_prefix = 16;
    static int block_suffix __attribute__((aligned(256))) = 17;
    static char inferred[] __attribute__((aligned(64))) = {"abc"};
    extern int block_redeclared __attribute__((aligned(128)));
    __attribute__((aligned(16))) char local_prefix = 18;
    int local_suffix __attribute__((aligned(16))) = 19;
    _Alignas(16) char c11_local = 20;
    if(check(&prefix,64) || check(&prefix_second,64) || check(cache,128)
       || check(&after_type,256) || check(&after_second,256)
       || check(&suffix,4096) || check(&redeclared,64) || check(&weaker,8)
       || check(&bare,16) || check(&tagged,64) || check(&block_redeclared,128)
       || check(&block_prefix,128) || check(&block_suffix,256) || check(inferred,64)
       || check(&local_prefix,16) || check(&local_suffix,16) || check(&c11_local,16)) return 1;
    if(sizeof(prefix)!=4 || sizeof(cache)!=24 || sizeof(suffix)!=1 || sizeof(inferred)!=4 || sizeof(struct Tagged)!=4) return 2;
    return prefix!=3 || prefix_second!=4 || cache[2]!=7 || after_type!=8 || after_second!=9
        || suffix!=10 || ordinary!=11 || redeclared!=12 || weaker!=13 || bare!=14
        || block_redeclared!=15 || block_prefix!=16 || block_suffix!=17 || inferred[2]!='c'
        || local_prefix!=18 || local_suffix!=19 || c11_local!=20 || tagged.n!=21;
}"#;
    let companion = "int check(void *p, unsigned long n) { return (unsigned long)p % n != 0; }";
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0));
}

#[test]
fn gnu_object_alignment_does_not_leak_between_declarators() {
    let ir = dynamite_compiler::generate(
        "int first __attribute__((aligned(64))), second; int main(){return 0;}",
        "llvm",
        false,
    );
    let first = ir
        .lines()
        .find(|line| line.starts_with("@first ="))
        .unwrap();
    let second = ir
        .lines()
        .find(|line| line.starts_with("@second ="))
        .unwrap();
    assert!(first.contains("align 64"), "{first}");
    assert!(second.contains("align 4"), "{second}");
}

#[test_matrix([
    "int main(){int x __attribute__((aligned(32)));return 0;}",
    "int f(void) __attribute__((aligned(64)));",
    "int f(int x __attribute__((aligned(16))));",
    "int x __attribute__((aligned(3)));",
    "struct S; struct S __attribute__((aligned(64)));",
    "int main(){_Alignas(16) register int x;return 0;}",
    "int main(){__attribute__((aligned(16))) return 0;}",
])]
fn gnu_object_alignment_rejects_unsupported_contexts(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn raised_record_typedef_alignment_matches_clang(output_option: &str, optimization: &str) {
    let code = r"
typedef struct {long words[13];} Buffer __attribute__((aligned));
struct Container{char tag;Buffer value;char end;};
Buffer global={};struct Container storage={};
int check(Buffer *,struct Container *);
int main(){Buffer local={};struct Container c={};local.words[12]=7;c.tag=3;c.value=local;c.end=9;storage=c;global=local;
return check(&global,&storage)+(sizeof(Buffer)!=104)+(_Alignof(Buffer)!=16)
+(sizeof(c)!=128)+((char*)&c.value-(char*)&c!=16)+((char*)&c.end-(char*)&c!=120);}
";
    let companion = "typedef struct{long words[13];} Buffer __attribute__((aligned));struct Container{char tag;Buffer value;char end;};int check(Buffer *b,struct Container *c){return b->words[12]!=7 || c->tag!=3 || c->value.words[12]!=7 || c->end!=9;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "typedef struct{long x;} Buffer __attribute__((aligned(16)));Buffer invalid[2];",
    "typedef struct{long x;} Buffer __attribute__((aligned(16)));Buffer f(Buffer);",
    "typedef struct{long x;} Buffer __attribute__((aligned(32)));",
])]
fn rejects_unsupported_raised_typedef_uses(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn aligned_records_preserve_layout_and_clang_calls(output_option: &str) {
    let code = r#"
struct __attribute__((aligned(16),unused)) Small{char x;};
struct Large{char bytes[17];} __attribute__((__aligned__(8)));
union __attribute__((aligned(16))) Choice{int x;char bytes[3];};
struct __attribute__((aligned(4))) Natural{double x;};
struct __attribute__((aligned)) Forward;
struct Forward{char x;};
struct Outer{char tag;struct Small inner;union Choice choice;char tail;};
struct Outer global={7,{9},{.x=11},13};
struct Small foreign(int n,struct Small x,double value);
int check(struct Outer x,struct Outer *p);
struct Small local(int n,struct Small x,double value){x.x+=n+(int)value;return x;}
int main(){
    struct Outer copy=global;
    struct Small array[2]={{1},{3}};
    struct Small result=foreign(2,array[1],4.0);
    copy.inner=result;
    return check(copy,&global)+(sizeof(struct Small)!=16)+(_Alignof(struct Small)!=16)
        +(sizeof(struct Large)!=24)+(sizeof(union Choice)!=16)
        +(_Alignof(struct Natural)!=8)+(sizeof(struct Forward)!=16)
        +(sizeof(array)!=32);
}"#;
    let companion = r#"
struct __attribute__((aligned(16))) Small{char x;};
union __attribute__((aligned(16))) Choice{int x;char bytes[3];};
struct Outer{char tag;struct Small inner;union Choice choice;char tail;};
struct Small local(int,struct Small,double);
struct Small foreign(int n,struct Small x,double value){return local(n,x,value);}
int check(struct Outer x,struct Outer *p){
    return x.tag!=7 || x.inner.x!=9 || x.choice.x!=11 || x.tail!=13
        || p->inner.x!=9 || p->choice.x!=11 || sizeof(*p)!=64;
}"#;
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "struct __attribute__((aligned(3))) S{int x;};",
    "struct S{char x;};struct S __attribute__((aligned(8192))) value;",
    "struct S{char x;};struct __attribute__((aligned(16))) S value;",
    "struct S{int x;} __attribute__((aligned(0)));",
    "struct __attribute__((aligned(8192))) S{int x;};",
    "int n;struct __attribute__((aligned(n))) S{int x;};",
])]
fn invalid_record_alignment_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn qualified_record_value_conversions(output_option: &str, optimization: &str) {
    let code = r#"
struct Pair { long x; int y; };
union Value { long n; int i; };
long foreign(const struct Pair);
long take(const struct Pair p) { return p.x + p.y; }
struct Pair copy(const struct Pair *p) { return *p; }
struct Pair relay(struct Pair p) { return copy(&p); }
long take_union(const union Value p) { return p.n; }
int main() {
    const struct Pair constant = { 10, 7 };
    volatile struct Pair observed = { 11, 9 };
    struct Pair a = constant;
    struct Pair b = observed;
    a = observed;
    b = copy(&constant);
    struct Pair (*callback)(struct Pair) = relay;
    struct Pair c = callback(a);
    const union Value u = { 13 };
    union Value v = u;
    v = u;
    if(take(constant) != 17 || take(a) != 20 || take(b) != 17) return 1;
    if(c.x != 11 || c.y != 9 || foreign(constant) != 17) return 2;
    if(take_union(u) != 13 || take_union(v) != 13) return 3;
    return 0;
}
"#;
    let companion = "struct Pair{long x;int y;};long foreign(const struct Pair p){return p.x+p.y;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "struct A{int x;};struct B{int x;};int f(const struct A x){return x.x;}int main(){struct B b={1};return f(b);}",
    "struct A{int x;};int f(struct A *p){return p->x;}int main(){const struct A a={1};return f(&a);}",
    "struct A{int x;};int main(){const struct A a={1};struct A b={2};a=b;return 0;}"
])]
fn invalid_qualified_record_value_conversions(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn promoted_complement_does_not_fit_unsigned_short_enum(output_option: &str) {
    let code = "enum E : unsigned short { A=~(unsigned short)0 };";
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix([32,64,128,256,4096], ["asm", "llvm"], ["-O0", "-O2"])]
fn extended_record_alignment_in_static_storage(
    alignment: usize,
    output_option: &str,
    optimization: &str,
) {
    let definitions = format!(
        "struct __attribute__((aligned({alignment}))) Wide{{long n;char tag;}};struct Container{{char prefix;struct Wide value;char tail;}};union Choice{{char tag;struct Wide value;}};"
    );
    let code=[definitions.as_str(),r#"
struct Wide array[2]={{7,3},{11,5}};
struct Container container={1,{13,9},2};
union Choice choice={4};
extern struct Wide foreign;
int check(struct Wide*,struct Container*,union Choice*,unsigned long,unsigned long);
struct Wide *stored(){static struct Wide value={17,6};return &value;}
int main(){
    unsigned long alignment=_Alignof(struct Wide);
    if((unsigned long)array%alignment || (unsigned long)&array[1]%alignment || (unsigned long)stored()%alignment) return 1;
    if(sizeof(struct Wide)!=alignment || (unsigned long)&array[1]-(unsigned long)&array[0]!=alignment) return 2;
    if(__builtin_offsetof(struct Container,value)!=alignment || __builtin_offsetof(struct Container,tail)!=2*alignment || sizeof(struct Container)!=3*alignment) return 3;
    if(_Alignof(struct Container)!=alignment || sizeof(union Choice)!=alignment) return 4;
    if(array[1].n!=11 || container.value.n!=13 || container.tail!=2 || stored()->n!=17 || foreign.n!=23) return 5;
    array[0]=*stored();
    if(array[0].n!=17 || array[0].tag!=6) return 6;
    return check(array,&container,&choice,sizeof(struct Wide),alignment);
}
"#].concat();
    let companion=[definitions.as_str(),r#"
struct Wide foreign={23,8};
int check(struct Wide *a,struct Container *c,union Choice *u,unsigned long size,unsigned long alignment){return a[0].n!=17 || a[1].n!=11 || c->prefix!=1 || c->value.tag!=9 || c->tail!=2 || u->tag!=4 || size!=sizeof(struct Wide) || alignment!=_Alignof(struct Wide);}
"#].concat();
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"])]
fn compatible_typedef_redeclarations_preserve_types_and_scopes(output_option: &str) {
    let code = r#"
typedef int Number;
typedef Number Number;
typedef const int Fixed;
typedef const int Fixed;
typedef int Function(const int);
typedef int Function(int);
typedef const int Row[2];
typedef Fixed Row[2];
typedef int Incomplete[];
typedef int Incomplete[];
struct S;
typedef struct S Record;
typedef struct S Record;
struct S{int x;};
typedef __builtin_va_list SystemList;
typedef __builtin_va_list SystemList;
int main(){
    Number n=3;
    Row row={4,5};
    Record record={7};
    {typedef char Number;typedef char Number;Number x=2;n+=x;}
    return n+row[0]+row[1]+record.x-21;
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "typedef int T;typedef char T;",
    "typedef int T;typedef const int T;",
    "typedef int *Pointer;typedef const int *Pointer;",
    "typedef int T[2];typedef const int T[2];",
    "typedef int T[2][3];typedef const int T[2][3];",
    "typedef int T[2];typedef int T[3];",
    "typedef int T[];typedef int T[3];",
    "typedef int (*ArrayPointer)[];typedef int (*ArrayPointer)[3];",
    "typedef void T(int(*)[]);typedef void T(int(*)[3]);",
    "typedef const int T(void);typedef int T(void);",
    "enum E{VALUE=-1};typedef enum E T;typedef int T;",
    "int value;typedef int value;",
    "int main(){int n=3;typedef int T[n];typedef int T[n];return 0;}",
    "int main(){int n=3;typedef int T[n];typedef int T[3];return 0;}",
])]
fn incompatible_typedef_redeclarations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn enumerator_annotations_preserve_constant_values(output_option: &str) {
    let code = r#"
#define AVAILABLE __attribute__((availability(macos,introduced=10.12)))
enum Values{
    FIRST __attribute__((deprecated("old value")))=3,
    SECOND AVAILABLE=FIRST+4,
    THIRD __attribute__((__deprecated__,unused)),
};
_Static_assert(SECOND==7,"annotated constant");
int main(){enum Values value=THIRD;return FIRST+SECOND+value-18;}
"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "enum E{VALUE __attribute__((aligned(8)))=1};",
    "enum E{VALUE __attribute__((unknown))=1};",
    "enum E{VALUE __attribute__((deprecated(\"unfinished\"))};",
])]
fn unsupported_or_malformed_enumerator_annotations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn natural_typedef_alignment_attributes_preserve_layout(output_option: &str) {
    let code = r#"
typedef char Byte __attribute__((aligned(1)));
typedef short Half __attribute__((__aligned__(2)));
typedef int Word __attribute__((aligned(4),unused));
typedef long Wide __attribute__((aligned(8)));
typedef double Real __attribute__((aligned(8)));
typedef int *Pointer __attribute__((aligned(8)));
typedef int Row[3] __attribute__((aligned(4)));
typedef int (*Callback)(int) __attribute__((aligned(8)));
struct S{char tag;Wide value;};
typedef struct S Record __attribute__((aligned(8)));
Record global={3,7};
int identity(int value){return value;}
int main(){
    Byte byte=1;Half half=2;Word word=3;Real real=4.5;
    Pointer p=&word;Row row={5,6,7};Record record=global;Callback call=identity;
    return (sizeof(Row)!=12)+(_Alignof(Word)!=4)+(_Alignof(Record)!=8)
        +(sizeof(record)!=16)+(record.value!=7)+(record.tag!=3)
        +(call(*p)!=3)+(row[2]!=7)+(real!=4.5)+(byte!=1)+(half!=2);
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "typedef int Word __attribute__((aligned(8)));",
    "typedef long Wide __attribute__((aligned(16)));",
    "typedef int Word __attribute__((aligned(8),aligned(2)));",
    "typedef int Function(void) __attribute__((aligned(1)));",
    "typedef struct Missing Record __attribute__((aligned(1)));",
    "typedef int Word __attribute__((aligned(3)));",
])]
fn unsupported_typedef_alignment_changes_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn extended_enumerator_ranges(output_option: &str, optimization: &str) {
    let definitions = r#"
enum Mask { NONE=0, ALL=~0U };
enum Grow { FIRST=2147483647, SECOND };
enum Large { LOW=0xffffffffU, HIGH };
enum MixedRange { NEGATIVE=-1, POSITIVE=0xffffffffU };
enum HugeRange { MAXIMUM=~0UL, RESET=0 };
enum SignedRange { MINIMUM=(-9223372036854775807L-1), MAX_SIGNED=9223372036854775807L };
enum Wrap { BEFORE=0xffffffffU, AFTER=BEFORE+1U };
struct Record { enum Mask flags; enum Large count; enum MixedRange delta; };
"#;
    let code=[definitions,r#"
int foreign(struct Record,enum HugeRange,enum SignedRange);
enum Mask mask=ALL;
enum Large large=HIGH;
int main(){
    if(sizeof(enum Mask)!=4 || sizeof(enum Large)!=8 || sizeof(enum MixedRange)!=8) return 1;
    if(mask!=4294967295U || SECOND!=2147483648U || large!=4294967296UL) return 2;
    if(_Generic(ALL,unsigned int:0,default:1) || _Generic(NONE,unsigned int:0,default:1)) return 3;
    if(_Generic(POSITIVE,long:0,default:1) || _Generic(NEGATIVE,long:0,default:1)) return 4;
    if(_Generic(HIGH,unsigned long:0,default:1) || _Generic(MAXIMUM,unsigned long:0,default:1)) return 5;
    if(MAXIMUM!=18446744073709551615UL || MINIMUM!=-9223372036854775807L-1 || MAX_SIGNED!=9223372036854775807L) return 6;
    if(AFTER!=0 || _Generic(AFTER,unsigned int:0,default:1)) return 7;
    enum Mask same=ALL;
    {enum Mask{LOCAL=-1};if(sizeof(enum Mask)!=4 || LOCAL!=-1)return 8;}
    struct Record r={same,HIGH,NEGATIVE};
    if(foreign(r,MAXIMUM,MINIMUM))return 9;
    return 0;
}
"#].concat();
    let companion=[definitions,r#"
int foreign(struct Record r,enum HugeRange max,enum SignedRange min){return r.flags!=ALL || r.count!=HIGH || r.delta!=-1 || max!=MAXIMUM || min!=MINIMUM;}
"#].concat();
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "enum E { LAST=18446744073709551615UL, OVERFLOW };",
    "enum E { NEGATIVE=-1, TOO_LARGE=18446744073709551615UL };"
])]
fn invalid_extended_enumerator_ranges(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn enum_type_identities_preserve_scopes_and_representation(
    (output_option, optimization): (&str, &str),
) {
    let code = r"
enum First{FIRST=3};enum Second{SECOND=5};
typedef enum First Alias;
enum Byte:unsigned char;typedef enum Byte Byte;
enum Byte:unsigned char{BYTE=250};
enum OtherByte:unsigned char{OTHER_BYTE=251};
enum Flag:_Bool{OFF,ON};
struct Record{volatile enum First value;enum Byte byte;enum Flag flag;};
struct Record global={FIRST,BYTE,ON};
enum Wide{WIDE_VALUE=0x100000000UL};
enum First identity(enum First value){return value;}
int main(){
    Alias a=FIRST;enum Second b=SECOND;Byte byte=BYTE;enum Wide wide=WIDE_VALUE;
    if(_Generic(a,enum First:0,enum Second:1,default:2))return 1;
    if(_Generic(b,enum Second:0,enum First:1,default:2))return 2;
    if(_Generic(&a,enum First*:0,enum Second*:1,default:2))return 3;
    if(_Generic(byte,enum Byte:0,enum OtherByte:1,default:2))return 4;
    if(_Generic(BYTE,enum Byte:0,enum OtherByte:1,default:2))return 5;
    {enum First{LOCAL=9};enum First local=LOCAL;
     if(_Generic(local,Alias:1,default:0)||_Generic(a,enum First:1,default:0))return 6;}
    volatile int nine=9;enum Flag flag=(enum Flag)nine;
    global.value=identity(a);global.flag=flag;
    if(flag!=1 || global.value!=3 || global.flag!=1 || global.byte!=250)return 7;
    if(sizeof(enum First)!=4 || sizeof(enum Byte)!=1 || sizeof(enum Wide)!=8 || wide!=WIDE_VALUE)return 8;
    return identity(a)!=3 || (enum Flag)9!=1;
}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "enum A{ONE};enum B{TWO};typedef enum A T;typedef enum B T;",
    "enum A{ONE};enum B{TWO};void f(enum A);void f(enum B);",
    "enum A{ONE};enum B{TWO};int main(){enum A a;enum B *p=&a;return 0;}",
    "enum A:int{ONE};enum B:int{TWO};int main(){return _Generic(ONE,enum B:1);}",
    "enum A{ONE};enum B{TWO};int main(){return _Generic(1,enum A:0,enum B:1);}",
    "enum A{ONE};enum B:enum A{TWO};",
])]
fn distinct_enum_type_mismatches_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"])]
fn fixed_underlying_enums_preserve_types_ranges_and_scopes(output_option: &str) {
    let code = r#"
enum Byte:unsigned char;
typedef enum Byte Byte;
enum Byte:unsigned char{BYTE_VALUE=250,BYTE_NEXT};
enum Mask:unsigned int{HIGH=0x80000000U,ALL=0xffffffffU};
enum Huge:unsigned long{HUGE_PREVIOUS=0xfffffffffffffffeULL,HUGE_MAXIMUM};
enum Signed:signed short{MINIMUM=-32768,MAXIMUM=32767};
enum Logic:_Bool{FALSE_VALUE=0,TRUE_VALUE=1};
enum :unsigned short{ANONYMOUS=65535};
enum Reset:unsigned char{LAST=255,RESET=0};
_Static_assert(HUGE_MAXIMUM==0xffffffffffffffffULL,"full unsigned range");
int scoped(enum Private:unsigned char{PRIVATE_VALUE=3} value){enum Private other=PRIVATE_VALUE;return value+other;}
int main(){
    Byte bytes[2]={BYTE_VALUE,BYTE_NEXT};
    int mismatch=0;
    {enum Byte:unsigned short{LOCAL_VALUE=500};mismatch+=sizeof(enum Byte)!=2;}
    return mismatch+(sizeof(enum Byte)!=1)+(_Alignof(enum Signed)!=2)
        +(sizeof(bytes)!=2)+(bytes[1]!=251)+(HIGH<=0)+(ALL!=0xffffffffU)
        +(HUGE_PREVIOUS+1!=HUGE_MAXIMUM)+(MINIMUM!=-32768)+(MAXIMUM!=32767)
        +(ANONYMOUS!=65535)+(RESET!=0)+(scoped(4)!=7)
        +_Generic(BYTE_VALUE,unsigned char:0,default:1)
        +_Generic(HIGH,unsigned int:0,default:1)
        +_Generic(HUGE_MAXIMUM,unsigned long:0,default:1)
        +_Generic(TRUE_VALUE,_Bool:0,default:1)+(FALSE_VALUE!=0);
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "enum E:float{VALUE};",
    "typedef int *Pointer;enum E:Pointer{VALUE};",
    "enum E:unsigned char{VALUE=256};",
    "enum E:signed char{VALUE=128};",
    "enum E:signed char{VALUE=-129};",
    "enum E:unsigned int{VALUE=-1};",
    "enum E:unsigned long{VALUE=-1};",
    "enum E:signed long{VALUE=0xffffffffffffffffULL};",
    "enum E:_Bool{VALUE=2};",
    "enum E:unsigned char{MAXIMUM=255,OVERFLOW};",
    "enum E:unsigned;enum E:unsigned long{VALUE};",
    "enum E:unsigned;enum E{VALUE};",
    "enum E{VALUE};enum E:int;",
    "enum E:unsigned{FIRST};enum E:unsigned{SECOND};",
    "enum E:unsigned{FIRST};enum E:unsigned;enum E:unsigned{SECOND};",
    "enum E:unsigned{VALUE=1.5};",
])]
fn invalid_fixed_underlying_enums_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn aligned_aggregate_calls_match_clang(output_option: &str, optimization: &str) {
    let layout = r#"
struct RecordPadding{double x;} __attribute__((aligned(16)));
struct MemberPadding{_Alignas(16) double x;};
struct NestedPadding{struct RecordPadding value;};
struct HfaMember{_Alignas(16) double x;double y;};
struct HfaRecord{double x;double y;} __attribute__((aligned(16)));
"#;
    let code = [layout, r#"
double foreign_record(int n,struct RecordPadding value,double tail);
double foreign_member(int n,struct MemberPadding value,double tail);
double foreign_nested(int n,struct NestedPadding value,double tail);
double foreign_stack_record(int a,int b,int c,int d,int e,int f,int g,int h,int spacer,struct RecordPadding value,int tail);
double foreign_stack_member(int a,int b,int c,int d,int e,int f,int g,int h,int spacer,struct MemberPadding value,int tail);
double foreign_hfa_member(double a,double b,double c,double d,double e,double f,double g,double h,double spacer,struct HfaMember value,double tail);
double foreign_hfa_record(double a,double b,double c,double d,double e,double f,double g,double h,double spacer,struct HfaRecord value,double tail);
double foreign_return(struct RecordPadding value);
double local_record(int n,struct RecordPadding value,double tail){return n+value.x+tail;}
double local_member(int n,struct MemberPadding value,double tail){return n+value.x+tail;}
double local_nested(int n,struct NestedPadding value,double tail){return n+value.value.x+tail;}
double local_stack_record(int a,int b,int c,int d,int e,int f,int g,int h,int spacer,struct RecordPadding value,int tail){return a+b+c+d+e+f+g+h+spacer+value.x+tail;}
double local_stack_member(int a,int b,int c,int d,int e,int f,int g,int h,int spacer,struct MemberPadding value,int tail){return a+b+c+d+e+f+g+h+spacer+value.x+tail;}
double local_hfa_member(double a,double b,double c,double d,double e,double f,double g,double h,double spacer,struct HfaMember value,double tail){return a+b+c+d+e+f+g+h+spacer+value.x+value.y+tail;}
double local_hfa_record(double a,double b,double c,double d,double e,double f,double g,double h,double spacer,struct HfaRecord value,double tail){return a+b+c+d+e+f+g+h+spacer+value.x+value.y+tail;}
struct RecordPadding local_return(struct RecordPadding value){value.x+=1.0;return value;}
int main(){
    struct RecordPadding record={2.5};struct MemberPadding member={2.5};
    struct NestedPadding nested={{2.5}};struct HfaMember hm={2.5,3.5};struct HfaRecord hr={2.5,3.5};
    if(foreign_record(7,record,3.5)!=13.0)return 1;
    if(foreign_member(7,member,3.5)!=13.0)return 2;
    if(foreign_nested(7,nested,3.5)!=13.0)return 3;
    if(foreign_stack_record(1,2,3,4,5,6,7,8,9,record,10)!=57.5)return 4;
    if(foreign_stack_member(1,2,3,4,5,6,7,8,9,member,10)!=57.5)return 5;
    if(foreign_hfa_member(1,2,3,4,5,6,7,8,9,hm,10)!=61.0)return 6;
    if(foreign_hfa_record(1,2,3,4,5,6,7,8,9,hr,10)!=61.0)return 7;
    if(foreign_return(record)!=3.5)return 8;
    return 0;
}
"#].concat();
    let companion = [layout, r#"
double local_record(int,struct RecordPadding,double);
double local_member(int,struct MemberPadding,double);
double local_nested(int,struct NestedPadding,double);
double local_stack_record(int,int,int,int,int,int,int,int,int,struct RecordPadding,int);
double local_stack_member(int,int,int,int,int,int,int,int,int,struct MemberPadding,int);
double local_hfa_member(double,double,double,double,double,double,double,double,double,struct HfaMember,double);
double local_hfa_record(double,double,double,double,double,double,double,double,double,struct HfaRecord,double);
struct RecordPadding local_return(struct RecordPadding);
double foreign_record(int n,struct RecordPadding v,double t){return local_record(n,v,t);}
double foreign_member(int n,struct MemberPadding v,double t){return local_member(n,v,t);}
double foreign_nested(int n,struct NestedPadding v,double t){return local_nested(n,v,t);}
double foreign_stack_record(int a,int b,int c,int d,int e,int f,int g,int h,int s,struct RecordPadding v,int t){return local_stack_record(a,b,c,d,e,f,g,h,s,v,t);}
double foreign_stack_member(int a,int b,int c,int d,int e,int f,int g,int h,int s,struct MemberPadding v,int t){return local_stack_member(a,b,c,d,e,f,g,h,s,v,t);}
double foreign_hfa_member(double a,double b,double c,double d,double e,double f,double g,double h,double s,struct HfaMember v,double t){return local_hfa_member(a,b,c,d,e,f,g,h,s,v,t);}
double foreign_hfa_record(double a,double b,double c,double d,double e,double f,double g,double h,double s,struct HfaRecord v,double t){return local_hfa_record(a,b,c,d,e,f,g,h,s,v,t);}
double foreign_return(struct RecordPadding v){return local_return(v).x;}
"#].concat();
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "failed case: {:?}", result);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn aligned_members_preserve_layout_and_clang_calls(output_option: &str, optimization: &str) {
    let shared = r#"
struct Layout{char lead;int a __attribute__((aligned(16))),b;char tail;};
struct Declaration{char lead;int a __attribute__((aligned(8))),b __attribute__((aligned(8)));};
struct Small{double x __attribute__((__aligned__(16),unused));};
struct Hfa{double x __attribute__((aligned(16)));double y;};
struct Mixed{char lead;int (*callback)(int) __attribute__((aligned));short row[3] __attribute__((aligned(8)));int small __attribute__((aligned(1)));};
union Choice{char byte;int word __attribute__((aligned(16)));};
struct Combined{struct Layout layout;union Choice choice;};
int add(int x){return x+2;}
"#;
    let code = [
        shared,
        r#"
struct Layout global={1,2,3,4};
struct Combined combined={.layout={5,6,7,8},.choice={.word=9}};
struct Small local_small(int n,struct Small value){value.x+=n;return value;}
struct Hfa local_hfa(struct Hfa value){value.x+=value.y;return value;}
struct Small foreign_small(int,struct Small);
struct Hfa foreign_hfa(struct Hfa);
int check(struct Layout,struct Declaration,struct Mixed *,struct Combined *);
int main(){
    struct Layout array[2]={{1,2,3,4},{5,6,7,8}};
    struct Declaration declaration={1,2,3};
    struct Mixed mixed={1,add,{2,3,4},5};
    struct Small small={2.5};struct Hfa hfa={3.5,4.5};
    small=foreign_small(3,small);hfa=foreign_hfa(hfa);
    return check(global,declaration,&mixed,&combined)
        || small.x!=5.5 || hfa.x!=8.0 || hfa.y!=4.5
        || sizeof(struct Layout)!=32 || _Alignof(struct Layout)!=16
        || __builtin_offsetof(struct Layout,b)!=20
        || sizeof(struct Declaration)!=24 || __builtin_offsetof(struct Declaration,b)!=16
        || sizeof(struct Mixed)!=48 || __builtin_offsetof(struct Mixed,row)!=24
        || __builtin_offsetof(struct Mixed,small)!=32 || sizeof(union Choice)!=16
        || array[1].b!=7 || sizeof(array)!=64;
}
"#,
    ]
    .concat();
    let companion = [shared.replace("int add(int x){return x+2;}",""),r#"
struct Small local_small(int,struct Small);
struct Hfa local_hfa(struct Hfa);
struct Small foreign_small(int n,struct Small value){return local_small(n,value);}
struct Hfa foreign_hfa(struct Hfa value){return local_hfa(value);}
int check(struct Layout value,struct Declaration declaration,struct Mixed *mixed,struct Combined *combined){
    return value.lead!=1 || value.a!=2 || value.b!=3 || value.tail!=4
        || declaration.a!=2 || declaration.b!=3 || sizeof(declaration)!=24
        || mixed->callback(3)!=5 || mixed->row[1]!=3 || mixed->small!=5
        || combined->layout.b!=7 || combined->choice.word!=9
        || sizeof(*mixed)!=48 || sizeof(*combined)!=48;
}
"#.to_owned()].concat();
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "struct S{int x __attribute__((aligned(3)));};",
    "struct S{int x __attribute__((aligned(8192)));};",
    "struct S{int x __attribute__((aligned(0)));};",
    "int n;struct S{int x __attribute__((aligned(n)));};",
    "struct S{int x __attribute__((aligned(8))):3;};",
    "struct S{int __attribute__((aligned(8))) x:3;};",
])]
fn invalid_member_alignment_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn pragma_packed_bitfields_preserve_layout_and_access(output_option: &str, optimization: &str) {
    let definitions = r#"
#pragma pack(push,1)
struct Cross { char a; unsigned int b:32; unsigned long c:48; unsigned short d:8; unsigned char e:8; char tail; };
struct Gap { unsigned char a:3; unsigned long b:64; unsigned int c:1; };
struct Signed { signed long n:7; _Bool b:1; };
struct Tiny { unsigned long n:1; };
struct Zero { char a; unsigned int :0; unsigned int b:1; };
struct Mixed { char pad[7]; unsigned long bit:1; float value; };
union U { unsigned long bits:17; unsigned char bytes[3]; };
#pragma pack(pop)
"#;
    let code = [definitions, r#"
struct Cross global={3,0x11223344U,0x123456789abcUL,0x5a,0xa5,9};
struct Gap initial={5,0xfedcba9876543210UL,1};
int check_cross(struct Cross);
struct Cross return_cross(struct Cross);
int check_gap(struct Gap);
struct Gap return_gap(struct Gap);
int check_mixed(struct Mixed);
int zero_layout(unsigned long,unsigned long);
void *boundary(void);
int main(){
    if(sizeof(struct Cross)!=14 || _Alignof(struct Cross)!=1 || sizeof(struct Gap)!=9) return 1;
    if(check_cross(global) || check_gap(initial)) return 2;
    struct Cross cross=return_cross(global);
    struct Gap gap=return_gap(initial);
    if(cross.b!=0x11223344U || cross.c!=0x123456789abcUL || cross.tail!=9) return 3;
    if(gap.a!=5 || gap.b!=0xfedcba9876543210UL || gap.c!=1) return 4;
    struct Gap array[2]={{1,7,1},{2,9,0}};
    int i=0;
    array[i++].b+=3;
    unsigned long old=array[0].b++;
    if(i!=1 || old!=10 || array[0].b!=11 || array[0].a!=1 || array[0].c!=1 || array[1].b!=9) return 5;
    struct Signed signed_value={-7,1};
    if(signed_value.n++!=-7 || signed_value.n!=-6 || !signed_value.b) return 6;
    union U u={0x12345};
    if(u.bits!=0x12345 || u.bytes[0]!=0x45 || u.bytes[1]!=0x23 || u.bytes[2]!=1) return 7;
    struct Mixed mixed={{0},1,3.5f};
    if(check_mixed(mixed)) return 8;
    if(zero_layout(sizeof(struct Zero),_Alignof(struct Zero))) return 9;
    volatile struct Tiny *last=(volatile struct Tiny*)boundary();
    last->n=1;
    if(last->n++!=1 || last->n!=0) return 10;
    return 0;
}
"#].concat();
    let companion = [definitions, r#"
#include <sys/mman.h>
#include <unistd.h>
#include <stdlib.h>
int check_cross(struct Cross x){return x.a!=3 || x.b!=0x11223344U || x.c!=0x123456789abcUL || x.d!=0x5a || x.e!=0xa5 || x.tail!=9;}
struct Cross return_cross(struct Cross x){return x;}
int check_gap(struct Gap x){return x.a!=5 || x.b!=0xfedcba9876543210UL || x.c!=1;}
struct Gap return_gap(struct Gap x){return x;}
int check_mixed(struct Mixed x){return x.bit!=1 || x.value!=3.5f;}
int zero_layout(unsigned long size,unsigned long alignment){return size!=sizeof(struct Zero) || alignment!=_Alignof(struct Zero);}
void *boundary(void){long size=sysconf(_SC_PAGESIZE);char *p=mmap(0,size*2,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0);if(p==MAP_FAILED || mprotect(p+size,size,PROT_NONE))abort();return p+size-1;}
"#].concat();
    if output_option == "llvm" {
        let ir = dynamite_compiler::generate(&code, output_option, false);
        assert!(ir.contains("load volatile i8"));
        assert!(ir.contains("store volatile i8"));
        assert!(!ir.contains("load volatile i64"));
        assert!(!ir.contains("store volatile i64"));
    }
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
}

#[test_matrix([2,4,16], ["asm", "llvm"])]
fn pragma_packed_bitfield_alignment_limits(limit: usize, output_option: &str) {
    let definitions = format!(
        "#pragma pack(push,{limit})\nstruct S{{char a;unsigned int b:32;unsigned long c:48;char tail;}};\n#pragma pack(pop)\n"
    );
    let code = format!(
        "{definitions}int check(struct S,unsigned long,unsigned long);int main(){{struct S x={{7,0x81234567U,0x123456789abcUL,9}};return check(x,sizeof(x),_Alignof(struct S));}}"
    );
    let companion = format!(
        "{definitions}int check(struct S x,unsigned long size,unsigned long alignment){{return x.a!=7||x.b!=0x81234567U||x.c!=0x123456789abcUL||x.tail!=9||size!=sizeof(x)||alignment!=_Alignof(struct S);}}"
    );
    assert_eq!(
        compile_and_run_with_c(&code, output_option, Some(&companion))
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn incomplete_external_records_allow_address_uses(output_option: &str, optimization: &str) {
    let code = r#"
struct Opaque;union Variant;typedef struct Hidden Hidden;
extern struct Opaque object;extern union Variant variant;extern const Hidden hidden;
struct Opaque *saved=&object;union Variant *saved_union=&variant;const Hidden *saved_const=&hidden;
int check(const void*,const void*,const void*,const void*);
struct Block;
int block(){extern struct Block block_object;return check(saved,saved_union,saved_const,&block_object);}
int main(){return saved!=&object || saved_union!=&variant || saved_const!=&hidden || block();}
"#;
    let companion = r#"
struct Opaque{long values[3];};union Variant{long number;double real;};
struct Hidden{int value;};struct Block{char bytes[5];};
struct Opaque object={{1,2,3}};union Variant variant={4};
const struct Hidden hidden={5};struct Block block_object={{6}};
int check(const void *a,const void *b,const void *c,const void *d){return a!=&object || b!=&variant || c!=&hidden || d!=&block_object;}
"#;
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn incomplete_external_records_observe_later_completion(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
extern struct Later later;struct Later *saved=&later;
extern union Choice choice;union Choice *saved_union=&choice;
struct Later{long value;char tail;};union Choice{long number;double real;};
extern struct Later later;struct Later later={23,7};union Choice choice={31};
int main(){struct Later copy=later;return saved->value!=23 || saved->tail!=7 || copy.value!=23 || sizeof(later)!=16 || saved_union->number!=31 || sizeof(choice)!=8;}
"#;
    let result = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "extern void object;",
    "extern struct S array[2];",
    "extern struct S object;struct S object;",
    "int main(){static struct S object;return 0;}",
    "extern struct S object;int main(){return sizeof(object);}",
    "extern union U object;int main(){return object.missing;}",
    "extern struct S left,right;int main(){left=right;return 0;}",
    "extern struct S object;void consume(struct S);int main(){consume(object);return 0;}",
    "extern union U object;void consume(int,...);int main(){consume(0,object);return 0;}",
])]
fn invalid_incomplete_external_record_uses_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn gnu_packed_records_preserve_layout_and_clang_calls(output_option: &str, optimization: &str) {
    let declarations = r#"
struct __attribute__((packed)) P{char a;long b;char c;};
struct M{char a;long b __attribute__((aligned(8)));char c;} __attribute__((__packed__));
#pragma pack(push,2)
struct __attribute__((packed)) Q{char a;long b __attribute__((aligned(8)));char c;};
#pragma pack(pop)
struct Inner{char a;long b;};
struct Outer{char lead;struct Inner inner;short tail;} __attribute__((packed));
union __attribute__((packed)) U{char byte;long number;};
struct __attribute__((packed,aligned(16))) A{char byte;long number;};
struct F{char tag;int length;char data[];} __attribute__((packed));
"#;
    let code = format!(
        r#"{declarations}
struct P global={{1,23,4}};long *saved=&global.b;
struct P array[2]={{{{2,61,3}},{{4,67,5}}}};
union U global_union={{.number=71}};
struct P echo(struct P);struct Outer echo_outer(struct Outer);union U echo_union(union U);struct A echo_aligned(struct A);int inspect(struct P*,struct M*,struct Q*);
struct P from_generated(struct P p){{p.b+=5;return p;}}
int main(){{
if(sizeof(struct P)!=10 || _Alignof(struct P)!=1 || (char*)&global.b-(char*)&global!=1 || *saved!=23)return 1;
struct M m={{2,31,5}};struct Q q={{3,37,6}};
if(sizeof(m)!=24 || _Alignof(struct M)!=8 || (char*)&m.b-(char*)&m!=8)return 2;
if(sizeof(q)!=12 || _Alignof(struct Q)!=2 || (char*)&q.b-(char*)&q!=2)return 3;
if(inspect(&global,&m,&q))return 4;
struct P p=echo(global);if(p.a!=1 || p.b!=33 || p.c!=4)return 5;
struct Outer o={{7,{{8,41}},9}};o=echo_outer(o);
if(sizeof(o)!=19 || _Alignof(struct Outer)!=1 || (char*)&o.inner-(char*)&o!=1 || o.inner.b!=43 || o.tail!=9)return 6;
union U u={{47}};u.number=47;u=echo_union(u);if(sizeof(u)!=8 || _Alignof(union U)!=1 || u.number!=50)return 7;
struct A a={{3,53}};a=echo_aligned(a);if(sizeof(a)!=16 || _Alignof(struct A)!=16 || a.number!=57)return 8;
if(sizeof(struct F)!=5 || _Alignof(struct F)!=1)return 9;
if(array[0].b!=61 || array[1].b!=67 || (char*)&array[1]-(char*)&array[0]!=10 || global_union.number!=71)return 10;
char storage[8]={{0}};struct F *f=(struct F*)(storage+1);f->tag=2;f->length=3;f->data[0]=7;
if(storage[1]!=2 || storage[6]!=7 || f->length!=3)return 11;
return 0;
}}
"#
    );
    let companion = format!(
        r#"{declarations}
struct P from_generated(struct P);
struct P echo(struct P p){{p=from_generated(p);p.b+=5;return p;}}
struct Outer echo_outer(struct Outer p){{p.inner.b+=2;return p;}}
union U echo_union(union U p){{p.number+=3;return p;}}
struct A echo_aligned(struct A p){{p.number+=4;return p;}}
int inspect(struct P*p,struct M*m,struct Q*q){{return p->b!=23 || m->b!=31 || q->b!=37;}}
"#
    );
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn zero_length_arrays_are_complete_without_inferred_elements(
    output_option: &str,
    optimization: &str,
) {
    let code = r#"
extern long zero[];long zero[0];extern long zero[];long *saved=zero;
static int empty[0]={};const int readonly[0]={};char empty_text[0]="";
extern int foreign[];int same_address(long*);
int inferred[]={2,5,7};char text[]="abc";
int first(int values[0]){return values[0];}
int main(){int local[1-1]={};char local_text[0]="";int actual[1]={11};long wide_zero[0];char narrow_zero[0];
if(!narrow_zero || ((unsigned long)wide_zero&7))return 6;
if(sizeof(zero)!=0 || sizeof(empty)!=0 || sizeof(readonly)!=0 || sizeof(empty_text)!=0 || sizeof(local)!=0 || sizeof(local_text)!=0)return 1;
if(_Alignof(int[0])!=4 || sizeof(int[2][0])!=0 || sizeof(int[0][3])!=0)return 2;
if(sizeof(inferred)!=12 || inferred[2]!=7 || sizeof(text)!=4 || text[3]!=0)return 3;
if(__builtin_object_size(zero,0)!=0 || __builtin_object_size(local,1)!=0 || __builtin_object_size(foreign,0)!=(unsigned long)-1)return 4;
if(same_address(saved) || foreign[1]!=13 || first(actual)!=11)return 5;
return _Generic(&zero,long(*)[0]:0,default:1);
}
"#;
    let companion = "int foreign[2]={3,13};extern long zero[0];int same_address(long *value){return value!=zero;}";
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn zero_length_record_members_preserve_layout_and_clang_calls(
    output_option: &str,
    optimization: &str,
) {
    let declarations = r#"
struct Tail{char tag;long count;int values[0];};
struct Middle{int prefix;long zero[0];char after;};
struct Packed{char tag;long zero[0];short after;} __attribute__((packed));
"#;
    let code = format!(
        r#"{declarations}
#include <stdlib.h>
struct Tail global={{3,7,{{}}}};struct Middle middle={{1,{{}},2}};struct Packed packed={{4,{{}},5}};
struct Tail echo(struct Tail);int inspect(struct Middle*,struct Packed*);
int main(){{
if(sizeof(struct Tail)!=16 || _Alignof(struct Tail)!=8 || sizeof(global.values)!=0 || (char*)global.values-(char*)&global!=16)return 1;
if(sizeof(struct Middle)!=16 || _Alignof(struct Middle)!=8 || (char*)&middle.after-(char*)&middle!=8)return 2;
if(sizeof(struct Packed)!=3 || _Alignof(struct Packed)!=1 || (char*)&packed.after-(char*)&packed!=1)return 3;
if(inspect(&middle,&packed))return 4;
struct Tail value=echo(global);if(value.tag!=3 || value.count!=10)return 5;
struct Tail *p=malloc(sizeof(struct Tail)+2*sizeof(int));p->tag=6;p->count=2;p->values[0]=17;p->values[1]=19;
struct Tail copy=*p;int bad=copy.tag!=6 || copy.count!=2 || p->values[0]+p->values[1]!=36;free(p);
if(bad || __builtin_object_size(global.values,1)!=0)return 6;
return 0;
}}
"#
    );
    let companion = format!(
        r#"{declarations}
struct Tail echo(struct Tail value){{value.count+=3;return value;}}
int inspect(struct Middle *m,struct Packed *p){{return sizeof(*m)!=16 || sizeof(*p)!=3 || m->prefix!=1 || m->after!=2 || p->tag!=4 || p->after!=5;}}
"#
    );
    let result =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn zero_length_members_allow_empty_record_values(output_option: &str, optimization: &str) {
    let code = r#"
#include <stdarg.h>
struct Empty{int values[0];};int calls;struct Empty global;
struct Empty generated(struct Empty value){calls++;return value;}
long variadic(int tag,...){va_list ap;va_start(ap,tag);long before=va_arg(ap,long);struct Empty value=va_arg(ap,struct Empty);long after=va_arg(ap,long);va_end(ap);return before+after+sizeof(value);}
long foreign(long,struct Empty,long);
int main(){struct Empty value={{}};if(sizeof(value)!=0)return 1;if(sizeof(global)!=0)return 2;if(_Alignof(struct Empty)!=4)return 3;long result=foreign(7,generated(value),11);if(result!=18)return 4;if(calls!=2)return 5;if(variadic(0,7l,generated(value),11l)!=18 || calls!=3)return 6;if(sizeof(generated(value))!=0 || calls!=3)return 7;return 0;}
"#;
    let companion = "struct Empty{int values[0];};struct Empty generated(struct Empty);long variadic(int,...);long foreign(long before,struct Empty value,long after){struct Empty result=generated(value);return variadic(0,before,result,after);}";
    let result =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(result.status.code(), Some(0), "{:?}", result);
}

#[test_matrix([
    "extern int values[];int main(){return sizeof(values);}",
    "int values[]={};",
    "int values[0]={1};",
    "int main(){int values[0]={1};return 0;}",
    "extern int values[0];extern int values[2];",
    "int first(int values[static 0]);",
    "int main(){int values[-1];return 0;}",
    "struct S{int n;int values[];int after;};",
    "int (*left)[0];int (*right)[2];int main(){left=right;return 0;}",
])]
fn invalid_zero_length_array_contexts_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn incomplete_record_prototypes_complete_before_use(output_option: &str, optimization: &str) {
    let code = r#"
struct Future;
struct Future add(struct Future);
typedef struct Future (*Callback)(struct Future);
struct Unused;
void unused(struct Unused);
struct Unused result(void);
struct Future { long x,y; };
struct Future add(struct Future value){value.x+=2;value.y+=3;return value;}
struct Future foreign(struct Future);
int main(void){Callback call=add;struct Future a={3,5};struct Future b=call(a);
    struct Future c=foreign(b);return c.x+c.y-17;}
"#;
    let companion =
        "struct Future{long x,y;};struct Future foreign(struct Future v){v.x++;v.y+=3;return v;}";
    let output =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "struct S;int f(struct S x){return 0;}",
    "struct S;struct S f(void){for(;;){}}",
    "struct S;struct S f(void);int main(){f();return 0;}",
    "struct S;void f(struct S);int main(){f(0);return 0;}",
])]
fn incomplete_records_in_definitions_and_calls_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn interspersed_type_qualifiers_preserve_types((output_option, optimization): (&str, &str)) {
    let code = r#"
unsigned const char a=255;
short volatile unsigned int b=65535;
long const unsigned long int c=18446744073709551615ULL;
signed const char d=-1;
const int volatile unsigned e=7;
const double volatile f=3.5;
typedef unsigned const short int Word;
unsigned const __int128 wide;
_Static_assert(__builtin_types_compatible_p(__typeof__(&a),const unsigned char*),"char qualification");
_Static_assert(__builtin_types_compatible_p(__typeof__(&b),volatile unsigned short*),"short qualification");
_Static_assert(__builtin_types_compatible_p(__typeof__(c),unsigned long long),"long long identity");
_Static_assert(__builtin_types_compatible_p(__typeof__(d),signed char),"signed char identity");
_Static_assert(sizeof(wide)==16,"integer128 storage");
int read_byte(unsigned const char *p){return *p;}
int main(void){Word n=9;int value=read_byte((unsigned const char*)&a);b=5;
    return value!=255 || b!=5 || c!=~0ULL || d!=-1 || e!=7 || f!=3.5 || n!=9;}
"#;
    let output = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn interspersed_long_double_qualifiers((output_option, optimization): (&str, &str)) {
    let code = "long const double a=1.5L;double volatile long b=2.5L;int main(){return a+b!=4.0L;}";
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test_matrix([
    "int main(){unsigned const char x=1;x=2;return 0;}",
    "long const long long x;",
    "unsigned volatile signed int x;",
    "short const char x;",
    "double const unsigned x;",
    "long volatile float x;",
    "int restrict unsigned x;",
])]
fn invalid_interspersed_type_specifiers_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([
    "typedef int Scalar __attribute__((transparent_union));",
    "struct __attribute__((transparent_union)) S{int value;};",
    "union __attribute__((transparent_union)) U{int integer;double real;};",
    "union __attribute__((transparent_union)) U{int integer;long wide;};",
    "union __attribute__((transparent_union)) U{int *pointer;long integer;};",
    "union __attribute__((transparent_union)) U{_Bool value;unsigned char byte;};",
    "union __attribute__((transparent_union)) U{signed char first;unsigned char second;};",
    "union __attribute__((transparent_union)) U{int bits:3;int integer;};",
    "union __attribute__((transparent_union)) U{int array[2];};",
    "union __attribute__((transparent_union)) U;",
    "typedef union U{int *p;}U __attribute__((transparent_union(1)));",
    "typedef union U{int *p;}U __attribute__((transparent_union));int accept(U v){return 0;}int main(){double x;return accept(&x);}",
    "typedef union U{int *p;}U __attribute__((transparent_union));int accept(U v){return 0;}int main(){const int x=3;return accept(&x);}",
    "typedef union U{int *p;}U __attribute__((transparent_union));int main(){int x;U value;value=&x;return 0;}",
])]
fn invalid_or_unsupported_transparent_unions_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn transparent_union_integer_members_preserve_bit_patterns(
    output_option: &str,
    optimization: &str,
) {
    let definition = "typedef union __attribute__((transparent_union)) Bits{int signed_value;unsigned unsigned_value;}Bits;";
    let code = format!(
        r#"{definition}
unsigned ours(Bits value){{return value.unsigned_value;}}
unsigned foreign(Bits);
int main(void){{unsigned high=4000000000U;unsigned (*callback)(unsigned)=ours;
    return ours(high)!=high || foreign(high)!=high || callback(high)!=high;}}
"#
    );
    let companion = format!(
        "{definition}\nunsigned ours(Bits);unsigned foreign(Bits value){{return ours(value);}}"
    );
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

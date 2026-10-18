use crate::support::{assert_frontend_error, compile_and_run, compile_and_run_with_c_flags};
use test_case::test_matrix;

#[test_matrix([
    "int a[3];",
    "int a[4] = {};",
    "int a[3] = {7};",
    "char a[3] = {7};",
    "int a[2][3] = {{7}, {}};",
], ["asm", "llvm"])]
fn it_zero_fills_global_arrays(declaration: &str, output_option: &str) {
    let code = format!("{} int main() {{ return 0; }}", declaration);
    let output = compile_and_run(&code, output_option);
    assert_eq!(output.status.code(), Some(0), "{:?}", output);
}

#[test_matrix([
    ("int main() { int a[] = {2, 5, 7,}; return sizeof(a) + a[2]; }", 19),
    ("int a[][2] = {{3}, {4, 5}}; int main() { return sizeof(a) + a[1][1]; }", 21),
    ("int main() { int a[][2] = {{3}, {4, 5}}; return sizeof(a) + a[1][1]; }", 21),
    ("int sum(int a[]) { return a[0] + a[1]; } int main() { int a[] = {2, 5}; return sum(a); }", 7),
], ["asm", "llvm"])]
fn it_infers_array_sizes(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    "int main(){char s[]=\"abc\";return sizeof(s)+s[3]-4;}",
    "char s[]=\"abc\"; int main(){return sizeof(s)+s[3]-4;}",
    "int main(){char s[3]=\"abc\";return s[2]-99;}",
    "char s[5]=\"abc\"; int main(){return s[3]+s[4];}",
], ["asm", "llvm"])]
fn it_initializes_character_arrays(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int x=7; int *p=&x; int main(){return *p-7;}",
    "int a[3]={1,2,7}; int *p=a+2; int main(){return *p-7;}",
    "int a[3]={1,2,8}; int *p=&a[2]; int main(){return *p-8;}",
    "char *p=\"abc\"+1; int main(){return *p-98;}",
], ["asm", "llvm"])]
fn it_initializes_global_addresses(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn braced_string_array_initializers((output_option, optimization): (&str, &str)) {
    let code = r#"
char global[] = { "ab" "c", };
unsigned char exact[3] = { "abc" };
char padded[6] = { "ab" };
char *pointers[] = { "abc" };
struct S { char text[4]; int value; } record = { { "abc" }, 7 };
int wide[] = { L"xy", };
unsigned short utf16[] = { u"xy" };
int main() {
    char local[] = { "abc" };
    static char stored[] = { "abc", };
    char empty[] = { "" };
    char bounded[5] = { "ab", };
    char *local_pointers[] = { "abc" };
    struct S nested = { .text = { "xyz", }, .value = 9 };
    if (sizeof(global) != 4 || sizeof(local) != 4 || sizeof(stored) != 4) return 1;
    if (global[2] != 'c' || global[3] || local[2] != 'c' || stored[3]) return 2;
    if (exact[2] != 'c' || padded[2] || padded[5] || bounded[4]) return 3;
    if (sizeof(empty) != 1 || empty[0]) return 4;
    if (sizeof(pointers) != sizeof(void*) || sizeof(local_pointers) != sizeof(void*)) return 5;
    if (pointers[0][2] != 'c' || local_pointers[0][0] != 'a') return 6;
    if (record.text[2] != 'c' || record.value != 7 || nested.text[2] != 'z' || nested.value != 9) return 7;
    if (sizeof(wide) != 12 || wide[1] != 'y' || wide[2]) return 8;
    if (sizeof(utf16) != 6 || utf16[1] != 'y' || utf16[2]) return 9;
    if (sizeof((char[]){ "abc" }) != 4 || ((char[]){ "xy" })[1] != 'y') return 10;
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
    "char a[2] = { \"abc\" }; int main(){return 0;}",
    "int main(){char a[2] = { \"abc\" };return 0;}",
    "char a[] = { \"ab\", 1 };int main(){return 0;}",
    "int main(){char a[] = { \"ab\", 1 };return 0;}"
])]
fn invalid_braced_string_array_initializers(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([
    "int main(){int a[4]={[2]=7,[0]=3};return a[0]+a[1]+a[2]+a[3]-10;}",
    "int a[]={[3]=7,9};int main(){return sizeof(a)+a[0]+a[4]-29;}",
    "struct S{int x;int y;};struct S s={.y=7,.x=3};int main(){return s.x+s.y-10;}",
    "struct S{int a[3];int x;};int main(){struct S s={.a[2]=7,.x=9};return s.a[0]+s.a[2]+s.x-16;}",
], ["asm", "llvm"])]
fn it_supports_designated_initializers(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "union Wide{int x;long y;};union Wide g={.y=0x1122334455667788L};int main(){union Wide s={.y=0x1122334455667788L};return g.y!=s.y;}",
    "union Selected{int x;struct{int y;int z;};};union Selected g={.x=7,.z=3};int main(){union Selected s={.x=7,.z=3};return g.y!=0 || g.z!=3 || s.y!=0 || s.z!=3;}",
    "int value=7;union Pointer{long number;struct{int *p;};};union Pointer g={.p=&value};int main(){return *g.p-7;}",
    "struct Bits{union{struct{unsigned x:3;unsigned y:5;};int raw;};};struct Bits g={.x=5,.y=17};int main(){struct Bits s={.x=5,.y=17};s.x++;return g.x!=5 || g.y!=17 || s.x!=6 || s.y!=17;}",
], ["asm", "llvm"])]
fn selected_union_initializers(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn global_byte_sized_integer_initializers(output_option: &str) {
    let code = "unsigned char table[4]={0,127,128,255}; signed char negative=-7; _Bool enabled=1; _Bool bool_values[2]={0,1}; unsigned short wide=513; int main(){return table[0]+table[1]+table[2]+table[3]+negative+enabled+bool_values[0]+bool_values[1]+wide-1018;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn static_objects_are_in_scope_in_their_initializers((output_option, optimization): (&str, &str)) {
    let code = r#"
struct Link { struct Link *next; long size; };
static struct Link root = { &root, sizeof(root) };
static int array[3] = { sizeof(array), 7, 9 };
static int *element = &array[1];
static void *self = &self;
static int first = sizeof(first), second = sizeof(first)+sizeof(second);
char shadow;
int check(void) {
    static struct Link local = { &local, sizeof(local) };
    static long shadow = sizeof(shadow);
    static int inferred[] = { 3, 5, 7 };
    static void *pointer = &pointer;
    return local.next == &local && local.size == sizeof(local) && shadow == sizeof(long)
        && sizeof(inferred) == 3*sizeof(int) && pointer == &pointer;
}
int main(void) {
    return !(root.next == &root && root.size == sizeof(root) && array[0] == sizeof(array)
        && *element == 7 && self == &self && first == sizeof(int)
        && second == 2*sizeof(int) && check());
}
"#;
    let output = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "static int a[sizeof(a)];",
    "static int incomplete[]={sizeof(incomplete)};",
    "int main(){static int a[sizeof(a)];return 0;}",
    "int main(){static int incomplete[]={sizeof(incomplete)};return 0;}",
    "_Thread_local void *p=&p;",
    "int main(){static _Thread_local void *p=&p;return 0;}",
])]
fn invalid_static_self_initializers_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test]
fn thread_local_self_size_initializers_remain_constant() {
    let code = "_Thread_local long size=sizeof(size);int main(){static _Thread_local int local=sizeof(local);return size!=sizeof(long)||local!=sizeof(int);}";
    assert_eq!(compile_and_run(code, "llvm").status.code(), Some(0));
}

#[test_matrix([
    "static int a[][2]={1,2,3,4,5};int evaluate(void){return sizeof(a)+a[0][0]+a[1][1]+a[2][0]+a[2][1];}",
    "static unsigned long count=sizeof((int[][2]){1,2,3,4,5});static int (*p)[2]=(int[][2]){2,3,4};int evaluate(void){return count+p[0][1]+p[1][0]+p[1][1];}",
    "struct Pair{int x,y;};struct Box{struct Pair p[2];int tail;};int evaluate(void){struct Box b={1,2,3,4,5};struct Box z={0};return b.p[0].x+3*b.p[0].y+5*b.p[1].x+7*b.p[1].y+11*b.tail+z.p[1].y;}",
    "struct Box{int a[3];int tail;};static struct Box g={.a[1]=4,5,6};int evaluate(void){int a[2][3]={[0][1]=4,5,6};struct Box b={.a[1]=4,5,6};return a[0][0]+a[0][1]+3*a[0][2]+5*a[1][0]+7*b.a[2]+11*b.tail+13*g.tail;}",
    "struct Pair{int x,y;};int evaluate(void){struct Pair first={3,5};struct Pair a[2]={first,7,9};return a[0].x+3*a[0].y+5*a[1].x+7*a[1].y;}",
    "union U{struct{unsigned a:3,b:4;}pair;long raw;};static union U g={3,7};int evaluate(void){union U a[2]={1,2,3,4};return g.pair.a+3*g.pair.b+5*a[0].pair.a+7*a[0].pair.b+11*a[1].pair.a+13*a[1].pair.b;}",
    "static char words[][4]={\"a\",\"bc\"};struct S{char text[4];int n;};int evaluate(void){struct S s={\"ab\",7};char local[2][4]={\"c\",\"de\"};return sizeof(words)+words[1][1]+local[1][1]+s.text[1]+s.n;}",
    "struct S{int a[2];int b;};int evaluate(void){int n=0;struct S s={++n,++n,++n};struct S t={{4},5};return n+3*s.a[0]+5*s.a[1]+7*s.b+11*t.a[0]+13*t.a[1]+17*t.b;}",
], ["asm", "llvm"], ["-O0", "-O2"])]
fn aggregate_brace_elision_matches_clang(body: &str, output_option: &str, optimization: &str) {
    let companion = body.replace("int evaluate(void)", "int reference(void)");
    let code =
        format!("{body}\nint reference(void);int main(void){{return evaluate()!=reference();}}");
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "int a[1][2]={1,2,3};int main(){return 0;}",
    "int main(){int a[1][2]={1,2,3};return 0;}",
    "union U{int x;long y;};union U u={1,2};int main(){return 0;}",
])]
fn invalid_brace_elision_is_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([
    "int evaluate(void){int n=0;int a[][2]={++n,++n,++n,++n,++n};_Static_assert(sizeof(a)==6*sizeof(int),\"bound\");return sizeof(a)+n+a[2][0]+a[2][1];}",
    "struct S{int x,y;};int evaluate(void){struct S zero[]={0};struct S a[]={1,2,3};struct S b[]={[2].x=7,9,11};return sizeof(zero)+sizeof(a)+sizeof(b)+zero[0].y+a[1].y+b[2].y+b[3].x+b[3].y;}",
    "int evaluate(void){char words[][4]={\"a\",\"bc\"};char text[]={\"xy\"};return sizeof(words)+sizeof(text)+words[1][1]+text[1]+text[2];}",
    "int evaluate(void){int a[]={((int[]){3,5})[1],((int[][2]){7,9,11})[1][0]};int b[][2]={((int[]){13,17})[0],19,23};return sizeof(a)+sizeof(b)+a[0]+a[1]+b[0][0]+b[1][0]+b[1][1];}",
    "int evaluate(void){void *p[]={p,&p[1]};return sizeof(p)+((void*)p==p[0])+((void*)&p[1]==p[1]);}",
    "typedef int Row[2];int evaluate(void){Row a[]={1,2,3};int b[][2][2]={4,5,6,7,8};return sizeof(a)+sizeof(b)+a[1][1]+b[1][0][0]+b[1][1][1];}",
    "int evaluate(void){int n=0;unsigned long size=sizeof((int[][2]){++n,++n,++n});return size+n;}",
    "struct S{int x,y;};int evaluate(void){struct S first={3,5};struct S a[]={first,7,9};return sizeof(a)+a[0].y+a[1].x+a[1].y;}",
], ["asm", "llvm"], ["-O0", "-O2"])]
fn inferred_local_array_initializers_match_clang(
    body: &str,
    output_option: &str,
    optimization: &str,
) {
    let companion = body.replace("int evaluate(void)", "int reference(void)");
    let code =
        format!("{body}\nint reference(void);int main(void){{return evaluate()!=reference();}}");
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "int main(){int a[];return 0;}",
    "int main(){int a[]={sizeof(a)};return 0;}",
    "int main(){long a;{int a[]={sizeof(a)};}return 0;}",
    "int main(){int other[2];int a[]=other;return 0;}",
])]
fn invalid_inferred_local_arrays_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn integer_address_initializers_preserve_relocations(output_option: &str, optimization: &str) {
    let code = r#"
int values[3]={3,5,7};
int function(int x){return x+2;}
static long base=(long)&values;
static unsigned long second=(unsigned long)&values[1];
static long byte_offset=(long)values+sizeof(int);
static long commuted=sizeof(int)+(long)values;
static long back=(long)&values[2]-sizeof(int);
static long function_address=(long)function;
struct Record{long address;long tag;};
static struct Record record={(long)&values[2],9};
union Storage{long integer;void *pointer;};
static union Storage storage={(long)&values[1]};
static long strings[2]={(long)"abc",(long)("def"+1)};
int foreign(long,long);
int main(void){static long local=(long)&values[2];
    if(base!=(long)values || second!=(unsigned long)&values[1] || byte_offset!=(long)&values[1]
        || commuted!=byte_offset || back!=byte_offset || *(int*)local!=7)return 1;
    if(((int(*)(int))function_address)(3)!=5 || *(int*)record.address!=7 || record.tag!=9)return 2;
    if(*(int*)storage.pointer!=5 || ((char*)strings[0])[1]!='b' || ((char*)strings[1])[0]!='e')return 3;
    return foreign(base,second);}
"#;
    let companion = "extern int values[3];int foreign(long a,long b){return a!=(long)values||b!=(long)&values[1];}";
    let output =
        compile_and_run_with_c_flags(code, output_option, Some(companion), &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "int object;long address=&object;",
    "int object;int address=(int)&object;",
    "int object;long address=(long)(int)&object;",
    "int object;float address=(long)&object;",
    "int object;int *address=(long)&object;",
    "_Thread_local int object;long address=(long)&object;",
])]
fn invalid_integer_address_initializers_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn aggregate_zero_fill_preserves_storage_and_explicit_initializers(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
int check(unsigned long*, unsigned long);
int calls;
int next(void){return ++calls;}
struct __attribute__((packed)) Packed {char tag; double value; char tail[3];};
union Choice {unsigned long words[3];char bytes[25];};
int main(void){
    unsigned long values[4099]={[4097]=next(),[2]=next()};
    if(calls!=2 || values[4097]!=1 || values[2]!=2)return 1;
    values[4097]=0;values[2]=0;
    if(check(values,4099))return 2;
    for(int pass=0;pass<2;pass++){
        struct Packed p={0};
        union Choice u={0};
        unsigned char *a=(unsigned char*)&p,*b=(unsigned char*)&u;
        for(unsigned long i=0;i<sizeof(p);i++){if(a[i])return 3;a[i]=255;}
        for(unsigned long i=0;i<sizeof(u);i++){if(b[i])return 4;b[i]=255;}
    }
    double numbers[2][3]={{0}};
    volatile int *pointers[3]={0};
    return numbers[1][2]!=0.0 || pointers[2]!=0;
}
"#;
    let output = compile_and_run_with_c_flags(
        code,
        output_option,
        Some(
            "int check(unsigned long *p,unsigned long n){for(unsigned long i=0;i<n;i++)if(p[i])return 1;return 0;}",
        ),
        &[optimization],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn aggregate_zero_fill_keeps_llvm_output_compact_and_volatile_stores_typed() {
    let generated = dynamite_compiler::generate(
        "void consume(unsigned long*);void fill(void){unsigned long a[16384]={0};consume(a);}",
        "llvm",
        false,
    );
    assert_eq!(
        generated.matches("call void @llvm.memset").count(),
        1,
        "{generated}"
    );
    assert!(generated.contains("i64 131072, i1 false"), "{generated}");
    assert!(
        generated.len() < 5000,
        "large initializer expanded to {} bytes",
        generated.len()
    );
    let generated = dynamite_compiler::generate(
        "struct V{volatile int a[2];};int main(){volatile int a[3]={0};struct V v={0};return a[2]+v.a[1];}",
        "llvm",
        false,
    );
    assert!(!generated.contains("call void @llvm.memset"), "{generated}");
    assert!(
        generated.matches("store volatile i32").count() >= 5,
        "{generated}"
    );
}

#[test_matrix([
    "struct Shape { char tag; long value; };",
    "struct Shape { void *p[7]; unsigned flag:1; };",
    "struct Inner { char tag; long value; }; struct Shape {char start;struct Inner inner;unsigned first:3,second:5;};",
    "struct Shape {unsigned first:1;unsigned :0;unsigned second:3;char tail;};",
    "#pragma pack(push,1)\nstruct Shape {char tag;unsigned first:3,second:9;char tail;};\n#pragma pack(pop)",
    "struct __attribute__((aligned(16))) Shape {char tag;unsigned flag:1;};",
], [("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn record_zero_initialization_clears_padding(
    code: &str,
    (output_option, optimization): (&str, &str),
) {
    let code = format!(
        r#"{code}
int main(void){{
    for(int pass=0;pass<2;pass++){{
        struct Shape *value=&(struct Shape){{0}};
        unsigned char *bytes=(unsigned char*)value;
        for(unsigned long i=0;i<sizeof(*value);i++)if(bytes[i])return 1;
        for(unsigned long i=0;i<sizeof(*value);i++)bytes[i]=255;
    }}
    return 0;
}}
"#
    );
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn record_zero_initialization_retains_volatile_member_stores() {
    let generated = dynamite_compiler::generate(
        "struct V{char tag;volatile int field;};int main(){struct V v={0};return v.field;}",
        "llvm",
        false,
    );
    assert_eq!(
        generated.matches("store volatile i32").count(),
        1,
        "{generated}"
    );
}

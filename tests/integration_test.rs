use std::{
    fs::{self, remove_file},
    io::Write,
    process::{Command, Output},
};
extern crate dynamite_compiler;
extern crate rand;
use rand::distributions::{Alphanumeric, DistString};
use test_case::test_matrix;

#[test_matrix(["asm", "llvm"])]
fn it_defines_one_global_variable(output_option: &str) {
    let code = "int a = 7; int main() { printf(\"%d\", a); return 0; }";
    let got = compile_and_get_stdout(code, output_option);
    assert_eq!(got, "7")
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_simple_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/simple.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    assert_eq!(got, "12, 2, 35, 5, 7\n")
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_simple_function_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/simple_function.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    assert_eq!(got, "10\n")
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_character_constant_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/character_constant.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    assert_eq!(got, "k, ', \\, \n, %\n\t")
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_string_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/string.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    assert_eq!(got, "a = 777, b = 755, c = 222\n")
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_simple_expr_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/simple_expr.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    got.split("\n")
        .filter(|s| !s.is_empty() && !s.ends_with("OK"))
        .for_each(|s| panic!("assertion failed:\n  {}\n", s));
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_expr_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/expr.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    got.split("\n")
        .filter(|s| !s.is_empty() && !s.ends_with("OK"))
        .for_each(|s| panic!("assertion failed:\n  {}\n", s));
}

#[test_matrix(["asm", "llvm"])]
fn it_compiles_many_functions_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/many_functions.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    got.split("\n")
        .filter(|s| !s.is_empty() && !s.ends_with("OK"))
        .for_each(|s| panic!("assertion failed:\n  {}\n", s));
}

fn compile_and_get_stdout(code: &str, output_option: &str) -> String {
    let output = compile_and_run(code, output_option);
    assert!(output.status.success(), "program failed: {:?}", output);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test_matrix([
    ("int main() { if (1) return 7; return 3; }", 7),
    ("int main() { int a[2]; if (1) a[0] = 6; a[1] = 8; return a[0] + a[1]; }", 14),
    ("int main() { if (0) return 7; return 3; }", 3),
    ("int main() { int x = -2; if (x) return 7; else return 3; }", 7),
    ("int main() { int x = 0; if (x) return 7; else return 3; }", 3),
    ("char x = 2; int main() { if (x) return 7; return 3; }", 7),
    ("int *p; int main() { if (p) return 7; return 3; }", 3),
    ("int main() { if (\"yes\") return 7; return 3; }", 7),
    ("int main() { if (1 < 2) return 7; return 3; }", 7),
    ("int main() { if (2 < 1) return 7; return 3; }", 3),
    ("int main() { if (1) { if (0) return 2; else return 7; } else return 3; }", 7),
    ("int main() { int x = 0; if (1) x = 7; else x = 3; return x; }", 7),
    ("int main() { int x = 0; if (0) x = 7; else x = 3; return x; }", 3),
    ("int main() { if (0) return 7; }", 0),
    ("char a = 2; char b = 3; int main() { if (a < b) return a < b; return 3; }", 1),
    ("int main() { return (1 < 2) + (3 == 3); }", 2),
    ("int main() { if ((1 < 2) == (3 < 4)) return 7; return 3; }", 7),
    ("int main() { return 7; printf(\"unreachable\"); }", 7),
], ["asm", "llvm"])]
fn llvm_if_returns_expected_status(case: (&str, i32), output_option: &str) {
    let (code, expected) = case;
    let output = compile_and_run(code, output_option);
    assert_eq!(output.status.code(), Some(expected), "{:?}", output);
    assert!(output.stdout.is_empty());
}

#[test_matrix([
    "int main() { return 7; printf(\"unreachable\"); }",
    "int main() { if (1) return 7; else return 3; printf(\"unreachable\"); }",
    "int main() { if (1) { if (0) return 2; else return 7; } else return 3; printf(\"unreachable\"); }",
])]
fn llvm_omits_unreachable_statements(code: &str) {
    let ir = dynamite_compiler::generate(code, "llvm", false);
    assert!(!ir.contains("call"));
    assert!(!ir.contains("ret i32 0"));
}

fn compile_and_run(code: &str, output_option: &str) -> Output {
    compile_and_run_with_c(code, output_option, None)
}

fn compile_and_run_with_c(code: &str, output_option: &str, companion: Option<&str>) -> Output {
    let assembly = dynamite_compiler::generate(code, output_option, false);
    let mut rng = rand::thread_rng();
    fs::create_dir_all("./tests/temp").unwrap();
    let binary_name = &format!("./tests/temp/{}", Alphanumeric.sample_string(&mut rng, 32));
    let mut command = Command::new("clang");
    command.args([
        "-x",
        match output_option {
            "asm" => "assembler",
            _ => "ir",
        },
        "-o",
        binary_name,
        "-",
    ]);
    let companion_path = format!("{binary_name}.c");
    if let Some(source) = companion {
        fs::write(&companion_path, source).unwrap();
        command.args(["-x", "c", &companion_path]);
    }
    let child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_ref()
        .map(|mut stdin| stdin.write_all(assembly.as_bytes()))
        .expect("Failed to open stdin")
        .expect("Failed to write to stdin");
    let cc_output = child.wait_with_output().unwrap();
    if companion.is_some() {
        remove_file(companion_path).unwrap();
    }
    assert!(
        cc_output.status.success(),
        "clang failed:\n{}\nGenerated code:\n{}",
        String::from_utf8_lossy(&cc_output.stderr),
        assembly,
    );
    let mut child = Command::new(binary_name)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            remove_file(binary_name).unwrap();
            panic!("generated program timed out:\n{}", assembly);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    remove_file(binary_name).unwrap();
    output
}

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

#[test_matrix([
    ("int main() { int n = 0; for (int i = 0; i < 5; i++) { if (i == 2) break; n += i; } return n; }", 1),
    ("int main() { int n = 0; while (n < 5) { n++; if (n == 3) break; } return n; }", 3),
    ("int main() { int n = 0; for (;;) { n++; break; } return n; }", 1),
    ("int main() { int n = 0; if (0 && n++) n = 9; if (1 || n++) n += 2; return n; }", 2),
    ("int main() { return (2 && 5) + (0 || 7); }", 2),
    ("int main() { int n = 0; return (1 ? (0 ? 3 : 7) : n++) + n; }", 7),
    ("int main() { int a[2][3] = {{4}, {5, 6,}}; return a[0][1] + a[1][0] + a[1][2]; }", 5),
    ("int a[2][3] = {{4}, {5, 6,}}; int main() { return a[0][1] + a[1][0] + a[1][2]; }", 5),
    ("int main() { char a = 100; char b = 100; return a + b; }", 200),
    ("int add(char a, int b) { return a + b; } int main() { return add(-3, 7); }", 4),
    ("int main() { return ~123 + 128; }", 4),
    ("int main() { int a[4]; return &a[3] - &a[0]; }", 3),
    ("int main() { int *p = 0; return p == 0; }", 1),
    ("int main() { char a = 2; return 0 ? a : 200; }", 200),
    ("int main() { return 0 ? 3 : 1 ? 5 : 9; }", 5),
    ("int sum(int a[3]) { return a[0] + a[2]; } int main() { int a[3] = {2, 3, 7}; return sum(a); }", 9),
    ("int abs(int n); int main() { return abs(-7); }", 7),
    ("char f(int n) { return n; } int main() { return f(300); }", 44),
    ("int main() { char c; int n = (c = 300); return n; }", 44),
    ("int n = {7}; int main() { ; return n; }", 7),
    ("int main() { int a[2] = {4, 7}; int i = 0; a[i++] += 3; return a[0] + a[1] + i; }", 15),
    ("int main() { int a[2] = {4, 7}; int *p = a; int n = ++*p++; return n + a[0] + a[1] + (p - a); }", 18),
    ("int main() { int a = 5; return -~a + +-a; }", 1),
    ("int n = (2 << 3) | 1; int main() { return n; }", 17),
    ("int main() { int a[2] = {3, 7}; int *p = 0 ? 0 : &a[1]; return *p; }", 7),
    ("int main() { int a[2] = {3, 7}; int *p = 1 ? a : 0; return p[0]; }", 3),
    ("int main() { int a[3000]; a[0] = 4; a[2999] = 9; return a[0] + a[2999]; }", 13),
    ("int *f() { } int main() { return f() == 0; }", 1),
    ("int n = 0 && (1 / 0); int m = 1 ? 5 : (1 / 0); int main() { return n + m; }", 5),
], ["asm", "llvm"])]
fn it_handles_control_flow_and_scalar_types(case: (&str, i32), output_option: &str) {
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
    ("int main() { int a[1 + 2]; return sizeof(a); }", 12),
    ("int a[sizeof(int)][2 * 3]; int main() { return sizeof(a); }", 96),
    ("int main() { return sizeof(char[1 ? 7 : 2]); }", 7),
], ["asm", "llvm"])]
fn it_supports_constant_array_dimensions(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
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
    ("int main() { int n = 0; for (int i = 0; i < 5; i++) { if (i == 2) continue; n += i; } return n; }", 8),
    ("int main() { int i = 0; int n = 0; while (i < 5) { i++; if (i == 3) continue; n += i; } return n; }", 12),
    ("int main() { int n = 0; for (int i = 0; i < 3; i++) { for (int j = 0; j < 3; j++) { if (j == 1) continue; n++; } } return n; }", 6),
], ["asm", "llvm"])]
fn it_supports_continue(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("int main() { int n = 0; do { n++; } while (0); return n; }", 1),
    ("int main() { int i = 0; int n = 0; do { i++; if (i == 2) continue; n += i; } while (i < 4); return n; }", 8),
    ("int main() { int n = 0; do { n++; if (n == 3) break; } while (1); return n; }", 3),
    ("int main() { int n = 0; do { int i = 0; do { i++; if (i == 2) continue; n++; } while (i < 3); } while (0); return n; }", 2),
], ["asm", "llvm"])]
fn it_supports_do_while(case: (&str, i32), output_option: &str) {
    let output = compile_and_run(case.0, output_option);
    assert_eq!(output.status.code(), Some(case.1), "{:?}", output);
}

#[test_matrix([
    ("int main() { int n = 0; switch (2) { case 1: n = 1; break; case 2: n = 7; break; default: n = 9; } return n; }", 7),
    ("int main() { int n = 0; switch (1) { case 1: n += 2; case 2: n += 5; break; default: n = 9; } return n; }", 7),
    ("int main() { switch (9) { case 1: return 3; default: return 7; } }", 7),
    ("int main() { int n = 0; switch (9) { case 1: n = 3; } return n; }", 0),
    ("int main() { int n = 0; for (int i = 0; i < 4; i++) { switch(i) { case 1: continue; case 2: break; default: n++; } n++; } return n; }", 5),
    ("int main() { int n = 1; switch(n++) { case 1: switch(2) { case 2: n += 5; break; } break; default: n = 9; } return n; }", 7),
    ("int main() { switch(-3) { case -3: case 1 + 2: return 7; default: return 1; } }", 7),
    ("int main() { switch(1) { if(0) { case 1: return 7; } } return 3; }", 7),
    ("int main() { switch(1) { while(0) { case 1: return 7; } } return 3; }", 7),
    ("int main() { switch(305419896) { case 305419896: return 7; } return 3; }", 7),
], ["asm", "llvm"])]
fn it_supports_switch(case: (&str, i32), output_option: &str) {
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
fn it_concatenates_adjacent_strings(output_option: &str) {
    let code =
        r#"int main(){char *p="\x41" /* gap */ "B" "\0" "C";return p[0]+p[1]+p[2]+p[3]-198;}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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
    "int x=7; int *p=&x; int main(){return *p-7;}",
    "int a[3]={1,2,7}; int *p=a+2; int main(){return *p-7;}",
    "int a[3]={1,2,8}; int *p=&a[2]; int main(){return *p-8;}",
    "char *p=\"abc\"+1; int main(){return *p-98;}",
], ["asm", "llvm"])]
fn it_initializes_global_addresses(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "enum E { A=3,B,C=A+7, }; int main(){enum E x=B;return x+C-14;}",
    "typedef enum { A=-2,B, } E; int main(){E x=B;enum Inner { A=7 };return x+A-6;}",
], ["asm", "llvm"])]
fn it_supports_enumerations(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){goto done;return 9;done:return 0;}",
    "int main(){int x=0;again:x++;if(x<3)goto again;return x-3;}",
    "int main(){goto inside;while(0){inside:return 0;}return 9;}",
], ["asm", "llvm"])]
fn it_supports_goto_labels(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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
    "int main(){int a[4]={[2]=7,[0]=3};return a[0]+a[1]+a[2]+a[3]-10;}",
    "int a[]={[3]=7,9};int main(){return sizeof(a)+a[0]+a[4]-29;}",
    "struct S{int x;int y;};struct S s={.y=7,.x=3};int main(){return s.x+s.y-10;}",
    "struct S{int a[3];int x;};int main(){struct S s={.a[2]=7,.x=9};return s.a[0]+s.a[2]+s.x-16;}",
], ["asm", "llvm"])]
fn it_supports_designated_initializers(code: &str, output_option: &str) {
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
    "struct S{unsigned :2;unsigned a:3;char c;};int main(){struct S s={7,9};s.a=2;return s.a+s.c-11;}",
    "struct S{unsigned long a:64;};struct S s={18446744073709551615UL};int main(){s.a++;return (s.a==0)-1;}",
    "struct S{_Bool b:1;unsigned a:3;};int main(){struct S s={9,7};s.b--;return s.b+s.a-7;}",
], ["asm", "llvm"])]
fn it_supports_bit_field_boundaries(code: &str, output_option: &str) {
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
    "struct S{int a;int b;int c;};struct S g={1,2,3};struct S f(){return g;}int main(){struct S (*p)()=f;return p().c-3;}",
    "struct S{long a;long b;long c;};struct S f(int n,struct S s){if(n)return f(n-1,s);return s;}int main(){struct S s={1,2,3};return f(3,s).c-3;}",
], ["asm", "llvm"])]
fn it_returns_global_and_recursive_aggregates(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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
fn it_starts_after_named_stack_and_floating_parameters(output_option: &str) {
    let code = "#include <stdarg.h>\ndouble local(int a,int b,int c,int d,int e,int f,int g,int h,int i,double x,...){va_list ap;va_start(ap,x);int y=va_arg(ap,int);double z=va_arg(ap,double);va_end(ap);return i+x+y+z;}int main(){return (local(1,2,3,4,5,6,7,8,9,1.5,3,2.5)==16.0)-1;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "#include <stdarg.h>\nint f(int n){va_list ap;va_start(ap,n);return 0;}",
    "#include <stdarg.h>\nint f(int a,int b,...){va_list ap;va_start(ap,a);return 0;}",
    "#include <stdarg.h>\nint f(int n,...){va_list ap;va_start(ap,n);return (int)va_arg(ap,float);}",
])]
fn it_diagnoses_invalid_variadic_builtins(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "asm", false)).is_err());
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
    "int f(int n,int m,int a[n][m]){return a[n-1][m-1]+sizeof(*a);}int main(){int n=2;int m=3;int a[n][m];a[1][2]=7;return f(n,m,a)-19;}",
    "int main(){int n=2;int m=3;int a[n][m];int (*p)[m]=a;p++;p[0][2]=7;return a[1][2]+(p-a)-8;}",
    "int main(){int n=3;int a[n][2];a[2][1]=5;return a[2][1]+sizeof(a)-29;}",
    "int main(){int n=2;int m=3;int a[n][m];int (*p)[m]=a;int (*q)[m]=p++;return (p-q)-1;}",
], ["asm", "llvm"])]
fn it_supports_variable_array_parameters_and_pointers(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn it_releases_variable_arrays_on_scope_exits(output_option: &str) {
    let code = "char pool[10000];unsigned long top;int live;void *malloc(unsigned long n){live++;void *p=pool+top;top+=n;return p;}void free(void *p){live--;}int f(int n){int a[n];a[0]=9;return a[0];}int main(){live=0;top=0;int n=3;typedef int A[n];{A a;goto out;}out:if(live)return 1;for(int i=0;i<4;i++){int a[n];if(i==1)continue;if(i==2)break;}if(live)return 2;int s=f(n);if(live)return 3;return s-9;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int n;int a[n];",
    "int main(){int n=2;static int a[n];}",
    "int main(){int n=2;int a[n]={1};}",
    "int main(){int n=2;goto label;int a[n];label:return 0;}",
    "int main(){int n=2;struct S{int a[n];};return 0;}",
    "int main(){int n=2;switch(n){int a[n];case 2:return 0;}return 1;}",
])]
fn it_diagnoses_invalid_variable_arrays(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "asm", false)).is_err());
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

#[test_matrix([
    "int main(){int n=3;int a[n];void *p=a;int (*q)[n]=(int (*)[n])p;(*q)[2]=7;return a[2]-7;}",
    "int main(){int n=3;goto label;int s=sizeof(int[n]);label:return 0;}",
    "int main(){int n=3;const int a[n];a[1]=2;return 0;}",
], ["asm", "llvm"])]
fn it_checks_variable_array_expression_scopes(code: &str, output_option: &str) {
    if code.contains("const int") {
        assert!(
            std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
                .is_err()
        );
    } else {
        assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
    }
}

#[test_matrix([
    "int main(){int n=2;int m=3;int a[n][(int)m];a[1][2]=7;return sizeof(a)+a[1][2]-31;}",
    "int f(int n,int a[n][n]);int f(int n,int a[n][n]){return a[n-1][n-1];}int main(){int n=2;int a[n][n];a[1][1]=7;return f(n,a)-7;}",
    "int main(){int n=2;int a[n][n];static int (*p)[n];p=a;p[1][1]=7;return a[1][1]-7;}",
], ["asm", "llvm"])]
fn it_preserves_variable_array_bound_evaluation(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){int n=3;int i=0;for(int a[n];i<2;i++){a[0]=i;}return i-2;}",
    "int main(){int n=3;int x=0;again:{int a[n];a[0]=1;x+=a[0];if(x<3)goto again;}return x-3;}",
    "int a[(int)3.0];int main(){return sizeof(a)-12;}",
], ["asm", "llvm"])]
fn it_supports_variable_array_loop_lifetimes(code: &str, output_option: &str) {
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
fn array_parameter_declarators(output_option: &str) {
    let code = "int sum(int n,int a[*]);int sum(int n,int a[const static n]){return a[0]+a[n-1]+sizeof(a);}int matrix(int n,int a[*][*]);int matrix(int n,int a[volatile static 2][n]){return a[1][n-1];}int main(){int a[3]={2,3,5};int b[2][3]={{0},{0,0,7}};return sum(3,a)+matrix(3,b)-22;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int a[static 3];",
    "int main(){int a[const 3];return 0;}",
    "int f(int a[*]){return 0;}",
    "int star(int a[static *]);",
    "int bound(int a[static]);",
    "int f(int a[const 3]){a=0;return 0;}",
], ["asm", "llvm"])]
fn rejects_invalid_array_parameter_modifiers(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"])]
fn flexible_array_members(output_option: &str) {
    let code = "struct S{char tag;double data[];};struct S global={7};struct S echo(struct S s){return s;}int main(){struct S *p=malloc(sizeof(struct S)+3*sizeof(double));p->tag=4;p->data[0]=1.5;p->data[2]=2.5;struct S s=echo(*p);int ok=sizeof(struct S)==8 && (char *)p->data-(char *)p==8 && p->data[0]+p->data[2]==4.0 && s.tag==4 && global.tag==7;free(p);return !ok;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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
], ["asm", "llvm"])]
fn rejects_invalid_flexible_array_members(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
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

#[test_matrix([
    "struct Duplicate{int x;union{int x;};};int main(){return 0;}",
    "struct Nested{struct{int x;};union{int x;};};int main(){return 0;}",
    "struct Qualified{struct{int x;};};int main(){const struct Qualified s={{1}};s.x=2;return 0;}",
], ["asm", "llvm"])]
fn rejects_invalid_anonymous_aggregate_members(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
fn restrict_qualified_pointers(output_option: &str) {
    let code = "typedef int *P;int sum(int n,int a[restrict static n],const int *restrict b){P restrict p=a;int result=0;for(int i=0;i<n;i++){p[i]+=b[i];result+=p[i];}return result;}int main(){int a[3]={1,2,3};int b[3]={4,5,6};return sum(3,a,b)-21;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "restrict int x;int main(){return 0;}",
    "int (*restrict function)(void);int main(){return 0;}",
    "struct S{int restrict x;};int main(){return 0;}",
], ["asm", "llvm"])]
fn rejects_invalid_restrict_types(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
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
], ["asm", "llvm"])]
fn rejects_invalid_inline(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn rejects_invalid_static_assert(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn rejects_invalid_automatic_storage(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"])]
fn register_vla_size(output_option: &str) {
    let code = "int main(){int n=3;register int a[n];return sizeof(a)-12;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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
], ["asm", "llvm"])]
fn rejects_invalid_generic_selection(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
fn global_byte_sized_integer_initializers(output_option: &str) {
    let code = "unsigned char table[4]={0,127,128,255}; signed char negative=-7; _Bool enabled=1; _Bool bool_values[2]={0,1}; unsigned short wide=513; int main(){return table[0]+table[1]+table[2]+table[3]+negative+enabled+bool_values[0]+bool_values[1]+wide-1018;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn pointer_arithmetic_preserves_integer_index_values(output_option: &str) {
    let code = "int values[256]; unsigned char bytes[40001]; int main(){unsigned char u=255; signed char n=-128; unsigned short wide=40000; _Bool one=1; int *p=values+255; values[u]=7; p[n]=9; bytes[wide]=11; return values[255]+values[127]+bytes[40000]+(p-u==values)+(p+one==values+256)+(p+n==values+127)+(values+u==p)-31;}";
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
], ["asm", "llvm"])]
fn aggregate_copy_rejects_invalid_qualifier_conversions(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"])]
fn string_literals_retain_array_type(output_option: &str) {
    let code = r#"char *global="hello"+2;char (*array_pointer)[4]=&"abc";int main(){char *selected=1?"x":"longer";return sizeof("")+sizeof("ab")+sizeof("a\0b")+sizeof("a" "bc")+sizeof(u8"é")+sizeof(u"😀")+sizeof(U"😀")+sizeof(L"ab")+sizeof(*array_pointer)+global[0]+(*array_pointer)[2]+selected[0]-372;}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn prefixed_string_literal_code_units(output_option: &str) {
    let code = r#"int main(){unsigned short *u=u"😀";unsigned int *v=U"😀";unsigned int *w=L"ab";return (u[0]!=55357)+(u[1]!=56832)+u[2]+(v[0]!=128512)+v[1]+(w[0]!=97)+(w[1]!=98)+w[2]+("a\0b"[2]!=98);}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

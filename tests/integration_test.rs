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
    compile_and_run_with_c_flags(code, output_option, companion, &[])
}

fn compile_and_run_with_c_flags(
    code: &str,
    output_option: &str,
    companion: Option<&str>,
    flags: &[&str],
) -> Output {
    let assembly = dynamite_compiler::generate(code, output_option, false);
    let mut rng = rand::thread_rng();
    fs::create_dir_all("./tests/temp").unwrap();
    let binary_name = &format!("./tests/temp/{}", Alphanumeric.sample_string(&mut rng, 32));
    let mut command = Command::new("clang");
    command.args(flags);
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn braced_string_array_initializers(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_braced_string_array_initializers(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_gnu_inline_attributes_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn typeof_preserves_arrays_functions_qualifiers_and_unevaluated_values(
    output_option: &str,
    optimization: &str,
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn typeof_variable_modified_types_capture_bounds_and_side_effects(
    output_option: &str,
    optimization: &str,
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
], ["asm", "llvm"])]
fn invalid_typeof_specifiers_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix(["asm", "llvm"])]
fn bundled_character_classification_header(output_option: &str) {
    let code = "#include <ctype.h>\n#include <ctype.h>\n#include <stdio.h>\nint main(){int (*lower)(int)=tolower;return !isalnum('7')+!isalpha('Z')+!isblank(' ')+!iscntrl('\\n')+!isdigit('2')+!isgraph('!')+!islower('a')+!isprint(' ')+!ispunct('!')+!isspace('\\t')+!isupper('A')+!isxdigit('f')+(lower('A')!='a')+(toupper('z')!='Z')+(tolower(EOF)!=EOF)+(isalpha(EOF)!=0)+(isdigit((unsigned char)255)!=0);}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn compiler_capabilities_select_portable_fallbacks(output_option: &str, optimization: &str) {
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
fn declaration_metadata_attributes_preserve_execution(output_option: &str) {
    let code = r#"__attribute__((unused)) static int calls;int add(int n __attribute__((unused))) __attribute__((__warn_unused_result__, availability(macos,introduced=10.10)));__attribute__((noinline)) int add(int n){calls++;return n+1;}typedef int scalar __attribute__((deprecated("message with ) parentheses")));struct __attribute__((deprecated)) Pair{int a;int b;};enum __attribute__((flag_enum,enum_extensibility(open))) Flags{Flag=1};int main(){__attribute__((unused)) static int local=Flag;scalar n=3;int (*p)(int) __attribute__((unused))=add;struct Pair pair={1,2};int result=p(n);return result+calls+pair.a+pair.b+local-9;}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn function_designators_convert_to_pointer_values(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn floating_function_designator_casts_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
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

#[test_matrix([
    "__thread int f(void);",
    "typedef __thread int T;",
    "__thread int x; int x;",
    "int main(){__thread int x;return 0;}",
    "int main(){static __thread int f(void);return 0;}",
], ["asm", "llvm"])]
fn invalid_thread_local_declarations_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["static __thread int x;", "extern _Thread_local int x;"])]
fn native_thread_local_storage_is_diagnosed(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "asm", false)).is_err());
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
], ["asm", "llvm"])]
fn gnu_object_alignment_rejects_unsupported_contexts(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix([
    "struct __attribute__((packed(1))) S{int x;};",
    "int value __attribute__((aligned(8192)));",
    "int value __attribute__((vector_size(16)));",
    "int value __attribute__((unknown_attribute));",
    "int value __attribute__((deprecated(\"unterminated\");",
    "int value __attribute__((unused);",
], ["asm", "llvm"])]
fn unsupported_or_malformed_attributes_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix([
    "int value __asm__(123);",
    "int value __asm__(\"\");",
    "int value __asm__(\"bad symbol\");",
    "int value __asm__(\"first\");extern int value __asm__(\"second\");",
], ["asm", "llvm"])]
fn invalid_assembler_labels_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_nullability_and_const_pointer_writes_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"])]
fn incomplete_extern_arrays_link_without_allocating_storage(output_option: &str) {
    let code = "extern int values[];int *value_tail=values+2;int read_values(){extern int values[];return values[0]+*value_tail;}int main(){return read_values()-9;}";
    let companion = "int values[3]={2,3,7};";
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
], ["asm", "llvm"])]
fn invalid_incomplete_array_declarations_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
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

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
#[test_matrix([
    "long double value;",
    "int main(){return 1.0L==1.0;}",
], ["asm", "llvm"])]
fn unsupported_long_double_formats_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn unsupported_float16_values_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix(["asm", "llvm"])]
fn extended_integer_static_storage_is_zeroed(output_option: &str) {
    let code = "__uint128_t words[2];union U{__int128 wide;char bytes[16];};union U data;int target=17;union P{__uint128_t wide;int *p;};union P pointer={.p=&target};struct Wrapper{char tag;union P payload;char end;};struct Wrapper wrapper={3,{.p=&target},5};int main(){unsigned char *bytes=(unsigned char*)words;int i=0;while(i<32){if(bytes[i])return 1;i++;}i=0;while(i<16){if(data.bytes[i])return 2;i++;}return sizeof(words)+_Alignof(__int128_t)+sizeof(wrapper)-96+(*pointer.p-17)+(wrapper.tag-3)+(wrapper.end-5)+(*wrapper.payload.p-17);}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
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
], ["asm", "llvm"])]
fn unsupported_extended_integer_values_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_record_alignment_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_population_count_builtins(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_qualified_record_value_conversions(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn qualified_scalar_casts(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_qualified_scalar_casts(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn typed_integer_complement_constants(output_option: &str, optimization: &str) {
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

#[test_matrix(["asm", "llvm"])]
fn promoted_complement_does_not_fit_unsigned_short_enum(output_option: &str) {
    let code = "enum E : unsigned short { A=~(unsigned short)0 };";
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix([
    "int main(){return __builtin_clz();}",
    "int main(){return __builtin_ctzll(1,2);}",
    "int main(){int x;return __builtin_clzl(&x);}",
    "struct S{int x;};int main(){struct S s={1};return __builtin_ctz(s);}"
], ["asm", "llvm"])]
fn invalid_integer_bit_scan_builtins(code: &str, output_option: &str) {
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

#[test_matrix([
    "struct __attribute__((aligned(64))) S{int x;};int main(){struct S s;return 0;}",
    "struct __attribute__((aligned(64))) S{int x;};struct S f(struct S);",
    "struct __attribute__((aligned(64))) S{int x;};struct S f(void);",
    "struct S;struct S f(void);struct __attribute__((aligned(64))) S{int x;};",
    "struct __attribute__((aligned(64))) S{int x;};void f(int,...);struct S s;int main(){f(0,s);return 0;}"
], ["asm", "llvm"])]
fn unsupported_extended_record_stack_abi_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn compile_time_constant_queries_do_not_evaluate_operands(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_compile_time_constant_queries(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn atomic_compare_exchange_preserves_pointers_and_argument_effects(
    output_option: &str,
    optimization: &str,
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
], ["asm", "llvm"])]
fn atomic_compare_exchange_rejects_invalid_arguments(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_atomic_load_store_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_atomic_read_modify_write_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
        +_Generic(__builtin_bswap64(1),unsigned long:0,default:1);
}"#;
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){return __builtin_bswap16();}",
    "int main(){return __builtin_bswap32(1,2);}",
    "int main(){int x;return __builtin_bswap64(&x);}",
    "struct S{int x;};int main(){struct S x;return __builtin_bswap32(x);}",
    "int x;int swapped=__builtin_bswap32(x);",
], ["asm", "llvm"])]
fn invalid_byte_swap_operands_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
    "int value;typedef int value;",
    "int main(){int n=3;typedef int T[n];typedef int T[n];return 0;}",
    "int main(){int n=3;typedef int T[n];typedef int T[3];return 0;}",
], ["asm", "llvm"])]
fn incompatible_typedef_redeclarations_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn unsupported_or_malformed_enumerator_annotations_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
    "typedef long Wide __attribute__((aligned(4)));",
    "typedef int Word __attribute__((aligned(4),aligned(2)));",
    "typedef int Function(void) __attribute__((aligned(1)));",
    "typedef struct Missing Record __attribute__((aligned(1)));",
    "typedef int Word __attribute__((aligned(3)));",
], ["asm", "llvm"])]
fn unsupported_typedef_alignment_changes_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_extended_enumerator_ranges(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_fixed_underlying_enums_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn nondeclarations_at_file_scope_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn over_aligned_hfa_variadic_round_trip(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_member_alignment_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_object_size_queries_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn checked_memory_builtins_preserve_results_and_single_evaluation(
    output_option: &str,
    optimization: &str,
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
], ["asm", "llvm"])]
fn invalid_checked_memory_operands_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_branch_expectation_operands_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix([
    "const char *name=__func__;",
    "const char *f(){return __func__;}const char *name=__func__;",
    "int main(){__func__[0]='x';}",
    "int main(){char *p=__func__;}",
    "inline int f(){static int values[2];return values[0];}",
    "inline const char *f(){static const char *values[1]={\"x\"};return values[0];}",
], ["asm", "llvm"])]
fn invalid_predefined_function_names_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn checked_integer_addition_matches_clang(output_option: &str, optimization: &str) {
    let mut code = String::from(
        "int global;\n_Static_assert(sizeof(__builtin_add_overflow(1,2,&global))==sizeof(_Bool),\"result size\");\n_Static_assert(_Generic(__builtin_add_overflow(1,2,&global),_Bool:1,default:0),\"result type\");\n",
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
    int got=__builtin_add_overflow(({left})a,({right})b,&actual);
    int want=reference_{tag}(a,b,&expected);
    if(got!=want || actual!=expected){{
        printf("{tag}: %lu %lu flags %d %d values %lu %lu\n",a,b,got,want,(unsigned long)actual,(unsigned long)expected);return 1;
    }}
    return 0;
}}
"#));
        companion.push_str(&format!("int reference_{tag}(unsigned long a,unsigned long b,{target} *out){{return __builtin_add_overflow(({left})a,({right})b,out);}}\n"));
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
], ["asm", "llvm"])]
fn invalid_checked_integer_addition_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"]) ]
fn checked_formatted_output_preserves_results_and_effects(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_checked_formatted_output_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

// Older Linux libcs lack these BSD APIs; macOS exercises the system runtime.
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
], ["asm", "llvm"])]
fn invalid_checked_bsd_string_output_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
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

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn incomplete_external_records_observe_later_completion(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_incomplete_external_record_uses_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn leading_nullability_on_pointer_typedefs_preserves_types(
    output_option: &str,
    optimization: &str,
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
], ["asm", "llvm"])]
fn invalid_leading_nullability_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix([
    "int main(){return 10_0;}",
    "int value=10_0;",
    "int values[10_0];",
    "enum E{VALUE=10_0};",
    "struct S{int value __attribute__((aligned(1_0)));};",
], ["asm", "llvm"])]
fn attribute_versions_are_not_c_numeric_constants(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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

#[test_matrix([
    "struct __attribute__((packed)) S;",
    "struct S{int x;} __attribute__((packed(1)));",
    "struct S{unsigned int field:3;} __attribute__((packed));",
    "int object __attribute__((packed));",
    "typedef int Scalar __attribute__((packed));",
    "struct S{int member __attribute__((packed));};",
], ["asm", "llvm"])]
fn unsupported_gnu_packing_is_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
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
#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn floating_absolute_builtins_preserve_binary64_long_double(
    output_option: &str,
    optimization: &str,
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
], ["asm", "llvm"])]
fn invalid_floating_absolute_builtins_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
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
    let result = compile_and_run_with_c_flags(
        &code,
        output_option,
        Some(companion),
        &[optimization, "-lm"],
    );
    assert_eq!(result.status.code(), Some(0), "{result:?}");
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn floating_nan_builtins_support_binary64_long_double(output_option: &str, optimization: &str) {
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
], ["asm", "llvm"])]
fn invalid_floating_nan_builtins_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
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
#[test_matrix(["asm", "llvm"], ["-O0", "-O2"])]
fn floating_infinity_builtins_preserve_binary64_long_double(
    output_option: &str,
    optimization: &str,
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
], ["asm", "llvm"])]
fn invalid_floating_infinity_builtins_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
#[test_matrix(["__builtin_infl", "__builtin_huge_vall"], ["asm", "llvm"])]
fn floating_infinity_builtins_reject_unsupported_long_double(name: &str, output_option: &str) {
    let code = format!("double value={name}();");
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(&code, output_option, false))
            .is_err()
    );
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
], ["asm", "llvm"])]
fn invalid_zero_length_array_contexts_are_diagnosed(code: &str, output_option: &str) {
    assert!(
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .is_err()
    );
}

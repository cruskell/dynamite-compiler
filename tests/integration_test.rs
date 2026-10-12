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
    let assembly = dynamite_compiler::generate(code, output_option, false);
    let mut rng = rand::thread_rng();
    fs::create_dir_all("./tests/temp").unwrap();
    let binary_name = &format!("./tests/temp/{}", Alphanumeric.sample_string(&mut rng, 32));
    let child = Command::new("clang")
        .args([
            "-x",
            match output_option {
                "asm" => "assembler",
                _ => "ir",
            },
            "-o",
            binary_name,
            "-",
        ])
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

//! Frontend rejection cases run once because they fail before backend selection.
//! Assembly without companion C uses one optimization setting: Clang does not
//! optimize assembly input. LLVM IR and companion C retain both optimization levels.

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

fn assert_frontend_error(code: &str) {
    use dynamite_compiler::ast::AstBuilder;

    assert!(
        std::panic::catch_unwind(|| {
            let source = dynamite_compiler::preprocessor::preprocess(code, None).unwrap();
            let tokens =
                dynamite_compiler::tokenizer::Tokenizer::tokenize(&source.code, false).unwrap();
            let mut builder = dynamite_compiler::ast::AstBuilderImpl::new(&source, &tokens);
            builder.build(false);
        })
        .is_err(),
        "frontend accepted invalid source: {code}"
    );
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

fn generated_program_timeout() -> std::time::Duration {
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    {
        static APPLE_SILICON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let translated = APPLE_SILICON.get_or_init(|| {
            Command::new("/usr/sbin/sysctl")
                .args(["-n", "hw.optional.arm64"])
                .output()
                .is_ok_and(|output| output.status.success() && output.stdout.trim_ascii() == b"1")
        });
        if *translated {
            // Translated x86_64 startup can exceed the native execution budget.
            return std::time::Duration::from_secs(60);
        }
    }
    std::time::Duration::from_secs(10)
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
    let deadline = std::time::Instant::now() + generated_program_timeout();
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline && child.try_wait().unwrap().is_none() {
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

#[test_matrix([
    "void *p=__builtin_alloca(4);",
    "int main(){__builtin_alloca(1.5);return 0;}",
    "int main(){int x;__builtin_alloca(&x);return 0;}",
    "int main(){__builtin_alloca(4,8);return 0;}",
])]
fn invalid_alloca_calls_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test]
fn native_alloca_has_a_source_diagnostic() {
    let result = std::panic::catch_unwind(|| {
        dynamite_compiler::generate(
            "int main(){void *p=__builtin_alloca(8);return p==0;}",
            "asm",
            false,
        )
    });
    let error = result.unwrap_err();
    let message = error
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains("alloca requires the LLVM backend") && message.contains('^'));
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
fn array_parameter_declarators(output_option: &str) {
    let code = "int sum(int n,int a[*]);int sum(int n,int a[const static n]){return a[0]+a[n-1]+sizeof(a);}int matrix(int n,int a[*][*]);int matrix(int n,int a[volatile static 2][n]){return a[1][n-1];}int main(){int a[3]={2,3,5};int b[2][3]={{0},{0,0,7}};return sum(3,a)+matrix(3,b)-22;}";
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
])]
fn rejects_invalid_flexible_array_members(code: &str) {
    assert_frontend_error(code);
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
])]
fn rejects_invalid_anonymous_aggregate_members(code: &str) {
    assert_frontend_error(code);
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

#[test_matrix(["asm", "llvm"])]
fn gnu_qualifier_spellings(output_option: &str) {
    for restrict in ["__restrict", "__restrict__"] {
        let code = format!(
            "typedef __signed__ char S;int sum(__const int *{restrict} p){{return *p;}}int main(){{__volatile__ int x=7;S s=-1;return sum((__const__ int *)&x)-7+(s>=0)+(__alignof__(S)!=1);}}"
        );
        assert_eq!(compile_and_run(&code, output_option).status.code(), Some(0));
    }
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
])]
fn aggregate_copy_rejects_invalid_qualifier_conversions(code: &str) {
    assert_frontend_error(code);
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
fn register_input_inline_assembly_preserves_operands((output_option, optimization): (&str, &str)) {
    let (wide, narrow, pointer, update) = if cfg!(target_arch = "aarch64") {
        (
            "add %x0, %x1, %x2",
            "add %w0, %w1, %w2",
            "mov %0, %1",
            "add %0, %0, %1",
        )
    } else {
        (
            "lea (%q1,%q2), %q0",
            "lea (%k1,%k2), %k0",
            "mov %1, %0",
            "add %1, %0",
        )
    };
    let code = format!(
        r#"
int calls;unsigned long left(void){{calls++;return 5;}}unsigned long right(void){{calls++;return 7;}}
volatile long source=4;
int main(){{
 unsigned long sum;asm("{wide}" : "=r"(sum) : "r"(left()),"r"(right()));if(sum!=12 || calls!=2)return 1;
 int signed_sum;asm("{narrow}" : "=r"(signed_sum) : "r"(-9),"r"(5));if(signed_sum!=-4)return 2;
 char array[4];void *pointer;asm("{pointer}" : "=r"(pointer) : "r"(array));if(pointer!=(void*)array)return 3;
 long value=3;asm volatile("{update}" : "+r"(value) : "r"(source));if(value!=7)return 4;
 long guard=17;asm("" : "+r"(guard) : "r"(1),"r"(2),"r"(3),"r"(4),"r"(5),"r"(6));if(guard!=17)return 5;
 return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[cfg(target_arch = "aarch64")]
#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn register_input_inline_assembly_supports_aarch64_width_modifiers(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
int calls;unsigned int left(void){calls++;return 4294967295U;}unsigned int right(void){calls++;return 2;}
unsigned long accumulator(void){calls++;return 7;}
int main(){unsigned long value;asm("umaddl %x0, %w1, %w2, %x3" : "=r"(value) : "r"(left()),"r"(right()),"r"(accumulator()));return value!=8589934597UL || calls!=3;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn register_input_inline_assembly_retains_explicit_operand_numbering() {
    let code = "volatile int output=7,input=11;int main(){asm volatile(\"\" : \"+r\"(output) : \"r\"(input));return 0;}";
    let generated = dynamite_compiler::generate(code, "llvm", false);
    assert!(generated.contains("=r,r,0,"), "{generated}");
    assert_eq!(
        generated.matches("load volatile i32").count(),
        2,
        "{generated}"
    );
}

#[test_matrix([
    "int main(){int x;asm(\"%2\" : \"=r\"(x) : \"r\"(1));}",
    "int main(){int x;asm(\"%99999999999999999999999999\" : \"=r\"(x));}",
    "int main(){int x;asm(\"nop\" : \"=r\"(x) : \"r\"(1.0));}",
    "int main(){int x;char y=1;asm(\"nop\" : \"=r\"(x) : \"r\"(y));}",
    "int main(){int x;asm(\"nop\" : \"=r\"(x) : \"r\"(1),\"r\"(2),\"r\"(3),\"r\"(4),\"r\"(5),\"r\"(6),\"r\"(7));}",
], ["asm", "llvm"])]
fn invalid_register_input_inline_assembly_has_source_diagnostics(code: &str, output_option: &str) {
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

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn readwrite_inline_assembly_evaluates_destinations_once(
    (output_option, optimization): (&str, &str),
) {
    let template = if cfg!(target_arch = "aarch64") {
        "add %0, %0, #5"
    } else {
        "add $5, %0"
    };
    let code = format!(
        r#"
volatile int global=12;int calls;volatile int *destination(void){{calls++;return &global;}}
int main(){{
 asm volatile("{template}" : "+r"(*destination()));if(calls!=1 || global!=17)return 1;
 int signed_value=-9;asm("{template}" : "+r"(signed_value));if(signed_value!=-4)return 2;
 unsigned int wrapped=4294967293U;asm("{template}" : "+r"(wrapped));if(wrapped!=2)return 3;
 unsigned long wide=4294967296UL;asm("{template}" : "+r"(wide));if(wide!=4294967301UL)return 4;
 void *pointer=(void*)37;asm("" : "+r"(pointer));if(pointer!=(void*)37)return 5;
 typedef int U __attribute__((aligned(1)));unsigned char bytes[8];U *unaligned=(U*)(bytes+1);*unaligned=21;
 asm("{template}" : "+r"(*unaligned));if(*unaligned!=26)return 6;
 return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn readwrite_inline_assembly_ties_input_and_output_in_llvm() {
    let generated = dynamite_compiler::generate(
        "volatile int output=7;int main(){asm volatile(\"\" : \"+r\"(output));return output!=7;}",
        "llvm",
        false,
    );
    assert!(generated.contains("=r,0,"), "{generated}");
    assert!(generated.contains("load volatile i32"), "{generated}");
    assert!(generated.contains("store volatile i32"), "{generated}");
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

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn register_output_inline_assembly_preserves_values_and_lvalues(
    (output_option, optimization): (&str, &str),
) {
    let (positive, negative, zero) = if cfg!(target_arch = "aarch64") {
        (
            "mov %0, #42\nmov %0, %0 // %% literal",
            "mov %0, #-7",
            "mov %0, #0",
        )
    } else {
        (
            "mov $42, %0\nmov %0, %0 # %% literal",
            "mov $-7, %0",
            "mov $0, %0",
        )
    };
    let positive = positive.replace('\n', "\\n");
    let code = format!(
        r#"
int calls;volatile int global;int *destination(void){{calls++;return (int*)&global;}}
long signed_value(void){{int n;__asm__("{negative}" : "=r"(n));return n;}}
unsigned long unsigned_value(void){{unsigned int n;__asm__ __volatile__("{negative}" : "=r"(n));return n;}}
#pragma pack(push,1)
struct Packed{{char tag;long value;}};
#pragma pack(pop)
int main(void){{
 long a=3,b=5,value;asm volatile("{positive}" : "=r"(value));
 if(a+b+value!=50)return 1;
 __asm volatile("{positive}" : "=r"(*destination()) : :);
 if(calls!=1 || global!=42)return 2;
 if(signed_value()!=-7 || unsigned_value()!=4294967289UL)return 3;
 void *pointer=(void*)1;__asm__("{zero}" : "=r"(pointer));if(pointer)return 4;
 struct Packed packed;asm("{positive}" : "=r"(packed.value));if(packed.value!=42)return 5;
 int n=2,index=0;int array[n];__asm__("{positive}" : "=r"(array[index++]));if(index!=1 || array[0]!=42)return 6;
 return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn register_output_inline_assembly_preserves_volatile_ir() {
    let template = if cfg!(target_arch = "aarch64") {
        "mov %0, #42"
    } else {
        "mov $42, %0"
    };
    for qualifier in ["volatile", ""] {
        let code = format!(
            "volatile int output;int main(){{asm {qualifier}(\"{template}\" : \"=r\"(output));return output!=42;}}"
        );
        let generated = dynamite_compiler::generate(&code, "llvm", false);
        assert_eq!(
            generated.contains("asm sideeffect"),
            !qualifier.is_empty(),
            "{generated}"
        );
        assert!(generated.contains("store volatile i32"), "{generated}");
    }
}

#[test_matrix([
    "int main(){int x;asm(\"nop\");}",
    "int main(){int x;asm goto(\"nop\");}",
    "int main(){int x;asm(\"%\" : \"=r\"(x));}",
    "int main(){int x;asm(\"%1\" : \"=r\"(x));}",
    "int main(){int x;asm(\"%Z0\" : \"=r\"(x));}",
    "int main(){int x;asm(\"%[name]\" : \"=r\"(x));}",
    "int main(){int memory;asm(\"nop\" : \"+m\"(memory));}",
    "int main(){int x;asm(\"nop\" : \"=m\"(x));}",
    "int main(){int x;asm(\"nop\" : \"=r\"(x) : \"m\"(1));}",
    "int main(){asm(\"\" : : : \"unsupported_register\");}",
    "int main(){const int x=0;asm(\"nop\" : \"=r\"(x));}",
    "int main(){asm(\"nop\" : \"=r\"(1));}",
    "int main(){double x;asm(\"nop\" : \"=r\"(x));}",
    "int main(){char x;asm(\"nop\" : \"=r\"(x));}",
    "struct S{int n;};int main(){struct S x;asm(\"nop\" : \"=r\"(x));}",
    "struct S{int n:3;};int main(){struct S x;asm(\"nop\" : \"=r\"(x.n));}",
    "int main(){int x;asm(L\"nop\" : \"=r\"(x));}",
    "int main(){int x;asm(\"nop\\0bad\" : \"=r\"(x));}",
], ["asm", "llvm"])]
fn unsupported_inline_assembly_has_source_diagnostics(code: &str, output_option: &str) {
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
])]
fn invalid_thread_local_declarations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
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
])]
fn gnu_object_alignment_rejects_unsupported_contexts(code: &str) {
    assert_frontend_error(code);
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

#[test_matrix([
    "int value __asm__(123);",
    "int value __asm__(\"\");",
    "int value __asm__(\"bad symbol\");",
    "int value __asm__(\"first\");extern int value __asm__(\"second\");",
])]
fn invalid_assembler_labels_are_diagnosed(code: &str) {
    assert_frontend_error(code);
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
])]
fn invalid_incomplete_array_declarations_are_diagnosed(code: &str) {
    assert_frontend_error(code);
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
])]
fn unsupported_float16_values_are_diagnosed(code: &str) {
    assert_frontend_error(code);
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
])]
fn unsupported_extended_integer_values_are_diagnosed(code: &str) {
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
])]
fn unsupported_extended_record_stack_abi_is_diagnosed(code: &str) {
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
])]
fn invalid_fixed_underlying_enums_are_diagnosed(code: &str) {
    assert_frontend_error(code);
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
])]
fn invalid_checked_bsd_string_output_is_diagnosed(code: &str) {
    assert_frontend_error(code);
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
])]
fn unsupported_gnu_packing_is_diagnosed(code: &str) {
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

#[test_matrix([
    "int evaluate(void){int n=0;int value=({int x=++n;({int y=x+3;y*2;});});return value+n;}",
    "int evaluate(void){int n=0;int a[3]={3,5,7};int *p=({a;});int size=sizeof(({n++;a;}));return size+p[1]+n;}",
    "static int helper(int x){return x+3;}int evaluate(void){int (*call)(int)=({helper;});return call(7);}",
    "struct Pair{long x,y;};int evaluate(void){struct Pair p=({struct Pair local={3,5};local;});return p.x+3*p.y+({struct Pair other={7,9};other;}).y;}",
    "int evaluate(void){int n=3;long result=({int values[n++];values[0]=7;values[n-2]=9;sizeof(values)+values[0]+values[n-2];});return result+n;}",
    "int evaluate(void){int sum=({int s=0;for(int i=0;i<7;i++){if(i==2)continue;if(i==5)break;s+=i;}switch(s){case 8:s+=3;break;default:s=0;}s;});return sum;}",
    "int evaluate(void){double value=({double a=1.5;double b=2.25;a+b;});return value*4;}",
    "int evaluate(void){const char *p=({\"asm\";});return p[1]+sizeof(({\"{\";}));}",
    "int evaluate(void){int x=7;_Static_assert(__builtin_types_compatible_p(__typeof__(({asm(\"\" : \"+r\"(x));})),void),\"asm statement\");({asm(\"\" : \"+r\"(x));});return x!=7;}",
    "int evaluate(void){int outer=7;return outer+({int n=0;while(n<3)n++;if(n==3)n+=2;n;});}",
    "int evaluate(void){int outer=11;return ({int n=0;do n++;while(n<3);n;})+outer;}",
    "int evaluate(void){int total=0;for(int i=0;i<1000;i++)total+=({int n=0;do n++;while(n<3);n;});return total-3000;}",
    "int evaluate(void){int n=0;({if(1)n++;});({int only_declaration=3;});({});_Static_assert(__builtin_types_compatible_p(__typeof__(({int x;})),void),\"declaration result\");return n+sizeof(({n++;7;}));}",
], ["asm", "llvm"], ["-O0", "-O2"])]
fn gnu_statement_expressions_match_clang(body: &str, output_option: &str, optimization: &str) {
    let companion = body.replace("int evaluate(void)", "int reference(void)");
    let code =
        format!("{body}\nint reference(void);int main(void){{return evaluate()!=reference();}}");
    let output =
        compile_and_run_with_c_flags(&code, output_option, Some(&companion), &[optimization]);
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

#[test_matrix([
    "int value=({3;});",
    "int main(){int n=({int hidden=3;hidden;});return hidden;}",
    "int main(){({int value=3;value;})=7;return 0;}",
    "int main(){int n=({int declaration;});return 0;}",
    "int main(){int n=({return 3;7;});return n;}",
    "int main(){while(1){int n=({break;7;});}return 0;}",
    "int main(){int n=({goto done;7;});done:return n;}",
])]
fn invalid_or_unsupported_statement_expressions_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn multiple_asm_outputs_preserve_values_and_lvalue_effects(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
int calls;int value=11;int *get_pointer(void){calls++;return &value;}
int main(void){volatile int values[2]={3,5};int i=0;unsigned long wide=7;
    asm volatile("" : "+r"(values[i++]), "+r"(*get_pointer()), "+r"(wide));
    long a=1,b=2,c=3,d=4,e=5,f=6,g=7;
    asm("" : "+r"(a), "+r"(b), "+r"(c), "+r"(d), "+r"(e), "+r"(f), "+r"(g));
    return i!=1 || calls!=1 || values[0]!=3 || value!=11 || wide!=7
        || a!=1 || b!=2 || c!=3 || d!=4 || e!=5 || f!=6 || g!=7;}
"#;
    let output = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn multiple_asm_outputs_preserve_types_and_operand_numbers(
    (output_option, optimization): (&str, &str),
) {
    let arithmetic = if cfg!(target_arch = "aarch64") {
        r"add %w0, %w0, %w3\n\tadd %x1, %x1, %x4\n\tmov %w2, #17"
    } else {
        r"addl %3, %0\n\taddq %4, %1\n\tmovl $17, %2"
    };
    let pointer = if cfg!(target_arch = "aarch64") {
        r"mov %x1, %x2\n\tmov %w0, #19"
    } else {
        r"movq %2, %1\n\tmovl $19, %0"
    };
    let boundary = if cfg!(target_arch = "aarch64") {
        "add %w0, %w0, %w2"
    } else {
        "addl %2, %0"
    };
    let code = format!(
        r#"
int object;
int main(void){{int a=-4;unsigned long b=9;volatile int c;int x=5;unsigned long y=13;
    asm volatile("{arithmetic}" : "+r"(a), "+r"(b), "=r"(c) : "r"(x), "r"(y));
    if(a!=1 || b!=22 || c!=17)return 1;
    void *address;int tag;
    asm("{pointer}" : "=r"(tag), "=r"(address) : "r"(&object));
    if(address!=&object || tag!=19)return 2;
    int first=7,second=11;
    asm("{boundary}" : "+r"(first), "+r"(second) : "r"(1), "r"(2), "r"(3), "r"(4), "r"(5));
    return first!=8 || second!=11;}}
"#
    );
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "int main(){int x=1;asm(\"\" : \"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x));}",
    "int main(){int x=1,y=2;asm(\"\" : \"+r\"(x),\"+r\"(y) : \"r\"(1),\"r\"(2),\"r\"(3),\"r\"(4),\"r\"(5),\"r\"(6));}",
    "int main(){int x,y;asm(\"%2\" : \"=r\"(x),\"=r\"(y));}",
    "int main(){int x,y;asm(\"\" : \"=r\"(x),\"=&r\"(y));}",
])]
fn unsupported_multiple_asm_outputs_are_diagnosed(code: &str) {
    assert_frontend_error(code);
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

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn memory_clobber_inline_assembly_preserves_memory_effects(
    (output_option, optimization): (&str, &str),
) {
    let store = if cfg!(target_arch = "aarch64") {
        "str %w1, [%0]"
    } else {
        "movl %1, (%0)"
    };
    let code = format!(
        r#"
int calls;int value(void){{calls++;return 19;}}
int read(void){{int local=3;
    asm("{store}" : : "r"(&local), "r"(value()) : "memory");
    return local;
}}
int main(void){{
    asm volatile("" ::: "memory");
    __asm__ __volatile__("" : : : "memory", "memory");
    asm("nop" :);asm("" : :);asm("" : : :);
    long same=7;asm("" : "+r"(same) : : "memory");
    int result=read();int expression=5+({{asm("" ::: "memory");3;}});
    return result!=19 || calls!=1 || same!=7 || expression!=8;
}}
"#
    );
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn memory_clobber_inline_assembly_emits_llvm_barriers() {
    let generated = dynamite_compiler::generate(
        "int value;void barrier(void){value=3;asm(\"\" ::: \"memory\");value=7;}",
        "llvm",
        false,
    );
    assert!(
        generated.contains("call void asm sideeffect"),
        "{generated}"
    );
    assert!(generated.contains("~{memory}"), "{generated}");
    assert!(!generated.contains("call {  } asm"), "{generated}");
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

struct DriverFixture(std::path::PathBuf);

impl DriverFixture {
    fn new() -> Self {
        let name = Alphanumeric.sample_string(&mut rand::thread_rng(), 24);
        let directory = std::path::PathBuf::from("tests/temp").join(format!("driver-{name}"));
        fs::create_dir_all(&directory).unwrap();
        Self(directory.canonicalize().unwrap())
    }

    fn write(&self, name: &str, code: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, code).unwrap();
    }

    fn cc(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
            .current_dir(&self.0)
            .env("DYNAMITE_CLANG", "clang")
            .args(arguments)
            .output()
            .unwrap()
    }

    fn success(&self, arguments: &[&str]) {
        let result = self.cc(arguments);
        assert!(result.status.success(), "{result:?}");
    }

    fn executable_status(&self, name: &str) -> Option<i32> {
        Command::new(self.0.join(name)).status().unwrap().code()
    }
}

impl Drop for DriverFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn driver_stage_timings_are_optional_and_preserve_parallel_results() {
    let files = DriverFixture::new();
    files.write(
        "first.c",
        "int second(void);int main(){return 20+second();}",
    );
    files.write("second.c", "int second(void){return 22;}");
    for enabled in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"));
        command
            .current_dir(&files.0)
            .env("DYNAMITE_CLANG", "clang")
            .env_remove("DYNAMITE_TIMINGS")
            .args(["--jobs", "2", "first.c", "second.c", "-o", "program"]);
        if enabled {
            command.env("DYNAMITE_TIMINGS", "1");
        }
        let result = command.output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(result.stdout.is_empty());
        assert_eq!(files.executable_status("program"), Some(42));
        let stderr = String::from_utf8(result.stderr).unwrap();
        let mut stages = std::collections::HashMap::new();
        for line in stderr
            .lines()
            .filter(|line| line.starts_with("dynamite-timing\t"))
        {
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 4);
            assert!(fields[1].parse::<u32>().unwrap() > 0);
            let seconds = fields[3].parse::<f64>().unwrap();
            assert!(seconds.is_finite() && seconds >= 0.0);
            *stages.entry(fields[2]).or_insert(0) += 1;
        }
        if enabled {
            for stage in [
                "preprocess",
                "preprocess.native",
                "tokenize",
                "parse",
                "parse.declarations",
                "parse.validate",
                "parse.lower",
                "parse.inline",
                "generate",
                "drop_ast",
                "drop_tokens",
                "frontend_total",
                "lower",
            ] {
                assert_eq!(stages.get(stage), Some(&2), "{stage}: {stderr}");
            }
            assert_eq!(stages.get("link"), Some(&1));
            assert_eq!(stages.get("write_ir"), Some(&2));
        } else {
            assert!(stages.is_empty());
        }
    }
}

#[test_matrix(["1", "4"])]
fn driver_parallel_translation_units_keep_types_and_symbols_local(jobs: &str) {
    let files = DriverFixture::new();
    files.write(
        "first.c",
        "struct S{int x;};int first(){struct S s={17};return s.x;}",
    );
    files.write(
        "second.c",
        "struct S{long x;long y;};int second(){struct S s={20,5};return s.x+s.y;}",
    );
    files.write("main.c", "#ifndef __DYNAMITE__\n#error wrong compiler profile\n#endif\nint first();int second();int main(){return first()+second();}");
    files.success(&[
        "--jobs", jobs, "-O2", "-MMD", "first.c", "second.c", "main.c", "-o", "program",
    ]);
    assert_eq!(files.executable_status("program"), Some(42));
    for index in 0..3 {
        let deps = fs::read_to_string(files.0.join(format!("program-{index}.d"))).unwrap();
        assert!(deps.starts_with("program:"), "{deps}");
    }
}

#[test]
fn driver_compiles_objects_and_assembly_and_links_existing_inputs() {
    let files = DriverFixture::new();
    files.write("source.c", "int main(){return 42;}");
    files.success(&["-c", "source.c"]);
    assert!(files.0.join("source.o").is_file());
    files.success(&["source.o", "-o", "from_object"]);
    assert_eq!(files.executable_status("from_object"), Some(42));
    files.success(&["-S", "-O2", "source.c", "-o", "source.s"]);
    files.success(&["source.s", "-o", "from_assembly"]);
    assert_eq!(files.executable_status("from_assembly"), Some(42));
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_dependency_scan_does_not_require_clang() {
    let files = DriverFixture::new();
    files.write("value.h", "#define VALUE 42\n");
    files.write(
        "source.c",
        "#include \"value.h\"\nint value(){return VALUE;}\n",
    );
    let result = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
        .current_dir(&files.0)
        .env("DYNAMITE_CLANG", "/missing-clang")
        .args(["-MM", "source.c"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(
        output.contains("source.o:") && output.contains("value.h"),
        "{output}"
    );
}

#[test]
#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
fn native_dependency_batch_resets_macros_and_once_state() {
    let files = DriverFixture::new();
    files.write("one.h", "#define VALUE 1\n");
    files.write("two.h", "#define VALUE 2\n");
    files.write("shared.h", "#pragma once\n#ifndef SHARED_H\n#define SHARED_H\n#if KIND==1\n#include \"one.h\"\n#else\n#include \"two.h\"\n#endif\n#endif\n");
    for (file, kind) in [("first.c", 1), ("second.c", 2)] {
        files.write(file,&format!("#define KIND {kind}\n#if __COUNTER__ != 0\n#error counter leaked\n#endif\n#include \"shared.h\"\n"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
        .current_dir(&files.0)
        .env("DYNAMITE_CLANG", "/missing-clang")
        .args(["-MM", "first.c", "second.c"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].contains("one.h") && !lines[0].contains("two.h"),
        "{text}"
    );
    assert!(
        lines[1].contains("two.h") && !lines[1].contains("one.h"),
        "{text}"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
        .current_dir(&files.0)
        .env("DYNAMITE_CLANG", "/missing-clang")
        .args(["-MM", "first.c", "second.c", "-MF", "all.d"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read_to_string(files.0.join("all.d")).unwrap(), text);
}

#[test_matrix(["asm","llvm"])]
#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
fn native_preprocessing_generates_executable_code_without_clang(format: &str) {
    let files = DriverFixture::new();
    files.write(
        "values.h",
        "#ifndef VALUES_H\n#define VALUES_H\n#define CAT(a,b) a##b\n#define VALUE 4\\\n2\n#endif\n",
    );
    files.write("source.c","#include \"values.h\"\n#include \"values.h\"\n#define PICK(x) (x)\n#if defined(VALUE) && VALUE==42\nint CAT(ma,in)(){const char *s=\"/*kept*/\"; /* ignored\\\ncomment */ return s[0]=='/'?PICK(VALUE):0;}\n#else\n#error wrong branch\n#endif\n");
    let result = Command::new(env!("CARGO_BIN_EXE_dynamite_compiler"))
        .current_dir(&files.0)
        .env("PATH", "/no-external-compiler")
        .args(["-o", format, "source.c"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let source = if format == "asm" {
        "output.s"
    } else {
        "output.ll"
    };
    fs::write(files.0.join(source), result.stdout).unwrap();
    assert!(
        Command::new("clang")
            .current_dir(&files.0)
            .args([source, "-o", "program"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(files.executable_status("program"), Some(42));
}

#[test]
fn driver_dependency_files_track_relative_headers_and_escape_targets() {
    let files = DriverFixture::new();
    files.write("forced.h", "#define BASE 17\n");
    files.write("include/value.h", "#define VALUE 25\n");
    files.write(
        "nested/source.c",
        "#include \"value.h\"\nint value(){return BASE+VALUE;}",
    );
    files.success(&[
        "-c",
        "nested/source.c",
        "-include",
        "forced.h",
        "-iquote",
        "include",
        "-MMD",
        "-MP",
        "-MF",
        "deps.d",
        "-o",
        "result object.o",
    ]);
    files.write("Makefile", "include deps.d\nresult\\ object.o:\n\t@false\n");
    let make = || {
        Command::new("make")
            .current_dir(&files.0)
            .args(["-q", "result object.o"])
            .output()
            .unwrap()
    };
    let result = make();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let header = fs::File::options()
        .write(true)
        .open(files.0.join("include/value.h"))
        .unwrap();
    header
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2)),
        )
        .unwrap();
    assert_eq!(make().status.code(), Some(1));
    fs::remove_file(files.0.join("include/value.h")).unwrap();
    assert_eq!(make().status.code(), Some(1));
}

#[test]
fn driver_dependency_modes_distinguish_system_headers() {
    let files = DriverFixture::new();
    files.write("system/value.h", "#define VALUE 42\n");
    files.write("source.c", "#include <value.h>\nint value(){return VALUE;}");
    for (mode, system) in [("-MD", true), ("-MMD", false)] {
        files.success(&["-c", "source.c", "-isystem", "system", mode]);
        let deps = fs::read_to_string(files.0.join("source.d")).unwrap();
        assert_eq!(deps.contains("value.h"), system, "{deps}");
        assert!(deps.contains("source.c"), "{deps}");
    }
    files.write("invalid.c", "int invalid = ;");
    files.success(&["-MM", "invalid.c", "-MF", "invalid.d"]);
    assert!(files.0.join("invalid.d").is_file());
    assert!(!files.0.join("invalid.o").exists());
}

#[test]
fn driver_preserves_static_library_order_and_separates_link_flags() {
    let files = DriverFixture::new();
    files.write("helper.c", "int helper(){return 42;}");
    files.write("main.c", "int helper();int main(){return helper();}");
    files.success(&["-c", "helper.c"]);
    assert!(
        Command::new("ar")
            .current_dir(&files.0)
            .args(["crs", "libhelper.a", "helper.o"])
            .status()
            .unwrap()
            .success()
    );
    files.success(&["-Werror", "main.c", "-L.", "-lhelper", "-o", "program"]);
    assert_eq!(files.executable_status("program"), Some(42));
}

#[test]
fn driver_diagnostics_keep_original_header_locations() {
    let files = DriverFixture::new();
    files.write("include/problem.h", "\n\nint invalid = ;\n");
    files.write("nested/source.c", "#include <problem.h>\n");
    let result = files.cc(&["-c", "nested/source.c", "-Iinclude"]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("include/problem.h") && error.contains("int invalid = ;"),
        "{error}"
    );
    files.write(
        "warning.c",
        "#warning expected_driver_warning\nint value;\n",
    );
    let warning = files.cc(&["-c", "warning.c"]);
    assert!(warning.status.success(), "{warning:?}");
    assert!(String::from_utf8_lossy(&warning.stderr).contains("expected_driver_warning"));
    let rejected = files.cc(&["-Werror", "-c", "warning.c", "-o", "rejected.o"]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(!files.0.join("rejected.o").exists());
}

#[test]
fn driver_never_falls_back_to_clang_for_unsupported_c() {
    let files = DriverFixture::new();
    files.write(
        "unsupported.c",
        "_Atomic(int) value;int main(){return value;}",
    );
    let result = files.cc(&["-std=c11", "-c", "unsupported.c"]);
    assert!(!result.status.success(), "{result:?}");
    assert!(!files.0.join("unsupported.o").exists());
    files.write("supported.c", "int helper(){return 42;}");
    let result = files.cc(&[
        "--jobs",
        "2",
        "unsupported.c",
        "supported.c",
        "-o",
        "program",
    ]);
    assert!(!result.status.success(), "{result:?}");
    assert!(!files.0.join("program").exists());
}

#[test]
fn driver_rejects_colliding_outputs_and_unsupported_frontend_modes() {
    let files = DriverFixture::new();
    files.write("one/source.c", "int one(){return 1;}");
    files.write("two/source.c", "int two(){return 2;}");
    files.write("wrong.so.cpp", "int main(){auto value=42;return value;}");
    for arguments in [
        vec!["--jobs", "0", "-c", "one/source.c"],
        vec!["--jobs", "4", "-c", "one/source.c", "two/source.c"],
        vec!["-c", "one/source.c", "two/source.c", "-o", "one.o"],
        vec!["-x", "c++", "one/source.c"],
        vec!["wrong.so.cpp", "-o", "wrong"],
        vec!["@arguments"],
    ] {
        let result = files.cc(&arguments);
        assert!(!result.status.success(), "{result:?}");
    }
}

#[test_matrix(["asm", "llvm"])]
fn prototype_fast_path_keeps_existing_definitions_and_unevaluated_bounds(output_option: &str) {
    let code = "int calls; int f(int a[++calls]); int g(int x); int g(int x){return x+1;} int g(int x); int main(){printf(\"%d,%d\",calls,g(41));return 0;}";
    assert_eq!(compile_and_get_stdout(code, output_option), "0,42");
}

#[test]
fn driver_preserves_backend_time_trace_requests() {
    let files = DriverFixture::new();
    files.write("main.c", "int main(){return 42;}");
    files.success(&[
        "-O0",
        "-c",
        "main.c",
        "-o",
        "main.o",
        "-ftime-trace=trace.json",
    ]);
    assert!(
        fs::read_to_string(files.0.join("trace.json"))
            .unwrap()
            .contains("traceEvents")
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[test]
#[ignore = "requires llc, opt, and ld.lld; run by the LLVM tools CI job"]
fn llvm_tools_backend_compiles_and_links_without_clang() {
    let files = DriverFixture::new();
    files.write(
        "worker.c",
        "#include <pthread.h>\n_Thread_local int value;void *worker(void *p){value=42;return (void *)(long)value;}",
    );
    files.write(
        "main.c",
        r#"
#include <stddef.h>
#include <pthread.h>
#include <math.h>
void *worker(void *);
extern _Thread_local int value;
int main(void){
    pthread_t thread;void *result;
    if(pthread_create(&thread,0,worker,0))return 1;
    if(pthread_join(thread,&result))return 2;
    long double x=9007199254740993.0L;x+=1.0L;
    volatile double y=1764.0;
    return (long)result!=42 || value!=0 || x!=9007199254740994.0L || sqrt(y)!=42.0;
}
"#,
    );
    for optimization in ["-O0", "-O2", "-O3", "-Os", "-Oz"] {
        let result = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
            .current_dir(&files.0)
            .env("DYNAMITE_BACKEND", "llvm")
            .env("DYNAMITE_CLANG", files.0.join("missing-clang"))
            .args([
                "--jobs",
                "2",
                optimization,
                "-MMD",
                "main.c",
                "worker.c",
                "-pthread",
                "-lm",
                "-o",
                "program",
            ])
            .output()
            .unwrap();
        assert!(result.status.success(), "{optimization}: {result:?}");
        assert_eq!(files.executable_status("program"), Some(0));
        for index in 0..2 {
            let deps = fs::read_to_string(files.0.join(format!("program-{index}.d"))).unwrap();
            assert!(deps.starts_with("program:"), "{deps}");
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[test]
#[ignore = "requires llc, opt, and ld.lld; run by the LLVM tools CI job"]
fn llvm_tools_backend_preserves_shared_libraries_and_object_linking() {
    let files = DriverFixture::new();
    files.write("library.c", "int value(void){return 42;}");
    files.write("main.c", "int value(void);int main(void){return value();}");
    let cc = |arguments: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
            .current_dir(&files.0)
            .env("DYNAMITE_BACKEND", "llvm")
            .env("DYNAMITE_CLANG", files.0.join("missing-clang"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(result.status.success(), "{arguments:?}: {result:?}");
    };
    cc(&["-O2", "-shared", "-fPIC", "library.c", "-o", "libvalue.so"]);
    cc(&["-O0", "-c", "main.c", "-o", "main.o"]);
    cc(&[
        "-std=gnu99",
        "-DUNUSED=1",
        "-I.",
        "main.o",
        "-L",
        ".",
        "-lvalue",
        "-Wl,-rpath,$ORIGIN",
        "-o",
        "program",
    ]);
    assert_eq!(files.executable_status("program"), Some(42));
    cc(&[
        "main.o", "-no-pie", "-L.", "-lvalue", "-Xlinker", "-rpath", "-Xlinker", "$ORIGIN", "-o",
        "program",
    ]);
    assert_eq!(files.executable_status("program"), Some(42));
    cc(&["-S", "library.c", "-o", "library.s"]);
    assert!(
        fs::read_to_string(files.0.join("library.s"))
            .unwrap()
            .contains("value:")
    );
}

#[cfg(any(
    all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"),
    target_os = "macos"
))]
#[test]
fn llvm_tools_backend_rejects_unsupported_options_without_clang_fallback() {
    let files = DriverFixture::new();
    files.write("main.c", "int main(void){return 0;}");
    for option in [
        "-Ofast",
        "-march=native",
        "-flto",
        "-fsanitize=address",
        "-emit-llvm",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
            .current_dir(&files.0)
            .env("DYNAMITE_BACKEND", "llvm")
            .env("DYNAMITE_CLANG", files.0.join("missing-clang"))
            .args([option, "-c", "main.c"])
            .output()
            .unwrap();
        assert!(!result.status.success(), "{option}: {result:?}");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains("LLVM"), "{stderr}");
        assert!(!files.0.join("main.o").exists());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_macos_preprocessing_uses_sdk_and_host_data_model() {
    let files = DriverFixture::new();
    files.write(
        "Frameworks/Local.framework/Headers/values.h",
        "#define LOCAL_VALUE 42\n",
    );
    files.write(
        "source.c",
        r#"
#include <Local/values.h>
#include <stddef.h>
#include <stdint.h>
#include <pthread.h>
#include <CoreFoundation/CFString.h>
#if !defined(__APPLE__) || !defined(__MACH__) || defined(__linux__)
#error incorrect OS profile
#endif
#if defined(__arm64__) && (defined(__x86_64__) || __SIZEOF_LONG_DOUBLE__ != 8)
#error incorrect ARM64 profile
#endif
_Static_assert(sizeof(long double)==__SIZEOF_LONG_DOUBLE__, "long double ABI");
_Static_assert(sizeof(uint64_t)==8, "stdint ABI");
_Static_assert(LOCAL_VALUE==42, "framework search");
int main(void){return 0;}
"#,
    );
    let result = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
        .current_dir(&files.0)
        .env("DYNAMITE_CLANG", "/missing-clang")
        .args(["-E", "-F", "Frameworks", "source.c", "-o", "output.i"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let contents = fs::read_to_string(files.0.join("output.i")).unwrap();
    assert!(!contents.contains("_PTHREAD_SWIFT_IMPORTER_NULLABILITY_COMPAT"));
    files.success(&["-x", "c", "output.i", "-o", "program"]);
    assert_eq!(files.executable_status("program"), Some(0));
}

#[cfg(target_os = "macos")]
fn macos_llvm_command(files: &DriverFixture) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynamite-cc"));
    command
        .current_dir(&files.0)
        .env("DYNAMITE_BACKEND", "llvm")
        .env("DYNAMITE_CLANG", files.0.join("missing-clang"))
        .env(
            "SDKROOT",
            dynamite_compiler::toolchain::macos_sdk_root().unwrap(),
        )
        .env("PATH", "/no-external-compiler");
    command
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires absolute DYNAMITE_LLC/DYNAMITE_OPT paths and the macOS SDK"]
fn macos_llvm_tools_compile_sdk_framework_threads_and_tls_without_clang() {
    let files = DriverFixture::new();
    files.write(
        "worker.c",
        "_Thread_local int local;void *worker(void *p){local=42;return (void *)(long)local;}",
    );
    files.write(
        "main.c",
        r#"
#include <pthread.h>
#include <math.h>
#include <CoreFoundation/CFString.h>
extern _Thread_local int local;
void *worker(void *);
int main(void){
    pthread_t thread;void *result;volatile double n=1764.0;
    CFStringRef text=CFStringCreateWithCString(0,"answer",kCFStringEncodingUTF8);
    int length=CFStringGetLength(text);CFRelease(text);
    if(pthread_create(&thread,0,worker,0) || pthread_join(thread,&result))return 1;
    return (long)result!=42 || local!=0 || sqrt(n)!=42.0 || length!=6;
}
"#,
    );
    for optimization in ["-O0", "-O2"] {
        let output = macos_llvm_command(&files)
            .args([
                "--jobs",
                "2",
                optimization,
                "-MMD",
                "main.c",
                "worker.c",
                "-pthread",
                "-lm",
                "-framework",
                "CoreFoundation",
                "-o",
                "program",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{optimization}: {output:?}");
        assert_eq!(files.executable_status("program"), Some(0));
        let dependencies = fs::read_to_string(files.0.join("program-0.d")).unwrap();
        assert!(dependencies.contains("main.c"));
        assert!(
            !dependencies.contains("CoreFoundation.framework"),
            "{dependencies}"
        );
    }
    let output = macos_llvm_command(&files)
        .args(["-E", "worker.c"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires absolute DYNAMITE_LLC/DYNAMITE_OPT paths and the macOS SDK"]
fn macos_llvm_tools_link_dylibs_archives_and_bundles_without_clang() {
    let files = DriverFixture::new();
    files.write("library.c", "int value(void){return 42;}");
    let output = macos_llvm_command(&files)
        .args([
            "-O2",
            "-dynamiclib",
            "library.c",
            "-install_name",
            "@rpath/libvalue.dylib",
            "-o",
            "libvalue.dylib",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    files.write("helper.c", "int helper(void){return 7;}");
    let output = macos_llvm_command(&files)
        .args(["-c", "helper.c", "-o", "helper.o"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        Command::new("/usr/bin/ar")
            .current_dir(&files.0)
            .args(["rcs", "libhelper.a", "helper.o"])
            .status()
            .unwrap()
            .success()
    );
    files.write(
        "plugin.c",
        "int host_value(void);int plugin(void){return host_value()+1;}",
    );
    let output = macos_llvm_command(&files)
        .args([
            "-bundle",
            "-undefined",
            "dynamic_lookup",
            "plugin.c",
            "-o",
            "plugin.bundle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    files.write(
        "main.c",
        r#"
#include <dlfcn.h>
int value(void);int helper(void);int host_value(void){return 41;}
int main(void){
    void *handle=dlopen("./plugin.bundle",RTLD_NOW);if(!handle)return 1;
    int (*plugin)(void)=(int (*)(void))dlsym(handle,"plugin");
    if(!plugin || plugin()!=42)return 2;
    dlclose(handle);return value()+helper()!=49;
}
"#,
    );
    let rpath = format!("-Wl,-rpath,{}", files.0.display());
    let output = macos_llvm_command(&files)
        .args([
            "-O2",
            "main.c",
            "libhelper.a",
            "-L.",
            "-lvalue",
            "-rdynamic",
            &rpath,
            "-o",
            "program",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(files.executable_status("program"), Some(0));
}

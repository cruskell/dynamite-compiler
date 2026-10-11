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

#[cfg(target_arch = "x86_64")]
#[test_matrix(["asm", "llvm"])]
fn it_compiles_string_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/string.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    assert_eq!(got, "a = 777, b = 755, c = 222\n")
}

#[cfg(target_arch = "x86_64")]
#[test_matrix(["asm", "llvm"])]
fn it_compiles_simple_expr_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/simple_expr.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    got.split("\n")
        .filter(|s| !s.is_empty() && !s.ends_with("OK"))
        .for_each(|s| panic!("assertion failed:\n  {}\n", s));
}

#[cfg(target_arch = "x86_64")]
#[test_matrix(["asm"])]
fn it_compiles_expr_c(output_option: &str) {
    let code = &fs::read_to_string("./tests/c/expr.c").unwrap();
    let got = compile_and_get_stdout(code, output_option);
    got.split("\n")
        .filter(|s| !s.is_empty() && !s.ends_with("OK"))
        .for_each(|s| panic!("assertion failed:\n  {}\n", s));
}

#[cfg(target_arch = "x86_64")]
#[test_matrix(["asm"])]
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
])]
fn llvm_if_returns_expected_status(case: (&str, i32)) {
    let (code, expected) = case;
    let output = compile_and_run(code, "llvm");
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
    let output = Command::new(binary_name).output().unwrap();
    remove_file(binary_name).unwrap();
    output
}

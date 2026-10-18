use rand::distributions::{Alphanumeric, DistString};
use std::{
    fs::{self, remove_file},
    io::Write,
    process::{Command, Output},
};

pub(super) fn assert_frontend_error(code: &str) {
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

pub(super) fn compile_and_get_stdout(code: &str, output_option: &str) -> String {
    let output = compile_and_run(code, output_option);
    assert!(output.status.success(), "program failed: {:?}", output);
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub(super) fn compile_and_run(code: &str, output_option: &str) -> Output {
    compile_and_run_with_c(code, output_option, None)
}

pub(super) fn compile_and_run_with_c(
    code: &str,
    output_option: &str,
    companion: Option<&str>,
) -> Output {
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

pub(super) fn compile_and_run_with_c_flags(
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

pub(super) struct DriverFixture(pub(super) std::path::PathBuf);

impl DriverFixture {
    pub(super) fn new() -> Self {
        let name = Alphanumeric.sample_string(&mut rand::thread_rng(), 24);
        let directory = std::path::PathBuf::from("tests/temp").join(format!("driver-{name}"));
        fs::create_dir_all(&directory).unwrap();
        Self(directory.canonicalize().unwrap())
    }

    pub(super) fn write(&self, name: &str, code: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, code).unwrap();
    }

    pub(super) fn cc(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dynamite-cc"))
            .current_dir(&self.0)
            .env("DYNAMITE_CLANG", "clang")
            .args(arguments)
            .output()
            .unwrap()
    }

    pub(super) fn success(&self, arguments: &[&str]) {
        let result = self.cc(arguments);
        assert!(result.status.success(), "{result:?}");
    }

    pub(super) fn executable_status(&self, name: &str) -> Option<i32> {
        Command::new(self.0.join(name)).status().unwrap().code()
    }
}

impl Drop for DriverFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

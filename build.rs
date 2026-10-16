use std::{env, path::PathBuf, process::Command};

fn output(program: &str, arguments: &[&str]) -> String {
    let result = Command::new(program)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {program}: {error}"));
    assert!(
        result.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout)
        .expect("tool output must be UTF-8")
        .trim()
        .to_string()
}

fn main() {
    for name in ["LLVM_CONFIG", "CXX", "AR", "DYNAMITE_CLANG_INCLUDE_DIR"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=src/native_clang.cpp");
    if env::var_os("CARGO_FEATURE_CLANG_IN_PROCESS").is_none() {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "linux",
        "clang-in-process currently supports Linux only"
    );
    assert_eq!(
        env::var("HOST").unwrap(),
        env::var("TARGET").unwrap(),
        "clang-in-process requires a native build"
    );
    let config = env::var("LLVM_CONFIG").unwrap_or_else(|_| "llvm-config-15".into());
    let version = output(&config, &["--version"]);
    assert!(
        version.starts_with("15."),
        "clang-in-process requires LLVM/Clang 15, found {version}"
    );
    let bindir = output(&config, &["--bindir"]);
    println!("cargo:rustc-env=DYNAMITE_NATIVE_CLANG={}/clang", bindir);
    let include = output(&config, &["--includedir"]);
    let library = PathBuf::from(output(&config, &["--libdir"]));
    let clang_include = env::var("DYNAMITE_CLANG_INCLUDE_DIR").unwrap_or_else(|_| include.clone());
    assert!(
        PathBuf::from(&clang_include)
            .join("clang/Frontend/CompilerInstance.h")
            .is_file(),
        "install libclang-15-dev or set DYNAMITE_CLANG_INCLUDE_DIR"
    );
    let cpp_library = ["libclang-cpp.so", "libclang-cpp.so.15"]
        .into_iter()
        .find(|name| library.join(name).is_file())
        .expect("install libclang-cpp15 or libclang-cpp15-dev");
    let directory = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = directory.join("native_clang.o");
    let compiler = env::var("CXX").unwrap_or_else(|_| "c++".into());
    let result = Command::new(&compiler)
        .args([
            "-std=c++17",
            "-O2",
            "-fPIC",
            "-I",
            &clang_include,
            "-I",
            &include,
            "-c",
            "src/native_clang.cpp",
            "-o",
        ])
        .arg(&object)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {compiler}: {error}"));
    assert!(
        result.success(),
        "Clang integration bridge compilation failed"
    );
    let archiver = env::var("AR").unwrap_or_else(|_| "ar".into());
    let result = Command::new(&archiver)
        .arg("crs")
        .arg(directory.join("libdynamite_clang.a"))
        .arg(object)
        .status()
        .expect("failed to run ar");
    assert!(result.success(), "Clang integration bridge archive failed");
    println!("cargo:rustc-link-search=native={}", directory.display());
    println!("cargo:rustc-link-search=native={}", library.display());
    println!("cargo:rustc-link-lib=static=dynamite_clang");
    println!("cargo:rustc-link-lib=dylib:+verbatim={cpp_library}");
    for flag in output(&config, &["--libs", "--link-shared"]).split_whitespace() {
        let name = flag
            .strip_prefix("-l")
            .expect("unexpected llvm-config library flag");
        println!("cargo:rustc-link-lib=dylib={name}");
    }
    println!("cargo:rustc-link-lib=dylib=stdc++");
}

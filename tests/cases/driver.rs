use crate::support::DriverFixture;
use std::{fs, process::Command};
use test_case::test_matrix;

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

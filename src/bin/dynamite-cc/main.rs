//! Native C build driver with external Clang or LLVM tools.

mod arguments;
mod llvm_tools;

use dynamite_compiler::timing::Timer;
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Link,
    Object,
    Assembly,
}

struct Input {
    name: String,
    source: bool,
}

enum LinkArgument {
    Option(String),
    Input(usize),
}

struct Arguments {
    mode: Mode,
    preprocess_only: bool,
    emit_llvm: bool,
    jobs: usize,

    inputs: Vec<Input>,
    output: Option<String>,

    preprocessing: Vec<String>,
    backend: Vec<String>,
    linker: Vec<LinkArgument>,
    forwarded: Vec<String>,

    dependencies: Option<bool>,
    dependency_file: Option<String>,
    dependency_targets: Vec<String>,
    phony_dependencies: bool,
}

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Result<Self, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        for attempt in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "dynamite-cc-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("failed to create compilation directory".into())
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn status(command: &mut Command) -> Result<i32, String> {
    command
        .status()
        .map(|status| status.code().unwrap_or(1))
        .map_err(|error| format!("failed to run Clang: {error}"))
}

fn compile(
    input: &Input,
    index: usize,
    args: &Arguments,
    clang: &str,
    temporary: &Path,
) -> Result<PathBuf, String> {
    let output = if args.mode == Mode::Link {
        temporary.join(format!("{index}.o"))
    } else {
        args.output_for(input)
    };
    let mut preprocessing = args.preprocessing.clone();
    if let Some(system) = args.dependencies {
        let dependency_output = if args.mode == Mode::Link {
            Cow::Owned(PathBuf::from(args.output.as_deref().unwrap_or("a.out")))
        } else {
            Cow::Borrowed(&output)
        };
        let default_dependency_file = if args.mode == Mode::Link
            && args.inputs.iter().filter(|input| input.source).count() > 1
        {
            PathBuf::from(format!("{}-{index}.d", dependency_output.display()))
        } else {
            dependency_output.with_extension("d")
        };
        preprocessing.extend([
            if system { "-MD" } else { "-MMD" }.into(),
            "-MF".into(),
            args.dependency_file
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or(default_dependency_file)
                .to_string_lossy()
                .into_owned(),
        ]);
        if args.dependency_targets.is_empty() {
            preprocessing.extend([
                "-MQ".into(),
                dependency_output.to_string_lossy().into_owned(),
            ]);
        } else {
            preprocessing.extend(args.dependency_targets.clone());
        }
        if args.phony_dependencies {
            preprocessing.push("-MP".into());
        }
    }
    let timer = Timer::new("frontend_total");
    let one_shot =
        args.mode != Mode::Link && args.inputs.iter().filter(|input| input.source).count() == 1;
    let generate = if one_shot {
        dynamite_compiler::generate_file_once
    } else {
        dynamite_compiler::generate_file
    };
    let ir = generate(Path::new(&input.name), "llvm", false, &preprocessing)?;
    drop(timer);
    let ir_path = temporary.join(format!("{index}.ll"));
    {
        let _timer = Timer::new("write_ir");
        fs::write(&ir_path, &ir).map_err(|e| format!("failed to write LLVM IR: {e}"))?;
    }
    if llvm_tools::selected()? {
        let _timer = Timer::new("lower");
        llvm_tools::lower(
            &args.backend,
            &ir_path,
            &output,
            args.mode == Mode::Assembly,
        )?;
        return Ok(output);
    }
    let mut command = Command::new(clang);
    let mut backend_index = 0;
    while backend_index < args.backend.len() {
        let arg = &args.backend[backend_index];
        if matches!(arg.as_str(), "-L" | "-l" | "-framework" | "-Xlinker") {
            backend_index += 2;
            continue;
        }
        if !(arg.starts_with("-Wl,")
            || arg.starts_with("-L")
            || arg.starts_with("-l")
            || arg.starts_with("-fuse-ld=")
            || matches!(
                arg.as_str(),
                "-shared" | "-dynamiclib" | "-static" | "-pie" | "-no-pie" | "-rdynamic" | "-s"
            ))
        {
            command.arg(arg);
        }
        backend_index += 1;
    }
    command
        .arg(if args.mode == Mode::Assembly {
            "-S"
        } else {
            "-c"
        })
        .arg(&ir_path)
        .arg("-o")
        .arg(&output)
        .arg("-Wno-override-module");
    let _timer = Timer::new("lower");
    let code = status(&mut command)?;
    if code != 0 {
        return Err(format!("LLVM lowering failed for {}", input.name));
    }
    Ok(output)
}

/// Run the C build driver. C inputs always pass through the Dynamite frontend.
/// `--jobs N` bounds independent translation units; the default is one worker.
fn run(arguments: &[String]) -> Result<i32, String> {
    if arguments.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Usage: dynamite-cc [--jobs N] [Clang options] FILE...\nCompile C through Dynamite. DYNAMITE_BACKEND=llvm selects external llc/opt and a native linker on Linux x86_64 GNU or macOS; the default backend is Clang. Supports -c, -S, -o, -I/-D/-U, and dependency files. Default: one worker."
        );
        return Ok(0);
    }
    let args = Arguments::parse(arguments)?;
    let llvm = llvm_tools::selected()?;
    let clang = std::env::var("DYNAMITE_CLANG").unwrap_or_else(|_| "clang".into());
    if args.preprocess_only {
        let native_backend_option = |arg: &String| {
            arg.starts_with("-O")
                || arg.starts_with("-W")
                || arg.starts_with("-g")
                || matches!(arg.as_str(), "-pthread" | "-pedantic")
        };
        let mut backend_index = 0;
        let mut unsupported = None;
        while backend_index < args.backend.len() {
            let option = &args.backend[backend_index];
            if cfg!(target_os = "macos")
                && matches!(option.as_str(), "-F" | "-iframework" | "-isysroot")
            {
                backend_index += 2;
            } else if native_backend_option(option)
                || cfg!(target_os = "macos")
                    && (option.starts_with("-F")
                        || option.starts_with("-iframework")
                        || option.starts_with("-isysroot"))
            {
                backend_index += 1;
            } else {
                unsupported = Some(option);
                break;
            }
        }
        let native_dependencies = args
            .forwarded
            .iter()
            .any(|arg| matches!(arg.as_str(), "-M" | "-MM"));
        if args.inputs.is_empty() {
            return Err("native preprocessing requires a source file".into());
        }
        let _timer = Timer::new(if native_dependencies {
            "dependency_scan"
        } else {
            "preprocess_only"
        });
        if let Some(option) = unsupported {
            return Err(format!("unsupported native preprocessing option: {option}"));
        }
        let mut preprocessing = args.preprocessing.clone();
        for flag in ["-M", "-MM", "-P", "-MP"] {
            if args.forwarded.iter().any(|arg| arg == flag) {
                preprocessing.push(flag.into());
            }
        }
        if let Some(file) = &args.dependency_file {
            preprocessing.extend(["-MF".into(), file.clone()]);
        }
        preprocessing.extend(args.dependency_targets.clone());
        let paths = args
            .inputs
            .iter()
            .map(|input| PathBuf::from(&input.name))
            .collect::<Vec<_>>();
        let output =
            dynamite_compiler::preprocessor::preprocess_files_native(&paths, &preprocessing)?;
        if let Some(path) = &args.output {
            fs::write(path, output).map_err(|e| e.to_string())?;
        } else {
            use std::io::Write;
            std::io::stdout()
                .write_all(output.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        return Ok(0);
    }
    let sources = args
        .inputs
        .iter()
        .filter(|input| input.source)
        .collect::<Vec<_>>();
    if sources.is_empty() {
        if llvm {
            if arguments == ["--version"] {
                println!(
                    "dynamite-cc {} (LLVM tools backend)",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(0);
            }
            if arguments == ["-dumpmachine"] {
                println!("{}", llvm_tools::target_triple()?);
                return Ok(0);
            }
            let _timer = Timer::new("link_or_other");
            llvm_tools::link(&args.link_arguments(&[]))?;
            return Ok(0);
        }
        if arguments.iter().any(|arg| arg == "--version") {
            println!("dynamite-cc {} (clang backend)", env!("CARGO_PKG_VERSION"));
        }
        let _timer = Timer::new("link_or_other");
        return status(Command::new(&clang).args(&args.forwarded));
    }
    let temporary = TemporaryDirectory::new()?;
    let workers = args.jobs.min(sources.len());
    let compile_one = |index: usize| {
        std::panic::catch_unwind(|| compile(sources[index], index, &args, &clang, &temporary.0))
            .unwrap_or_else(|_| Err(format!("compilation failed for {}", sources[index].name)))
    };
    let outputs = if workers == 1 {
        (0..sources.len())
            .map(compile_one)
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let next = AtomicUsize::new(0);
        let results = std::thread::scope(|scope| {
            let handles = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        let mut completed = vec![];
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            if index >= sources.len() {
                                break;
                            }
                            completed.push((index, compile_one(index)));
                        }
                        completed
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        let mut results = results;
        results.sort_by_key(|(index, _)| *index);
        results
            .into_iter()
            .map(|(_, output)| output)
            .collect::<Result<Vec<_>, _>>()?
    };
    if args.mode != Mode::Link {
        return Ok(0);
    }
    let arguments = args.link_arguments(&outputs);
    let _timer = Timer::new("link");
    if llvm {
        llvm_tools::link(&arguments)?;
        Ok(0)
    } else {
        status(Command::new(&clang).args(&arguments))
    }
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match run(&arguments) {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("dynamite-cc: {message}");
            std::process::exit(1);
        }
    }
}

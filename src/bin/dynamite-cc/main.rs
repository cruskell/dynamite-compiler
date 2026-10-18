//! Native C build driver with external Clang or LLVM tools.

#[path = "dynamite-cc/llvm_tools.rs"]
mod llvm_tools;

use dynamite_compiler::timing::Timer;
use std::{
    borrow::Cow,
    collections::HashSet,
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
    inputs: Vec<Input>,
    backend: Vec<String>,
    linker: Vec<LinkArgument>,
    preprocessing: Vec<String>,
    forwarded: Vec<String>,
    output: Option<String>,
    mode: Mode,
    jobs: usize,
    dependencies: Option<bool>,
    dependency_file: Option<String>,
    dependency_targets: Vec<String>,
    phony_dependencies: bool,
    preprocess_only: bool,
    emit_llvm: bool,
}

fn argument_value(args: &[String], index: &mut usize) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{} requires an argument", args[*index - 1]))
}

impl Arguments {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut result = Self {
            inputs: vec![],
            backend: vec![],
            linker: vec![],
            preprocessing: vec![],
            forwarded: vec![],
            output: None,
            mode: Mode::Link,
            jobs: 1,
            dependencies: None,
            dependency_file: None,
            dependency_targets: vec![],
            phony_dependencies: false,
            preprocess_only: false,
            emit_llvm: false,
        };
        let preprocess_only = args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-E" | "-M" | "-MM"));
        let mut c_language = false;
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            let start = index;
            let backend_start = result.backend.len();
            let input_start = result.inputs.len();
            match arg.as_str() {
                "-j" | "--jobs" => {
                    result.jobs = argument_value(args, &mut index)?
                        .parse()
                        .map_err(|_| "jobs must be a positive integer")?;
                }
                _ if arg.starts_with("--jobs=") || arg.starts_with("-j") => {
                    let value = arg.strip_prefix("--jobs=").unwrap_or(&arg[2..]);
                    result.jobs = value
                        .parse()
                        .map_err(|_| "jobs must be a positive integer")?;
                }
                "-o" | "--output" => result.output = Some(argument_value(args, &mut index)?),
                _ if arg.starts_with("--output=") => {
                    result.output = Some(arg[9..].into());
                }
                _ if arg.starts_with("-o") && arg.len() > 2 => {
                    result.output = Some(arg[2..].into());
                }
                "-c" => result.mode = Mode::Object,
                "-S" => result.mode = Mode::Assembly,
                "-emit-llvm" => {
                    result.emit_llvm = true;
                    result.backend.push(arg.clone());
                }
                "-E" | "-M" | "-MM" => result.preprocess_only = true,
                "-MD" | "-MMD" => result.dependencies = Some(arg == "-MD"),
                "-MF" => result.dependency_file = Some(argument_value(args, &mut index)?),
                "-MT" | "-MQ" => {
                    result.dependency_targets.push(arg.clone());
                    result
                        .dependency_targets
                        .push(argument_value(args, &mut index)?);
                }
                "-MP" => result.phony_dependencies = true,
                _ if ["-MF", "-MT", "-MQ"].iter().any(|p| arg.starts_with(p)) => {
                    let value = arg[3..].to_string();
                    if arg.starts_with("-MF") {
                        result.dependency_file = Some(value);
                    } else {
                        result.dependency_targets.extend([arg[..3].into(), value]);
                    }
                }
                "-I" | "-D" | "-U" | "-F" | "-iframework" | "-isystem" | "-iquote" | "-include"
                | "-imacros" | "-idirafter" | "-isysroot" | "--sysroot" => {
                    result.preprocessing.push(arg.clone());
                    let value = argument_value(args, &mut index)?;
                    result.preprocessing.push(value.clone());
                    if matches!(
                        arg.as_str(),
                        "-isysroot" | "--sysroot" | "-F" | "-iframework"
                    ) {
                        result.backend.extend([arg.clone(), value]);
                    }
                }
                "-x" => {
                    let language = argument_value(args, &mut index)?;
                    c_language = match language.as_str() {
                        "c" => true,
                        "none" => false,
                        _ => return Err(format!("unsupported input language: {language}")),
                    };
                }
                _ if arg.starts_with("-x") && arg.len() > 2 => {
                    c_language = match &arg[2..] {
                        "c" => true,
                        "none" => false,
                        language => return Err(format!("unsupported input language: {language}")),
                    };
                }
                "--" => return Err("use ./ prefixes for input names starting with '-'".into()),
                "-target" | "--target" | "-arch" | "-m32" | "-fshort-enums" | "-fshort-wchar"
                | "-funsigned-char" | "-fsigned-char" => {
                    return Err(format!("unsupported target or language option: {arg}"));
                }
                "-Xclang" | "-Xpreprocessor" | "-include-pch" | "-fsyntax-only" | "--compile"
                | "--assemble" | "--preprocess" | "--analyze" | "--precompile" | "-emit-ast"
                | "-fdriver-only" | "-###" => {
                    return Err(format!("unsupported frontend option: {arg}"));
                }
                "-L"
                | "-l"
                | "-framework"
                | "-Xlinker"
                | "-Xassembler"
                | "-undefined"
                | "-install_name"
                | "-rpath"
                | "-compatibility_version"
                | "-current_version" => {
                    result.backend.push(arg.clone());
                    result.backend.push(argument_value(args, &mut index)?);
                }
                "-" if !preprocess_only => {
                    return Err("C input from stdin is not supported; use a source file".into());
                }
                _ if arg.starts_with('@') => return Err("response files are not supported".into()),
                _ if arg.starts_with("--target=") || arg.starts_with("-fpack-struct") => {
                    return Err(format!("unsupported target or language option: {arg}"));
                }
                _ if arg.starts_with('-') => {
                    if arg.starts_with("-include-pch")
                        || arg.starts_with("-Xclang")
                        || arg.starts_with("-Xpreprocessor")
                        || arg.starts_with("-save-temps")
                    {
                        return Err(format!("unsupported frontend option: {arg}"));
                    }
                    if arg.starts_with("-Wp,") {
                        return Err("use explicit preprocessing options instead of -Wp,".into());
                    }
                    if [
                        "-I",
                        "-F",
                        "-iframework",
                        "-D",
                        "-U",
                        "-isystem",
                        "-iquote",
                        "-include",
                        "-imacros",
                        "-idirafter",
                    ]
                    .iter()
                    .any(|prefix| arg.starts_with(prefix))
                    {
                        result.preprocessing.push(arg.clone());
                        if arg.starts_with("-F") || arg.starts_with("-iframework") {
                            result.backend.push(arg.clone());
                        }
                    } else if arg.starts_with("--sysroot=") || arg.starts_with("-isysroot") {
                        result.preprocessing.push(arg.clone());
                        result.backend.push(arg.clone());
                    } else {
                        if arg.starts_with("-O")
                            || arg.starts_with("-std=")
                            || arg == "-pthread"
                            || (arg.starts_with("-W")
                                && !arg.starts_with("-Wl,")
                                && !arg.starts_with("-Wa,"))
                        {
                            result.preprocessing.push(arg.clone());
                        }
                        if !arg.starts_with("-std=") {
                            result.backend.push(arg.clone());
                        }
                    }
                }
                _ => {
                    let extension = Path::new(arg).extension().and_then(|s| s.to_str());
                    let source = c_language || matches!(extension, Some("c" | "i"));
                    if source && Path::new(arg).file_name().is_none() {
                        return Err(format!("invalid source path: {arg}"));
                    }
                    let versioned_library = Path::new(arg)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.rsplit_once(".so."))
                        .is_some_and(|(_, version)| {
                            version.split('.').all(|part| {
                                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                            })
                        });
                    if !source
                        && !preprocess_only
                        && !matches!(
                            extension,
                            None | Some(
                                "s" | "S"
                                    | "o"
                                    | "obj"
                                    | "a"
                                    | "so"
                                    | "dylib"
                                    | "lib"
                                    | "dll"
                                    | "ll"
                                    | "bc"
                                    | "ld"
                                    | "lds"
                                    | "tbd"
                            )
                        )
                        && !versioned_library
                    {
                        return Err(format!("unsupported frontend input: {arg}"));
                    }
                    result.inputs.push(Input {
                        name: arg.clone(),
                        source,
                    });
                }
            }
            result.linker.extend(
                result.backend[backend_start..]
                    .iter()
                    .cloned()
                    .map(LinkArgument::Option),
            );
            if result.inputs.len() > input_start {
                result.linker.push(LinkArgument::Input(input_start));
            }
            if arg != "-j"
                && arg != "--jobs"
                && !arg.starts_with("--jobs=")
                && !arg.starts_with("-j")
            {
                result.forwarded.extend_from_slice(&args[start..=index]);
            }
            index += 1;
        }
        if result.jobs == 0 {
            return Err("jobs must be a positive integer".into());
        }
        if result.preprocess_only {
            return Ok(result);
        }
        let count = result.inputs.iter().filter(|input| input.source).count();
        if count > 0 && result.mode != Mode::Link && result.inputs.iter().any(|input| !input.source)
        {
            return Err("compile mixed C and non-C inputs in separate invocations".into());
        }
        if count > 1 && result.mode != Mode::Link && result.output.is_some() {
            return Err("cannot use one -o output with multiple compilation inputs".into());
        }
        if count > 1 && result.dependency_file.is_some() {
            return Err("multiple sources require separate default dependency files".into());
        }
        if result.dependency_file.as_deref() == Some("-") && !result.preprocess_only {
            return Err("dependency output to stdout is not supported when compiling".into());
        }
        if result.mode != Mode::Link {
            let mut outputs = HashSet::new();
            for input in result.inputs.iter().filter(|input| input.source) {
                if !outputs.insert(result.output_for(input)) {
                    return Err("compilation inputs would overwrite the same output".into());
                }
            }
        }
        Ok(result)
    }

    fn link_arguments(&self, outputs: &[PathBuf]) -> Vec<String> {
        let mut objects = outputs.iter();
        let mut arguments = vec![];
        for argument in &self.linker {
            match argument {
                LinkArgument::Option(option) => arguments.push(option.clone()),
                LinkArgument::Input(index) => {
                    let input = &self.inputs[*index];
                    arguments.push(if input.source {
                        objects.next().unwrap().to_string_lossy().into_owned()
                    } else {
                        input.name.clone()
                    });
                }
            }
        }
        if let Some(output) = &self.output {
            arguments.extend(["-o".into(), output.clone()]);
        }
        arguments
    }

    fn output_for(&self, input: &Input) -> PathBuf {
        if let Some(output) = &self.output {
            return output.into();
        }
        let suffix = match (self.mode, self.emit_llvm) {
            (Mode::Assembly, true) => "ll",
            (Mode::Assembly, false) => "s",
            (_, true) => "bc",
            _ => "o",
        };
        PathBuf::from(Path::new(&input.name).file_name().unwrap()).with_extension(suffix)
    }
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

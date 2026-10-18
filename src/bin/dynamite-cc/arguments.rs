//! Parse compiler arguments and preserve source/linker argument ordering.

use super::{Arguments, Input, LinkArgument, Mode};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

fn argument_value(args: &[String], index: &mut usize) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{} requires an argument", args[*index - 1]))
}

impl Arguments {
    pub(super) fn parse(args: &[String]) -> Result<Self, String> {
        let mut result = Self {
            mode: Mode::Link,
            preprocess_only: false,
            emit_llvm: false,
            jobs: 1,
            inputs: vec![],
            output: None,
            preprocessing: vec![],
            backend: vec![],
            linker: vec![],
            forwarded: vec![],
            dependencies: None,
            dependency_file: None,
            dependency_targets: vec![],
            phony_dependencies: false,
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

    pub(super) fn link_arguments(&self, outputs: &[PathBuf]) -> Vec<String> {
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

    pub(super) fn output_for(&self, input: &Input) -> PathBuf {
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

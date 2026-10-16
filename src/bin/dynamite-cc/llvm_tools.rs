//! External LLVM tools for supported native GNU and Darwin toolchains.
use std::{fs, path::PathBuf, process::Command};

use dynamite_compiler::timing::Timer;

pub(super) fn selected() -> Result<bool, String> {
    match std::env::var("DYNAMITE_BACKEND").as_deref() {
        Err(_) | Ok("") | Ok("clang") => Ok(false),
        Ok("llvm")
            if cfg!(any(
                all(
                    target_os = "linux",
                    target_arch = "x86_64",
                    target_env = "gnu"
                ),
                all(
                    target_os = "macos",
                    any(target_arch = "aarch64", target_arch = "x86_64")
                )
            )) =>
        {
            Ok(true)
        }
        Ok("llvm") => {
            Err("the LLVM tools backend requires Linux x86_64 GNU or macOS ARM64/x86_64".into())
        }
        Ok(value) => Err(format!("unknown DYNAMITE_BACKEND: {value}")),
    }
}

fn tool(variable: &str, default: &str) -> Command {
    Command::new(std::env::var_os(variable).unwrap_or_else(|| default.into()))
}

fn execute(command: &mut Command) -> Result<(), String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let status = command
        .status()
        .map_err(|error| format!("failed to run {name}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} failed with {status}"))
    }
}

fn optimization(options: &[String]) -> Result<&str, String> {
    let mut level = "0";
    for option in options {
        if let Some(value) = option.strip_prefix("-O") {
            level = match value {
                "0" | "1" | "2" | "3" => value,
                "s" | "z" => value,
                "g" => "1",
                "fast" => return Err("the LLVM tools backend does not support -Ofast".into()),
                _ => return Err(format!("unsupported LLVM optimization option: {option}")),
            };
        }
    }
    Ok(level)
}

fn link_only(option: &str) -> bool {
    option.starts_with("-Wl,")
        || option.starts_with("-L")
        || option.starts_with("-l")
        || matches!(
            option,
            "-shared"
                | "-dynamiclib"
                | "-bundle"
                | "-dynamic"
                | "-pie"
                | "-no-pie"
                | "-rdynamic"
                | "-s"
                | "-Xlinker"
        )
}

pub(super) fn target_triple() -> Result<String, String> {
    if cfg!(target_os = "macos") {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };
        Ok(format!(
            "{arch}-apple-macosx{}",
            dynamite_compiler::toolchain::macos_deployment_target()?
        ))
    } else {
        Ok("x86_64-pc-linux-gnu".into())
    }
}

pub(super) fn lower(
    options: &[String],
    input: &std::path::Path,
    output: &std::path::Path,
    assembly: bool,
) -> Result<(), String> {
    let level = optimization(options)?;
    let triple = target_triple()?;
    let mut llc = tool("DYNAMITE_LLC", "llc");
    let mut index = 0;
    while index < options.len() {
        let option = &options[index];
        if matches!(option.as_str(), "-L" | "-l" | "-Xlinker")
            || cfg!(target_os = "macos")
                && matches!(
                    option.as_str(),
                    "-framework"
                        | "-F"
                        | "-iframework"
                        | "-undefined"
                        | "-install_name"
                        | "-rpath"
                        | "-compatibility_version"
                        | "-current_version"
                        | "-isysroot"
                )
        {
            index += 2;
            continue;
        }
        if cfg!(target_os = "macos")
            && (option.starts_with("-F")
                || option.starts_with("-iframework")
                || option.starts_with("-isysroot")
                || matches!(option.as_str(), "-fno-common" | "-fno-strict-aliasing"))
        {
            // Default definitions already have non-common linkage; IR has no TBAA.
        } else if option.starts_with("-O")
            || link_only(option)
            || (option.starts_with("-W") && !option.starts_with("-Wa,"))
            || matches!(
                option.as_str(),
                "-g" | "-g0"
                    | "-ggdb"
                    | "-pthread"
                    | "-pedantic"
                    | "-fPIC"
                    | "-fpic"
                    | "-fPIE"
                    | "-fpie"
            )
        {
            // Debug information is not emitted by Dynamite's IR generator.
        } else if option == "-fomit-frame-pointer" {
            llc.arg("--frame-pointer=none");
        } else if option == "-fno-omit-frame-pointer" {
            llc.arg("--frame-pointer=all");
        } else {
            return Err(format!("unsupported LLVM tools backend option: {option}"));
        }
        index += 1;
    }
    let optimized = input.with_extension("optimized.ll");
    let source = if level != "0" {
        let _timer = Timer::new("optimize");
        execute(
            tool("DYNAMITE_OPT", "opt")
                .arg(format!("-passes=default<O{level}>"))
                .arg(format!("-mtriple={triple}"))
                .arg("-S")
                .arg(input)
                .arg("-o")
                .arg(&optimized),
        )?;
        &optimized
    } else {
        input
    };
    llc.arg(if assembly {
        "-filetype=asm"
    } else {
        "-filetype=obj"
    })
    .arg(format!(
        "-O{}",
        if matches!(level, "s" | "z") {
            "2"
        } else {
            level
        }
    ))
    .arg("-relocation-model=pic")
    .arg(format!("-mtriple={triple}"))
    .arg(if cfg!(target_arch = "aarch64") {
        "-mcpu=generic"
    } else {
        "-mcpu=x86-64"
    })
    .arg(source)
    .arg("-o")
    .arg(output);
    execute(&mut llc)
}

fn gcc_runtime() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DYNAMITE_GCC_RUNTIME") {
        let path = PathBuf::from(path);
        if path.join("crtbeginS.o").is_file() && path.join("crtendS.o").is_file() {
            return Ok(path);
        }
        return Err("DYNAMITE_GCC_RUNTIME must contain crtbeginS.o and crtendS.o".into());
    }
    let mut versions = fs::read_dir("/usr/lib/gcc/x86_64-linux-gnu")
        .map_err(|error| {
            format!("cannot find GNU runtime files; set DYNAMITE_GCC_RUNTIME: {error}")
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let version = entry
                .file_name()
                .to_string_lossy()
                .split('.')
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let path = entry.path();
            (path.join("crtbeginS.o").is_file() && path.join("crtendS.o").is_file())
                .then_some((version, path))
        })
        .collect::<Vec<_>>();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions
        .into_iter()
        .next()
        .map(|(_, path)| path)
        .ok_or_else(|| "cannot find GNU runtime files; set DYNAMITE_GCC_RUNTIME".into())
}

pub(super) fn link(arguments: &[String]) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return link_macos(arguments);
    }
    link_gnu(arguments)
}

fn link_gnu(arguments: &[String]) -> Result<(), String> {
    let runtime = gcc_runtime()?;
    let system = PathBuf::from("/usr/lib/x86_64-linux-gnu");
    let shared = arguments.iter().any(|arg| arg == "-shared");
    let pie = !shared
        && arguments
            .iter()
            .rev()
            .find(|arg| matches!(arg.as_str(), "-pie" | "-no-pie"))
            .is_none_or(|arg| arg != "-no-pie");
    let mut command = tool("DYNAMITE_LLD", "ld.lld");
    command.arg("--eh-frame-hdr");
    if shared {
        command.arg("-shared");
    } else {
        if pie {
            command.arg("-pie");
        }
        command.args(["-dynamic-linker", "/lib64/ld-linux-x86-64.so.2"]);
        command.arg(system.join(if pie { "Scrt1.o" } else { "crt1.o" }));
    }
    command
        .arg(system.join("crti.o"))
        .arg(runtime.join(if shared || pie {
            "crtbeginS.o"
        } else {
            "crtbegin.o"
        }));
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        match argument.as_str() {
            "-o" | "-L" | "-l" | "-Xlinker" => {
                if argument != "-Xlinker" {
                    command.arg(argument);
                }
                index += 1;
                command.arg(
                    arguments
                        .get(index)
                        .ok_or_else(|| format!("{argument} requires an argument"))?,
                );
            }
            "-pthread" => {
                command.arg("-lpthread");
            }
            "-rdynamic" => {
                command.arg("--export-dynamic");
            }
            "-shared" | "-pie" | "-no-pie" => {}
            _ if argument.starts_with("-Wl,") => {
                command.args(argument[4..].split(','));
            }
            _ if argument.starts_with("-O")
                || (argument.starts_with("-W") && !argument.starts_with("-Wa,"))
                || matches!(
                    argument.as_str(),
                    "-g" | "-g0"
                        | "-ggdb"
                        | "-fPIC"
                        | "-fpic"
                        | "-fPIE"
                        | "-fpie"
                        | "-fomit-frame-pointer"
                        | "-fno-omit-frame-pointer"
                ) => {}
            _ if argument.starts_with("-L") || argument.starts_with("-l") || argument == "-s" => {
                command.arg(argument);
            }
            _ if argument.starts_with('-') => {
                return Err(format!("unsupported LLVM tools linker option: {argument}"));
            }
            _ => {
                if !matches!(
                    std::path::Path::new(argument)
                        .extension()
                        .and_then(|part| part.to_str()),
                    Some("o" | "a" | "so")
                ) && !argument.contains(".so.")
                {
                    return Err(format!("unsupported LLVM tools linker input: {argument}"));
                }
                command.arg(argument);
            }
        }
        index += 1;
    }
    command
        .arg(format!("-L{}", runtime.display()))
        .arg("-L/usr/lib/x86_64-linux-gnu")
        .arg("-L/lib/x86_64-linux-gnu")
        .args(["-lc", "-lgcc", "--as-needed", "-lgcc_s", "--no-as-needed"])
        .arg(runtime.join(if shared || pie {
            "crtendS.o"
        } else {
            "crtend.o"
        }))
        .arg(system.join("crtn.o"));
    execute(&mut command)
}

fn link_macos(arguments: &[String]) -> Result<(), String> {
    let mut sdk = None;
    for (index, argument) in arguments.iter().enumerate() {
        if argument == "-isysroot" {
            sdk = Some(PathBuf::from(
                arguments
                    .get(index + 1)
                    .ok_or("-isysroot requires an argument")?,
            ));
        } else if let Some(root) = argument.strip_prefix("-isysroot") {
            sdk = Some(PathBuf::from(root));
        }
    }
    let sdk = sdk
        .map(Ok)
        .unwrap_or_else(dynamite_compiler::toolchain::macos_sdk_root)?;
    let version = Command::new("/usr/bin/xcrun")
        .arg("--sdk")
        .arg(&sdk)
        .arg("--show-sdk-version")
        .output()
        .map_err(|e| format!("cannot read macOS SDK version: {e}"))?;
    if !version.status.success() {
        return Err("cannot read macOS SDK version".into());
    }
    let version = String::from_utf8(version.stdout).map_err(|e| e.to_string())?;
    dynamite_compiler::toolchain::macos_version_number(version.trim())?;
    let minimum = dynamite_compiler::toolchain::macos_deployment_target()?;
    let mut command = tool("DYNAMITE_LD", "/usr/bin/ld");
    command
        .args([
            "-arch",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            },
        ])
        .arg("-syslibroot")
        .arg(&sdk)
        .args(["-platform_version", "macos", &minimum, version.trim()]);
    let dylib = arguments
        .iter()
        .any(|a| matches!(a.as_str(), "-shared" | "-dynamiclib"));
    let bundle = arguments.iter().any(|a| a == "-bundle");
    if dylib && bundle {
        return Err("cannot combine a dynamic library and a bundle".into());
    }
    if dylib {
        command.arg("-dylib");
    }
    if bundle {
        command.arg("-bundle");
    }
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        match argument.as_str() {
            "-o"
            | "-L"
            | "-l"
            | "-framework"
            | "-F"
            | "-iframework"
            | "-Xlinker"
            | "-undefined"
            | "-install_name"
            | "-rpath"
            | "-compatibility_version"
            | "-current_version"
            | "-isysroot" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| format!("{argument} requires an argument"))?;
                if argument != "-isysroot" {
                    if argument != "-Xlinker" {
                        command.arg(if argument == "-iframework" {
                            "-F"
                        } else {
                            argument
                        });
                    }
                    command.arg(value);
                }
                index += 1;
            }
            "-shared" | "-dynamiclib" | "-bundle" | "-dynamic" | "-pthread" => {}
            "-rdynamic" => {
                command.arg("-export_dynamic");
            }
            "-no-pie" => {
                command.arg("-no_pie");
            }
            "-pie" | "-s" => {
                command.arg(argument);
            }
            _ if argument.starts_with("-Wl,") => {
                command.args(argument[4..].split(','));
            }
            _ if argument.starts_with("-iframework") => {
                command.arg("-F").arg(&argument[11..]);
            }
            _ if argument.starts_with("-isysroot")
                || argument.starts_with("-O")
                || (argument.starts_with("-W") && !argument.starts_with("-Wa,"))
                || matches!(
                    argument.as_str(),
                    "-g" | "-g0"
                        | "-ggdb"
                        | "-fPIC"
                        | "-fpic"
                        | "-fPIE"
                        | "-fpie"
                        | "-fomit-frame-pointer"
                        | "-fno-omit-frame-pointer"
                        | "-fno-common"
                        | "-fno-strict-aliasing"
                ) => {}
            _ if argument.starts_with("-F")
                || argument.starts_with("-L")
                || argument.starts_with("-l") =>
            {
                command.arg(argument);
            }
            _ if argument.starts_with('-') => {
                return Err(format!("unsupported LLVM tools linker option: {argument}"));
            }
            _ => {
                if !matches!(
                    std::path::Path::new(argument)
                        .extension()
                        .and_then(|s| s.to_str()),
                    Some("o" | "a" | "dylib" | "tbd")
                ) {
                    return Err(format!("unsupported LLVM tools linker input: {argument}"));
                }
                command.arg(argument);
            }
        }
        index += 1;
    }
    command.arg("-lSystem");
    execute(&mut command)
}

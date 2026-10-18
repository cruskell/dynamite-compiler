//! External LLVM tools for supported native GNU and Darwin toolchains.
mod darwin;
mod gnu;

use darwin::link_macos;
use gnu::link_gnu;
use std::process::Command;

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

pub(super) fn link(arguments: &[String]) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return link_macos(arguments);
    }
    link_gnu(arguments)
}

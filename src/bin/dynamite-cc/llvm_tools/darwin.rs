//! Native Darwin SDK and linker arguments.

use super::{execute, tool};
use std::{path::PathBuf, process::Command};

pub(super) fn link_macos(arguments: &[String]) -> Result<(), String> {
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

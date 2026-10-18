//! Interpret preprocessing arguments and emit source or dependency output.

use super::{Header, Preprocessor, Query, profile};
use crate::preprocessor::{IncludeDirectory, PreprocessorOptions};
use std::rc::Rc;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

fn make_quote(text: &str) -> String {
    text.replace('$', "$$")
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace('\t', "\\\t")
        .replace('#', "\\#")
}

pub(in crate::preprocessor) fn preprocess(
    code: &str,
    path: &Path,
    options: &PreprocessorOptions,
    arguments: &[String],
) -> Result<String, String> {
    preprocess_cached(code, path, options, arguments, &mut HashMap::new())
}

pub(in crate::preprocessor) fn preprocess_many(
    paths: &[PathBuf],
    arguments: &[String],
) -> Result<String, String> {
    let dependencies = arguments
        .iter()
        .any(|arg| matches!(arg.as_str(), "-M" | "-MM"));
    let mut dependency_file = None;
    let mut filtered = vec![];
    let mut index = 0;
    while index < arguments.len() {
        let arg = &arguments[index];
        if dependencies && arg == "-MF" {
            index += 1;
            dependency_file = Some(
                arguments
                    .get(index)
                    .ok_or("-MF requires an argument")?
                    .clone(),
            );
        } else if dependencies && arg.starts_with("-MF") {
            dependency_file = Some(arg[3..].to_string());
        } else {
            filtered.push(arg.clone());
        }
        index += 1;
    }
    let mut headers = HashMap::new();
    let mut result = String::new();
    for path in paths {
        let code = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        result.push_str(&preprocess_cached(
            &code,
            path,
            &PreprocessorOptions {
                system_headers: true,
                ..Default::default()
            },
            &filtered,
            &mut headers,
        )?);
    }
    if let Some(file) = dependency_file.filter(|file| file != "-") {
        fs::write(file, &result).map_err(|e| e.to_string())?;
        Ok(String::new())
    } else {
        Ok(result)
    }
}

fn preprocess_cached(
    code: &str,
    path: &Path,
    options: &PreprocessorOptions,
    arguments: &[String],
    headers: &mut HashMap<PathBuf, Header>,
) -> Result<String, String> {
    if !cfg!(all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    )) {
        return Err("native preprocessing supports Linux/macOS ARM64/x86_64".into());
    }
    let bundled = if options.system_headers {
        Some(IncludeDirectory::native()?)
    } else {
        Some(IncludeDirectory::new()?)
    };
    let mut includes = options.include_directories.clone();
    let mut quotes = vec![];
    let mut after = vec![];
    let mut system_includes = vec![];
    let mut sdk = None;
    let mut frameworks = vec![];
    let mut definitions = vec![];
    let mut forced = vec![];
    let mut dependency_file = None;
    let mut dependency_targets = vec![];
    let mut phony = false;
    let mut dependency_only = false;
    let mut omit_system = false;
    let mut no_markers = false;
    let mut optimization = "0";
    let mut standard = "gnu99";
    let mut warnings_as_errors = false;
    let mut index = 0;
    while index < arguments.len() {
        let arg = &arguments[index];
        let mut value = |prefix: &str| -> Result<String, String> {
            if arg.len() > prefix.len() {
                Ok(arg[prefix.len()..].to_string())
            } else {
                index += 1;
                arguments
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("{arg} requires an argument"))
            }
        };
        match arg.as_str() {
            "-MD" => {}
            "-MMD" => omit_system = true,
            "-M" | "-MM" => {
                dependency_only = true;
                omit_system = arg == "-MM";
            }
            "-MP" => phony = true,
            "-P" => no_markers = true,
            "-E" => {}
            "-Werror" | "-Werror=cpp" => warnings_as_errors = true,
            "-Wno-error" | "-Wno-error=cpp" => warnings_as_errors = false,
            "-pthread" => definitions.push((true, "_REENTRANT=1".into())),
            _ if arg.starts_with("-MF") => dependency_file = Some(value("-MF")?),
            _ if arg.starts_with("-MT") || arg.starts_with("-MQ") => {
                let text = value(&arg[..3])?;
                dependency_targets.push(if arg.starts_with("-MQ") {
                    make_quote(&text)
                } else {
                    text
                });
            }
            _ if arg.starts_with("-isysroot") && cfg!(target_os = "macos") => {
                sdk = Some(PathBuf::from(value("-isysroot")?))
            }
            _ if arg.starts_with("-iframework") => {
                let root = PathBuf::from(value("-iframework")?);
                system_includes.push(root.clone());
                frameworks.push(root)
            }
            _ if arg.starts_with("-F") => frameworks.push(PathBuf::from(value("-F")?)),
            _ if arg.starts_with("-isystem") => {
                system_includes.push(PathBuf::from(value("-isystem")?))
            }
            _ if arg.starts_with("-iquote") => quotes.push(value("-iquote")?.into()),
            _ if arg.starts_with("-idirafter") => after.push(PathBuf::from(value("-idirafter")?)),
            _ if arg.starts_with("-include") => forced.push((false, value("-include")?)),
            _ if arg.starts_with("-imacros") => forced.push((true, value("-imacros")?)),
            _ if arg.starts_with("-I") => includes.push(value("-I")?.into()),
            _ if arg.starts_with("-D") => definitions.push((true, value("-D")?)),
            _ if arg.starts_with("-U") => definitions.push((false, value("-U")?)),
            _ if arg.starts_with("-O") => optimization = &arg[2..],
            _ if arg.starts_with("-std=") => standard = &arg[5..],
            _ if arg.starts_with("-W") => {}
            _ => return Err(format!("unsupported native preprocessing option: {arg}")),
        }
        index += 1;
    }
    if let Some(paths) = std::env::var_os("CPATH").filter(|p| !p.is_empty()) {
        includes.extend(std::env::split_paths(&paths).map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p
            }
        }));
    }
    if let Some(paths) = std::env::var_os("C_INCLUDE_PATH").filter(|p| !p.is_empty()) {
        system_includes.extend(std::env::split_paths(&paths).map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p
            }
        }));
    }
    let mut system_paths = system_includes;
    let builtin_path = bundled.as_ref().unwrap().0.clone();
    system_paths.push(builtin_path.clone());
    if options.system_headers {
        if cfg!(target_os = "macos") {
            let sdk = sdk
                .map(Ok)
                .unwrap_or_else(crate::toolchain::macos_sdk_root)?;
            system_paths.push(sdk.join("usr/include"));
            let framework_root = sdk.join("System/Library/Frameworks");
            system_paths.push(framework_root.clone());
            frameworks.push(framework_root);
        } else {
            system_paths.extend([
                PathBuf::from("/usr/local/include"),
                PathBuf::from(if cfg!(target_arch = "aarch64") {
                    "/usr/include/aarch64-linux-gnu"
                } else {
                    "/usr/include/x86_64-linux-gnu"
                }),
                PathBuf::from("/usr/include"),
            ]);
        }
    }
    includes.extend(system_paths.iter().cloned());
    includes.extend(after);
    let mut pp = Preprocessor::new(includes, quotes);
    pp.frameworks = frameworks;
    pp.headers = std::mem::take(headers);
    pp.dependency_only = dependency_only;
    pp.warnings_as_errors = warnings_as_errors;
    profile::define(&mut pp)?;
    if optimization != "0" {
        pp.define("__OPTIMIZE__ 1")?;
        pp.undefine("__NO_INLINE__");
        if matches!(optimization, "s" | "z") {
            pp.define("__OPTIMIZE_SIZE__ 1")?;
        }
    }
    let version = match standard {
        "gnu89" | "gnu90" | "c89" | "c90" => None,
        "gnu99" | "c99" => Some("199901L"),
        "gnu11" | "c11" => Some("201112L"),
        "gnu17" | "c17" | "gnu18" | "c18" => Some("201710L"),
        _ => {
            return Err(format!(
                "unsupported native preprocessing standard: {standard}"
            ));
        }
    };
    if let Some(version) = version {
        pp.define(&format!("__STDC_VERSION__ {version}"))?;
    } else {
        pp.undefine("__STDC_VERSION__");
        pp.undefine("__GNUC_STDC_INLINE__");
        pp.define("__GNUC_GNU_INLINE__ 1")?;
    }
    if !standard.starts_with("gnu") {
        pp.define("__STRICT_ANSI__ 1")?;
        pp.undefine("linux");
        pp.undefine("unix");
    }
    for arg in crate::preprocessor::compiler_macro_arguments() {
        if let Some(name) = arg.strip_prefix("-U") {
            pp.undefine(name);
        }
        if let Some(definition) = arg.strip_prefix("-D") {
            let (name, value) = definition.split_once('=').unwrap_or((definition, "1"));
            pp.define(&format!("{name} {value}"))?;
        }
    }
    for (name, query) in [
        ("__has_builtin", Query::Builtin),
        ("__has_attribute", Query::Attribute),
        ("__has_feature", Query::Zero),
        ("__has_extension", Query::Zero),
        ("__has_c_attribute", Query::Zero),
        ("__has_cpp_attribute", Query::Zero),
        ("__has_declspec_attribute", Query::Zero),
        ("__has_warning", Query::Zero),
        ("__has_constexpr_builtin", Query::Zero),
    ] {
        Rc::get_mut(pp.macros.get_mut(name).unwrap()).unwrap().query = Some(query);
    }
    for definition in &options.definitions {
        definitions.push((true, definition.clone()));
    }
    for name in &options.undefinitions {
        definitions.push((false, name.clone()));
    }
    for (define, text) in definitions {
        if define {
            let (name, value) = text.split_once('=').unwrap_or((&text, "1"));
            pp.define(&format!("{name} {value}"))?;
        } else {
            pp.undefine(&text);
        }
    }
    pp.base_file = path.to_string_lossy().into_owned();
    for (macros_only, file) in forced {
        let included = if Path::new(&file).is_file() {
            PathBuf::from(file)
        } else {
            pp.find_include(&file, true, path, false)
                .ok_or_else(|| format!("header not found: {file}"))?
        };
        let contents = fs::read_to_string(&included).map_err(|e| e.to_string())?;
        pp.file(&contents, &included)?;
        if macros_only {
            pp.output.clear();
        }
    }
    let output = pp.process(code, path)?;
    *headers = std::mem::take(&mut pp.headers);
    if dependency_file.is_some() || dependency_only {
        let escape = make_quote;
        let target = if dependency_targets.is_empty() {
            escape(
                &Path::new(path.file_name().unwrap_or(path.as_os_str()))
                    .with_extension("o")
                    .to_string_lossy(),
            )
        } else {
            dependency_targets.join(" ")
        };
        let deps: Vec<_> = pp
            .dependencies
            .iter()
            .filter(|p| {
                !p.starts_with(&builtin_path)
                    && (!omit_system || !system_paths.iter().any(|base| p.starts_with(base)))
            })
            .collect();
        let mut text = format!("{target}:");
        for dependency in &deps {
            text.push(' ');
            text.push_str(&escape(&dependency.to_string_lossy()));
        }
        text.push('\n');
        if phony {
            for dependency in deps.iter().filter(|p| p.as_path() != path) {
                text.push_str(&format!("{}:\n", escape(&dependency.to_string_lossy())));
            }
        }
        if let Some(file) = dependency_file.as_deref().filter(|file| *file != "-") {
            fs::write(file, &text).map_err(|e| e.to_string())?;
        }
        if dependency_only {
            return Ok(
                if dependency_file.as_deref().is_none_or(|file| file == "-") {
                    text
                } else {
                    String::new()
                },
            );
        }
    }
    if no_markers {
        Ok(output
            .lines()
            .filter(|line| !line.starts_with("# "))
            .map(|l| format!("{l}\n"))
            .collect())
    } else {
        Ok(output)
    }
}

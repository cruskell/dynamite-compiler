use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use crate::error::{ErrorLogger, ErrorPrinter};

/// Explicit preprocessing configuration shared by the CLI and library callers.
#[derive(Default, Debug, Clone)]
pub struct PreprocessorOptions {
    pub system_headers: bool,
    pub include_directories: Vec<std::path::PathBuf>,
    pub definitions: Vec<String>,
    pub undefinitions: Vec<String>,
}

fn compiler_macros(command: &mut Command) {
    // Keep host ABI/GNU compatibility macros, but do not impersonate Clang.
    command.args([
        "-std=c99",
        "-Wno-builtin-macro-redefined",
        "-U__clang__",
        "-U__clang_major__",
        "-U__clang_minor__",
        "-U__clang_patchlevel__",
        "-U__clang_version__",
        "-U__llvm__",
        // Layout-only support must not select code requiring integer arithmetic.
        "-U__SIZEOF_INT128__",
        "-D__DYNAMITE__=1",
        "-D__has_feature(x)=0",
        "-D__has_extension(x)=0",
        "-D__has_c_attribute(x)=0",
        "-D__has_cpp_attribute(x)=0",
        "-D__has_declspec_attribute(x)=0",
        "-D__has_warning(x)=0",
        "-D__has_constexpr_builtin(x)=0",
        "-D__DYNAMITE_SECOND(_0,_1,...)=_1",
        "-D__DYNAMITE_PROBE(...)=__DYNAMITE_SECOND(__VA_ARGS__,0,0)",
        "-D__has_builtin(x)=__DYNAMITE_PROBE(__DYNAMITE_BUILTIN_##x)",
        "-D__has_attribute(x)=__DYNAMITE_PROBE(__DYNAMITE_ATTRIBUTE_##x)",
    ]);
    for name in [
        "__builtin_bzero",
        "__atomic_exchange_n",
        "__atomic_fetch_add",
        "__atomic_add_fetch",
        "__atomic_fetch_sub",
        "__atomic_sub_fetch",
        "__atomic_fetch_and",
        "__atomic_and_fetch",
        "__atomic_fetch_or",
        "__atomic_or_fetch",
        "__atomic_fetch_xor",
        "__atomic_xor_fetch",
        "__atomic_fetch_nand",
        "__atomic_nand_fetch",
        "__atomic_compare_exchange_n",
        "__atomic_load_n",
        "__atomic_store_n",
        "__builtin_constant_p",
        "__builtin_clz",
        "__builtin_clzl",
        "__builtin_clzll",
        "__builtin_ctz",
        "__builtin_ctzl",
        "__builtin_ctzll",
        "__builtin_popcount",
        "__builtin_popcountl",
        "__builtin_popcountll",
        "__builtin_bswap16",
        "__builtin_bswap32",
        "__builtin_bswap64",
        "__builtin_expect",
        "__builtin_object_size",
        "__builtin_offsetof",
        "__builtin_add_overflow",
        "__builtin_sub_overflow",
        "__builtin_nanf",
        "__builtin_nan",
        "__builtin_fabsf",
        "__builtin_fabs",
        "__builtin_inff",
        "__builtin_inf",
        "__builtin_huge_valf",
        "__builtin_huge_val",
        "__builtin_va_start",
        "__builtin_va_end",
        "__builtin_va_arg",
        "__builtin_va_copy",
        "__builtin___memcpy_chk",
        "__builtin___memmove_chk",
        "__builtin___memset_chk",
        "__builtin___sprintf_chk",
        "__builtin___snprintf_chk",
        "__builtin___strlcpy_chk",
        "__builtin___strlcat_chk",
    ] {
        command.arg(format!("-D__DYNAMITE_BUILTIN_{name}=~,1"));
    }
    if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        for name in [
            "__builtin_fabsl",
            "__builtin_infl",
            "__builtin_huge_vall",
            "__builtin_nanl",
        ] {
            command.arg(format!("-D__DYNAMITE_BUILTIN_{name}=~,1"));
        }
    }
    for name in crate::ast::METADATA_ATTRIBUTES.iter().copied().chain([
        "aligned",
        "packed",
        "gnu_inline",
        "weak_import",
        "common",
        "visibility",
    ]) {
        for spelling in [name.to_string(), format!("__{name}__")] {
            command.arg(format!("-D__DYNAMITE_ATTRIBUTE_{spelling}=~,1"));
        }
    }
}

struct IncludeDirectory(std::path::PathBuf);

impl IncludeDirectory {
    fn new() -> Result<Self, String> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let mut attempt = 0;
        let directory = loop {
            let path = std::env::temp_dir().join(format!(
                "dynamite-preprocessor-{}-{nonce}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => break Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                }
                Err(error) => {
                    return Err(format!(
                        "failed to create preprocessor include directory: {error}"
                    ));
                }
            }
        };
        for (name, contents) in [
            ("stdio.h", include_str!("../include/stdio.h")),
            ("stdlib.h", include_str!("../include/stdlib.h")),
            ("stdarg.h", include_str!("../include/stdarg.h")),
            ("stddef.h", include_str!("../include/stddef.h")),
            ("assert.h", include_str!("../include/assert.h")),
            ("stdint.h", include_str!("../include/stdint.h")),
            ("stdbool.h", include_str!("../include/stdbool.h")),
            ("string.h", include_str!("../include/string.h")),
            ("ctype.h", include_str!("../include/ctype.h")),
            ("math.h", include_str!("../include/math.h")),
            ("limits.h", include_str!("../include/limits.h")),
            ("float.h", include_str!("../include/float.h")),
        ] {
            fs::write(directory.0.join(name), contents)
                .map_err(|error| format!("failed to prepare builtin headers: {error}"))?;
        }
        Ok(directory)
    }
}
impl Drop for IncludeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Source {
    name: String,
    code: String,
    lines: Vec<usize>,
}

impl Source {
    fn new(name: String, code: String) -> Self {
        let lines = std::iter::once(0)
            .chain(code.match_indices('\n').map(|(pos, _)| pos + 1))
            .collect();
        Self { name, code, lines }
    }
}

struct Origin {
    start: usize,
    source: usize,
    line: usize,
}

/// Expanded source with line origins for parser and backend diagnostics.
pub struct Preprocessed {
    pub code: String,
    sources: Vec<Source>,
    origins: Vec<Origin>,
}

impl Preprocessed {
    fn location(&self, pos: usize) -> (&Source, usize) {
        let origin = self.origins.iter().rev().find(|origin| origin.start <= pos);
        let Some(origin) = origin else {
            return (&self.sources[0], 0);
        };
        let source = &self.sources[origin.source];
        let start = source
            .lines
            .get(origin.line.saturating_sub(1))
            .copied()
            .unwrap_or(source.code.len());
        let end = source.code[start..]
            .find('\n')
            .map(|len| start + len)
            .unwrap_or(source.code.len());
        let mut column = pos.saturating_sub(origin.start).min(end - start);
        while !source.code.is_char_boundary(start + column) {
            column -= 1;
        }
        (source, start + column)
    }
}

impl ErrorLogger for Preprocessed {
    fn line_from_position(&self, pos: usize) -> (usize, usize, &str) {
        let (source, position) = self.location(pos);
        let row = source
            .lines
            .partition_point(|start| *start <= position)
            .max(1);
        let start = source.lines[row - 1];
        let end = source.code[start..]
            .find('\n')
            .map(|len| start + len)
            .unwrap_or(source.code.len());
        (row, position - start, &source.code[start..end])
    }
    fn print_error_position(&self, pos: usize, message: &str) {
        let (source, position) = self.location(pos);
        ErrorPrinter::new(&source.code)
            .print_error_position(position, &format!("{}: {}", source.name, message));
    }
}

/// Use Clang's preprocessor for directives and predefined macros. Plain source
/// takes the existing path without requiring an external preprocessor.
pub fn preprocess(code: &str, path: Option<&Path>) -> Result<Preprocessed, String> {
    preprocess_with_options(code, path, &PreprocessorOptions::default())
}

pub fn preprocess_with_options(
    code: &str,
    path: Option<&Path>,
    options: &PreprocessorOptions,
) -> Result<Preprocessed, String> {
    let absolute_path = path
        .map(|path| {
            if path.is_absolute() {
                Ok(path.to_path_buf())
            } else {
                std::env::current_dir().map(|directory| directory.join(path))
            }
        })
        .transpose()
        .map_err(|error| format!("failed to resolve source path: {error}"))?;
    let name = absolute_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<stdin>".to_string());
    let source = Source::new(name.clone(), code.to_string());
    if !code.contains('#')
        && !code.contains("__")
        && options.definitions.is_empty()
        && options.undefinitions.is_empty()
    {
        let origins = source
            .lines
            .iter()
            .enumerate()
            .map(|(i, start)| Origin {
                start: *start,
                source: 0,
                line: i + 1,
            })
            .collect();
        return Ok(Preprocessed {
            code: code.to_string(),
            sources: vec![source],
            origins,
        });
    }
    let includes = if options.system_headers {
        None
    } else {
        Some(IncludeDirectory::new()?)
    };
    let mut command = Command::new("clang");
    command.args(["-E", "-x", "c", "-fno-blocks", "-"]);
    compiler_macros(&mut command);
    if let Some(includes) = &includes {
        command.arg("-nostdinc").arg("-I").arg(&includes.0);
    }
    if let Some(parent) = absolute_path
        .as_deref()
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
    {
        command.current_dir(parent).arg("-iquote").arg(parent);
    }
    for directory in &options.include_directories {
        let directory = if directory.is_absolute() {
            directory.clone()
        } else {
            std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(directory)
        };
        command.arg("-I").arg(directory);
    }
    for definition in &options.definitions {
        command.arg("-D").arg(definition);
    }
    for name in &options.undefinitions {
        command.arg("-U").arg(name);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("preprocessing requires clang in PATH: {error}"))?;
    let input = format!(
        "#line 1 \"{}\"\n{}",
        name.replace('\\', "\\\\").replace('"', "\\\""),
        code
    );
    let mut stdin = child.stdin.take().unwrap();
    let output = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(input.as_bytes()));
        let output = child.wait_with_output();
        let written = writer.join().unwrap();
        output.and_then(|output| written.map(|_| output))
    })
    .map_err(|error| format!("failed to preprocess source: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let expanded = String::from_utf8(output.stdout)
        .map_err(|error| format!("preprocessor returned invalid UTF-8: {error}"))?;
    let mut result = Preprocessed {
        code: String::new(),
        sources: vec![source],
        origins: vec![],
    };
    let mut source_ids = HashMap::from([(name, 0)]);
    let mut current_source = 0;
    let mut current_line = 1;
    for line in expanded.split_inclusive('\n') {
        if let Some(marker) = line.strip_prefix("# ") {
            let Some((number, rest)) = marker.split_once(' ') else {
                continue;
            };
            let Ok(number) = number.parse::<usize>() else {
                continue;
            };
            if let Some(rest) = rest.strip_prefix('"') {
                let mut end = 0;
                let mut escaped = false;
                for (i, ch) in rest.char_indices() {
                    if ch == '"' && !escaped {
                        end = i;
                        break;
                    }
                    escaped = ch == '\\' && !escaped;
                }
                let bytes = crate::tokenizer::decode_string_literal(&rest[..end])
                    .ok_or("invalid preprocessor file marker")?;
                let filename =
                    String::from_utf8(bytes).map_err(|_| "invalid preprocessor filename")?;
                current_source = *source_ids.entry(filename.clone()).or_insert_with(|| {
                    let index = result.sources.len();
                    let filename_path = Path::new(&filename);
                    let resolved = if filename_path.is_absolute() {
                        filename_path.to_path_buf()
                    } else {
                        absolute_path
                            .as_deref()
                            .and_then(Path::parent)
                            .unwrap_or(Path::new("."))
                            .join(filename_path)
                    };
                    let text = fs::read_to_string(resolved).unwrap_or_default();
                    result.sources.push(Source::new(filename, text));
                    index
                });
                current_line = number;
                continue;
            }
        }
        result.origins.push(Origin {
            start: result.code.len(),
            source: current_source,
            line: current_line,
        });
        result.code.push_str(line);
        current_line += 1;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_include_directories_remain_independent() {
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            let barrier = &barrier;
            for _ in 0..16 {
                scope.spawn(move || {
                    let directory = IncludeDirectory::new().unwrap();
                    barrier.wait();
                    assert!(directory.0.join("stdio.h").is_file());
                    assert!(directory.0.join("stdlib.h").is_file());
                    barrier.wait();
                });
            }
        });
    }

    #[test]
    fn macro_expansion_preserves_original_line_numbers() {
        let source = "#define VALUE 7\n\nint main() { return VALUE; }\n";
        let expanded = preprocess(source, None).unwrap();
        let position = expanded.code.find("return 7").unwrap();
        let (line, _, text) = expanded.line_from_position(position);
        assert_eq!(line, 3);
        assert_eq!(text, "int main() { return VALUE; }");
    }

    #[test]
    fn includes_resolve_relative_to_source_path() {
        let directory = IncludeDirectory::new().unwrap();
        fs::write(
            directory.0.join("values.h"),
            "#define VALUE 7\nint from_header = VALUE;\n",
        )
        .unwrap();
        let source = "#include \"values.h\"\nint main() { return VALUE; }";
        let expanded = preprocess(source, Some(&directory.0.join("main.c"))).unwrap();
        assert!(expanded.code.contains("return 7"));
        let position = expanded.code.find("int from_header").unwrap();
        let (line, _, source_line) = expanded.line_from_position(position);
        assert_eq!(line, 2);
        assert_eq!(source_line, "int from_header = VALUE;");
        let expanded = preprocess("int line = __LINE__;", None).unwrap();
        assert!(expanded.code.contains("int line = 1;"));
    }

    #[test]
    fn invalid_directives_are_errors() {
        for source in [
            "#error expected failure\n",
            "#if 1\n",
            "#include \"missing-dynamite-header.h\"\n",
        ] {
            assert!(preprocess(source, None).is_err(), "{source}");
        }
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn storage_only_integer128_is_not_advertised_as_an_arithmetic_extension() {
        let code = "#ifdef __SIZEOF_INT128__\n#error unsupported integer arithmetic advertised\n#endif\n#if !defined(__GNUC__) || __SIZEOF_POINTER__ != 8\n#error missing host ABI compatibility macros\n#endif\nint selected_portable_path;\n";
        for system_headers in [false, true] {
            let expanded = preprocess_with_options(
                code,
                None,
                &PreprocessorOptions {
                    system_headers,
                    ..PreprocessorOptions::default()
                },
            )
            .unwrap();
            assert!(expanded.code.contains("int selected_portable_path;"));
        }
    }

    #[test]
    fn command_line_macros_apply_without_directives() {
        let options = PreprocessorOptions {
            definitions: vec!["ANSWER=7".into()],
            undefinitions: vec!["__STDC__".into()],
            ..Default::default()
        };
        let result = preprocess_with_options("int value=ANSWER;", None, &options).unwrap();
        assert!(result.code.contains("int value=7;"));
    }
    #[test]
    fn blocks_extension_is_disabled_in_both_header_modes() {
        let code = "#if defined(__BLOCKS__) || __has_extension(blocks)\n#error Blocks are not supported\n#endif\nint value = 7;\n";
        for system_headers in [false, true] {
            let options = PreprocessorOptions {
                system_headers,
                ..Default::default()
            };
            let source = preprocess_with_options(code, None, &options).unwrap();
            assert!(source.code.contains("int value = 7;"));
        }
    }

    #[test]
    fn compiler_capabilities_do_not_inherit_clang_features() {
        let code = r"
#if defined(__clang__) || defined(__clang_major__) || defined(__llvm__)
#error Incorrect compiler identity
#endif
#if !__DYNAMITE__ || __STDC_VERSION__ != 199901L
#error Incorrect language profile
#endif
#if __has_feature(blocks) || __has_extension(c_atomic) || __has_c_attribute(nodiscard)
#error Unsupported language feature
#endif
#if !__has_builtin(__atomic_fetch_add) || !__has_builtin(__atomic_nand_fetch) || !__has_builtin(__atomic_exchange_n)
#error Atomic updates missing
#endif
#if !__has_builtin(__atomic_compare_exchange_n)
#error missing compare-exchange builtin
#endif
#if !__has_builtin(__atomic_load_n) || !__has_builtin(__atomic_store_n)
#error Atomic load/store missing
#endif
#if !__has_attribute(may_alias) || !__has_attribute(__may_alias__)
#error missing alias annotation
#endif
#if !__has_builtin(__builtin_bzero)
#error missing memory zero builtin
#endif
#if !__has_attribute(common) || !__has_attribute(__common__)
#error missing common attribute
#endif
#if !__has_attribute(visibility) || !__has_attribute(__visibility__)
#error missing default visibility attribute
#endif
#if !__has_builtin(__builtin_sub_overflow)
#error missing subtraction overflow builtin
#endif
#if !__has_attribute(weak_import) || !__has_attribute(__weak_import__)
#error missing weak import attribute
#endif
#if !__has_builtin(__builtin_nan) || !__has_builtin(__builtin_nanf)
#error missing NaN builtin
#endif
#if !__has_builtin(__builtin_constant_p)
#error Constant query missing
#endif
#if !__has_builtin(__builtin_clz) || !__has_builtin(__builtin_ctzll)
#error Bit-scan operation missing
#endif
#if !__has_builtin(__builtin_popcount) || !__has_builtin(__builtin_popcountl) || !__has_builtin(__builtin_popcountll)
#error Population-count operation missing
#endif
#if !__has_builtin(__builtin_bswap32) || !__has_attribute(__packed__)
#error Supported operation missing
#endif
#if __has_builtin(__builtin_rotateleft32) || __has_builtin(__is_target_os)
#error Unsupported builtin
#endif
#if __has_attribute(vector_size) || __has_attribute(weak)
#error Unsupported attribute
#endif
int values[]={__has_builtin(unknown),__has_builtin(__builtin_expect),__has_attribute(unused)};
";
        for system_headers in [false, true] {
            let options = PreprocessorOptions {
                system_headers,
                ..Default::default()
            };
            let source = preprocess_with_options(code, None, &options).unwrap();
            assert!(source.code.contains("int values[]={0,1,1};"));
        }
    }

    #[test]
    fn explicit_macro_options_override_the_compiler_profile() {
        let options = PreprocessorOptions {
            definitions: vec!["__DYNAMITE__=9".into(), "__has_feature(x)=7".into()],
            undefinitions: vec!["__GNUC__".into()],
            ..Default::default()
        };
        let source = preprocess_with_options(
            "#ifdef __GNUC__\n#error macro override failed\n#endif\nint values[]={__DYNAMITE__,__has_feature(custom)};",
            None, &options,
        ).unwrap();
        assert!(source.code.contains("int values[]={9,7};"));
    }

    #[test]
    fn system_headers_are_an_explicit_preprocessing_option() {
        let options = PreprocessorOptions {
            system_headers: true,
            ..PreprocessorOptions::default()
        };
        let source = preprocess_with_options(
            "#include <unistd.h>\n#include <stdio.h>\n#include <stdlib.h>\nFILE *stream;\n",
            None,
            &options,
        )
        .unwrap();
        assert!(source.code.contains("getpid"));
        assert!(source.code.contains("FILE *stream"));
        assert!(!source.code.contains("atexit_b("));
        assert!(source.sources.len() > 2);
        assert!(!PreprocessorOptions::default().system_headers);
    }
}

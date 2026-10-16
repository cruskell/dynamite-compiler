pub mod ast;
mod bitfield;
pub mod ctype;
pub mod error;
mod floating;
pub mod func;
pub mod generator;
pub mod global;
pub mod node;
pub mod preprocessor;
pub mod token;
pub mod tokenizer;
pub mod trie;

use ast::{AstBuilder, ProgramAst};
use generator::{Arch, Os};

pub fn generate(code: &str, output_option: &str, is_debug: bool) -> String {
    generate_with_path(code, output_option, is_debug, None)
}

/// Generate output with quoted includes resolved relative to the source file.
pub fn generate_with_path(
    code: &str,
    output_option: &str,
    is_debug: bool,
    path: Option<&std::path::Path>,
) -> String {
    generate_with_options(
        code,
        output_option,
        is_debug,
        path,
        &preprocessor::PreprocessorOptions::default(),
    )
}

/// Generate output with an explicit source location and preprocessing options.
pub fn generate_with_options(
    code: &str,
    output_option: &str,
    is_debug: bool,
    path: Option<&std::path::Path>,
    options: &preprocessor::PreprocessorOptions,
) -> String {
    let source =
        preprocessor::preprocess_with_options(code, path, options).unwrap_or_else(|message| {
            error::ErrorLogger::print_error_position(&error::ErrorPrinter::new(code), 0, &message);
            unreachable!()
        });
    generate_preprocessed(&source, output_option, is_debug)
}

/// Generate a file using host system headers and explicit Clang preprocessing arguments.
/// Included-source text for diagnostics is loaded on demand during file compilation.
/// Dependency arguments are handled in the same preprocessing pass. As with
/// `generate`, source-language diagnostics use the existing panic-based interface.
pub fn generate_file(
    path: &std::path::Path,
    output_option: &str,
    is_debug: bool,
    clang: &str,
    preprocessing_arguments: &[String],
) -> Result<String, String> {
    let source = preprocessor::preprocess_file(path, clang, preprocessing_arguments)?;
    Ok(generate_preprocessed(&source, output_option, is_debug))
}

fn generate_preprocessed(
    source: &preprocessor::Preprocessed,
    output_option: &str,
    is_debug: bool,
) -> String {
    let error_printer = source;
    let tokens = tokenizer::Tokenizer::tokenize(&source.code, is_debug).unwrap_or_else(|e| {
        error::ErrorLogger::print_syntax_error_position(error_printer, e);
        std::process::exit(1)
    });
    let mut builder = ast::AstBuilderImpl::new(error_printer, &tokens);
    let ast = builder.build(is_debug);
    match output_option {
        "asm" => gen_asm(ast, error_printer),
        _ => gen_llvm_ir(ast, error_printer),
    }
}

fn gen_asm(ast: ProgramAst, error_printer: &dyn error::ErrorLogger) -> String {
    #[cfg(target_os = "linux")]
    let target_os = Os::Linux;
    #[cfg(target_os = "macos")]
    let target_os = Os::MacOS;
    #[cfg(target_arch = "x86_64")]
    let target_arch = Arch::X86_64;
    #[cfg(target_arch = "aarch64")]
    let target_arch = Arch::Aarch64;
    generator::new(target_arch, target_os, error_printer)
        .generate(ast)
        .to_string(target_os)
}

fn gen_llvm_ir(ast: ProgramAst, error_printer: &dyn error::ErrorLogger) -> String {
    generator::llvm::generator::IrGenerator::new(error_printer).generate(ast)
}

pub mod ast;
pub mod ctype;
pub mod error;
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
    let source = preprocessor::preprocess(code, path).unwrap_or_else(|message| {
        error::ErrorLogger::print_error_position(&error::ErrorPrinter::new(code), 0, &message);
        unreachable!()
    });
    let error_printer = &source;
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

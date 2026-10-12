pub mod aarch64;
pub mod llvm;
pub mod x86_64;

#[derive(Clone, Copy)]
pub enum Os {
    Linux,
    MacOS,
}

pub enum Arch {
    Aarch64,
    X86_64,
}

use crate::{ast::ProgramAst, error::ErrorLogger};

pub trait Assembly {
    fn to_string(&self, target_os: Os) -> String;
}

pub trait Generator {
    fn generate(&self, ast: ProgramAst) -> Box<dyn Assembly>;
}

pub fn new<'a>(
    target_arch: Arch,
    target_os: Os,
    error_printer: &'a dyn ErrorLogger,
) -> Box<dyn Generator + 'a> {
    match target_arch {
        Arch::Aarch64 => Box::new(aarch64::generator::AsmGenerator::new(
            error_printer,
            target_os,
        )),
        Arch::X86_64 => Box::new(x86_64::generator::AsmGenerator::new(
            error_printer,
            target_os,
        )),
    }
}

pub mod aarch64;
pub mod abi;
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

/// Register width requested by the supported GNU target operand modifiers.
pub(crate) fn asm_register_width(
    arch: Arch,
    modifier: Option<char>,
    natural: usize,
) -> Option<usize> {
    match (arch, modifier) {
        (_, None) => Some(natural),
        (Arch::Aarch64, Some('x')) | (Arch::X86_64, Some('q')) => Some(8),
        (Arch::Aarch64, Some('w')) | (Arch::X86_64, Some('k')) => Some(4),
        (Arch::X86_64, Some('w')) => Some(2),
        (Arch::X86_64, Some('b')) => Some(1),
        _ => None,
    }
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

#[derive(Clone, Copy)]
pub enum ScalarLocation {
    General(usize),
    Floating(usize),
    Stack(usize, usize),
}

pub fn scalar_arguments(
    types: &[crate::ctype::Type],
    general_limit: usize,
    compact_stack: bool,
    fixed: usize,
    stack_variadic: bool,
) -> (Vec<ScalarLocation>, usize, usize) {
    let (mut general, mut floating, mut stack) = (0usize, 0usize, 0usize);
    let mut locations = vec![];
    for (index, ty) in types.iter().enumerate() {
        let forced_stack = stack_variadic && index >= fixed;
        let location = if !forced_stack && ty.float_bits().is_some() && floating < 8 {
            let register = floating;
            floating += 1;
            ScalarLocation::Floating(register)
        } else if !forced_stack && ty.float_bits().is_none() && general < general_limit {
            let register = general;
            general += 1;
            ScalarLocation::General(register)
        } else {
            let compact = compact_stack && index < fixed;
            let size = if compact { ty.size_of() } else { 8 };
            let align = if compact {
                ty.unqualified().align_of()
            } else {
                8
            };
            stack = stack.div_ceil(align) * align;
            let location = ScalarLocation::Stack(stack, size);
            stack += size;
            location
        };
        locations.push(location);
    }
    (locations, stack, floating)
}

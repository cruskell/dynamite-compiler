use crate::{ast::ProgramAst, error::ErrorLogger};

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

#[derive(Clone, Copy)]
pub enum ScalarLocation {
    General(usize),
    Floating(usize),
    Stack(usize, usize),
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

/// References reachable from emitted definitions and global initializers.
/// Unreferenced static inline bodies must not create linker dependencies.
pub(crate) fn referenced_symbols<'a>(
    ast: &'a ProgramAst<'_>,
) -> std::collections::HashSet<&'a str> {
    use crate::{global::GlobalVariableData, node::Node};
    use std::collections::HashSet;
    fn add_name<'a>(name: &'a str, names: &mut HashSet<&'a str>, pending: &mut Vec<&'a str>) {
        if names.insert(name) {
            pending.push(name);
        }
    }
    fn node_names<'a>(node: &'a Node, names: &mut HashSet<&'a str>, pending: &mut Vec<&'a str>) {
        if !node.global_name.is_empty() {
            add_name(&node.global_name, names, pending);
        }
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            node_names(child, names, pending);
        }
        for child in node.children.iter().chain(&node.args) {
            node_names(child, names, pending);
        }
    }
    fn data_names<'a>(
        data: &'a GlobalVariableData,
        names: &mut HashSet<&'a str>,
        pending: &mut Vec<&'a str>,
    ) {
        match data {
            GlobalVariableData::Elem(value) if value.starts_with('@') => {
                add_name(&value[1..], names, pending)
            }
            GlobalVariableData::Address { symbol, .. } => add_name(symbol, names, pending),
            GlobalVariableData::Arr(values) => {
                for value in values {
                    data_names(value, names, pending);
                }
            }
            GlobalVariableData::Union { value, .. } => data_names(value, names, pending),
            _ => {}
        }
    }
    let mut names = HashSet::new();
    let mut pending = vec![];
    for function in ast.functions.values().filter(|function| {
        !(function.inline_only() || function.internal && function.inline_definition)
    }) {
        if let Some(body) = &function.body {
            node_names(body, &mut names, &mut pending);
        }
    }
    for object in ast.global_variables.values() {
        if let Some(data) = &object.data {
            data_names(data, &mut names, &mut pending);
        }
    }
    while let Some(name) = pending.pop() {
        if let Some(function) = ast
            .functions
            .get(name)
            .filter(|function| function.internal && function.inline_definition)
        {
            if let Some(body) = &function.body {
                node_names(body, &mut names, &mut pending);
            }
        }
    }
    names
}

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

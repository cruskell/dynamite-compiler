mod aggregate;
mod atomic;
mod calls;
mod control_flow;
mod globals;
mod inline_asm;

use super::assembly::Assembly;
use super::instruction::{
    InstOperand::{self, *},
    InstOperator::*,
    Register::{self, *},
};
use crate::ast::ProgramAst;
use crate::ctype::Type;
use crate::error;
use crate::func::Func;
use crate::generator::Os;
use crate::generator::{ScalarLocation, scalar_arguments};
use crate::node::{Node, NodeType};
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
};

pub struct AsmGenerator<'a> {
    error_logger: &'a dyn error::ErrorLogger,
    target_os: Os,
    defined_symbols: HashSet<String>,
    symbol_names: HashMap<String, String>,
}

const ARGS_REG: [Register; 8] = [X0, X1, X2, X3, X4, X5, X6, X7];

impl crate::generator::Generator for AsmGenerator<'_> {
    fn generate(&self, ast: ProgramAst) -> Box<dyn crate::generator::Assembly> {
        self.generate(ast)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    offset: usize,
    breakable_branch_num: usize,
}

impl<'a> AsmGenerator<'a> {
    pub fn new(error_logger: &'a dyn error::ErrorLogger, target_os: Os) -> Self {
        Self {
            error_logger,
            target_os,
            defined_symbols: HashSet::new(),
            symbol_names: HashMap::new(),
        }
    }
    fn generate(&self, ast: ProgramAst) -> Box<dyn crate::generator::Assembly> {
        if ast
            .global_variables
            .values()
            .any(|object| object.thread_local)
        {
            self.error_logger
                .print_error_position(0, "thread-local storage requires the LLVM backend");
        }
        let generator = Self {
            error_logger: self.error_logger,
            target_os: self.target_os,
            defined_symbols: ast
                .functions
                .iter()
                .filter(|(_, function)| function.body.is_some() && !function.inline_only())
                .map(|(name, _)| name.clone())
                .chain(
                    ast.global_variables
                        .iter()
                        .filter(|(_, object)| !object.declaration_only)
                        .map(|(name, _)| name.clone()),
                )
                .collect(),
            symbol_names: ast.symbol_names.clone(),
        };
        let references =
            if ast.functions.values().any(|function| {
                function.hidden || (function.internal && function.inline_definition)
            }) || ast.global_variables.values().any(|object| object.hidden)
            {
                crate::generator::referenced_symbols(&ast)
            } else {
                std::collections::HashSet::new()
            };
        let hidden = ast
            .functions
            .iter()
            .filter(|(name, function)| {
                function.hidden
                    && !function.internal
                    && ((function.body.is_some() && !function.inline_only())
                        || references.contains(name.as_str()))
            })
            .map(|(name, _)| name)
            .chain(
                ast.global_variables
                    .iter()
                    .filter(|(name, object)| {
                        object.hidden
                            && !object.internal
                            && (!object.declaration_only || references.contains(name.as_str()))
                    })
                    .map(|(name, _)| name),
            )
            .map(|name| {
                format!(
                    "{} {}",
                    match self.target_os {
                        Os::Linux => ".hidden",
                        Os::MacOS => ".private_extern",
                    },
                    generator.with_prefix(name)
                )
                .into()
            })
            .collect::<Vec<Assembly>>();
        Box::<Assembly>::new(
            vec![
                hidden.into(),
                if let Os::MacOS = self.target_os {
                    ".section __TEXT,__text,regular,pure_instructions"
                } else {
                    ".section .text"
                }
                .into(),
                ast.functions
                    .iter()
                    .filter(|(name, function)| {
                        !(function.internal && function.inline_definition)
                            || references.contains(name.as_str())
                    })
                    .map(|(name, f)| {
                        let func_offset_with_alignment = f.offset_size.div_ceil(16) * 16;
                        generator.gen_func(name, f, func_offset_with_alignment)
                    })
                    .collect::<Vec<Assembly>>()
                    .into(),
                ".data".into(),
                ast.global_variables
                    .iter()
                    .map(|(s, gv)| generator.gen_global_variable(s, gv))
                    .collect::<Vec<Assembly>>()
                    .into(),
                generator.gen_string_literals(ast.string_literals),
            ]
            .into(),
        )
    }

    fn stack_arguments(&self, types: &[Type], fixed: usize) -> (Vec<(usize, usize, usize)>, usize) {
        let mut offset = 0usize;
        let mut slots = vec![];
        for (i, ty) in types.iter().enumerate() {
            if i < fixed.min(ARGS_REG.len()) {
                continue;
            }
            let compact = matches!(self.target_os, Os::MacOS) && i < fixed;
            let size = if compact { ty.size_of() } else { 8 };
            let align = if compact {
                ty.unqualified().align_of()
            } else {
                8
            };
            offset = offset.div_ceil(align) * align;
            slots.push((i, offset, size));
            offset += size;
        }
        (slots, offset)
    }

    fn gen_func(&self, name: &str, func: &Func, offset: usize) -> Assembly {
        if func.body.is_some()
            && func
                .cty
                .function_signature_ref()
                .is_some_and(|(args, result, _)| {
                    args.iter()
                        .chain(std::iter::once(result))
                        .any(Type::contains_extended_long_double)
                })
        {
            self.error_logger.print_error_position(
                func.token.as_ref().map_or(0, |token| token.pos),
                "extended long double function definitions require the LLVM backend",
            );
        }

        if func.body.is_none() || func.inline_only() {
            return if func.weak_import {
                self.weak_reference(name)
            } else {
                vec![].into()
            };
        }
        if func.cty.function_signature().unwrap().2
            || matches!(func.cty.function_signature().unwrap().1, Type::Struct(_))
            || func
                .args
                .iter()
                .any(|arg| matches!(arg.resolve_type(), Some(Type::Struct(_))))
        {
            return self.aggregate_function(name, func, offset);
        }
        let types = func
            .args
            .iter()
            .map(|arg| arg.resolve_type().unwrap())
            .collect::<Vec<_>>();
        let (locations, _, _) = scalar_arguments(
            &types,
            8,
            matches!(self.target_os, Os::MacOS),
            types.len(),
            false,
        );
        vec![
            if func.internal {
                vec![].into()
            } else {
                format!(".globl {}", self.with_prefix(name)).into()
            },
            "	.p2align	2".into(),
            format!("{}:", self.with_prefix(name)).into(),
            // prologue
            Assembly::inst3(SUB, SP, SP, offset + 32),
            Assembly::inst3(ADD, X10, SP, offset),
            Assembly::inst3(STP, X29, X30, PtrAdd(X10, "#0".to_string())),
            Assembly::inst3(STP, X19, X20, PtrAdd(X10, "#16".to_string())),
            Assembly::inst2(MOV, X29, SP),
            Assembly::inst2(MOV, X19, SP),
            func.args
                .iter()
                .enumerate()
                .map(|(i, arg)| {
                    if let NodeType::LocalVar = arg.nt {
                        vec![
                            Assembly::inst3(ADD, X10, X29, offset - arg.offset.unwrap()),
                            match locations[i] {
                                ScalarLocation::General(register) => Assembly::inst2(
                                    STR,
                                    ARGS_REG[register],
                                    Ptr(X10, types[i].size_of()),
                                ),
                                ScalarLocation::Floating(register) => {
                                    let bits = types[i].float_bits().unwrap();
                                    vec![
                                        Assembly::inst2(
                                            FMOV,
                                            if bits == 32 { W8 } else { X8 },
                                            Float(register, bits),
                                        ),
                                        Assembly::inst2(STR, X8, Ptr(X10, types[i].size_of())),
                                    ]
                                    .into()
                                }
                                ScalarLocation::Stack(slot, size) => vec![
                                    Assembly::inst3(ADD, X11, X29, offset + 32 + slot),
                                    Assembly::inst2(LDRU, X8, Ptr(X11, size)),
                                    Assembly::inst2(STR, X8, Ptr(X10, types[i].size_of())),
                                ]
                                .into(),
                            },
                        ]
                        .into()
                    } else {
                        self.error_logger.print_error_position(
                            arg.token.as_ref().unwrap().pos,
                            "ident expected",
                        );
                        unreachable!()
                    }
                })
                .collect::<Vec<Assembly>>()
                .into(),
            self.gen_node(
                func.body.as_ref().unwrap(),
                Options {
                    offset,
                    breakable_branch_num: 0,
                },
            ),
            Assembly::inst2(MOV, X0, 0), // default return value
            Self::epilogue(offset),
        ]
        .into()
    }
    fn push<T1>(operand: T1) -> Assembly
    where
        T1: Into<InstOperand>,
    {
        vec![
            Assembly::inst3(SUB, X19, X19, 8),
            Assembly::inst3(AND, X20, X19, "#-16"),
            Assembly::inst2(MOV, SP, X20),
            Assembly::inst2(MOV, X8, operand),
            Assembly::inst2(STR, X8, Ptr(X19, 8)),
        ]
        .into()
    }
    fn pop<T1>(operand: T1) -> Assembly
    where
        T1: Into<InstOperand>,
    {
        vec![
            Assembly::inst2(LDR, operand, Ptr(X19, 8)),
            Assembly::inst3(ADD, X19, X19, 8),
            Assembly::inst3(AND, X20, X19, "#-16"),
            Assembly::inst2(MOV, SP, X20),
        ]
        .into()
    }

    fn epilogue(offset: usize) -> Assembly {
        Assembly::Group(vec![
            Assembly::inst2(MOV, SP, X29),
            Assembly::inst3(ADD, X10, SP, offset),
            Assembly::inst3(LDP, X19, X20, PtrAdd(X10, "#16".to_string())),
            Assembly::inst3(LDP, X29, X30, PtrAdd(X10, "#0".to_string())),
            Assembly::inst3(ADD, SP, SP, offset + 32),
            Assembly::inst0(RET),
        ])
    }

    fn normalize_integer(ty: &Type, register: Register) -> Assembly {
        if *ty == Type::Bool {
            return vec![
                Assembly::inst2(CMP, register, 0),
                Assembly::inst2(CSET, register, "ne"),
            ]
            .into();
        }
        if let Some((bits, signed)) = ty.integer_info() {
            if bits < 64 {
                return vec![
                    Assembly::inst3(SHL, register, register, 64 - bits as usize),
                    Assembly::inst3(
                        if signed { SAR } else { SHR },
                        register,
                        register,
                        64 - bits as usize,
                    ),
                ]
                .into();
            }
        }
        vec![].into()
    }

    fn float_conversion(source: &Type, target: &Type) -> Assembly {
        if source == target || *target == Type::Void {
            return vec![].into();
        }
        if *target == Type::Bool && source.float_bits().is_some() {
            return vec![
                Assembly::inst3(SHL, X8, X8, 65 - source.float_bits().unwrap() as usize),
                Assembly::inst2(CMP, X8, 0),
                Assembly::inst2(CSET, X8, "ne"),
            ]
            .into();
        }
        let working_bits = if source.integer_info().is_some() && target.float_bits() == Some(32) {
            32
        } else {
            64
        };
        let mut lines = vec![];
        if let Some(bits) = source.float_bits() {
            lines.push(Assembly::inst2(
                FMOV,
                Float(16, bits),
                if bits == 32 { W8 } else { X8 },
            ));
            if bits == 32 {
                lines.push(Assembly::inst2(FCVT, Float(16, 64), Float(16, 32)));
            }
        } else {
            lines.push(Self::normalize_integer(source, X8));
            lines.push(Assembly::inst2(
                if source.integer_info().unwrap().1 {
                    SCVTF
                } else {
                    UCVTF
                },
                Float(16, working_bits),
                X8,
            ));
        }
        if let Some(bits) = target.float_bits() {
            if bits == 32 && working_bits != 32 {
                lines.push(Assembly::inst2(FCVT, Float(16, 32), Float(16, 64)));
            }
            lines.push(Assembly::inst2(
                FMOV,
                if bits == 32 { W8 } else { X8 },
                Float(16, bits),
            ));
        } else {
            lines.push(Assembly::inst2(
                if target.integer_info().unwrap().1 {
                    FCVTZS
                } else {
                    FCVTZU
                },
                X8,
                Float(16, 64),
            ));
            lines.push(Self::normalize_integer(target, X8));
        }
        lines.into()
    }
    fn float_operation(&self, node: &Node, options: Options) -> Assembly {
        let bits = node
            .lhs
            .as_ref()
            .unwrap()
            .resolve_type()
            .unwrap()
            .float_bits()
            .unwrap();
        let mut lines = vec![
            self.gen_node(node.rhs.as_ref().unwrap(), options),
            self.gen_node(node.lhs.as_ref().unwrap(), options),
            Self::pop(X8),
            Assembly::inst2(FMOV, Float(16, bits), if bits == 32 { W8 } else { X8 }),
            Self::pop(X8),
            Assembly::inst2(FMOV, Float(17, bits), if bits == 32 { W8 } else { X8 }),
        ];
        if matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        ) {
            lines.push(Assembly::inst2(FCMP, Float(16, bits), Float(17, bits)));
            lines.push(Assembly::inst2(
                CSET,
                X8,
                match node.nt {
                    NodeType::Eq => "eq",
                    NodeType::Ne => "ne",
                    NodeType::Lt => "mi",
                    NodeType::Le => "ls",
                    _ => unreachable!(),
                },
            ));
        } else {
            lines.push(Assembly::inst3(
                match node.nt {
                    NodeType::Add => FADD,
                    NodeType::Sub => FSUB,
                    NodeType::Mul => FMUL,
                    NodeType::Div => FDIV,
                    _ => unreachable!(),
                },
                Float(16, bits),
                Float(16, bits),
                Float(17, bits),
            ));
            lines.push(Assembly::inst2(
                FMOV,
                if bits == 32 { W8 } else { X8 },
                Float(16, bits),
            ));
        }
        lines.push(Self::push(X8));
        lines.into()
    }
    fn float_call(&self, node: &Node, options: Options) -> Assembly {
        let types = node
            .args
            .iter()
            .rev()
            .map(|arg| arg.resolve_type().unwrap())
            .collect::<Vec<_>>();
        let macos = matches!(self.target_os, Os::MacOS);
        let (locations, stack_size, _) = scalar_arguments(
            &types,
            8,
            macos,
            node.parameter_types.len(),
            macos && node.variadic,
        );
        let mut lines = vec![];
        if let Some(callee) = &node.lhs {
            lines.push(self.gen_node(callee, options));
        }
        for arg in &node.args {
            lines.push(self.gen_node(arg, options));
        }
        lines.extend([
            Assembly::inst3(SUB, X10, X19, stack_size),
            Assembly::inst3(AND, X10, X10, "#-16"),
            Assembly::inst2(MOV, SP, X10),
        ]);
        for (index, location) in locations.iter().enumerate() {
            let source = PtrAdd(X19, format!("#{}", index * 8));
            lines.push(match *location {
                ScalarLocation::General(register) => {
                    Assembly::inst2(LDR, ARGS_REG[register], source)
                }
                ScalarLocation::Floating(register) => {
                    let bits = types[index].float_bits().unwrap();
                    vec![
                        Assembly::inst2(LDR, X8, source),
                        Assembly::inst2(
                            FMOV,
                            Float(register, bits),
                            if bits == 32 { W8 } else { X8 },
                        ),
                    ]
                    .into()
                }
                ScalarLocation::Stack(slot, size) => vec![
                    Assembly::inst2(LDR, X8, source),
                    Assembly::inst3(ADD, X11, SP, slot),
                    Assembly::inst2(STR, X8, Ptr(X11, size)),
                ]
                .into(),
            });
        }
        if node.lhs.is_some() {
            lines.extend([
                Assembly::inst2(LDR, X12, PtrAdd(X19, format!("#{}", node.args.len() * 8))),
                Assembly::inst1(BLR, X12),
            ]);
        } else {
            lines.push(Assembly::inst1(BL, self.with_prefix(&node.global_name)));
        }
        lines.extend([
            Assembly::inst3(
                ADD,
                X19,
                X19,
                (node.args.len() + usize::from(node.lhs.is_some())) * 8,
            ),
            Assembly::inst2(MOV, SP, X29),
        ]);
        if let Some(bits) = node.resolve_type().unwrap().float_bits() {
            lines.push(Assembly::inst2(
                FMOV,
                if bits == 32 { W0 } else { X0 },
                Float(0, bits),
            ));
        } else if let Some(ty) = &node.cty {
            lines.push(Self::normalize_integer(ty, X0));
        }
        lines.push(Self::push(X0));
        lines.into()
    }

    fn gen_prefetch(&self, node: &Node, write: bool, locality: u8, options: Options) -> Assembly {
        let level = [1, 3, 2, 1][locality as usize];
        let hint = format!(
            "{}l{level}{}",
            if write { "pst" } else { "pld" },
            if locality == 0 { "strm" } else { "keep" }
        );
        vec![
            self.gen_node(node.lhs.as_ref().unwrap(), options),
            Self::pop(X8),
            Assembly::inst2(PRFM, hint, Ptr(X8, 8)),
            Self::push(0),
        ]
        .into()
    }

    // Keep recursive logical chains out of the large dispatch frame in debug builds.
    fn gen_node(&self, node: &Node, options: Options) -> Assembly {
        if node
            .resolve_type_view()
            .is_some_and(|ty| matches!(ty.unqualified(), Type::LongDouble(80 | 128)))
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "extended long double value operations require the LLVM backend",
            );
        }
        if node.nt == NodeType::StackAlloc {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "alloca requires the LLVM backend",
            );
        }
        match node.nt {
            NodeType::LogicalAnd | NodeType::LogicalOr => self.gen_logical(node, options),
            _ => self.gen_node_inner(node, options),
        }
    }

    fn gen_logical(&self, node: &Node, options: Options) -> Assembly {
        let branch_num = node.token.as_ref().unwrap().pos;
        vec![
            self.gen_node(node.lhs.as_ref().unwrap(), options),
            Self::pop(X8),
            Assembly::inst2(CMP, X8, 0),
            Assembly::inst1(
                if node.nt == NodeType::LogicalAnd {
                    JE
                } else {
                    JNE
                },
                EndFlag(branch_num),
            ),
            self.gen_node(node.rhs.as_ref().unwrap(), options),
            Self::pop(X8),
            format!("{}:", EndFlag(branch_num)).into(),
            Assembly::inst2(CMP, X8, 0),
            Assembly::inst2(CSET, X8, "ne"),
            Self::push(X8),
        ]
        .into()
    }

    fn gen_node_inner(&self, node: &Node, options: Options) -> Assembly {
        match node.nt {
            NodeType::Num => {
                return Self::push(node.value.unwrap());
            }
            NodeType::LocalVar | NodeType::GlobalVar => {
                return vec![
                    self.gen_addr(node, options),
                    if let Some(
                        Type::Arr(..)
                        | Type::Vla(..)
                        | Type::Struct(_)
                        | Type::Func(..)
                        | Type::VariadicFunc(..),
                    ) = node.resolve_type()
                    {
                        vec![].into()
                    } else {
                        vec![Self::pop(X8), self.deref(node), Self::push(X8)].into()
                    },
                ]
                .into();
            }
            NodeType::Addr => {
                return self.gen_addr(node.lhs.as_ref().unwrap(), options);
            }
            NodeType::Deref => {
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    if let Some(
                        Type::Arr(..)
                        | Type::Vla(..)
                        | Type::Struct(_)
                        | Type::Func(..)
                        | Type::VariadicFunc(..),
                    ) = node.lhs.as_ref().unwrap().dest_type()
                    {
                        vec![].into()
                    } else {
                        vec![Self::pop(X8), self.deref(node), Self::push(X8)].into()
                    },
                ]
                .into();
            }
            NodeType::Cast => {
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                ];
                if let Some(ty) = &node.cty {
                    let source = node.lhs.as_ref().unwrap().resolve_type().unwrap();
                    lines.push(
                        if source.float_bits().is_some() || ty.float_bits().is_some() {
                            Self::float_conversion(&source, ty)
                        } else {
                            Self::normalize_integer(ty, X8)
                        },
                    );
                }
                lines.push(Self::push(X8));
                return lines.into();
            }
            NodeType::BitLeft | NodeType::BitRight => {
                return vec![
                    self.gen_node(node.rhs.as_ref().unwrap(), options),
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Self::pop(X10),
                    Assembly::inst3(
                        if matches!(node.nt, NodeType::BitLeft) {
                            SHL
                        } else if node.resolve_type().unwrap().integer_info().unwrap().1 {
                            SAR
                        } else {
                            SHR
                        },
                        X8,
                        X8,
                        X10,
                    ),
                    Self::normalize_integer(&node.resolve_type().unwrap(), X8),
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::BitNot => {
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(MVN, X8, X8),
                    Self::normalize_integer(&node.resolve_type().unwrap(), X8),
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::Conditional
            | NodeType::If
            | NodeType::Block
            | NodeType::Return
            | NodeType::While
            | NodeType::DoWhile
            | NodeType::For
            | NodeType::Switch
            | NodeType::Case
            | NodeType::Continue
            | NodeType::Goto
            | NodeType::Label => {
                return self.gen_control_flow(node, options);
            }

            NodeType::Assign => {
                return vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), options),
                    self.gen_node(node.rhs.as_ref().unwrap(), options),
                    Self::pop(X13),
                    Self::pop(X8),
                    Assembly::inst2(
                        STR,
                        X13,
                        Ptr(
                            X8,
                            node.lhs.as_ref().unwrap().resolve_type().unwrap().size_of(),
                        ),
                    ),
                    self.deref(node.lhs.as_ref().unwrap()),
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::SuffixIncr | NodeType::SuffixDecr => {
                let op = if let NodeType::SuffixIncr = node.nt {
                    ADD
                } else {
                    SUB
                };
                return vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(MOV, X13, 1),
                    if let Some(t) = node.lhs.as_ref().unwrap().dest_type() {
                        vec![
                            Assembly::inst2(MOV, X10, t.size_of()),
                            Assembly::inst3(MUL, X13, X13, X10),
                        ]
                        .into()
                    } else {
                        vec![].into()
                    },
                    Assembly::inst2(MOV, X11, X8),
                    self.deref(node.lhs.as_ref().unwrap()),
                    Assembly::inst3(op, X13, X8, X13),
                    Self::normalize_integer(&node.resolve_type().unwrap(), X13),
                    Assembly::inst2(STR, X13, Ptr(X11, node.resolve_type().unwrap().size_of())),
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::CallFunc | NodeType::VaStart | NodeType::VaEnd => {
                return self.gen_calls(node, options);
            }
            NodeType::Sequence => {
                let mut lines = vec![];
                for (index, child) in node.children.iter().enumerate() {
                    lines.push(self.gen_node(child, options));
                    if index + 1 < node.children.len()
                        && !matches!(child.nt, NodeType::DefVar | NodeType::Block)
                    {
                        lines.push(Self::pop(X8));
                    }
                }
                return lines.into();
            }
            NodeType::DefVar => {
                return self.gen_statements(&node.children, options);
            }

            NodeType::Break => {
                if options.breakable_branch_num != 0 {
                    return Assembly::inst1(JMP, EndFlag(options.breakable_branch_num));
                } else {
                    self.error_logger.print_error_position(
                        node.token.as_ref().unwrap().pos,
                        "unexpected break found",
                    );
                }
            }
            NodeType::FloatAbs => {
                let shift = 65 - node.resolve_type().unwrap().float_bits().unwrap() as usize;
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst3(SHL, X8, X8, shift),
                    Assembly::inst3(SHR, X8, X8, shift),
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::BitScan { bits, leading } => {
                let register = if bits == 32 { W8 } else { X8 };
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                ];
                if !leading {
                    lines.push(Assembly::inst2(RBIT, register, register));
                }
                lines.extend([Assembly::inst2(CLZ, register, register), Self::push(X8)]);
                return lines.into();
            }

            NodeType::Prefetch(write, locality) => {
                return self.gen_prefetch(node, write, locality, options);
            }
            NodeType::Atomic(operation) => {
                return self.gen_atomic(node, operation, options);
            }
            NodeType::InlineAsm(_, _) => return self.gen_inline_assembly(node, options),
            NodeType::ZeroFill { bytes } => {
                let mut lines = vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X11),
                    Assembly::inst2(MOV, X9, 0),
                ];
                if bytes >= 8 {
                    lines.extend([
                        Assembly::inst2(MOV, X10, bytes / 8),
                        // Numeric labels stay local to this closed instruction sequence.
                        "901:".into(),
                        Assembly::inst2(STR, X9, Ptr(X11, 8)),
                        Assembly::inst3(ADD, X11, X11, 8),
                        Assembly::inst3(SUB, X10, X10, 1),
                        Assembly::inst2(CMP, X10, 0),
                        Assembly::inst1(JNE, "901b"),
                    ]);
                }
                for _ in 0..bytes % 8 {
                    lines.extend([
                        Assembly::inst2(STR, X9, Ptr(X11, 1)),
                        Assembly::inst3(ADD, X11, X11, 1),
                    ]);
                }
                lines.push(Self::push(0));
                return lines.into();
            }
            NodeType::CopyBytes { bytes } => {
                let mut lines = vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), options),
                    self.gen_addr(node.rhs.as_ref().unwrap(), options),
                    Self::pop(X10),
                    Self::pop(X11),
                    Self::push(X11),
                ];
                if bytes >= 8 {
                    lines.extend([
                        Assembly::inst2(MOV, X9, bytes / 8),
                        "902:".into(),
                        Assembly::inst2(LDR, X12, Ptr(X10, 8)),
                        Assembly::inst2(STR, X12, Ptr(X11, 8)),
                        Assembly::inst3(ADD, X10, X10, 8),
                        Assembly::inst3(ADD, X11, X11, 8),
                        Assembly::inst3(SUB, X9, X9, 1),
                        Assembly::inst2(CMP, X9, 0),
                        Assembly::inst1(JNE, "902b"),
                    ]);
                }
                for _ in 0..bytes % 8 {
                    lines.extend([
                        Assembly::inst2(LDRU, X12, Ptr(X10, 1)),
                        Assembly::inst2(STR, X12, Ptr(X11, 1)),
                        Assembly::inst3(ADD, X10, X10, 1),
                        Assembly::inst3(ADD, X11, X11, 1),
                    ]);
                }
                return lines.into();
            }
            _ => {}
        }
        if node
            .lhs
            .as_ref()
            .is_some_and(|lhs| lhs.resolve_type().unwrap().float_bits().is_some())
        {
            return self.float_operation(node, options);
        }
        let left_ty = node.lhs.as_ref().unwrap().resolve_type().unwrap();
        let right_ty = node.rhs.as_ref().unwrap().resolve_type().unwrap();
        let arithmetic = if left_ty.integer_info().is_some() && right_ty.integer_info().is_some() {
            Some(Type::arithmetic_type(&left_ty, &right_ty))
        } else {
            None
        };
        let unsigned = arithmetic
            .as_ref()
            .is_some_and(|ty| ty.integer_info().is_some_and(|(_, signed)| !signed));
        let left_first = node.nt.left_operand_first();
        let (first, second) = if left_first {
            (node.lhs.as_ref().unwrap(), node.rhs.as_ref().unwrap())
        } else {
            (node.rhs.as_ref().unwrap(), node.lhs.as_ref().unwrap())
        };
        vec![
            self.gen_node(first, options),
            self.gen_node(second, options),
            Self::pop(if left_first { X13 } else { X8 }),
            Self::pop(if left_first { X8 } else { X13 }),
            arithmetic
                .as_ref()
                .map(|ty| Self::normalize_integer(ty, X8))
                .unwrap_or_else(|| vec![].into()),
            arithmetic
                .as_ref()
                .map(|ty| Self::normalize_integer(ty, X13))
                .unwrap_or_else(|| vec![].into()),
            match node.nt {
                NodeType::Add => vec![
                    if let Some(t) = node.lhs.as_ref().unwrap().dest_type() {
                        vec![
                            Assembly::inst2(MOV, X10, t.size_of()),
                            Assembly::inst3(MUL, X13, X13, X10),
                        ]
                        .into()
                    } else {
                        vec![].into()
                    },
                    Assembly::inst3(ADD, X8, X8, X13),
                ]
                .into(),
                NodeType::Sub
                    if node.lhs.as_ref().unwrap().dest_type().is_some()
                        && node.rhs.as_ref().unwrap().dest_type().is_some() =>
                {
                    vec![
                        Assembly::inst3(SUB, X8, X8, X13),
                        Assembly::inst2(
                            MOV,
                            X10,
                            node.lhs.as_ref().unwrap().dest_type().unwrap().size_of(),
                        ),
                        Assembly::inst3(SDIV, X8, X8, X10),
                    ]
                    .into()
                }
                NodeType::Sub => vec![
                    if let Some(t) = node.lhs.as_ref().unwrap().dest_type() {
                        vec![
                            Assembly::inst2(MOV, X10, t.size_of()),
                            Assembly::inst3(MUL, X13, X13, X10),
                        ]
                        .into()
                    } else {
                        vec![].into()
                    },
                    Assembly::inst3(SUB, X8, X8, X13),
                ]
                .into(),
                NodeType::Mul => Assembly::inst3(MUL, X8, X8, X13),
                NodeType::Div => Assembly::inst3(if unsigned { UDIV } else { SDIV }, X8, X8, X13),
                NodeType::Mod => vec![
                    Assembly::inst3(if unsigned { UDIV } else { SDIV }, X10, X8, X13),
                    Assembly::inst4(MSUB, X8, X10, X13, X8),
                ]
                .into(),
                NodeType::BitAnd => Assembly::inst3(AND, X8, X8, X13),
                NodeType::BitXor => Assembly::inst3(XOR, X8, X8, X13),
                NodeType::BitOr => Assembly::inst3(OR, X8, X8, X13),
                NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le => vec![
                    Assembly::inst2(CMP, X8, X13),
                    Assembly::inst2(
                        CSET,
                        X8,
                        match node.nt {
                            NodeType::Eq => "eq",
                            NodeType::Ne => "ne",
                            NodeType::Lt => {
                                if unsigned {
                                    "lo"
                                } else {
                                    "lt"
                                }
                            }
                            NodeType::Le => {
                                if unsigned {
                                    "ls"
                                } else {
                                    "le"
                                }
                            }
                            _ => unreachable!(),
                        },
                    ),
                ]
                .into(),
                _ => {
                    self.error_logger
                        .print_error_position(node.token.as_ref().unwrap().pos, "unexpected node");
                    unreachable!();
                }
            },
            Self::normalize_integer(&node.resolve_type().unwrap(), X8),
            Self::push(X8),
        ]
        .into()
    }

    fn gen_statements(&self, v: &[Node], options: Options) -> Assembly {
        v.iter()
            .map(|node| {
                vec![
                    self.gen_node(node, options),
                    if matches!(
                        node.nt,
                        NodeType::DefVar
                            | NodeType::Block
                            | NodeType::Return
                            | NodeType::If
                            | NodeType::While
                            | NodeType::DoWhile
                            | NodeType::For
                            | NodeType::Switch
                            | NodeType::Case
                            | NodeType::Break
                            | NodeType::Continue
                            | NodeType::Goto
                            | NodeType::Label
                    ) {
                        vec![].into()
                    } else {
                        Self::pop(X8)
                    },
                ]
                .into()
            })
            .collect::<Vec<Assembly>>()
            .into()
    }

    fn gen_addr(&self, node: &Node, options: Options) -> Assembly {
        match node.nt {
            NodeType::LocalVar => vec![
                Assembly::inst3(ADD, X8, X29, options.offset - node.offset.unwrap()),
                Self::push(X8),
            ]
            .into(),
            NodeType::GlobalVar => vec![
                if !node.dest.is_empty() {
                    vec![
                        Assembly::inst2(
                            ADRP,
                            X8,
                            match self.target_os {
                                Os::Linux => node.dest.clone().replace('@', "L_"),
                                Os::MacOS => node.dest.clone().replace('@', "L_") + "@PAGE",
                            },
                        ),
                        Assembly::inst3(
                            ADD,
                            X8,
                            X8,
                            match self.target_os {
                                Os::Linux => {
                                    ":lo12:".to_string() + &node.dest.clone().replace('@', "L_")
                                }
                                Os::MacOS => node.dest.clone().replace('@', "L_") + "@PAGEOFF",
                            },
                        ),
                    ]
                    .into()
                } else if !self.defined_symbols.contains(&node.global_name) {
                    let symbol = self.with_prefix(&node.global_name);
                    let (page, offset) = match self.target_os {
                        Os::Linux => (format!(":got:{symbol}"), format!(":got_lo12:{symbol}")),
                        Os::MacOS => (format!("{symbol}@GOTPAGE"), format!("{symbol}@GOTPAGEOFF")),
                    };
                    vec![
                        Assembly::inst2(ADRP, X8, page),
                        Assembly::inst2(LDR, X8, PtrAdd(X8, offset)),
                    ]
                    .into()
                } else {
                    vec![
                        Assembly::inst2(
                            ADRP,
                            X8,
                            match self.target_os {
                                Os::Linux => self.with_prefix(&node.global_name),
                                Os::MacOS => self.with_prefix(&node.global_name) + "@PAGE",
                            },
                        ),
                        Assembly::inst3(
                            ADD,
                            X8,
                            X8,
                            match self.target_os {
                                Os::Linux => {
                                    ":lo12:".to_string()
                                        + &self.with_prefix(&node.global_name).replace('@', "L_")
                                }
                                Os::MacOS => self.with_prefix(&node.global_name) + "@PAGEOFF",
                            },
                        ),
                    ]
                    .into()
                },
                Self::push(X8),
            ]
            .into(),
            NodeType::Deref => self.gen_node(node.lhs.as_ref().unwrap(), options),
            NodeType::Conditional => self.gen_node(node, options),
            NodeType::CallFunc => self.gen_node(node, options),
            NodeType::Sequence => {
                let mut lines = vec![];
                for child in &node.children[..node.children.len() - 1] {
                    lines.push(self.gen_node(child, options));
                    if !matches!(child.nt, NodeType::DefVar | NodeType::Block) {
                        lines.push(Self::pop(X8));
                    }
                }
                lines.push(self.gen_addr(node.children.last().unwrap(), options));
                lines.into()
            }
            NodeType::CopyBytes { .. } => self.gen_node(node, options),
            _ => {
                unreachable!();
            }
        }
    }

    fn deref(&self, node: &Node) -> Assembly {
        match node.resolve_type() {
            Some(Type::I8) => Assembly::inst2(LDR, X8, Ptr(X8, 1)),
            Some(Type::Float(32)) => Assembly::inst2(LDRU, X8, Ptr(X8, 4)),
            Some(Type::I32) => Assembly::inst2(LDR, X8, Ptr(X8, 4)),
            Some(ty) if ty.integer_info().is_some() => Assembly::inst2(
                if ty.integer_info().unwrap().1 {
                    LDR
                } else {
                    LDRU
                },
                X8,
                Ptr(X8, ty.size_of()),
            ),
            _ => Assembly::inst2(LDR, X8, Ptr(X8, 8)),
        }
    }

    fn address_symbol(&self, symbol: &str) -> String {
        if symbol.starts_with(".str.") {
            format!("L_{symbol}")
        } else {
            self.with_prefix(symbol)
        }
    }

    fn with_prefix<T: Display>(&self, s: T) -> String {
        if let Some(label) = self.symbol_names.get(&s.to_string()) {
            return format!("\"{label}\"");
        }
        format!(
            "{}{}",
            if let Os::MacOS = self.target_os {
                "_"
            } else {
                ""
            },
            s
        )
        .replace('@', "L_")
    }
}

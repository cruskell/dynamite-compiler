mod aggregate;
mod atomic;
mod calls;
mod control_flow;
mod globals;
mod inline_asm;

use super::assembly::Assembly;
use super::instruction::{
    InstOperand::*,
    InstOperator::{self, *},
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

const ARGS_REG: [Register; 6] = [RDI, RSI, RDX, RCX, R8, R9];

impl crate::generator::Generator for AsmGenerator<'_> {
    fn generate(&self, ast: ProgramAst) -> Box<dyn crate::generator::Assembly> {
        self.generate(ast)
    }
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
                ".intel_syntax noprefix".into(),
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
                    .map(|(name, func)| {
                        generator.gen_func(name, func, func.offset_size.div_ceil(16) * 16)
                    })
                    .collect::<Vec<Assembly>>()
                    .into(),
                if let Os::MacOS = self.target_os {
                    ".section __DATA,__data"
                } else {
                    ".section .data"
                }
                .into(),
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
        let (locations, _, _) = scalar_arguments(&types, 6, false, types.len(), false);
        // Preserve incoming general registers before floating stores use RDI as scratch.
        let mut argument_order = func.args.iter().enumerate().collect::<Vec<_>>();
        argument_order
            .sort_by_key(|(index, _)| matches!(locations[*index], ScalarLocation::Floating(_)));
        vec![
            if func.internal {
                vec![].into()
            } else {
                format!(".globl {}", self.with_prefix(name)).into()
            },
            format!("{}:", self.with_prefix(name)).into(),
            // prologue
            Assembly::inst1(PUSH, RBP),
            Assembly::inst2(MOV, RBP, RSP),
            Assembly::inst2(SUB, RSP, offset),
            argument_order
                .into_iter()
                .map(|(i, arg)| {
                    if let NodeType::LocalVar = arg.nt {
                        vec![
                            Assembly::inst2(MOV, RAX, RBP),
                            Assembly::inst2(SUB, RAX, arg.offset.unwrap()),
                            match locations[i] {
                                ScalarLocation::General(index) => {
                                    Assembly::inst2(MOV, RDI, ARGS_REG[index])
                                }
                                ScalarLocation::Floating(index) => Assembly::inst2(
                                    if types[i].float_bits() == Some(32) {
                                        MOVD
                                    } else {
                                        MOVQ
                                    },
                                    if types[i].float_bits() == Some(32) {
                                        EDI
                                    } else {
                                        RDI
                                    },
                                    Xmm(index),
                                ),
                                ScalarLocation::Stack(slot, _) => {
                                    Assembly::inst2(MOV, RDI, PtrAdd(RBP, format!("{}", 16 + slot)))
                                }
                            },
                            self.operation2rdi(arg.resolve_type(), MOV, RAX),
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
            self.gen_node(func.body.as_ref().unwrap(), offset, 0),
            Assembly::inst2(MOV, RAX, 0), // default return value
            Assembly::epilogue(),
        ]
        .into()
    }

    fn normalize_integer(ty: &Type, register: Register) -> Assembly {
        if *ty == Type::Bool {
            let byte = if matches!(register, RDI) { DIL } else { AL };
            return vec![
                Assembly::inst2(CMP, register, 0),
                Assembly::inst1(SETNE, byte),
                Assembly::inst2(MOVZX, register, byte),
            ]
            .into();
        }
        if let Some((bits, signed)) = ty.integer_info() {
            if bits < 64 {
                return vec![
                    Assembly::inst2(SHL, register, 64 - bits as usize),
                    Assembly::inst2(if signed { SAR } else { SHR }, register, 64 - bits as usize),
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
                Assembly::inst2(SHL, RAX, 65 - source.float_bits().unwrap() as usize),
                Assembly::inst2(CMP, RAX, 0),
                Assembly::inst1(SETNE, AL),
                Assembly::inst2(MOVZX, RAX, AL),
            ]
            .into();
        }
        let single_integer = source.integer_info().is_some() && target.float_bits() == Some(32);
        let mut lines = vec![];
        if let Some(bits) = source.float_bits() {
            lines.push(Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                Xmm(0),
                if bits == 32 { EAX } else { RAX },
            ));
            if bits == 32 {
                lines.push(Assembly::inst2(CVTSS2SD, Xmm(0), Xmm(0)));
            }
        } else {
            lines.push(Self::normalize_integer(source, RAX));
            if source.integer_info() == Some((64, false)) {
                lines.extend([
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JGE, "1f"),
                    Assembly::inst2(MOV, RDI, RAX),
                    Assembly::inst2(AND, RDI, 1),
                    Assembly::inst2(SHR, RAX, 1),
                    Assembly::inst2(OR, RAX, RDI),
                    Assembly::inst2(
                        if single_integer { CVTSI2SS } else { CVTSI2SD },
                        Xmm(0),
                        RAX,
                    ),
                    Assembly::inst2(if single_integer { ADDSS } else { ADDSD }, Xmm(0), Xmm(0)),
                    Assembly::inst1(JMP, "2f"),
                    "1:".into(),
                    Assembly::inst2(
                        if single_integer { CVTSI2SS } else { CVTSI2SD },
                        Xmm(0),
                        RAX,
                    ),
                    "2:".into(),
                ]);
            } else {
                lines.push(Assembly::inst2(
                    if single_integer { CVTSI2SS } else { CVTSI2SD },
                    Xmm(0),
                    RAX,
                ));
            }
        }
        if let Some(bits) = target.float_bits() {
            if bits == 32 && !single_integer {
                lines.push(Assembly::inst2(CVTSD2SS, Xmm(0), Xmm(0)));
            }
            lines.push(Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                if bits == 32 { EAX } else { RAX },
                Xmm(0),
            ));
        } else if target.integer_info() == Some((64, false)) {
            lines.extend([
                Assembly::inst2(MOV, RAX, (9223372036854775808.0f64).to_bits() as usize),
                Assembly::inst2(MOVQ, Xmm(1), RAX),
                Assembly::inst2(UCOMISD, Xmm(0), Xmm(1)),
                Assembly::inst1(SETB, DIL),
                Assembly::inst2(MOVZX, RDI, DIL),
            ]);
            // A branch avoids converting the upper unsigned range as signed.
            lines.push(Assembly::inst2(CMP, RDI, 0));
            // Numeric labels are local to each conversion sequence.
            lines.push(Assembly::inst1(JNE, "1f"));
            lines.extend([
                Assembly::inst2(SUBSD, Xmm(0), Xmm(1)),
                Assembly::inst2(CVTTSD2SI, RAX, Xmm(0)),
                Assembly::inst2(MOV, RDI, 1usize << 63),
                Assembly::inst2(OR, RAX, RDI),
                Assembly::inst1(JMP, "2f"),
                "1:".into(),
                Assembly::inst2(CVTTSD2SI, RAX, Xmm(0)),
                "2:".into(),
            ]);
        } else {
            lines.push(Assembly::inst2(CVTTSD2SI, RAX, Xmm(0)));
            lines.push(Self::normalize_integer(target, RAX));
        }
        lines.into()
    }
    fn float_operation(&self, node: &Node, offset: usize, branch: usize) -> Assembly {
        let bits = node
            .lhs
            .as_ref()
            .unwrap()
            .resolve_type()
            .unwrap()
            .float_bits()
            .unwrap();
        let mut lines = vec![
            self.gen_node(node.rhs.as_ref().unwrap(), offset, branch),
            self.gen_node(node.lhs.as_ref().unwrap(), offset, branch),
            Assembly::inst1(POP, RAX),
            Assembly::inst1(POP, RDI),
            Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                Xmm(0),
                if bits == 32 { EAX } else { RAX },
            ),
            Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                Xmm(1),
                if bits == 32 { EDI } else { RDI },
            ),
        ];
        if matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        ) {
            lines.extend([
                Assembly::inst2(if bits == 32 { UCOMISS } else { UCOMISD }, Xmm(0), Xmm(1)),
                Assembly::inst1(
                    match node.nt {
                        NodeType::Eq => SETE,
                        NodeType::Ne => SETNE,
                        NodeType::Lt => SETB,
                        NodeType::Le => SETBE,
                        _ => unreachable!(),
                    },
                    AL,
                ),
                Assembly::inst1(if node.nt == NodeType::Ne { SETP } else { SETNP }, DL),
                Assembly::inst2(if node.nt == NodeType::Ne { OR } else { AND }, AL, DL),
                Assembly::inst2(MOVZX, RAX, AL),
            ]);
        } else {
            lines.push(Assembly::inst2(
                match (node.nt.clone(), bits) {
                    (NodeType::Add, 32) => ADDSS,
                    (NodeType::Add, _) => ADDSD,
                    (NodeType::Sub, 32) => SUBSS,
                    (NodeType::Sub, _) => SUBSD,
                    (NodeType::Mul, 32) => MULSS,
                    (NodeType::Mul, _) => MULSD,
                    (NodeType::Div, 32) => DIVSS,
                    (NodeType::Div, _) => DIVSD,
                    _ => unreachable!(),
                },
                Xmm(0),
                Xmm(1),
            ));
            lines.push(Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                if bits == 32 { EAX } else { RAX },
                Xmm(0),
            ));
        }
        lines.push(Assembly::inst1(PUSH, RAX));
        lines.into()
    }
    fn float_call(&self, node: &Node, offset: usize, branch: usize) -> Assembly {
        let types = node
            .args
            .iter()
            .rev()
            .map(|arg| arg.resolve_type().unwrap())
            .collect::<Vec<_>>();
        let (locations, stack_size, floating) =
            scalar_arguments(&types, 6, false, node.parameter_types.len(), false);
        let mut lines = vec![];
        if let Some(callee) = &node.lhs {
            lines.push(self.gen_node(callee, offset, branch));
        }
        for arg in &node.args {
            lines.push(self.gen_node(arg, offset, branch));
        }
        lines.extend([
            Assembly::inst2(MOV, R11, RSP),
            Assembly::inst2(SUB, RSP, stack_size + 8),
            Assembly::inst2(AND, RSP, "-16"),
            Assembly::inst2(MOV, PtrAdd(RSP, stack_size.to_string()), R11),
        ]);
        for (index, location) in locations.iter().enumerate() {
            let source = PtrAdd(R11, (index * 8).to_string());
            lines.push(match *location {
                ScalarLocation::General(register) => {
                    Assembly::inst2(MOV, ARGS_REG[register], source)
                }
                ScalarLocation::Floating(register) => {
                    let bits = types[index].float_bits().unwrap();
                    vec![
                        Assembly::inst2(MOV, RAX, source),
                        Assembly::inst2(
                            if bits == 32 { MOVD } else { MOVQ },
                            Xmm(register),
                            if bits == 32 { EAX } else { RAX },
                        ),
                    ]
                    .into()
                }
                ScalarLocation::Stack(slot, _) => vec![
                    Assembly::inst2(MOV, RAX, source),
                    Assembly::inst2(MOV, PtrAdd(RSP, slot.to_string()), RAX),
                ]
                .into(),
            });
        }
        lines.push(Assembly::inst2(MOV, RAX, floating));
        if node.lhs.is_some() {
            lines.extend([
                Assembly::inst2(MOV, R10, PtrAdd(R11, (node.args.len() * 8).to_string())),
                Assembly::inst1(CALL, R10),
            ]);
        } else {
            lines.push(Assembly::inst1(CALL, self.with_prefix(&node.global_name)));
        }
        lines.extend([
            Assembly::inst2(MOV, RSP, PtrAdd(RSP, stack_size.to_string())),
            Assembly::inst2(
                ADD,
                RSP,
                (node.args.len() + usize::from(node.lhs.is_some())) * 8,
            ),
        ]);
        if let Some(bits) = node.resolve_type().unwrap().float_bits() {
            lines.push(Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                if bits == 32 { EAX } else { RAX },
                Xmm(0),
            ));
        } else if let Some(ty) = &node.cty {
            lines.push(Self::normalize_integer(ty, RAX));
        }
        lines.push(Assembly::inst1(PUSH, RAX));
        lines.into()
    }

    fn gen_prefetch(
        &self,
        node: &Node,
        locality: u8,
        offset: usize,
        breakable_branch_num: usize,
    ) -> Assembly {
        // Baseline x86_64 can use read-prefetch hints for either access intent.
        let instruction = match locality {
            0 => PREFETCHNTA,
            1 => PREFETCHT2,
            2 => PREFETCHT1,
            3 => PREFETCHT0,
            _ => unreachable!(),
        };
        vec![
            self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
            Assembly::inst1(POP, RAX),
            Assembly::inst1(instruction, Ptr(RAX, 1)),
            Assembly::inst1(PUSH, 0),
        ]
        .into()
    }

    fn gen_node(&self, node: &Node, offset: usize, breakable_branch_num: usize) -> Assembly {
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
            NodeType::Num => {
                let value = node.value.unwrap();
                return if value <= i32::MAX as usize {
                    Assembly::inst1(PUSH, value)
                } else {
                    vec![Assembly::inst2(MOV, RAX, value), Assembly::inst1(PUSH, RAX)].into()
                };
            }
            NodeType::LocalVar | NodeType::GlobalVar => {
                return vec![
                    self.gen_addr(node, offset, breakable_branch_num),
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
                        vec![
                            Assembly::inst1(POP, RAX),
                            self.deref_rax(node),
                            Assembly::inst1(PUSH, RAX),
                        ]
                        .into()
                    },
                ]
                .into();
            }
            NodeType::Addr => {
                return self.gen_addr(node.lhs.as_ref().unwrap(), offset, breakable_branch_num);
            }
            NodeType::Deref => {
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
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
                        vec![
                            Assembly::inst1(POP, RAX),
                            self.deref_rax(node),
                            Assembly::inst1(PUSH, RAX),
                        ]
                        .into()
                    },
                ]
                .into();
            }
            NodeType::Cast => {
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                ];
                if let Some(ty) = &node.cty {
                    let source = node.lhs.as_ref().unwrap().resolve_type().unwrap();
                    lines.push(
                        if source.float_bits().is_some() || ty.float_bits().is_some() {
                            Self::float_conversion(&source, ty)
                        } else {
                            Self::normalize_integer(ty, RAX)
                        },
                    );
                }
                lines.push(Assembly::inst1(PUSH, RAX));
                return lines.into();
            }
            NodeType::BitLeft | NodeType::BitRight => {
                return vec![
                    self.gen_node(node.rhs.as_ref().unwrap(), offset, breakable_branch_num),
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst1(POP, RCX),
                    Assembly::inst2(
                        match node.nt {
                            NodeType::BitLeft => SHL,
                            NodeType::BitRight => {
                                if node.resolve_type().unwrap().integer_info().unwrap().1 {
                                    SAR
                                } else {
                                    SHR
                                }
                            }
                            _ => {
                                unreachable!()
                            }
                        },
                        RAX,
                        CL,
                    ),
                    Self::normalize_integer(&node.resolve_type().unwrap(), RAX),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::BitNot => {
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst1(NOT, RAX),
                    Self::normalize_integer(&node.resolve_type().unwrap(), RAX),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::LogicalAnd
            | NodeType::LogicalOr
            | NodeType::Conditional
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
                return self.gen_control_flow(node, offset, breakable_branch_num);
            }

            NodeType::Assign => {
                return vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    self.gen_node(node.rhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RDI),
                    Assembly::inst1(POP, RAX),
                    self.operation2rdi(node.lhs.as_ref().unwrap().resolve_type(), MOV, RAX),
                    self.deref_rax(node.lhs.as_ref().unwrap()),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::SuffixIncr | NodeType::SuffixDecr => {
                let op = if let NodeType::SuffixIncr = node.nt {
                    ADD
                } else {
                    SUB
                };
                if node.resolve_type() == Some(Type::Bool) {
                    return vec![
                        self.gen_addr(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                        Assembly::inst1(POP, RAX),
                        Assembly::inst2(MOV, RDX, RAX),
                        self.deref_rax(node.lhs.as_ref().unwrap()),
                        Assembly::inst2(MOV, RDI, RAX),
                        Assembly::inst2(op, RDI, 1),
                        Self::normalize_integer(&Type::Bool, RDI),
                        Assembly::inst2(MOV, Ptr(RDX, 1), DIL),
                        Assembly::inst1(PUSH, RAX),
                    ]
                    .into();
                }
                return vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(MOV, RDI, 1),
                    if let Some(t) = node.lhs.as_ref().unwrap().dest_type() {
                        Assembly::inst2(IMUL, RDI, t.size_of())
                    } else {
                        vec![].into()
                    },
                    Assembly::inst2(MOV, RDX, RAX),
                    self.deref_rax(node.lhs.as_ref().unwrap()),
                    self.operation2rdi(node.lhs.as_ref().unwrap().resolve_type(), op, RDX),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::CallFunc | NodeType::VaStart | NodeType::VaEnd => {
                return self.gen_calls(node, offset, breakable_branch_num);
            }
            NodeType::Sequence => {
                let mut lines = vec![];
                for (index, child) in node.children.iter().enumerate() {
                    lines.push(self.gen_node(child, offset, breakable_branch_num));
                    if index + 1 < node.children.len()
                        && !matches!(child.nt, NodeType::DefVar | NodeType::Block)
                    {
                        lines.push(Assembly::inst1(POP, RAX));
                    }
                }
                return lines.into();
            }
            NodeType::DefVar => {
                return self.gen_statements(&node.children, offset, breakable_branch_num);
            }

            NodeType::Break => {
                if breakable_branch_num != 0 {
                    return Assembly::inst1(JMP, EndFlag(breakable_branch_num));
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
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(SHL, RAX, shift),
                    Assembly::inst2(SHR, RAX, shift),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::BitScan { bits, leading } => {
                let (input, count) = if bits == 32 { (EAX, ECX) } else { (RAX, RCX) };
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(if leading { BSR } else { BSF }, count, input),
                    Assembly::inst2(
                        MOV,
                        R11,
                        if leading {
                            usize::MAX
                        } else {
                            usize::from(bits)
                        },
                    ),
                    // BSF/BSR leave the destination undefined for zero; select a fallback first.
                    Assembly::inst2(CMOVE, RCX, R11),
                ];
                if leading {
                    lines.extend([
                        Assembly::inst2(MOV, RAX, usize::from(bits - 1)),
                        Assembly::inst2(SUB, RAX, RCX),
                    ]);
                } else {
                    lines.push(Assembly::inst2(MOV, RAX, RCX));
                }
                lines.push(Assembly::inst1(PUSH, RAX));
                return lines.into();
            }

            NodeType::Prefetch(_, locality) => {
                return self.gen_prefetch(node, locality, offset, breakable_branch_num);
            }
            NodeType::Atomic(operation) => {
                return self.gen_atomic(node, operation, offset, breakable_branch_num);
            }
            NodeType::InlineAsm(_, _) => {
                return self.gen_inline_assembly(node, offset, breakable_branch_num);
            }
            NodeType::ZeroFill { bytes } => {
                return vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RDI),
                    Assembly::inst2(MOV, RCX, bytes),
                    Assembly::inst2(XOR, EAX, EAX),
                    Assembly::inst0(REPSTOSB),
                    Assembly::inst1(PUSH, 0),
                ]
                .into();
            }
            NodeType::CopyBytes { bytes } => {
                return vec![
                    self.gen_addr(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    self.gen_addr(node.rhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RSI),
                    Assembly::inst1(POP, RDI),
                    Assembly::inst1(PUSH, RDI),
                    Assembly::inst2(MOV, RCX, bytes),
                    Assembly::inst0(REPMOVSB),
                ]
                .into();
            }
            _ => {}
        }
        if node
            .lhs
            .as_ref()
            .is_some_and(|lhs| lhs.resolve_type().unwrap().float_bits().is_some())
        {
            return self.float_operation(node, offset, breakable_branch_num);
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
            self.gen_node(first, offset, breakable_branch_num),
            self.gen_node(second, offset, breakable_branch_num),
            Assembly::inst1(POP, if left_first { RDI } else { RAX }),
            Assembly::inst1(POP, if left_first { RAX } else { RDI }),
            arithmetic
                .as_ref()
                .map(|ty| Self::normalize_integer(ty, RAX))
                .unwrap_or_else(|| vec![].into()),
            arithmetic
                .as_ref()
                .map(|ty| Self::normalize_integer(ty, RDI))
                .unwrap_or_else(|| vec![].into()),
            match node.nt {
                NodeType::Add => vec![
                    if let Some(t) = node.lhs.as_ref().unwrap().dest_type() {
                        Assembly::inst2(IMUL, RDI, t.size_of())
                    } else {
                        vec![].into()
                    },
                    Assembly::inst2(ADD, RAX, RDI),
                ]
                .into(),
                NodeType::Sub
                    if node.lhs.as_ref().unwrap().dest_type().is_some()
                        && node.rhs.as_ref().unwrap().dest_type().is_some() =>
                {
                    vec![
                        Assembly::inst2(SUB, RAX, RDI),
                        Assembly::inst2(
                            MOV,
                            RDI,
                            node.lhs.as_ref().unwrap().dest_type().unwrap().size_of(),
                        ),
                        Assembly::inst0(CQO),
                        Assembly::inst1(IDIV, RDI),
                    ]
                    .into()
                }
                NodeType::Sub => vec![
                    if let Some(t) = node.lhs.as_ref().unwrap().dest_type() {
                        Assembly::inst2(IMUL, RDI, t.size_of())
                    } else {
                        vec![].into()
                    },
                    Assembly::inst2(SUB, RAX, RDI),
                ]
                .into(),
                NodeType::Mul => Assembly::inst2(IMUL, RAX, RDI),
                NodeType::Div => vec![
                    if unsigned {
                        Assembly::inst2(XOR, RDX, RDX)
                    } else {
                        Assembly::inst0(CQO)
                    },
                    Assembly::inst1(if unsigned { DIV } else { IDIV }, RDI),
                ]
                .into(),
                NodeType::Mod => vec![
                    if unsigned {
                        Assembly::inst2(XOR, RDX, RDX)
                    } else {
                        Assembly::inst0(CQO)
                    },
                    Assembly::inst1(if unsigned { DIV } else { IDIV }, RDI),
                    Assembly::inst2(MOV, RAX, RDX),
                ]
                .into(),
                NodeType::BitAnd => Assembly::inst2(AND, RAX, RDI),
                NodeType::BitXor => Assembly::inst2(XOR, RAX, RDI),
                NodeType::BitOr => Assembly::inst2(OR, RAX, RDI),
                NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le => vec![
                    Assembly::inst2(CMP, RAX, RDI),
                    Assembly::inst1(
                        match node.nt {
                            NodeType::Eq => SETE,
                            NodeType::Ne => SETNE,
                            NodeType::Lt => {
                                if unsigned {
                                    SETB
                                } else {
                                    SETL
                                }
                            }
                            NodeType::Le => {
                                if unsigned {
                                    SETBE
                                } else {
                                    SETLE
                                }
                            }
                            _ => unreachable!(),
                        },
                        AL,
                    ),
                    Assembly::inst2(MOVZX, RAX, AL),
                ]
                .into(),
                _ => {
                    self.error_logger
                        .print_error_position(node.token.as_ref().unwrap().pos, "unexpected node");
                    unreachable!();
                }
            },
            Assembly::inst1(PUSH, RAX),
        ]
        .into()
    }

    fn gen_statements(&self, v: &[Node], offset: usize, branch_num: usize) -> Assembly {
        v.iter()
            .map(|node| {
                vec![
                    self.gen_node(node, offset, branch_num),
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
                        Assembly::inst1(POP, RAX)
                    },
                ]
                .into()
            })
            .collect::<Vec<Assembly>>()
            .into()
    }

    fn gen_addr(&self, node: &Node, offset: usize, branch_num: usize) -> Assembly {
        match node.nt {
            NodeType::LocalVar => vec![
                Assembly::inst2(MOV, RAX, RBP),
                Assembly::inst2(SUB, RAX, node.offset.unwrap()),
                Assembly::inst1(PUSH, RAX),
            ]
            .into(),
            NodeType::GlobalVar => vec![
                if !node.dest.is_empty() {
                    Assembly::inst2(LEA, RAX, PtrAdd(RIP, node.dest.clone().replace('@', "L_")))
                } else if !self.defined_symbols.contains(&node.global_name) {
                    Assembly::inst2(
                        MOV,
                        RAX,
                        PtrAdd(
                            RIP,
                            format!("{}@GOTPCREL", self.with_prefix(&node.global_name)),
                        ),
                    )
                } else {
                    Assembly::inst2(LEA, RAX, PtrAdd(RIP, self.with_prefix(&node.global_name)))
                },
                Assembly::inst1(PUSH, RAX),
            ]
            .into(),
            NodeType::Deref => self.gen_node(node.lhs.as_ref().unwrap(), offset, branch_num),
            NodeType::Conditional => self.gen_node(node, offset, branch_num),
            NodeType::CallFunc => self.gen_node(node, offset, branch_num),
            NodeType::Sequence => {
                let mut lines = vec![];
                for child in &node.children[..node.children.len() - 1] {
                    lines.push(self.gen_node(child, offset, branch_num));
                    if !matches!(child.nt, NodeType::DefVar | NodeType::Block) {
                        lines.push(Assembly::inst1(POP, RAX));
                    }
                }
                lines.push(self.gen_addr(node.children.last().unwrap(), offset, branch_num));
                lines.into()
            }
            NodeType::CopyBytes { .. } => self.gen_node(node, offset, branch_num),
            _ => {
                unreachable!();
            }
        }
    }

    fn operation2rdi(
        &self,
        c_type: Option<Type>,
        operator: InstOperator,
        from: Register,
    ) -> Assembly {
        match c_type {
            Some(Type::I8) => Assembly::inst2(operator, Ptr(from, 1), DIL),
            Some(Type::I32 | Type::Float(32)) => Assembly::inst2(operator, Ptr(from, 4), EDI),
            Some(ty) if ty.integer_info().is_some() => Assembly::inst2(
                operator,
                Ptr(from, ty.size_of()),
                match ty.size_of() {
                    1 => DIL,
                    2 => DI,
                    4 => EDI,
                    _ => RDI,
                },
            ),
            _ => Assembly::inst2(operator, Ptr(from, 8), RDI),
        }
    }

    fn deref_rax(&self, node: &Node) -> Assembly {
        match node.resolve_type() {
            Some(Type::Float(32)) => Assembly::inst2(MOV, EAX, Ptr(RAX, 4)),
            Some(Type::I32) => Assembly::inst2(MOVSXD, RAX, Ptr(RAX, 4)),
            Some(Type::I8) => Assembly::inst2(MOVSX, RAX, Ptr(RAX, 1)),
            Some(ty) if ty.integer_info().is_some() => {
                let (bits, signed) = ty.integer_info().unwrap();
                if bits == 64 {
                    Assembly::inst2(MOV, RAX, Ptr(RAX, 8))
                } else if bits == 32 && !signed {
                    Assembly::inst2(MOV, EAX, Ptr(RAX, 4))
                } else {
                    Assembly::inst2(
                        if signed { MOVSX } else { MOVZX },
                        RAX,
                        Ptr(RAX, ty.size_of()),
                    )
                }
            }
            _ => Assembly::inst2(MOV, RAX, Ptr(RAX, 8)),
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
        let symbol = format!(
            "{}{}",
            if let Os::MacOS = self.target_os {
                "_"
            } else {
                ""
            },
            s
        )
        .replace('@', "L_");
        format!("\"{symbol}\"")
    }
}

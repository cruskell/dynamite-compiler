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
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{AtomicOperation, AtomicRmwOperation, Node, NodeType, render_inline_assembly};
use crate::token::Token;
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
            symbol_names: ast.symbol_names.clone(),
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
        };
        Box::<Assembly>::new(
            vec![
                ".intel_syntax noprefix".into(),
                if let Os::MacOS = self.target_os {
                    ".section __TEXT,__text,regular,pure_instructions"
                } else {
                    ".section .text"
                }
                .into(),
                ast.functions
                    .iter()
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

    fn gen_string_literals(&self, string_literals: &[Token]) -> Assembly {
        if string_literals.is_empty() {
            vec![]
        } else {
            vec![
                if let Os::MacOS = self.target_os {
                    ".section __TEXT,__const".into()
                } else {
                    ".section .data".into()
                },
                string_literals
                    .iter()
                    .enumerate()
                    .map(|(i, token)| {
                        let bytes = token.encoded_string();
                        vec![
                            format!("  .balign {}", token.string_encoding.unit_size()).into(),
                            format!("L_.str.{}:", i).into(),
                            format!(
                                "  .byte {}",
                                bytes
                                    .into_iter()
                                    .map(|byte| byte.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                            .into(),
                        ]
                        .into()
                    })
                    .collect::<Vec<Assembly>>()
                    .into(),
            ]
        }
        .into()
    }

    fn weak_reference(&self, name: &str) -> Assembly {
        let directive = match self.target_os {
            Os::MacOS => ".weak_reference",
            Os::Linux => ".weak",
        };
        format!("{directive} {}", self.with_prefix(name)).into()
    }

    fn gen_global_variable(&self, name: &str, gv: &GlobalVariable) -> Assembly {
        if gv.declaration_only {
            return if gv.weak_import {
                self.weak_reference(name)
            } else {
                vec![].into()
            };
        }
        if gv.common && !gv.internal && !gv.thread_local && gv.data.is_none() {
            let alignment = gv.ty.align_of().max(gv.alignment).max(1);
            let alignment = match self.target_os {
                Os::MacOS => alignment.trailing_zeros() as usize,
                Os::Linux => alignment,
            };
            return format!(
                ".comm {},{},{}",
                self.with_prefix(name),
                gv.ty.size_of(),
                alignment
            )
            .into();
        }
        vec![
            if let Os::MacOS = self.target_os {
                ".section __DATA,__data".into()
            } else {
                ".section .data".into()
            },
            if gv.internal {
                vec![].into()
            } else {
                format!(".globl {}", self.with_prefix(name)).into()
            },
            format!("  .balign {}", gv.ty.align_of().max(gv.alignment)).into(),
            format!("{}:", self.with_prefix(name)).into(),
            self.gen_initializer_element(&gv.ty, gv.data.as_ref()),
        ]
        .into()
    }

    fn gen_initializer_element(&self, ty: &Type, data: Option<&GlobalVariableData>) -> Assembly {
        let data = if !matches!(ty, Type::Arr(..) | Type::Struct(_)) {
            let mut value = data;
            while let Some(GlobalVariableData::Arr(elements)) = value {
                value = elements.first();
            }
            value
        } else {
            data
        };
        match ty {
            Type::Aligned(child, _) => self.gen_initializer_element(child, data),
            Type::Struct(struct_type)
                if struct_type.has_bit_fields() || struct_type.definition().borrow().is_union =>
            {
                let mut bytes = vec![0; ty.size_of()];
                let mut addresses = std::collections::HashMap::new();
                crate::global::initializer_bytes(ty, data, 0, &mut bytes, &mut addresses);
                let mut lines = vec![];
                let mut index = 0;
                while index < bytes.len() {
                    if let Some((symbol, delta)) = addresses.get(&index) {
                        let symbol = self.address_symbol(symbol);
                        lines.push(format!("  .8byte {symbol}{delta:+}").into());
                        index += 8;
                    } else {
                        lines.push(format!("  .byte {}", bytes[index]).into());
                        index += 1;
                    }
                }
                lines.into()
            }
            Type::Struct(struct_type) => {
                let elements = if let Some(GlobalVariableData::Arr(elements)) = data {
                    Some(elements)
                } else {
                    None
                };
                let mut lines = vec![];
                let mut end = 0;
                for (i, member) in struct_type
                    .members()
                    .iter()
                    .take(if struct_type.definition().borrow().is_union {
                        1
                    } else {
                        usize::MAX
                    })
                    .enumerate()
                {
                    if member.offset > end {
                        lines.push(format!("  .zero {}", member.offset - end).into());
                    }
                    lines.push(self.gen_initializer_element(
                        &member.ty,
                        elements.and_then(|elements| elements.get(i)),
                    ));
                    end = member.offset + member.ty.size_of();
                }
                if ty.size_of() > end {
                    lines.push(format!("  .zero {}", ty.size_of() - end).into());
                }
                lines.into()
            }
            Type::Arr(children_ty, size) => {
                let size = size.unwrap_or(0);
                if let Some(GlobalVariableData::Arr(v)) = data {
                    vec![
                        v.iter()
                            .enumerate()
                            .filter(|(i, _)| *i < size)
                            .map(|(_, d)| {
                                self.gen_initializer_element(children_ty.as_ref(), Some(d))
                            })
                            .collect::<Vec<Assembly>>()
                            .into(),
                        format!(
                            "  .zero {}",
                            children_ty.size_of()
                                * v.iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i < size)
                                    .fold(size, |_, _| { size.saturating_sub(v.len()) })
                        )
                        .into(),
                    ]
                    .into()
                } else {
                    format!("  .zero {}", children_ty.size_of() * size).into()
                }
            }
            Type::Enum(value) => self.gen_initializer_element(&value.underlying.borrow(), data),
            Type::I8 | Type::SignedChar => format!(
                "  .byte {}",
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            )
            .into(),
            Type::Integer128 { .. } => "  .zero 16".into(),
            Type::Float16 => "  .zero 2".into(),
            Type::Vector(..) => format!("  .zero {}", ty.size_of()).into(),
            Type::Float(_) | Type::LongDouble(_) => format!(
                "  .{}byte {}",
                ty.size_of(),
                if let Some(GlobalVariableData::Elem(value)) = data {
                    value.as_str()
                } else {
                    "0"
                }
            )
            .into(),
            Type::Integer { .. } | Type::LongLong { .. } | Type::Bool => format!(
                "  {} {}",
                if ty.size_of() == 1 {
                    ".byte".to_string()
                } else {
                    format!(".{}byte", ty.size_of())
                },
                if let Some(GlobalVariableData::Address { symbol, offset }) = data {
                    format!("{}{offset:+}", self.address_symbol(symbol))
                } else if let Some(GlobalVariableData::Elem(value)) = data {
                    ty.convert_constant(value.parse().unwrap()).to_string()
                } else {
                    "0".into()
                }
            )
            .into(),
            Type::I32 => format!(
                "  .4byte {}",
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            )
            .into(),
            _ => format!(
                "  .8byte {}",
                if let Some(GlobalVariableData::Address { symbol, offset }) = data {
                    let symbol = self.address_symbol(symbol);
                    format!("{symbol}{offset:+}")
                } else if let Some(GlobalVariableData::Elem(s)) = data {
                    if s.starts_with("@.str.") {
                        s.replace('@', "L_")
                    } else if let Some(name) = s.strip_prefix('@') {
                        self.address_symbol(name)
                    } else {
                        s.clone()
                    }
                } else {
                    "0".to_string()
                }
            )
            .into(),
        }
    }

    fn gen_func(&self, name: &str, func: &Func, offset: usize) -> Assembly {
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
    fn aggregate_load(offset: usize, size: usize) -> Assembly {
        let mut lines = vec![Assembly::inst2(MOV, R11, 0)];
        for index in (0..size).rev() {
            lines.extend([
                Assembly::inst2(SHL, R11, 8),
                Assembly::inst2(MOVZX, RAX, PtrOffset(R10, offset + index, 1)),
                Assembly::inst2(OR, R11, RAX),
            ]);
        }
        lines.push(Assembly::inst2(MOV, RAX, R11));
        lines.into()
    }
    fn aggregate_store(size: usize) -> Assembly {
        let mut lines = vec![];
        for index in 0..size {
            lines.push(Assembly::inst2(MOV, PtrOffset(R11, index, 1), AL));
            if index + 1 < size {
                lines.push(Assembly::inst2(SHR, RAX, 8));
            }
        }
        lines.into()
    }
    fn aggregate_copy(size: usize) -> Assembly {
        (0..size)
            .map(|index| {
                vec![
                    Assembly::inst2(MOVZX, RAX, PtrOffset(R10, index, 1)),
                    Assembly::inst2(MOV, PtrOffset(R11, index, 1), AL),
                ]
                .into()
            })
            .collect::<Vec<Assembly>>()
            .into()
    }
    fn aggregate_function(&self, name: &str, func: &Func, locals: usize) -> Assembly {
        use crate::generator::abi::{self, Location};
        let (types, result, variadic) = func.cty.function_signature().unwrap();
        let signature = abi::signature(
            &types,
            &result,
            crate::generator::Arch::X86_64,
            self.target_os,
            types.len(),
            variadic,
        );
        let offset = locals + if variadic { 192 } else { 128 };
        let mut lines = vec![];
        if !func.internal {
            lines.push(format!(".globl {}", self.with_prefix(name)).into());
        }
        lines.extend([
            format!("{}:", self.with_prefix(name)).into(),
            Assembly::inst1(PUSH, RBP),
            Assembly::inst2(MOV, RBP, RSP),
            Assembly::inst2(SUB, RSP, offset),
        ]);
        for (index, register) in ARGS_REG.iter().enumerate() {
            lines.push(Assembly::inst2(
                MOV,
                PtrOffset(RSP, index * 8, 8),
                *register,
            ));
        }
        for index in 0..8 {
            lines.push(Assembly::inst2(
                MOVQ,
                PtrOffset(RSP, 48 + index * if variadic { 16 } else { 8 }, 8),
                Xmm(index),
            ));
        }
        if signature.indirect_result {
            lines.push(Assembly::inst2(MOV, RAX, PtrOffset(RSP, 0, 8)));
            lines.extend([
                Assembly::inst2(MOV, R11, RBP),
                Assembly::inst2(SUB, R11, func.return_pointer.unwrap()),
                Assembly::inst2(MOV, Ptr(R11, 8), RAX),
            ]);
        }
        for (index, argument) in signature.arguments.iter().enumerate() {
            let destination = func.args[index].offset.unwrap();
            if let Some(stack) = argument.memory {
                lines.extend([
                    Assembly::inst2(LEA, R10, PtrOffset(RBP, 16 + stack, 8)),
                    Assembly::inst2(MOV, R11, RBP),
                    Assembly::inst2(SUB, R11, destination),
                    Self::aggregate_copy(types[index].size_of()),
                ]);
            } else {
                for (part, location) in &argument.parts {
                    match *location {
                        Location::Stack(stack) => {
                            lines.push(Assembly::inst2(MOV, RAX, PtrOffset(RBP, 16 + stack, 8)))
                        }
                        location => {
                            let slot = match location {
                                Location::General(n) => n * 8,
                                Location::Floating(n) => 48 + n * if variadic { 16 } else { 8 },
                                _ => unreachable!(),
                            };
                            lines.extend([
                                Assembly::inst2(MOV, R10, RBP),
                                Assembly::inst2(SUB, R10, offset - slot),
                                Assembly::inst2(MOV, RAX, Ptr(R10, 8)),
                            ]);
                        }
                    }
                    lines.extend([
                        Assembly::inst2(MOV, R11, RBP),
                        Assembly::inst2(SUB, R11, destination),
                        Assembly::inst2(ADD, R11, part.offset),
                        Self::aggregate_store(
                            (types[index].size_of() - part.offset).min(part.ty.size_of()),
                        ),
                    ]);
                }
            }
        }
        lines.extend([
            self.gen_node(func.body.as_ref().unwrap(), offset, 0),
            Assembly::inst2(MOV, RAX, 0),
            Assembly::epilogue(),
        ]);
        lines.into()
    }
    fn aggregate_call(&self, node: &Node, offset: usize, branch: usize) -> Assembly {
        use crate::generator::abi::{self, Location};
        let types = node
            .args
            .iter()
            .rev()
            .map(|arg| arg.resolve_type().unwrap())
            .collect::<Vec<_>>();
        let result = node.resolve_type().unwrap();
        let signature = abi::signature(
            &types,
            &result,
            crate::generator::Arch::X86_64,
            self.target_os,
            node.parameter_types.len(),
            node.variadic,
        );
        let mut lines = vec![];
        if let Some(callee) = &node.lhs {
            lines.push(self.gen_node(callee, offset, branch));
        }
        for arg in &node.args {
            lines.push(self.gen_node(arg, offset, branch));
        }
        lines.extend([
            Assembly::inst2(MOV, R11, RSP),
            Assembly::inst2(SUB, RSP, signature.stack_size + 8),
            Assembly::inst2(AND, RSP, "-16"),
            Assembly::inst2(MOV, PtrOffset(RSP, signature.stack_size, 8), R11),
        ]);
        // Copy stack aggregates before loading argument registers.
        for (index, argument) in signature.arguments.iter().enumerate() {
            if let Some(stack) = argument.memory {
                lines.extend([
                    Assembly::inst2(MOV, R10, PtrOffset(RSP, signature.stack_size, 8)),
                    Assembly::inst2(MOV, R10, PtrOffset(R10, index * 8, 8)),
                    Assembly::inst2(LEA, R11, PtrOffset(RSP, stack, 8)),
                    Self::aggregate_copy(types[index].size_of()),
                ]);
            }
        }
        for (index, argument) in signature.arguments.iter().enumerate() {
            if argument.memory.is_some() {
                continue;
            }
            for (part, location) in &argument.parts {
                lines.push(Assembly::inst2(
                    MOV,
                    R10,
                    PtrOffset(RSP, signature.stack_size, 8),
                ));
                if matches!(types[index], Type::Struct(_)) {
                    lines.extend([
                        Assembly::inst2(MOV, R10, PtrOffset(R10, index * 8, 8)),
                        Self::aggregate_load(
                            part.offset,
                            (types[index].size_of() - part.offset).min(part.ty.size_of()),
                        ),
                    ]);
                } else {
                    lines.push(Assembly::inst2(MOV, RAX, PtrOffset(R10, index * 8, 8)));
                }
                lines.push(match *location {
                    Location::General(register) => Assembly::inst2(MOV, ARGS_REG[register], RAX),
                    Location::Floating(register) => Assembly::inst2(
                        if part.ty.float_bits() == Some(32) {
                            MOVD
                        } else {
                            MOVQ
                        },
                        Xmm(register),
                        if part.ty.float_bits() == Some(32) {
                            EAX
                        } else {
                            RAX
                        },
                    ),
                    Location::Stack(stack) => Assembly::inst2(MOV, PtrOffset(RSP, stack, 8), RAX),
                });
            }
        }
        if signature.indirect_result {
            lines.extend([
                Assembly::inst2(MOV, RDI, RBP),
                Assembly::inst2(SUB, RDI, node.offset.unwrap()),
            ]);
        }
        if node.lhs.is_some() {
            lines.extend([
                Assembly::inst2(MOV, R10, PtrOffset(RSP, signature.stack_size, 8)),
                Assembly::inst2(MOV, R10, PtrOffset(R10, node.args.len() * 8, 8)),
            ]);
        }
        lines.push(Assembly::inst2(MOV, RAX, signature.floating));
        lines.push(if node.lhs.is_some() {
            Assembly::inst1(CALL, R10)
        } else {
            Assembly::inst1(CALL, self.with_prefix(&node.global_name))
        });
        lines.extend([
            Assembly::inst2(MOV, RSP, PtrOffset(RSP, signature.stack_size, 8)),
            Assembly::inst2(
                ADD,
                RSP,
                (node.args.len() + usize::from(node.lhs.is_some())) * 8,
            ),
        ]);
        if matches!(result, Type::Struct(_)) {
            if !signature.indirect_result {
                lines.extend([Assembly::inst1(PUSH, RAX), Assembly::inst1(PUSH, RDX)]);
                let (mut gp, mut fp) = (0, 0);
                for part in &signature.result {
                    if part.ty.float_bits().is_some() {
                        lines.push(Assembly::inst2(
                            if part.ty.float_bits() == Some(32) {
                                MOVD
                            } else {
                                MOVQ
                            },
                            if part.ty.float_bits() == Some(32) {
                                EAX
                            } else {
                                RAX
                            },
                            Xmm(fp),
                        ));
                        fp += 1;
                    } else {
                        lines.push(Assembly::inst2(
                            MOV,
                            RAX,
                            PtrOffset(RSP, if gp == 0 { 8 } else { 0 }, 8),
                        ));
                        gp += 1;
                    }
                    lines.extend([
                        Assembly::inst2(MOV, R11, RBP),
                        Assembly::inst2(SUB, R11, node.offset.unwrap()),
                        Assembly::inst2(ADD, R11, part.offset),
                        Self::aggregate_store(
                            (result.size_of() - part.offset).min(part.ty.size_of()),
                        ),
                    ]);
                }
                lines.push(Assembly::inst2(ADD, RSP, 16));
            }
            lines.extend([
                Assembly::inst2(MOV, RAX, RBP),
                Assembly::inst2(SUB, RAX, node.offset.unwrap()),
            ]);
        } else if let Some(bits) = result.float_bits() {
            lines.push(Assembly::inst2(
                if bits == 32 { MOVD } else { MOVQ },
                if bits == 32 { EAX } else { RAX },
                Xmm(0),
            ));
        } else {
            lines.push(Self::normalize_integer(&result, RAX));
        }
        lines.push(Assembly::inst1(PUSH, RAX));
        lines.into()
    }
    fn aggregate_return(&self, node: &Node, offset: usize, branch: usize) -> Assembly {
        use crate::generator::abi;
        let ty = node.resolve_type().unwrap();
        let parts = abi::aggregate_parts(&ty, &crate::generator::Arch::X86_64);
        let mut lines = vec![
            self.gen_node(node.lhs.as_ref().unwrap(), offset, branch),
            Assembly::inst1(POP, R10),
        ];
        if parts.is_empty() {
            lines.extend([
                Assembly::inst2(MOV, R11, RBP),
                Assembly::inst2(SUB, R11, node.offset.unwrap()),
                Assembly::inst2(MOV, R11, Ptr(R11, 8)),
                Self::aggregate_copy(ty.size_of()),
                Assembly::inst2(MOV, RAX, R11),
            ]);
        } else {
            let (mut gp, mut fp) = (0, 0);
            let mut outputs = vec![];
            for part in parts {
                let register = if part.ty.float_bits().is_some() {
                    let n = fp;
                    fp += 1;
                    Xmm(n)
                } else {
                    let r = if gp == 0 { RAX } else { RDX };
                    gp += 1;
                    r
                };
                outputs.push((part, register));
            }
            // Load RAX last because it is also the byte-assembly scratch register.
            outputs.sort_by_key(|(_, register)| matches!(register, RAX));
            for (part, register) in &outputs {
                lines.push(Self::aggregate_load(
                    part.offset,
                    (ty.size_of() - part.offset).min(part.ty.size_of()),
                ));
                lines.push(Assembly::inst2(
                    if part.ty.float_bits() == Some(32) {
                        MOVD
                    } else if part.ty.float_bits().is_some() {
                        MOVQ
                    } else {
                        MOV
                    },
                    *register,
                    if part.ty.float_bits() == Some(32) {
                        EAX
                    } else {
                        RAX
                    },
                ));
            }
        }
        lines.push(Assembly::epilogue());
        lines.into()
    }
    fn atomic_update(node: &Node, operation: AtomicRmwOperation, return_new: bool) -> Assembly {
        let ty = node.args[0].dest_type().unwrap();
        let size = ty.size_of();
        let label = format!(".Latomic.{}", node.token.as_ref().unwrap().pos);
        let mut lines = vec![
            Assembly::inst2(MOV, RSI, RAX),
            if size == 8 {
                Assembly::inst2(MOV, RAX, Ptr(RDI, 8))
            } else if size == 4 {
                Assembly::inst2(MOV, EAX, Ptr(RDI, 4))
            } else {
                Assembly::inst2(MOVZX, RAX, Ptr(RDI, size))
            },
            format!("{label}:").into(),
            Assembly::inst2(MOV, RCX, RAX),
        ];
        lines.push(match operation {
            AtomicRmwOperation::Exchange => Assembly::inst2(MOV, RCX, RSI),
            AtomicRmwOperation::Nand => {
                vec![Assembly::inst2(AND, RCX, RSI), Assembly::inst1(NOT, RCX)].into()
            }
            _ => Assembly::inst2(
                match operation {
                    AtomicRmwOperation::Add => ADD,
                    AtomicRmwOperation::Sub => SUB,
                    AtomicRmwOperation::And => AND,
                    AtomicRmwOperation::Or => OR,
                    AtomicRmwOperation::Xor => XOR,
                    _ => unreachable!(),
                },
                RCX,
                RSI,
            ),
        });
        lines.extend([
            Assembly::inst2(
                CMPXCHG,
                Ptr(RDI, size),
                match size {
                    1 => CL,
                    2 => CX,
                    4 => ECX,
                    _ => RCX,
                },
            ),
            Assembly::inst1(JNE, label),
        ]);
        if return_new {
            lines.push(Assembly::inst2(MOV, RAX, RCX));
        }
        lines.extend([
            Self::normalize_integer(&ty, RAX),
            Assembly::inst1(PUSH, RAX),
        ]);
        lines.into()
    }

    fn atomic_compare_exchange(node: &Node) -> Assembly {
        let size = node.args[0].dest_type().unwrap().size_of();
        let done = format!(".Latomic.cas.done.{}", node.token.as_ref().unwrap().pos);
        vec![
            Assembly::inst1(POP, R11),
            Assembly::inst1(POP, R11),
            Assembly::inst1(POP, R11),
            Assembly::inst1(POP, RCX),
            Assembly::inst1(POP, RDX),
            Assembly::inst1(POP, RDI),
            if size == 8 {
                Assembly::inst2(MOV, RAX, Ptr(RDX, 8))
            } else if size == 4 {
                Assembly::inst2(MOV, EAX, Ptr(RDX, 4))
            } else {
                Assembly::inst2(MOVZX, RAX, Ptr(RDX, size))
            },
            Assembly::inst2(
                CMPXCHG,
                Ptr(RDI, size),
                match size {
                    1 => CL,
                    2 => CX,
                    4 => ECX,
                    _ => RCX,
                },
            ),
            Assembly::inst1(JE, done.clone()),
            Assembly::inst2(
                MOV,
                Ptr(RDX, size),
                match size {
                    1 => AL,
                    2 => AX,
                    4 => EAX,
                    _ => RAX,
                },
            ),
            format!("{done}:").into(),
            Assembly::inst1(SETE, AL),
            Assembly::inst2(MOVZX, RAX, AL),
            Assembly::inst1(PUSH, RAX),
        ]
        .into()
    }

    fn gen_atomic(
        &self,
        node: &Node,
        operation: AtomicOperation,
        offset: usize,
        breakable_branch_num: usize,
    ) -> Assembly {
        let mut lines = vec![];
        for argument in &node.args {
            lines.push(self.gen_node(argument, offset, breakable_branch_num));
        }
        if matches!(operation, AtomicOperation::CompareExchange(_)) {
            lines.push(Self::atomic_compare_exchange(node));
            return lines.into();
        }
        lines.push(Assembly::inst1(POP, R11));
        if operation == AtomicOperation::Load {
            lines.extend([
                Assembly::inst1(POP, RAX),
                Assembly::inst0(MFENCE),
                self.deref_rax(node),
                Assembly::inst0(MFENCE),
                Self::normalize_integer(&node.resolve_type().unwrap(), RAX),
                Assembly::inst1(PUSH, RAX),
            ]);
        } else {
            let size = node.args[0].dest_type().unwrap().size_of();
            lines.extend([Assembly::inst1(POP, RAX), Assembly::inst1(POP, RDI)]);
            if let AtomicOperation::Update(op, return_new) = operation {
                lines.push(Self::atomic_update(node, op, return_new));
            } else {
                lines.extend([
                    Assembly::inst2(
                        XCHG,
                        Ptr(RDI, size),
                        match size {
                            1 => AL,
                            2 => AX,
                            4 => EAX,
                            _ => RAX,
                        },
                    ),
                    Assembly::inst1(PUSH, 0),
                ]);
            }
        }
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

    fn gen_inline_assembly(
        &self,
        node: &Node,
        offset: usize,
        breakable_branch_num: usize,
    ) -> Assembly {
        let registers = [RAX, R10, R11, RCX, RDX, RSI, RDI];
        let names = [
            ["%al", "%ax", "%eax", "%rax"],
            ["%r10b", "%r10w", "%r10d", "%r10"],
            ["%r11b", "%r11w", "%r11d", "%r11"],
            ["%cl", "%cx", "%ecx", "%rcx"],
            ["%dl", "%dx", "%edx", "%rdx"],
            ["%sil", "%si", "%esi", "%rsi"],
            ["%dil", "%di", "%edi", "%rdi"],
        ];
        let operands = node.children.iter().chain(&node.args).collect::<Vec<_>>();
        let template = render_inline_assembly(&node.dest, |index, modifier| {
            let natural = operands.get(index)?.resolve_type()?.size_of();
            let width = crate::generator::asm_register_width(
                crate::generator::Arch::X86_64,
                modifier,
                natural,
            )?;
            Some(names.get(index)?[width.trailing_zeros() as usize].into())
        })
        .unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "unsupported asm register modifier for x86_64",
            );
            unreachable!()
        });
        let mut lines = vec![];
        let matching = node
            .children
            .iter()
            .enumerate()
            .filter(|(index, _)| node.value.unwrap() & (1 << index) != 0)
            .collect::<Vec<_>>();
        for (_, input) in &matching {
            lines.push(self.gen_node(input, offset, breakable_branch_num));
        }
        for input in &node.args {
            lines.push(self.gen_node(input, offset, breakable_branch_num));
        }
        for (index, input) in node
            .args
            .iter()
            .enumerate()
            .rev()
            .map(|(index, input)| (index + node.children.len(), input))
            .chain(matching.into_iter().rev())
        {
            let register = registers[index];
            lines.push(Assembly::inst1(POP, register));
            if input.resolve_type().unwrap().size_of() == 4 {
                lines.push(Self::normalize_integer(&Type::integer(32, false), register));
            }
        }
        lines.push(Assembly::inst1(InlineAsm, InlineAsmTemplate(template)));
        // Buffer every result before address generation can overwrite operand registers.
        for register in registers[..node.children.len()].iter().rev() {
            lines.push(Assembly::inst1(PUSH, *register));
        }
        for output in &node.children {
            lines.push(self.gen_addr(output, offset, breakable_branch_num));
            lines.extend([
                Assembly::inst1(POP, RAX),
                Assembly::inst1(POP, RDI),
                self.operation2rdi(output.resolve_type(), MOV, RAX),
            ]);
        }
        lines.push(Assembly::inst1(PUSH, 0));
        lines.into()
    }

    fn gen_node(&self, node: &Node, offset: usize, breakable_branch_num: usize) -> Assembly {
        match node.nt {
            NodeType::Prefetch(_, locality) => {
                return self.gen_prefetch(node, locality, offset, breakable_branch_num);
            }
            NodeType::InlineAsm(_, _) => {
                return self.gen_inline_assembly(node, offset, breakable_branch_num);
            }
            NodeType::Atomic(operation) => {
                return self.gen_atomic(node, operation, offset, breakable_branch_num);
            }
            NodeType::VaStart | NodeType::VaEnd => {
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, R11),
                ];
                if node.nt == NodeType::VaStart {
                    let (gp, fp, stack) = crate::generator::abi::variadic_state(
                        node,
                        crate::generator::Arch::X86_64,
                        self.target_os,
                    );
                    lines.extend([
                        Assembly::inst2(MOV, EAX, gp * 8),
                        Assembly::inst2(MOV, PtrOffset(R11, 0, 4), EAX),
                        Assembly::inst2(MOV, EAX, 48 + fp * 16),
                        Assembly::inst2(MOV, PtrOffset(R11, 4, 4), EAX),
                        Assembly::inst2(LEA, RAX, PtrOffset(RBP, 16 + stack, 8)),
                        Assembly::inst2(MOV, PtrOffset(R11, 8, 8), RAX),
                        Assembly::inst2(MOV, RAX, RBP),
                        Assembly::inst2(SUB, RAX, offset),
                        Assembly::inst2(MOV, PtrOffset(R11, 16, 8), RAX),
                    ]);
                }
                lines.push(Assembly::inst1(PUSH, 0));
                return lines.into();
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
            NodeType::Sequence => {
                let mut lines = vec![];
                for (index, child) in node.children.iter().enumerate() {
                    lines.push(self.gen_node(child, offset, breakable_branch_num));
                    if index + 1 < node.children.len()
                        && !matches!(child.nt, NodeType::Block | NodeType::DefVar)
                    {
                        lines.push(Assembly::inst1(POP, RAX));
                    }
                }
                return lines.into();
            }
            NodeType::DefVar => {
                return self.gen_statements(&node.children, offset, breakable_branch_num);
            }
            NodeType::CallFunc => {
                if matches!(node.resolve_type(), Some(Type::Struct(_)))
                    || node
                        .args
                        .iter()
                        .any(|arg| matches!(arg.resolve_type(), Some(Type::Struct(_))))
                {
                    return self.aggregate_call(node, offset, breakable_branch_num);
                }
                if node.resolve_type().unwrap().float_bits().is_some()
                    || node
                        .args
                        .iter()
                        .any(|arg| arg.resolve_type().unwrap().float_bits().is_some())
                {
                    return self.float_call(node, offset, breakable_branch_num);
                }
                let stack_size = node.args.len().saturating_sub(ARGS_REG.len()) * 8;
                return vec![
                    node.lhs
                        .as_ref()
                        .map(|callee| self.gen_node(callee, offset, breakable_branch_num))
                        .unwrap_or_else(|| vec![].into()),
                    Assembly::inst2(MOV, RAX, RSP),
                    Assembly::inst2(ADD, RAX, 8),
                    Assembly::inst2(SUB, RAX, node.args.len().saturating_sub(ARGS_REG.len()) * 8),
                    Assembly::inst2(MOV, RDI, 16),
                    Assembly::inst0(CQO),
                    Assembly::inst1(IDIV, RDI),
                    Assembly::inst2(SUB, RSP, RDX),
                    Assembly::inst1(PUSH, RDX),
                    node.args
                        .iter()
                        .map(|node| self.gen_node(node, offset, breakable_branch_num))
                        .collect::<Vec<Assembly>>()
                        .into(),
                    ARGS_REG
                        .iter()
                        .take(node.args.len())
                        .map(|op| Assembly::inst1(POP, *op))
                        .collect::<Vec<Assembly>>()
                        .into(),
                    Assembly::inst2(MOV, RAX, 0),
                    if node.lhs.is_some() {
                        vec![
                            // The saved callee sits above the alignment padding.
                            Assembly::inst2(MOV, R10, PtrAdd(RSP, format!("{stack_size}"))),
                            Assembly::inst2(ADD, R10, stack_size + 8),
                            Assembly::inst2(ADD, R10, RSP),
                            Assembly::inst2(MOV, R10, Ptr(R10, 8)),
                            Assembly::inst1(CALL, R10),
                        ]
                        .into()
                    } else {
                        Assembly::inst1(CALL, self.with_prefix(&node.global_name))
                    },
                    Assembly::inst2(ADD, RSP, node.args.len().saturating_sub(ARGS_REG.len()) * 8),
                    Assembly::inst1(POP, RDI),
                    Assembly::inst2(ADD, RSP, RDI),
                    if node.lhs.is_some() {
                        Assembly::inst2(ADD, RSP, 8)
                    } else {
                        vec![].into()
                    },
                    node.cty
                        .as_ref()
                        .map(|ty| Self::normalize_integer(ty, RAX))
                        .unwrap_or_else(|| vec![].into()),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::If | NodeType::Conditional => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let branch = |child: &Node| {
                    if node.nt == NodeType::If {
                        self.gen_statements(
                            std::slice::from_ref(child),
                            offset,
                            breakable_branch_num,
                        )
                    } else {
                        self.gen_node(child, offset, breakable_branch_num)
                    }
                };
                return vec![
                    self.gen_node(node.cond.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JE, ElseFlag(branch_num)),
                    branch(node.then.as_deref().unwrap()),
                    Assembly::inst1(JMP, EndFlag(branch_num)),
                    format!("{}:", ElseFlag(branch_num)).into(),
                    node.els
                        .as_deref()
                        .map(branch)
                        .unwrap_or_else(|| vec![].into()),
                    format!("{}:", EndFlag(branch_num)).into(),
                    if matches!(node.nt, NodeType::Conditional)
                        && node
                            .resolve_type()
                            .is_some_and(|ty| ty.integer_info().is_some())
                    {
                        vec![
                            Assembly::inst1(POP, RAX),
                            Self::normalize_integer(&node.resolve_type().unwrap(), RAX),
                            Assembly::inst1(PUSH, RAX),
                        ]
                        .into()
                    } else {
                        vec![].into()
                    },
                ]
                .into();
            }
            NodeType::Switch => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let mut lines = vec![
                    self.gen_node(node.cond.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                ];
                let mut default = format!("{}", EndFlag(branch_num));
                for case in &node.children {
                    let label = format!(".Lcase.{}", case.token.as_ref().unwrap().pos);
                    if let Some(value) = case.value {
                        lines.push(Assembly::inst2(MOV, RDI, value));
                        lines.push(Assembly::inst2(CMP, RAX, RDI));
                        lines.push(Assembly::inst1(JE, label));
                    } else {
                        default = label;
                    }
                }
                lines.push(Assembly::inst1(JMP, default));
                lines.push(self.gen_node(node.then.as_ref().unwrap(), offset, branch_num));
                lines.push(format!("{}:", EndFlag(branch_num)).into());
                return lines.into();
            }
            NodeType::Goto => {
                return Assembly::inst1(JMP, format!(".Lgoto.{}", node.value.unwrap()));
            }
            NodeType::Label => {
                return vec![
                    format!(".Lgoto.{}:", node.value.unwrap()).into(),
                    self.gen_statements(
                        std::slice::from_ref(node.lhs.as_deref().unwrap()),
                        offset,
                        breakable_branch_num,
                    ),
                ]
                .into();
            }
            NodeType::Case => {
                return vec![
                    format!(".Lcase.{}:", node.token.as_ref().unwrap().pos).into(),
                    self.gen_statements(
                        std::slice::from_ref(node.lhs.as_deref().unwrap()),
                        offset,
                        breakable_branch_num,
                    ),
                ]
                .into();
            }
            NodeType::DoWhile => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    format!("{}:", BeginFlag(branch_num)).into(),
                    self.gen_statements(
                        std::slice::from_ref(node.then.as_deref().unwrap()),
                        offset,
                        branch_num,
                    ),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    self.gen_node(node.cond.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JNE, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ]
                .into();
            }
            NodeType::While => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let v = vec![
                    format!(".Lcontinue.{}:", branch_num).into(),
                    format!("{}:", BeginFlag(branch_num)).into(),
                    self.gen_node(node.cond.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_statements(
                        std::slice::from_ref(node.then.as_deref().unwrap()),
                        offset,
                        branch_num,
                    ),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                return v.into();
            }
            NodeType::For => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let v = vec![
                    node.ini.as_ref().map_or(Vec::new().into(), |node| {
                        self.gen_statements(std::slice::from_ref(node.as_ref()), offset, branch_num)
                    }),
                    format!("{}:", BeginFlag(branch_num)).into(),
                    node.cond
                        .as_ref()
                        .map_or(Assembly::inst2(MOV, RAX, 1), |node| {
                            vec![
                                self.gen_node(node, offset, branch_num),
                                Assembly::inst1(POP, RAX),
                            ]
                            .into()
                        }),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_statements(
                        std::slice::from_ref(node.then.as_deref().unwrap()),
                        offset,
                        branch_num,
                    ),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    node.upd.as_ref().map_or(Vec::new().into(), |node| {
                        vec![
                            self.gen_node(node, offset, branch_num),
                            Assembly::inst1(POP, RAX),
                        ]
                        .into()
                    }),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                return v.into();
            }
            NodeType::Block => {
                return self.gen_statements(&node.children, offset, breakable_branch_num);
            }
            NodeType::Continue => {
                return Assembly::inst1(JMP, format!(".Lcontinue.{}", node.value.unwrap()));
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
            NodeType::Return => {
                if matches!(node.resolve_type(), Some(Type::Struct(_))) {
                    return self.aggregate_return(node, offset, breakable_branch_num);
                }
                if node.lhs.is_none() {
                    return Assembly::epilogue();
                }
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    if let Some(bits) = node.resolve_type().and_then(|ty| ty.float_bits()) {
                        Assembly::inst2(
                            if bits == 32 { MOVD } else { MOVQ },
                            Xmm(0),
                            if bits == 32 { EAX } else { RAX },
                        )
                    } else {
                        vec![].into()
                    },
                    Assembly::epilogue(),
                ]
                .into();
            }
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
            NodeType::LogicalAnd => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_node(node.rhs.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    format!("{}:", EndFlag(branch_num)).into(),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(SETNE, AL),
                    Assembly::inst2(MOVZX, RAX, AL),
                    Assembly::inst1(PUSH, RAX),
                ]
                .into();
            }
            NodeType::LogicalOr => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JNE, EndFlag(branch_num)),
                    self.gen_node(node.rhs.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    format!("{}:", EndFlag(branch_num)).into(),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(SETNE, AL),
                    Assembly::inst2(MOVZX, RAX, AL),
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
                NodeType::BitAnd => Assembly::inst2(AND, RAX, RDI),
                NodeType::BitXor => Assembly::inst2(XOR, RAX, RDI),
                NodeType::BitOr => Assembly::inst2(OR, RAX, RDI),
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
                        NodeType::Block
                            | NodeType::DefVar
                            | NodeType::If
                            | NodeType::While
                            | NodeType::DoWhile
                            | NodeType::For
                            | NodeType::Switch
                            | NodeType::Case
                            | NodeType::Label
                            | NodeType::Goto
                            | NodeType::Break
                            | NodeType::Continue
                            | NodeType::Return
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
            NodeType::CallFunc => self.gen_node(node, offset, branch_num),
            NodeType::Conditional => self.gen_node(node, offset, branch_num),

            NodeType::Sequence => {
                let mut lines = vec![];
                for child in &node.children[..node.children.len() - 1] {
                    lines.push(self.gen_node(child, offset, branch_num));
                    if !matches!(child.nt, NodeType::Block | NodeType::DefVar) {
                        lines.push(Assembly::inst1(POP, RAX));
                    }
                }
                lines.push(self.gen_addr(node.children.last().unwrap(), offset, branch_num));
                lines.into()
            }

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
            NodeType::LocalVar => vec![
                Assembly::inst2(MOV, RAX, RBP),
                Assembly::inst2(SUB, RAX, node.offset.unwrap()),
                Assembly::inst1(PUSH, RAX),
            ]
            .into(),
            NodeType::Deref => self.gen_node(node.lhs.as_ref().unwrap(), offset, branch_num),
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

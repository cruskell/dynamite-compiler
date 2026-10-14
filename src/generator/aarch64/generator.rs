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
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use std::fmt::Display;

pub struct AsmGenerator<'a> {
    error_logger: &'a dyn error::ErrorLogger,
    target_os: Os,
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
        }
    }
    fn generate(&self, ast: ProgramAst) -> Box<dyn crate::generator::Assembly> {
        Box::<Assembly>::new(
            vec![
                if let Os::MacOS = self.target_os {
                    ".section __TEXT,__text,regular,pure_instructions"
                } else {
                    ".section .text"
                }
                .into(),
                ast.functions
                    .iter()
                    .map(|(name, f)| {
                        let func_offset_with_alignment = f.offset_size.div_ceil(16) * 16;
                        self.gen_func(name, f, func_offset_with_alignment)
                    })
                    .collect::<Vec<Assembly>>()
                    .into(),
                ".data".into(),
                ast.global_variables
                    .iter()
                    .map(|(s, gv)| self.gen_global_variable(s, gv))
                    .collect::<Vec<Assembly>>()
                    .into(),
                self.gen_string_literals(ast.string_literals),
            ]
            .into(),
        )
    }

    fn gen_string_literals(&self, string_literals: &[String]) -> Assembly {
        if string_literals.is_empty() {
            vec![]
        } else {
            vec![
                if let Os::MacOS = self.target_os {
                    ".section __TEXT,__cstring,cstring_literals".into()
                } else {
                    ".section .data".into()
                },
                string_literals
                    .iter()
                    .enumerate()
                    .map(|(i, str)| {
                        vec![
                            format!("L_.str.{}:", i).into(),
                            format!(
                                "  .byte {}",
                                crate::tokenizer::decode_string_literal(str)
                                    .unwrap()
                                    .into_iter()
                                    .chain([0])
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

    fn gen_global_variable(&self, name: &str, gv: &GlobalVariable) -> Assembly {
        if gv.declaration_only {
            return vec![].into();
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
            Self::gen_initializer_element(&gv.ty, gv.data.as_ref(), self.target_os),
        ]
        .into()
    }

    fn gen_initializer_element(
        ty: &Type,
        data: Option<&GlobalVariableData>,
        target_os: Os,
    ) -> Assembly {
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
                        let prefix = if symbol.starts_with(".str.") {
                            "L_"
                        } else if matches!(target_os, Os::MacOS) {
                            "_"
                        } else {
                            ""
                        };
                        lines.push(format!("  .8byte {prefix}{symbol}{delta:+}").into());
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
                    lines.push(Self::gen_initializer_element(
                        &member.ty,
                        elements.and_then(|elements| elements.get(i)),
                        target_os,
                    ));
                    end = member.offset + member.ty.size_of();
                }
                if ty.size_of() > end {
                    lines.push(format!("  .zero {}", ty.size_of() - end).into());
                }
                lines.into()
            }
            Type::Arr(children_ty, size) => {
                if let Some(GlobalVariableData::Arr(v)) = data {
                    vec![
                        v.iter()
                            .enumerate()
                            .filter(|(i, _)| i < size)
                            .map(|(_, d)| {
                                Self::gen_initializer_element(
                                    children_ty.as_ref(),
                                    Some(d),
                                    target_os,
                                )
                            })
                            .collect::<Vec<Assembly>>()
                            .into(),
                        format!(
                            "  .zero {}",
                            children_ty.size_of()
                                * v.iter()
                                    .enumerate()
                                    .filter(|(i, _)| i < size)
                                    .fold(*size, |_, _| { size.saturating_sub(v.len()) })
                        )
                        .into(),
                    ]
                    .into()
                } else {
                    format!("  .zero {}", children_ty.size_of() * *size).into()
                }
            }
            Type::I8 => format!(
                "  .byte {}",
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            )
            .into(),
            Type::Float(_) => format!(
                "  .{}byte {}",
                ty.size_of(),
                if let Some(GlobalVariableData::Elem(value)) = data {
                    value.as_str()
                } else {
                    "0"
                }
            )
            .into(),
            Type::Integer { .. } | Type::Bool => format!(
                "  .{}byte {}",
                ty.size_of(),
                if let Some(GlobalVariableData::Elem(value)) = data {
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
                    let prefix = if symbol.starts_with(".str.") {
                        "L_"
                    } else if matches!(target_os, Os::MacOS) {
                        "_"
                    } else {
                        ""
                    };
                    format!("{prefix}{symbol}{offset:+}")
                } else if let Some(GlobalVariableData::Elem(s)) = data {
                    if s.starts_with("@.str.") {
                        s.replace('@', "L_")
                    } else if let Some(name) = s.strip_prefix('@') {
                        format!(
                            "{}{}",
                            if matches!(target_os, Os::MacOS) {
                                "_"
                            } else {
                                ""
                            },
                            name
                        )
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

    fn stack_arguments(&self, types: &[Type], fixed: usize) -> (Vec<(usize, usize, usize)>, usize) {
        let mut offset = 0usize;
        let mut slots = vec![];
        for (i, ty) in types.iter().enumerate() {
            if i < fixed.min(ARGS_REG.len()) {
                continue;
            }
            let compact = matches!(self.target_os, Os::MacOS) && i < fixed;
            let size = if compact { ty.size_of() } else { 8 };
            let align = if compact { ty.align_of() } else { 8 };
            offset = offset.div_ceil(align) * align;
            slots.push((i, offset, size));
            offset += size;
        }
        (slots, offset)
    }

    fn gen_func(&self, name: &str, func: &Func, offset: usize) -> Assembly {
        if func.body.is_none() || func.inline_only() {
            return vec![].into();
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
    fn aggregate_load(offset: usize, size: usize) -> Assembly {
        let mut lines = vec![Assembly::inst2(MOV, X8, 0)];
        for index in (0..size).rev() {
            lines.extend([
                Assembly::inst3(SHL, X8, X8, 8),
                Assembly::inst3(ADD, X12, X10, offset + index),
                Assembly::inst2(LDRU, X13, Ptr(X12, 1)),
                Assembly::inst3(OR, X8, X8, X13),
            ]);
        }
        lines.into()
    }
    fn aggregate_store(size: usize) -> Assembly {
        let mut lines = vec![];
        for index in 0..size {
            lines.extend([
                Assembly::inst3(ADD, X12, X11, index),
                Assembly::inst2(STR, X8, Ptr(X12, 1)),
            ]);
            if index + 1 < size {
                lines.push(Assembly::inst3(SHR, X8, X8, 8));
            }
        }
        lines.into()
    }
    fn aggregate_copy(size: usize) -> Assembly {
        (0..size)
            .map(|index| {
                vec![
                    Assembly::inst3(ADD, X12, X10, index),
                    Assembly::inst2(LDRU, X8, Ptr(X12, 1)),
                    Assembly::inst3(ADD, X13, X11, index),
                    Assembly::inst2(STR, X8, Ptr(X13, 1)),
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
            crate::generator::Arch::Aarch64,
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
            "\t.p2align\t2".into(),
            format!("{}:", self.with_prefix(name)).into(),
            Assembly::inst3(SUB, SP, SP, offset + 32),
            Assembly::inst3(ADD, X10, SP, offset),
            Assembly::inst3(STP, X29, X30, PtrAdd(X10, "#0".into())),
            Assembly::inst3(STP, X19, X20, PtrAdd(X10, "#16".into())),
            Assembly::inst2(MOV, X29, SP),
            Assembly::inst2(MOV, X19, SP),
        ]);
        if signature.indirect_result {
            lines.extend([
                Assembly::inst3(ADD, X10, X29, offset - func.return_pointer.unwrap()),
                Assembly::inst2(STR, X8, Ptr(X10, 8)),
            ]);
        }
        for (index, register) in ARGS_REG.iter().enumerate() {
            lines.extend([
                Assembly::inst3(ADD, X10, X29, index * 8),
                Assembly::inst2(STR, *register, Ptr(X10, 8)),
            ]);
        }
        for index in 0..8 {
            lines.extend([
                Assembly::inst2(FMOV, X8, Float(index, 64)),
                Assembly::inst3(ADD, X10, X29, 64 + index * if variadic { 16 } else { 8 }),
                Assembly::inst2(STR, X8, Ptr(X10, 8)),
            ]);
        }
        for (index, argument) in signature.arguments.iter().enumerate() {
            let destination = offset - func.args[index].offset.unwrap();
            if let Some(stack) = argument.memory {
                lines.extend([
                    Assembly::inst3(ADD, X10, X29, offset + 32 + stack),
                    Assembly::inst3(ADD, X11, X29, destination),
                    Self::aggregate_copy(types[index].size_of()),
                ]);
            } else {
                for (part, location) in &argument.parts {
                    let slot = match *location {
                        Location::General(n) => n * 8,
                        Location::Floating(n) => 64 + n * if variadic { 16 } else { 8 },
                        Location::Stack(stack) => offset + 32 + stack,
                    };
                    lines.extend([
                        Assembly::inst3(ADD, X10, X29, slot),
                        Assembly::inst2(LDRU, X8, Ptr(X10, 8)),
                    ]);
                    if argument.indirect {
                        lines.extend([
                            Assembly::inst2(MOV, X10, X8),
                            Assembly::inst3(ADD, X11, X29, destination),
                            Self::aggregate_copy(types[index].size_of()),
                        ]);
                    } else {
                        lines.extend([
                            Assembly::inst3(ADD, X11, X29, destination + part.offset),
                            Self::aggregate_store(
                                (types[index].size_of() - part.offset).min(part.ty.size_of()),
                            ),
                        ]);
                    }
                }
            }
        }
        lines.extend([
            self.gen_node(
                func.body.as_ref().unwrap(),
                Options {
                    offset,
                    breakable_branch_num: 0,
                },
            ),
            Assembly::inst2(MOV, X0, 0),
            Self::epilogue(offset),
        ]);
        lines.into()
    }
    fn aggregate_call(&self, node: &Node, options: Options) -> Assembly {
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
            crate::generator::Arch::Aarch64,
            self.target_os,
            node.parameter_types.len(),
            node.variadic,
        );
        let mut lines = vec![];
        if let Some(callee) = &node.lhs {
            lines.push(self.gen_node(callee, options));
        }
        for arg in &node.args {
            lines.push(self.gen_node(arg, options));
        }
        lines.extend([
            Assembly::inst3(SUB, X10, X19, signature.stack_size),
            Assembly::inst3(AND, X10, X10, "#-16"),
            Assembly::inst2(MOV, SP, X10),
        ]);
        for (index, argument) in signature.arguments.iter().enumerate() {
            if let Some(stack) = argument.memory {
                lines.extend([
                    Assembly::inst2(LDR, X10, PtrAdd(X19, format!("#{}", index * 8))),
                    Assembly::inst3(ADD, X11, SP, stack),
                    Self::aggregate_copy(types[index].size_of()),
                ]);
            }
        }
        for (index, argument) in signature.arguments.iter().enumerate() {
            if argument.memory.is_some() {
                continue;
            }
            for (part, location) in &argument.parts {
                if matches!(types[index], Type::Struct(_)) && !argument.indirect {
                    lines.extend([
                        Assembly::inst2(LDR, X10, PtrAdd(X19, format!("#{}", index * 8))),
                        Self::aggregate_load(
                            part.offset,
                            (types[index].size_of() - part.offset).min(part.ty.size_of()),
                        ),
                    ]);
                } else {
                    lines.push(Assembly::inst2(
                        LDR,
                        X8,
                        PtrAdd(X19, format!("#{}", index * 8)),
                    ));
                }
                lines.push(match *location {
                    Location::General(register) => Assembly::inst2(MOV, ARGS_REG[register], X8),
                    Location::Floating(register) => Assembly::inst2(
                        FMOV,
                        Float(register, part.ty.float_bits().unwrap()),
                        if part.ty.float_bits() == Some(32) {
                            W8
                        } else {
                            X8
                        },
                    ),
                    Location::Stack(stack) => vec![
                        Assembly::inst3(ADD, X11, SP, stack),
                        Assembly::inst2(
                            STR,
                            X8,
                            Ptr(
                                X11,
                                if matches!(self.target_os, Os::MacOS)
                                    && index < node.parameter_types.len()
                                {
                                    part.ty.size_of()
                                } else {
                                    8
                                },
                            ),
                        ),
                    ]
                    .into(),
                });
            }
        }
        if signature.indirect_result {
            lines.push(Assembly::inst3(
                ADD,
                X8,
                X29,
                options.offset - node.offset.unwrap(),
            ));
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
        if matches!(result, Type::Struct(_)) {
            if !signature.indirect_result {
                let (mut gp, mut fp) = (0, 0);
                for part in &signature.result {
                    if let Some(bits) = part.ty.float_bits() {
                        lines.push(Assembly::inst2(
                            FMOV,
                            if bits == 32 { W8 } else { X8 },
                            Float(fp, bits),
                        ));
                        fp += 1;
                    } else {
                        lines.push(Assembly::inst2(MOV, X8, ARGS_REG[gp]));
                        gp += 1;
                    }
                    lines.extend([
                        Assembly::inst3(
                            ADD,
                            X11,
                            X29,
                            options.offset - node.offset.unwrap() + part.offset,
                        ),
                        Self::aggregate_store(
                            (result.size_of() - part.offset).min(part.ty.size_of()),
                        ),
                    ]);
                }
            }
            lines.push(Assembly::inst3(
                ADD,
                X0,
                X29,
                options.offset - node.offset.unwrap(),
            ));
        } else if let Some(bits) = result.float_bits() {
            lines.push(Assembly::inst2(
                FMOV,
                if bits == 32 { W0 } else { X0 },
                Float(0, bits),
            ));
        } else {
            lines.push(Self::normalize_integer(&result, X0));
        }
        lines.push(Self::push(X0));
        lines.into()
    }
    fn aggregate_return(&self, node: &Node, options: Options) -> Assembly {
        use crate::generator::abi;
        let ty = node.resolve_type().unwrap();
        let parts = abi::aggregate_parts(&ty, &crate::generator::Arch::Aarch64);
        let mut lines = vec![
            self.gen_node(node.lhs.as_ref().unwrap(), options),
            Self::pop(X10),
        ];
        if parts.is_empty() {
            lines.extend([
                Assembly::inst3(ADD, X11, X29, options.offset - node.offset.unwrap()),
                Assembly::inst2(LDR, X11, Ptr(X11, 8)),
                Self::aggregate_copy(ty.size_of()),
            ]);
        } else {
            let (mut gp, mut fp) = (0, 0);
            for part in parts {
                lines.push(Self::aggregate_load(
                    part.offset,
                    (ty.size_of() - part.offset).min(part.ty.size_of()),
                ));
                lines.push(if let Some(bits) = part.ty.float_bits() {
                    let n = fp;
                    fp += 1;
                    Assembly::inst2(FMOV, Float(n, bits), if bits == 32 { W8 } else { X8 })
                } else {
                    let n = gp;
                    gp += 1;
                    Assembly::inst2(MOV, ARGS_REG[n], X8)
                });
            }
        }
        lines.push(Self::epilogue(options.offset));
        lines.into()
    }
    fn gen_node(&self, node: &Node, options: Options) -> Assembly {
        match node.nt {
            NodeType::VaStart | NodeType::VaEnd => {
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X11),
                ];
                if node.nt == NodeType::VaStart {
                    let (gp, fp, stack) = crate::generator::abi::variadic_state(
                        node,
                        crate::generator::Arch::Aarch64,
                        self.target_os,
                    );
                    lines.extend([
                        Assembly::inst3(ADD, X8, X29, options.offset + 32 + stack),
                        Assembly::inst2(STR, X8, Ptr(X11, 8)),
                    ]);
                    if !matches!(self.target_os, Os::MacOS) {
                        for (position, top) in [(8, 64), (16, 192)] {
                            lines.extend([
                                Assembly::inst3(ADD, X8, X29, top),
                                Assembly::inst3(ADD, X10, X11, position),
                                Assembly::inst2(STR, X8, Ptr(X10, 8)),
                            ]);
                        }
                        for (position, remaining) in [(24, (8 - gp) * 8), (28, (8 - fp) * 16)] {
                            lines.extend([
                                Assembly::inst2(MOV, X8, 0),
                                Assembly::inst3(SUB, X8, X8, remaining),
                                Assembly::inst3(ADD, X10, X11, position),
                                Assembly::inst2(STR, X8, Ptr(X10, 4)),
                            ]);
                        }
                    }
                }
                lines.push(Self::push(0));
                return lines.into();
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
            NodeType::Sequence => {
                let mut lines = vec![];
                for (index, child) in node.children.iter().enumerate() {
                    lines.push(self.gen_node(child, options));
                    if index + 1 < node.children.len()
                        && !matches!(child.nt, NodeType::Block | NodeType::DefVar)
                    {
                        lines.push(Self::pop(X8));
                    }
                }
                return lines.into();
            }
            NodeType::DefVar => {
                return vec![self.gen_statements(&node.children, options), Self::push(0)].into();
            }
            NodeType::CallFunc => {
                if matches!(node.resolve_type(), Some(Type::Struct(_)))
                    || node
                        .args
                        .iter()
                        .any(|arg| matches!(arg.resolve_type(), Some(Type::Struct(_))))
                {
                    return self.aggregate_call(node, options);
                }
                if node.resolve_type().unwrap().float_bits().is_some()
                    || node
                        .args
                        .iter()
                        .any(|arg| arg.resolve_type().unwrap().float_bits().is_some())
                {
                    return self.float_call(node, options);
                }
                let fixed = if matches!(self.target_os, Os::MacOS) && node.variadic {
                    node.parameter_types.len()
                } else {
                    node.args.len()
                };
                let types = node
                    .args
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(i, arg)| {
                        node.parameter_types
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| arg.resolve_type().unwrap())
                    })
                    .collect::<Vec<_>>();
                let (stack_slots, stack_size) = self.stack_arguments(&types, fixed);
                let mut lines = vec![
                    node.lhs
                        .as_ref()
                        .map(|callee| self.gen_node(callee, options))
                        .unwrap_or_else(|| vec![].into()),
                    node.args
                        .iter()
                        .map(|arg| self.gen_node(arg, options))
                        .collect::<Vec<Assembly>>()
                        .into(),
                ];
                lines.push(Assembly::inst3(SUB, X10, X19, stack_size));
                lines.push(Assembly::inst3(AND, X10, X10, "#-16"));
                lines.push(Assembly::inst2(MOV, SP, X10));
                for (i, register) in ARGS_REG.iter().take(fixed).enumerate() {
                    lines.push(Assembly::inst2(
                        LDR,
                        *register,
                        PtrAdd(X19, format!("#{}", i * 8)),
                    ));
                }
                for (i, slot, size) in stack_slots {
                    lines.push(Assembly::inst2(LDR, X8, PtrAdd(X19, format!("#{}", i * 8))));
                    lines.push(Assembly::inst3(ADD, X11, SP, slot));
                    lines.push(Assembly::inst2(STR, X8, Ptr(X11, size)));
                }
                lines.extend([
                    if node.lhs.is_some() {
                        vec![
                            Assembly::inst2(
                                LDR,
                                X12,
                                PtrAdd(X19, format!("#{}", node.args.len() * 8)),
                            ),
                            Assembly::inst1(BLR, X12),
                        ]
                        .into()
                    } else {
                        Assembly::inst1(BL, self.with_prefix(&node.global_name))
                    },
                    Assembly::inst3(
                        ADD,
                        X19,
                        X19,
                        (node.args.len() + usize::from(node.lhs.is_some())) * 8,
                    ),
                    Assembly::inst2(MOV, SP, X29),
                    node.cty
                        .as_ref()
                        .map(|ty| Self::normalize_integer(ty, X0))
                        .unwrap_or_else(|| vec![].into()),
                    Self::push(X0),
                ]);
                return lines.into();
            }
            NodeType::If | NodeType::Conditional => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JE, ElseFlag(branch_num)),
                    self.gen_node(node.then.as_ref().unwrap(), options),
                    Assembly::inst1(JMP, EndFlag(branch_num)),
                    format!("{}:", ElseFlag(branch_num)).into(),
                    node.els
                        .as_ref()
                        .map(|node| self.gen_node(node, options))
                        .unwrap_or_else(|| vec![].into()),
                    format!("{}:", EndFlag(branch_num)).into(),
                    if matches!(node.nt, NodeType::Conditional)
                        && node
                            .resolve_type()
                            .is_some_and(|ty| ty.integer_info().is_some())
                    {
                        vec![
                            Self::pop(X8),
                            Self::normalize_integer(&node.resolve_type().unwrap(), X8),
                            Self::push(X8),
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
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                ];
                let mut default = format!("{}", EndFlag(branch_num));
                for case in &node.children {
                    let label = format!(".Lcase.{}", case.token.as_ref().unwrap().pos);
                    if let Some(value) = case.value {
                        lines.push(Assembly::inst2(MOV, X9, value));
                        lines.push(Assembly::inst2(CMP, X8, X9));
                        lines.push(Assembly::inst1(JE, label));
                    } else {
                        default = label;
                    }
                }
                lines.push(Assembly::inst1(JMP, default));
                lines.push(self.gen_node(
                    node.then.as_ref().unwrap(),
                    Options {
                        breakable_branch_num: branch_num,
                        ..options
                    },
                ));
                lines.push(format!("{}:", EndFlag(branch_num)).into());
                lines.push(Assembly::inst2(MOV, X19, X29));
                return lines.into();
            }
            NodeType::Goto => {
                return Assembly::inst1(JMP, format!(".Lgoto.{}", node.value.unwrap()));
            }
            NodeType::Label => {
                return vec![
                    format!(".Lgoto.{}:", node.value.unwrap()).into(),
                    Assembly::inst2(MOV, X19, X29),
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                ]
                .into();
            }
            NodeType::Case => {
                return vec![
                    format!(".Lcase.{}:", node.token.as_ref().unwrap().pos).into(),
                    Assembly::inst2(MOV, X19, X29),
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                ]
                .into();
            }
            NodeType::DoWhile => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let options = Options {
                    breakable_branch_num: branch_num,
                    ..options
                };
                return vec![
                    format!("{}:", BeginFlag(branch_num)).into(),
                    Assembly::inst2(MOV, X19, X29),
                    self.gen_node(node.then.as_ref().unwrap(), options),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    Assembly::inst2(MOV, X19, X29),
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JNE, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ]
                .into();
            }
            NodeType::While => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let options = Options {
                    breakable_branch_num: branch_num,
                    ..options
                };
                let v = vec![
                    format!(".Lcontinue.{}:", branch_num).into(),
                    format!("{}:", BeginFlag(branch_num)).into(),
                    Assembly::inst2(MOV, X19, X29),
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_node(node.then.as_ref().unwrap(), options),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                return v.into();
            }
            NodeType::For => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let options = Options {
                    breakable_branch_num: branch_num,
                    ..options
                };
                let v = vec![
                    node.ini.as_ref().map_or(Vec::new().into(), |node| {
                        vec![self.gen_node(node, options), Self::pop(X8)].into()
                    }),
                    format!("{}:", BeginFlag(branch_num)).into(),
                    Assembly::inst2(MOV, X19, X29),
                    node.cond
                        .as_ref()
                        .map_or(Assembly::inst2(MOV, X8, 1), |node| {
                            vec![self.gen_node(node, options), Self::pop(X8)].into()
                        }),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_node(node.then.as_ref().unwrap(), options),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    Assembly::inst2(MOV, X19, X29),
                    node.upd.as_ref().map_or(Vec::new().into(), |node| {
                        vec![self.gen_node(node, options), Self::pop(X8)].into()
                    }),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                return v.into();
            }
            NodeType::Block => {
                return self.gen_statements(&node.children, options);
            }
            NodeType::Continue => {
                return Assembly::inst1(JMP, format!(".Lcontinue.{}", node.value.unwrap()));
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
            NodeType::Return => {
                if matches!(node.resolve_type(), Some(Type::Struct(_))) {
                    return self.aggregate_return(node, options);
                }
                if node.lhs.is_none() {
                    return Self::epilogue(options.offset);
                }
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X0),
                    if let Some(bits) = node.resolve_type().and_then(|ty| ty.float_bits()) {
                        Assembly::inst2(FMOV, Float(0, bits), if bits == 32 { W0 } else { X0 })
                    } else {
                        vec![].into()
                    },
                    Self::epilogue(options.offset),
                ]
                .into();
            }
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
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::LogicalAnd => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_node(node.rhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    format!("{}:", EndFlag(branch_num)).into(),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst2(CSET, X8, "ne"),
                    Self::push(X8),
                ]
                .into();
            }
            NodeType::LogicalOr => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JNE, EndFlag(branch_num)),
                    self.gen_node(node.rhs.as_ref().unwrap(), options),
                    Self::pop(X8),
                    format!("{}:", EndFlag(branch_num)).into(),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst2(CSET, X8, "ne"),
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
        vec![
            self.gen_node(node.rhs.as_ref().unwrap(), options),
            self.gen_node(node.lhs.as_ref().unwrap(), options),
            Self::pop(X8),
            Self::pop(X13),
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
                NodeType::BitAnd => Assembly::inst3(AND, X8, X8, X13),
                NodeType::BitXor => Assembly::inst3(XOR, X8, X8, X13),
                NodeType::BitOr => Assembly::inst3(OR, X8, X8, X13),
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
                        NodeType::Block
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
            NodeType::CallFunc => self.gen_node(node, options),
            NodeType::Conditional => self.gen_node(node, options),

            NodeType::Sequence => {
                let mut lines = vec![];
                for child in &node.children[..node.children.len() - 1] {
                    lines.push(self.gen_node(child, options));
                    if !matches!(child.nt, NodeType::Block | NodeType::DefVar) {
                        lines.push(Self::pop(X8));
                    }
                }
                lines.push(self.gen_addr(node.children.last().unwrap(), options));
                lines.into()
            }

            NodeType::GlobalVar => vec![
                if !node.dest.is_empty() {
                    vec![
                        Assembly::inst2(
                            ADRP,
                            X8,
                            match self.target_os {
                                Os::MacOS => node.dest.clone().replace('@', "L_") + "@PAGE",
                                Os::Linux => node.dest.clone().replace('@', "L_"),
                            },
                        ),
                        Assembly::inst3(
                            ADD,
                            X8,
                            X8,
                            match self.target_os {
                                Os::MacOS => node.dest.clone().replace('@', "L_") + "@PAGEOFF",
                                Os::Linux => {
                                    ":lo12:".to_string() + &node.dest.clone().replace('@', "L_")
                                }
                            },
                        ),
                    ]
                    .into()
                } else {
                    vec![
                        Assembly::inst2(
                            ADRP,
                            X8,
                            match self.target_os {
                                Os::MacOS => self.with_prefix(&node.global_name) + "@PAGE",
                                Os::Linux => self.with_prefix(&node.global_name),
                            },
                        ),
                        Assembly::inst3(
                            ADD,
                            X8,
                            X8,
                            match self.target_os {
                                Os::MacOS => self.with_prefix(&node.global_name) + "@PAGEOFF",
                                Os::Linux => {
                                    ":lo12:".to_string()
                                        + &self.with_prefix(&node.global_name).replace('@', "L_")
                                }
                            },
                        ),
                    ]
                    .into()
                },
                Self::push(X8),
            ]
            .into(),
            NodeType::LocalVar => vec![
                Assembly::inst3(ADD, X8, X29, options.offset - node.offset.unwrap()),
                Self::push(X8),
            ]
            .into(),
            NodeType::Deref => self.gen_node(node.lhs.as_ref().unwrap(), options),
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

    fn with_prefix<T: Display>(&self, s: T) -> String {
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

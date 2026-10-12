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
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use std::fmt::Display;

pub struct AsmGenerator<'a> {
    error_logger: &'a dyn error::ErrorLogger,
    target_os: Os,
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
        }
    }

    fn generate(&self, ast: ProgramAst) -> Box<dyn crate::generator::Assembly> {
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
                        self.gen_func(name, func, func.offset_size.div_ceil(16) * 16)
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
            if gv.internal {
                vec![].into()
            } else {
                format!(".globl {}", self.with_prefix(name)).into()
            },
            format!("  .balign {}", gv.ty.align_of()).into(),
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
            Type::Struct(struct_type) => {
                let elements = if let Some(GlobalVariableData::Arr(elements)) = data {
                    Some(elements)
                } else {
                    None
                };
                let mut lines = vec![];
                let mut end = 0;
                for (i, member) in struct_type.members().iter().enumerate() {
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

    fn gen_func(&self, name: &str, func: &Func, offset: usize) -> Assembly {
        if func.body.is_none() {
            return vec![].into();
        }
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
            func.args
                .iter()
                .enumerate()
                .map(|(i, arg)| {
                    if let NodeType::LocalVar = arg.nt {
                        vec![
                            Assembly::inst2(MOV, RAX, RBP),
                            Assembly::inst2(SUB, RAX, arg.offset.unwrap()),
                            if let Some(register) = ARGS_REG.get(i) {
                                Assembly::inst2(MOV, RDI, *register)
                            } else {
                                Assembly::inst2(
                                    MOV,
                                    RDI,
                                    PtrAdd(RBP, format!("{}", 16 + (i - ARGS_REG.len()) * 8)),
                                )
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

    fn gen_node(&self, node: &Node, offset: usize, breakable_branch_num: usize) -> Assembly {
        match node.nt {
            NodeType::Cast => {
                let mut lines = vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                ];
                if let Some(ty) = &node.cty {
                    lines.push(Self::normalize_integer(ty, RAX));
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
                            Assembly::inst2(MOV, R10, PtrAdd(RSP, format!("{}", stack_size + 8))),
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
                return vec![
                    self.gen_node(node.cond.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JE, ElseFlag(branch_num)),
                    self.gen_node(node.then.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(JMP, EndFlag(branch_num)),
                    format!("{}:", ElseFlag(branch_num)).into(),
                    node.els
                        .as_ref()
                        .map(|node| self.gen_node(node, offset, breakable_branch_num))
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
                lines.push(Assembly::reset_stack(offset));
                return lines.into();
            }
            NodeType::Goto => {
                return Assembly::inst1(JMP, format!(".Lgoto.{}", node.value.unwrap()));
            }
            NodeType::Label => {
                return vec![
                    format!(".Lgoto.{}:", node.value.unwrap()).into(),
                    Assembly::reset_stack(offset),
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                ]
                .into();
            }
            NodeType::Case => {
                return vec![
                    format!(".Lcase.{}:", node.token.as_ref().unwrap().pos).into(),
                    Assembly::reset_stack(offset),
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                ]
                .into();
            }
            NodeType::DoWhile => {
                let branch_num = node.token.as_ref().unwrap().pos;
                return vec![
                    format!("{}:", BeginFlag(branch_num)).into(),
                    Assembly::reset_stack(offset),
                    self.gen_node(node.then.as_ref().unwrap(), offset, branch_num),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    Assembly::reset_stack(offset),
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
                    Assembly::reset_stack(offset),
                    self.gen_node(node.cond.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(POP, RAX),
                    Assembly::inst2(CMP, RAX, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_node(node.then.as_ref().unwrap(), offset, branch_num),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                return v.into();
            }
            NodeType::For => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let v = vec![
                    node.ini.as_ref().map_or(Vec::new().into(), |node| {
                        vec![
                            self.gen_node(node, offset, branch_num),
                            Assembly::inst1(POP, RAX),
                        ]
                        .into()
                    }),
                    format!("{}:", BeginFlag(branch_num)).into(),
                    Assembly::reset_stack(offset),
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
                    self.gen_node(node.then.as_ref().unwrap(), offset, branch_num),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    Assembly::reset_stack(offset),
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
                if node.lhs.is_none() {
                    return Assembly::epilogue();
                }
                return vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
                    Assembly::inst1(POP, RAX),
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
                        Type::Arr(..) | Type::Struct(_) | Type::Func(..) | Type::VariadicFunc(..),
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
                        Type::Arr(..) | Type::Struct(_) | Type::Func(..) | Type::VariadicFunc(..),
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
            self.gen_node(node.rhs.as_ref().unwrap(), offset, breakable_branch_num),
            self.gen_node(node.lhs.as_ref().unwrap(), offset, breakable_branch_num),
            Assembly::inst1(POP, RAX),
            Assembly::inst1(POP, RDI),
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
                    Assembly::inst1(POP, RAX),
                    Assembly::reset_stack(offset),
                ]
                .into()
            })
            .collect::<Vec<Assembly>>()
            .into()
    }

    fn gen_addr(&self, node: &Node, offset: usize, branch_num: usize) -> Assembly {
        match node.nt {
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
            Some(Type::I32) => Assembly::inst2(operator, Ptr(from, 4), EDI),
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

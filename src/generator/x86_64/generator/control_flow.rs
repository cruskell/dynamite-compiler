use super::super::assembly::Assembly;
use super::super::instruction::{InstOperand::*, InstOperator::*, Register::*};
use super::AsmGenerator;
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};

impl AsmGenerator<'_> {
    pub(super) fn gen_control_flow(
        &self,
        node: &Node,
        offset: usize,
        breakable_branch_num: usize,
    ) -> Assembly {
        match node.nt {
            NodeType::LogicalAnd => {
                let branch_num = node.token.as_ref().unwrap().pos;
                vec![
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
                .into()
            }
            NodeType::LogicalOr => {
                let branch_num = node.token.as_ref().unwrap().pos;
                vec![
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
                .into()
            }
            NodeType::Conditional | NodeType::If => {
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
                vec![
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
                .into()
            }
            NodeType::Block => self.gen_statements(&node.children, offset, breakable_branch_num),
            NodeType::Return => {
                if matches!(node.resolve_type(), Some(Type::Struct(_))) {
                    return self.aggregate_return(node, offset, breakable_branch_num);
                }
                if node.lhs.is_none() {
                    return Assembly::epilogue();
                }
                vec![
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
                .into()
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
                v.into()
            }
            NodeType::DoWhile => {
                let branch_num = node.token.as_ref().unwrap().pos;
                vec![
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
                .into()
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
                v.into()
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
                lines.into()
            }
            NodeType::Case => vec![
                format!(".Lcase.{}:", node.token.as_ref().unwrap().pos).into(),
                self.gen_statements(
                    std::slice::from_ref(node.lhs.as_deref().unwrap()),
                    offset,
                    breakable_branch_num,
                ),
            ]
            .into(),
            NodeType::Continue => {
                Assembly::inst1(JMP, format!(".Lcontinue.{}", node.value.unwrap()))
            }
            NodeType::Goto => Assembly::inst1(JMP, format!(".Lgoto.{}", node.value.unwrap())),
            NodeType::Label => vec![
                format!(".Lgoto.{}:", node.value.unwrap()).into(),
                self.gen_statements(
                    std::slice::from_ref(node.lhs.as_deref().unwrap()),
                    offset,
                    breakable_branch_num,
                ),
            ]
            .into(),
            _ => unreachable!("node dispatched to the wrong generator"),
        }
    }
}

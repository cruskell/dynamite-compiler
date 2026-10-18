use super::super::assembly::Assembly;
use super::super::instruction::{InstOperand::*, InstOperator::*, Register::*};
use super::{AsmGenerator, Options};
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};

impl AsmGenerator<'_> {
    pub(super) fn gen_control_flow(&self, node: &Node, options: Options) -> Assembly {
        match node.nt {
            NodeType::Conditional | NodeType::If => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let branch = |child: &Node| {
                    if node.nt == NodeType::If {
                        self.gen_statements(std::slice::from_ref(child), options)
                    } else {
                        self.gen_node(child, options)
                    }
                };
                vec![
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
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
                            Self::pop(X8),
                            Self::normalize_integer(&node.resolve_type().unwrap(), X8),
                            Self::push(X8),
                        ]
                        .into()
                    } else {
                        vec![].into()
                    },
                ]
                .into()
            }
            NodeType::Block => self.gen_statements(&node.children, options),
            NodeType::Return => {
                if matches!(node.resolve_type(), Some(Type::Struct(_))) {
                    return self.aggregate_return(node, options);
                }
                if node.lhs.is_none() {
                    return Self::epilogue(options.offset);
                }
                vec![
                    self.gen_node(node.lhs.as_ref().unwrap(), options),
                    Self::pop(X0),
                    if let Some(bits) = node.resolve_type().and_then(|ty| ty.float_bits()) {
                        Assembly::inst2(FMOV, Float(0, bits), if bits == 32 { W0 } else { X0 })
                    } else {
                        vec![].into()
                    },
                    Self::epilogue(options.offset),
                ]
                .into()
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
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_statements(
                        std::slice::from_ref(node.then.as_deref().unwrap()),
                        options,
                    ),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                v.into()
            }
            NodeType::DoWhile => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let options = Options {
                    breakable_branch_num: branch_num,
                    ..options
                };
                vec![
                    format!("{}:", BeginFlag(branch_num)).into(),
                    self.gen_statements(
                        std::slice::from_ref(node.then.as_deref().unwrap()),
                        options,
                    ),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    self.gen_node(node.cond.as_ref().unwrap(), options),
                    Self::pop(X8),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JNE, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ]
                .into()
            }
            NodeType::For => {
                let branch_num = node.token.as_ref().unwrap().pos;
                let options = Options {
                    breakable_branch_num: branch_num,
                    ..options
                };
                let v = vec![
                    node.ini.as_ref().map_or(Vec::new().into(), |node| {
                        self.gen_statements(std::slice::from_ref(node.as_ref()), options)
                    }),
                    format!("{}:", BeginFlag(branch_num)).into(),
                    node.cond
                        .as_ref()
                        .map_or(Assembly::inst2(MOV, X8, 1), |node| {
                            vec![self.gen_node(node, options), Self::pop(X8)].into()
                        }),
                    Assembly::inst2(CMP, X8, 0),
                    Assembly::inst1(JE, EndFlag(branch_num)),
                    self.gen_statements(
                        std::slice::from_ref(node.then.as_deref().unwrap()),
                        options,
                    ),
                    format!(".Lcontinue.{}:", branch_num).into(),
                    node.upd.as_ref().map_or(Vec::new().into(), |node| {
                        vec![self.gen_node(node, options), Self::pop(X8)].into()
                    }),
                    Assembly::inst1(JMP, BeginFlag(branch_num)),
                    format!("{}:", EndFlag(branch_num)).into(),
                ];
                v.into()
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
                lines.into()
            }
            NodeType::Case => vec![
                format!(".Lcase.{}:", node.token.as_ref().unwrap().pos).into(),
                self.gen_statements(std::slice::from_ref(node.lhs.as_deref().unwrap()), options),
            ]
            .into(),
            NodeType::Continue => {
                Assembly::inst1(JMP, format!(".Lcontinue.{}", node.value.unwrap()))
            }
            NodeType::Goto => Assembly::inst1(JMP, format!(".Lgoto.{}", node.value.unwrap())),
            NodeType::Label => vec![
                format!(".Lgoto.{}:", node.value.unwrap()).into(),
                self.gen_statements(std::slice::from_ref(node.lhs.as_deref().unwrap()), options),
            ]
            .into(),
            _ => unreachable!("node dispatched to the wrong generator"),
        }
    }
}

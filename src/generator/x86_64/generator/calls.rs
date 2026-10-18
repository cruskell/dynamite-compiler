use super::super::assembly::Assembly;
use super::super::instruction::{InstOperand::*, InstOperator::*, Register::*};
use super::{ARGS_REG, AsmGenerator};
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};

impl AsmGenerator<'_> {
    pub(super) fn gen_calls(
        &self,
        node: &Node,
        offset: usize,
        breakable_branch_num: usize,
    ) -> Assembly {
        match node.nt {
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
                vec![
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
                .into()
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
                lines.into()
            }
            _ => unreachable!("node dispatched to the wrong generator"),
        }
    }
}

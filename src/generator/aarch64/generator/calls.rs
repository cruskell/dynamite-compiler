use super::super::assembly::Assembly;
use super::super::instruction::{InstOperand::*, InstOperator::*, Register::*};
use super::{ARGS_REG, AsmGenerator, Options};
use crate::generator::Os;
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};

impl AsmGenerator<'_> {
    pub(super) fn gen_calls(&self, node: &Node, options: Options) -> Assembly {
        match node.nt {
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
                lines.into()
            }
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
                lines.into()
            }
            _ => unreachable!("node dispatched to the wrong generator"),
        }
    }
}

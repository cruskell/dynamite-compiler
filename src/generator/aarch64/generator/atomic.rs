use super::AsmGenerator;
use super::Options;
use crate::generator::aarch64::assembly::Assembly;
use crate::generator::aarch64::instruction::{InstOperand::*, InstOperator::*, Register::*};
use crate::node::{AtomicOperation, AtomicRmwOperation, Node};

impl AsmGenerator<'_> {
    pub(super) fn atomic_update(
        node: &Node,
        operation: AtomicRmwOperation,
        return_new: bool,
    ) -> Assembly {
        let ty = node.args[0].dest_type().unwrap();
        let label = format!(".Latomic.{}", node.token.as_ref().unwrap().pos);
        let mut lines = vec![
            format!("{label}:").into(),
            Assembly::inst2(LDAXR, X10, Ptr(X8, ty.size_of())),
        ];
        lines.push(match operation {
            AtomicRmwOperation::Exchange => Assembly::inst2(MOV, X11, X9),
            AtomicRmwOperation::Nand => vec![
                Assembly::inst3(AND, X11, X10, X9),
                Assembly::inst2(MVN, X11, X11),
            ]
            .into(),
            _ => Assembly::inst3(
                match operation {
                    AtomicRmwOperation::Add => ADD,
                    AtomicRmwOperation::Sub => SUB,
                    AtomicRmwOperation::And => AND,
                    AtomicRmwOperation::Or => OR,
                    AtomicRmwOperation::Xor => XOR,
                    _ => unreachable!(),
                },
                X11,
                X10,
                X9,
            ),
        });
        lines.extend([
            Assembly::inst3(STLXR, W0, X11, Ptr(X8, ty.size_of())),
            Assembly::inst2(CMP, X0, 0),
            Assembly::inst1(JNE, label),
        ]);
        let result = if return_new { X11 } else { X10 };
        lines.extend([Self::normalize_integer(&ty, result), Self::push(result)]);
        lines.into()
    }

    pub(super) fn atomic_compare_exchange(node: &Node) -> Assembly {
        let size = node.args[0].dest_type().unwrap().size_of();
        let prefix = format!(".Latomic.cas.{}", node.token.as_ref().unwrap().pos);
        vec![
            Self::pop(X0),
            Self::pop(X0),
            Self::pop(X0),
            Self::pop(X10),
            Self::pop(X9),
            Self::pop(X8),
            Assembly::inst2(LDRU, X12, Ptr(X9, size)),
            format!("{prefix}.retry:").into(),
            Assembly::inst2(LDAXR, X11, Ptr(X8, size)),
            Assembly::inst2(CMP, X11, X12),
            Assembly::inst1(JNE, format!("{prefix}.failure")),
            Assembly::inst3(STLXR, W0, X10, Ptr(X8, size)),
            Assembly::inst2(CMP, X0, 0),
            Assembly::inst1(JNE, format!("{prefix}.retry")),
            Assembly::inst2(MOV, X0, 1),
            Assembly::inst1(JMP, format!("{prefix}.done")),
            format!("{prefix}.failure:").into(),
            Assembly::inst0(CLREX),
            Assembly::inst2(STR, X11, Ptr(X9, size)),
            Assembly::inst2(MOV, X0, 0),
            format!("{prefix}.done:").into(),
            Self::push(X0),
        ]
        .into()
    }

    pub(super) fn gen_atomic(
        &self,
        node: &Node,
        operation: AtomicOperation,
        options: Options,
    ) -> Assembly {
        let mut lines = vec![];
        for argument in &node.args {
            lines.push(self.gen_node(argument, options));
        }
        if matches!(operation, AtomicOperation::CompareExchange(_)) {
            lines.push(Self::atomic_compare_exchange(node));
            return lines.into();
        }
        lines.push(Self::pop(X10));
        let ty = node.args[0].dest_type().unwrap();
        if operation == AtomicOperation::Load {
            lines.extend([
                Self::pop(X8),
                Assembly::inst2(LDAR, X8, Ptr(X8, ty.size_of())),
                Self::normalize_integer(&ty, X8),
                Self::push(X8),
            ]);
        } else {
            lines.extend([Self::pop(X9), Self::pop(X8)]);
            if let AtomicOperation::Update(op, return_new) = operation {
                lines.push(Self::atomic_update(node, op, return_new));
            } else {
                lines.extend([
                    Assembly::inst2(STLR, X9, Ptr(X8, ty.size_of())),
                    Self::push(0),
                ]);
            }
        }
        lines.into()
    }
}

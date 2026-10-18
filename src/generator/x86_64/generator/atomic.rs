use super::AsmGenerator;
use crate::generator::x86_64::assembly::Assembly;
use crate::generator::x86_64::instruction::{InstOperand::*, InstOperator::*, Register::*};
use crate::node::{AtomicOperation, AtomicRmwOperation, Node};

impl AsmGenerator<'_> {
    pub(super) fn atomic_update(
        node: &Node,
        operation: AtomicRmwOperation,
        return_new: bool,
    ) -> Assembly {
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
            AtomicRmwOperation::Nand => {
                vec![Assembly::inst2(AND, RCX, RSI), Assembly::inst1(NOT, RCX)].into()
            }
            AtomicRmwOperation::Exchange => Assembly::inst2(MOV, RCX, RSI),
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

    pub(super) fn atomic_compare_exchange(node: &Node) -> Assembly {
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

    pub(super) fn gen_atomic(
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
}

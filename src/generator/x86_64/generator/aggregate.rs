use super::ARGS_REG;
use super::AsmGenerator;
use crate::ctype::Type;
use crate::func::Func;
use crate::generator::x86_64::assembly::Assembly;
use crate::generator::x86_64::instruction::{InstOperand::*, InstOperator::*, Register::*};
use crate::node::Node;

impl AsmGenerator<'_> {
    pub(super) fn aggregate_load(offset: usize, size: usize) -> Assembly {
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

    pub(super) fn aggregate_store(size: usize) -> Assembly {
        let mut lines = vec![];
        for index in 0..size {
            lines.push(Assembly::inst2(MOV, PtrOffset(R11, index, 1), AL));
            if index + 1 < size {
                lines.push(Assembly::inst2(SHR, RAX, 8));
            }
        }
        lines.into()
    }

    pub(super) fn aggregate_copy(size: usize) -> Assembly {
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

    pub(super) fn aggregate_function(&self, name: &str, func: &Func, locals: usize) -> Assembly {
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

    pub(super) fn aggregate_call(&self, node: &Node, offset: usize, branch: usize) -> Assembly {
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

    pub(super) fn aggregate_return(&self, node: &Node, offset: usize, branch: usize) -> Assembly {
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
}

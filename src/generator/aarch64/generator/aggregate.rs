use super::ARGS_REG;
use super::AsmGenerator;
use super::Options;
use crate::ctype::Type;
use crate::func::Func;
use crate::generator::Os;
use crate::generator::aarch64::assembly::Assembly;
use crate::generator::aarch64::instruction::{InstOperand::*, InstOperator::*, Register::*};
use crate::node::Node;

impl AsmGenerator<'_> {
    pub(super) fn aggregate_load(offset: usize, size: usize) -> Assembly {
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

    pub(super) fn aggregate_store(size: usize) -> Assembly {
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

    pub(super) fn aggregate_copy(size: usize) -> Assembly {
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

    pub(super) fn aggregate_function(&self, name: &str, func: &Func, locals: usize) -> Assembly {
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

    pub(super) fn aggregate_call(&self, node: &Node, options: Options) -> Assembly {
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

    pub(super) fn aggregate_return(&self, node: &Node, options: Options) -> Assembly {
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
}

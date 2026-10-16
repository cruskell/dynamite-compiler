use super::*;
use std::rc::Rc;

fn number(value: usize) -> Node {
    Node::new_with_num(None, value)
}
fn cast(node: Node, ty: Type) -> Node {
    Node {
        nt: NodeType::Cast,
        lhs: Some(Box::new(node)),
        cty: Some(ty),
        ..Node::default()
    }
}
fn binary(nt: NodeType, lhs: Node, rhs: Node) -> Node {
    Node::new_with_op(None, nt, lhs, rhs)
}
fn sequence(children: Vec<Node>) -> Node {
    Node {
        nt: NodeType::Sequence,
        children,
        ..Node::default()
    }
}
fn address(node: Node) -> Node {
    Node::new_with_op_and_lhs(None, NodeType::Addr, node)
}
fn pointer() -> Type {
    Type::Ptr(Rc::new(Type::I8))
}
fn field(base: Node, offset: usize, ty: Type) -> Node {
    let byte_pointer = binary(NodeType::Add, cast(base, pointer()), number(offset));
    Node::new_with_op_and_lhs(
        None,
        NodeType::Deref,
        cast(byte_pointer, Type::Ptr(Rc::new(ty))),
    )
}
fn align_pointer(value: Node, alignment: usize) -> Node {
    let integer = Type::integer(64, false);
    let offset = Node {
        cty: Some(integer.clone()),
        ..number(alignment - 1)
    };
    let mask = Node {
        cty: Some(integer.clone()),
        ..number(!(alignment - 1))
    };
    cast(
        binary(
            NodeType::BitAnd,
            binary(NodeType::Add, cast(value, integer), offset),
            mask,
        ),
        pointer(),
    )
}

fn conditional(cond: Node, then: Node, els: Node) -> Node {
    Node {
        nt: NodeType::Conditional,
        cond: Some(Box::new(cond)),
        then: Some(Box::new(then)),
        els: Some(Box::new(els)),
        ..Node::default()
    }
}

impl AstBuilderImpl<'_> {
    pub(super) fn builtin_va_list_type(&mut self) -> Type {
        if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            return pointer();
        }
        let fields = if cfg!(target_arch = "x86_64") {
            vec![
                ("gp_offset", Type::integer(32, false)),
                ("fp_offset", Type::integer(32, false)),
                ("overflow_arg_area", Type::Ptr(Rc::new(Type::Void))),
                ("reg_save_area", Type::Ptr(Rc::new(Type::Void))),
            ]
        } else {
            vec![
                ("__stack", Type::Ptr(Rc::new(Type::Void))),
                ("__gr_top", Type::Ptr(Rc::new(Type::Void))),
                ("__vr_top", Type::Ptr(Rc::new(Type::Void))),
                ("__gr_offs", Type::I32),
                ("__vr_offs", Type::I32),
            ]
        };
        let (structure, owner) = StructType::new(self.struct_definitions.len());
        let mut offset: usize = 0;
        let mut alignment: usize = 1;
        let mut members = vec![];
        for (name, ty) in fields {
            alignment = alignment.max(ty.align_of());
            offset = offset.div_ceil(ty.align_of()) * ty.align_of();
            let size = ty.size_of();
            members.push(StructMember {
                name: name.into(),
                anonymous: false,
                requested_alignment: 0,
                ty,
                offset,
                bit_field: None,
            });
            offset += size;
        }
        *owner.borrow_mut() = StructDefinition {
            members,
            size: offset.div_ceil(alignment) * alignment,
            align: alignment,
            natural_align: alignment,
            complete: true,
            ..StructDefinition::default()
        };
        self.struct_definitions.push(owner);
        if cfg!(target_arch = "x86_64") {
            Type::Arr(Rc::new(Type::Struct(structure)), Some(1))
        } else {
            Type::Struct(structure)
        }
    }

    fn va_temporary(&mut self, token: &Token, ty: Type) -> Node {
        self.register_local_variable(
            Token {
                s_value: format!(".va.{}.{}", token.pos, self.offset_size).into(),
                ..token.clone()
            },
            ty,
        )
    }
    fn va_address(&self, ap: Node, token: &Token) -> Node {
        let ty = ap.resolve_type().unwrap();
        let valid = if cfg!(target_arch = "x86_64") {
            matches!(ty.unqualified(), Type::Arr(..) | Type::Ptr(_))
        } else if cfg!(target_os = "macos") {
            matches!(ty.unqualified(), Type::Ptr(_))
        } else {
            matches!(ty.unqualified(), Type::Struct(_))
        };
        if !valid {
            self.error_logger
                .print_error_position(token.pos, "expected a va_list object");
        }
        if cfg!(target_arch = "x86_64") && matches!(ty.unqualified(), Type::Ptr(_)) {
            cast(ap, pointer())
        } else {
            cast(address(ap), pointer())
        }
    }
    pub(super) fn varargs_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let ap = self.assign();
        let ap = self.va_address(ap, &token);
        match token.s_value.as_str() {
            "__builtin_va_start" => {
                self.expect_reserved(",");
                let last = self.assign();
                self.expect_reserved(")");
                let Some(parameters) = self.variadic_parameters.clone() else {
                    self.error_logger
                        .print_error_position(token.pos, "va_start requires a variadic function");
                    unreachable!()
                };
                if last.nt != NodeType::LocalVar || last.offset != self.variadic_last {
                    self.error_logger.print_error_position(
                        token.pos,
                        "va_start requires the last named parameter",
                    );
                }
                Node {
                    nt: NodeType::VaStart,
                    lhs: Some(Box::new(ap)),
                    rhs: Some(Box::new(Node {
                        cty: self.return_type.clone(),
                        ..Node::default()
                    })),
                    parameter_types: parameters,
                    cty: Some(Type::Void),
                    token: Some(token),
                    ..Node::default()
                }
            }
            "__builtin_va_arg" => {
                self.expect_reserved(",");
                let base = self.expect_type();
                let (_, ty, _, _, _) = self.declarator().resolve(base);
                self.expect_reserved(")");
                self.expect_object_type(&ty, &token);
                if matches!(ty.unqualified(), Type::Arr(..) | Type::Vla(..))
                    || ty.float_bits() == Some(32)
                    || ty.integer_info().is_some_and(|(bits, _)| bits < 32)
                {
                    self.error_logger.print_error_position(
                        token.pos,
                        "va_arg requires a type after default argument promotions",
                    );
                }
                if ty.contains_float16() {
                    self.error_logger.print_error_position(
                        token.pos,
                        "va_arg for types containing _Float16 is not supported",
                    );
                }
                if ty.float_bits() == Some(128)
                    || (matches!(ty.unqualified(), Type::Struct(_))
                        && ty.contains_extended_long_double())
                {
                    self.error_logger.print_error_position(
                        token.pos,
                        "va_arg for this extended long double type is not supported",
                    );
                }
                self.va_argument(ap, ty, &token)
            }
            "__builtin_va_copy" => {
                self.expect_reserved(",");
                let source = self.assign();
                let source = self.va_address(source, &token);
                self.expect_reserved(")");
                let dest = self.va_temporary(&token, pointer());
                let src = self.va_temporary(&token, pointer());
                let size = if cfg!(target_arch = "x86_64") {
                    24
                } else if cfg!(target_os = "macos") {
                    8
                } else {
                    32
                };
                let mut children = vec![
                    binary(NodeType::Assign, dest.clone(), ap),
                    binary(NodeType::Assign, src.clone(), source),
                ];
                for offset in 0..size {
                    children.push(binary(
                        NodeType::Assign,
                        field(dest.clone(), offset, Type::I8),
                        field(src.clone(), offset, Type::I8),
                    ));
                }
                children.push(number(0));
                sequence(children)
            }
            "__builtin_va_end" => {
                self.expect_reserved(")");
                Node {
                    nt: NodeType::VaEnd,
                    lhs: Some(Box::new(ap)),
                    cty: Some(Type::Void),
                    token: Some(token),
                    ..Node::default()
                }
            }
            _ => {
                self.error_logger
                    .print_error_position(token.pos, "unknown variadic builtin");
                unreachable!()
            }
        }
    }
    fn va_argument(&mut self, ap: Node, ty: Type, token: &Token) -> Node {
        use crate::generator::{Arch, Os, abi};
        let state = self.va_temporary(token, pointer());
        let result = self.va_temporary(token, ty.clone());
        let mut children = vec![binary(NodeType::Assign, state.clone(), ap)];
        if ty.size_of() == 0 {
            // Empty GNU records have no payload and do not advance the argument cursor.
            children.push(result);
            return sequence(children);
        }
        let destination = address(result.clone());
        let arm = cfg!(target_arch = "aarch64");
        let mac_arm = arm && cfg!(target_os = "macos");
        let arch = if arm { Arch::Aarch64 } else { Arch::X86_64 };
        let os = if cfg!(target_os = "macos") {
            Os::MacOS
        } else {
            Os::Linux
        };
        let parts = abi::aggregate_parts(&ty, &arch);
        let stack_alignment = abi::stack_argument_alignment(&ty, &arch, os, true);
        let indirect = arm && matches!(ty.unqualified(), Type::Struct(_)) && parts.is_empty();
        let mut stack = field(state.clone(), if arm { 0 } else { 8 }, pointer());
        let stack_size = if indirect {
            8
        } else {
            ty.size_of().div_ceil(8) * 8
        };
        let source = self.va_temporary(token, pointer());
        let mut from_stack = {
            let mut steps = vec![
                binary(
                    NodeType::Assign,
                    source.clone(),
                    align_pointer(stack.clone(), stack_alignment),
                ),
                binary(
                    NodeType::Assign,
                    stack.clone(),
                    binary(NodeType::Add, source.clone(), number(stack_size)),
                ),
            ];
            if indirect {
                steps.push(binary(
                    NodeType::Assign,
                    source.clone(),
                    field(source.clone(), 0, pointer()),
                ));
            }
            for offset in 0..ty.size_of() {
                steps.push(binary(
                    NodeType::Assign,
                    field(destination.clone(), offset, Type::I8),
                    field(source.clone(), offset, Type::I8),
                ));
            }
            steps.push(result.clone());
            sequence(steps)
        };
        if mac_arm {
            // Darwin's va_list is the cursor itself, rather than a state structure.
            stack = field(state.clone(), 0, pointer());
            let mut steps = vec![
                binary(
                    NodeType::Assign,
                    source.clone(),
                    align_pointer(stack.clone(), stack_alignment),
                ),
                binary(
                    NodeType::Assign,
                    stack.clone(),
                    binary(NodeType::Add, source.clone(), number(stack_size)),
                ),
            ];
            if indirect {
                steps.push(binary(
                    NodeType::Assign,
                    source.clone(),
                    field(source.clone(), 0, pointer()),
                ));
            }
            for offset in 0..ty.size_of() {
                steps.push(binary(
                    NodeType::Assign,
                    field(destination.clone(), offset, Type::I8),
                    field(source.clone(), offset, Type::I8),
                ));
            }
            steps.push(result);
            children.push(sequence(steps));
            return sequence(children);
        }
        let mut parts = parts;
        if indirect {
            parts = vec![abi::Part {
                offset: 0,
                ty: pointer(),
            }];
        }
        if parts.is_empty() {
            children.push(from_stack);
            return sequence(children);
        }
        let gp = parts
            .iter()
            .filter(|part| part.ty.float_bits().is_none())
            .count();
        let fp = parts.len() - gp;
        let gp_offset = field(
            state.clone(),
            if arm { 24 } else { 0 },
            if arm {
                Type::I32
            } else {
                Type::integer(32, false)
            },
        );
        let fp_offset = field(
            state.clone(),
            if arm { 28 } else { 4 },
            if arm {
                Type::I32
            } else {
                Type::integer(32, false)
            },
        );
        let gp_alignment = abi::general_register_alignment(&ty, &arch, os) * 8;
        if gp > 0 && !indirect && gp_alignment > 8 {
            children.push(binary(
                NodeType::Assign,
                gp_offset.clone(),
                binary(
                    NodeType::BitAnd,
                    binary(NodeType::Add, gp_offset.clone(), number(gp_alignment - 1)),
                    number(!(gp_alignment - 1)),
                ),
            ));
        }
        if arm {
            // A spilled aggregate exhausts its register class for later arguments.
            for (offset, count) in [(gp_offset.clone(), gp), (fp_offset.clone(), fp)] {
                if count > 0 {
                    from_stack
                        .children
                        .insert(0, binary(NodeType::Assign, offset, number(0)));
                }
            }
        }
        let mut condition = number(1);
        for (offset, count, limit, stride) in [
            (gp_offset.clone(), gp, 48, 8),
            (fp_offset.clone(), fp, 176, 16),
        ] {
            if count > 0 {
                let fits = if arm {
                    binary(
                        NodeType::Le,
                        offset,
                        cast(
                            number((0isize - (count * stride) as isize) as usize),
                            Type::I32,
                        ),
                    )
                } else {
                    binary(NodeType::Le, offset, number(limit - count * stride))
                };
                condition = binary(NodeType::Mul, condition, fits);
            }
        }
        let mut register_steps = vec![];
        let (mut gp_index, mut fp_index) = (0, 0);
        for part in parts {
            let floating = part.ty.float_bits().is_some();
            let (index, offset, stride, base) = if floating {
                let index = fp_index;
                fp_index += 1;
                (index, fp_offset.clone(), 16, 16)
            } else {
                let index = gp_index;
                gp_index += 1;
                (index, gp_offset.clone(), 8, if arm { 8 } else { 16 })
            };
            let register_pointer = binary(
                NodeType::Add,
                field(state.clone(), base, pointer()),
                binary(
                    NodeType::Add,
                    cast(offset, Type::integer(64, true)),
                    number(index * stride),
                ),
            );
            if indirect {
                register_steps.push(binary(
                    NodeType::Assign,
                    source.clone(),
                    field(register_pointer, 0, pointer()),
                ));
                for byte in 0..ty.size_of() {
                    register_steps.push(binary(
                        NodeType::Assign,
                        field(destination.clone(), byte, Type::I8),
                        field(source.clone(), byte, Type::I8),
                    ));
                }
            } else {
                for byte in 0..(ty.size_of() - part.offset).min(part.ty.size_of()) {
                    register_steps.push(binary(
                        NodeType::Assign,
                        field(destination.clone(), part.offset + byte, Type::I8),
                        field(register_pointer.clone(), byte, Type::I8),
                    ));
                }
            }
        }
        for (offset, count, stride) in [(gp_offset, gp, 8), (fp_offset, fp, 16)] {
            if count > 0 {
                register_steps.push(binary(
                    NodeType::Assign,
                    offset.clone(),
                    binary(NodeType::Add, offset, number(count * stride)),
                ));
            }
        }
        register_steps.push(result);
        let mut choice = conditional(condition, sequence(register_steps), from_stack);
        choice.token = Some(token.clone());
        children.push(choice);
        sequence(children)
    }
}

//! Validate GNU transparent unions and normalize their fixed-parameter conversions.

use super::*;

impl AstBuilderImpl<'_> {
    fn transparent_mode(ty: &Type) -> Option<(u8, usize)> {
        if matches!(ty.lowered_for_codegen().unqualified(), Type::Bool) {
            return None;
        }
        let kind = if matches!(ty.unqualified(), Type::Ptr(_)) {
            0
        } else if let Some((bits, signed)) = ty.integer_info() {
            if !matches!(bits, 8 | 16 | 32 | 64) {
                return None;
            }
            if bits < 32 {
                if signed { 1 } else { 2 }
            } else {
                3
            }
        } else if ty.float_bits().is_some_and(|bits| matches!(bits, 32 | 64)) {
            4
        } else {
            return None;
        };
        Some((kind, ty.size_of()))
    }

    pub(super) fn apply_transparent_union(&self, ty: &Type, token: &Token) {
        let Type::Struct(record) = ty.unqualified() else {
            self.error_logger.print_error_position(
                token.pos,
                "transparent_union requires a complete union type",
            );
            return;
        };
        let definition = record.definition();
        let definition = definition.borrow();
        let members = &definition.members;
        let valid = record.definition().borrow().is_union
            && ty.is_complete_object()
            && members.first().is_some_and(|first| {
                let mode = Self::transparent_mode(&first.ty);
                mode.is_some()
                    && ty.size_of() == first.ty.size_of()
                    && ty.align_of() == first.ty.align_of()
                    && members.iter().all(|member| {
                        member.bit_field.is_none()
                            && !member.ty.contains_typedef_alignment()
                            && Self::transparent_mode(&member.ty) == mode
                    })
            });
        drop(definition);
        if !valid {
            self.error_logger.print_error_position(token.pos, "transparent unions require scalar members with matching calling representations and layout");
        }
        record.definition().borrow_mut().transparent_union = true;
    }

    pub(super) fn convert_transparent_argument(
        &mut self,
        expected: &Type,
        argument: &mut Node,
        token: &Token,
    ) {
        let Some(members) = expected.transparent_union_members() else {
            return;
        };
        let source = argument.resolve_type().unwrap().decayed();
        if expected.compatible(&source) {
            return;
        }
        let null = source.integer_info().is_some()
            && Self::integer_constant(argument)
            && self.eval(argument) == 0;
        let member = members
            .iter()
            .find(|member| {
                member.ty.compatible(&source)
                    || matches!(member.ty.unqualified(), Type::Ptr(_))
                        && (null || Self::can_convert(&member.ty, argument))
            })
            .unwrap_or_else(|| {
                self.error_logger.print_error_position(
                    argument.token.as_ref().map_or(token.pos, |token| token.pos),
                    "incompatible transparent union argument",
                );
                unreachable!()
            });
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".transparent_argument.{}.{}", token.pos, self.offset_size).into(),
                ..token.clone()
            },
            expected.unqualified().clone(),
        );
        let mut value = std::mem::take(argument);
        if null && matches!(member.ty.unqualified(), Type::Ptr(_)) {
            value = Node {
                nt: NodeType::Cast,
                token: Some(token.clone()),
                cty: Some(member.ty.clone()),
                lhs: Some(Box::new(value)),
                ..Node::default()
            };
        }
        let destination = Self::member_node(temporary.clone(), member, token);
        let initialization = self.local_expression_initializer(&destination, value, token);
        *argument = Node {
            nt: NodeType::Sequence,
            token: Some(token.clone()),
            children: vec![
                Self::zero_initializer(&temporary, token),
                initialization,
                temporary,
            ],
            ..Node::default()
        };
    }

    pub(super) fn lower_transparent_call(node: &mut Node) {
        if node.nt != NodeType::CallFunc {
            return;
        }
        for (argument, parameter) in node.args.iter_mut().rev().zip(&node.parameter_types) {
            if let Some(members) = parameter.transparent_union_members() {
                *argument = Self::member_node(
                    std::mem::take(argument),
                    &members[0],
                    node.token.as_ref().unwrap(),
                );
            }
        }
    }
}

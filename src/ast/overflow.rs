//! Defer checked-arithmetic temporaries until unevaluated expressions have been removed.

use std::rc::Rc;

use super::*;

fn number(value: usize, ty: Type, token: &Token) -> Node {
    Node {
        cty: Some(ty),
        ..Node::new_with_num(Some(token.clone()), value)
    }
}
fn cast(value: Node, ty: Type, token: &Token) -> Node {
    Node {
        nt: NodeType::Cast,
        token: Some(token.clone()),
        cty: Some(ty),
        lhs: Some(Box::new(value)),
        ..Node::default()
    }
}
fn binary(nt: NodeType, lhs: Node, rhs: Node, token: &Token) -> Node {
    Node::new_with_op(Some(token.clone()), nt, lhs, rhs)
}
fn temporary(ty: Type, offset: &mut usize, token: &Token) -> Node {
    *offset = (*offset + ty.size_of()).div_ceil(ty.align_of()) * ty.align_of();
    Node {
        nt: NodeType::LocalVar,
        token: Some(token.clone()),
        cty: Some(ty),
        offset: Some(*offset),
        ..Node::default()
    }
}
fn sign_high(value: Node, signed: bool, token: &Token) -> Node {
    let ty = Type::integer(64, true);
    if signed {
        binary(
            NodeType::BitRight,
            cast(value, ty.clone(), token),
            number(63, ty, token),
            token,
        )
    } else {
        number(0, ty, token)
    }
}

impl AstBuilderImpl<'_> {
    pub(super) fn overflow_builtin(&mut self, token: Token, operation: NodeType) -> Node {
        self.expect_reserved("(");
        let lhs = self.assign();
        self.expect_reserved(",");
        let rhs = self.assign();
        self.expect_reserved(",");
        let pointer = self.assign();
        self.expect_reserved(")");
        for operand in [&lhs, &rhs] {
            if operand
                .resolve_type()
                .and_then(|ty| ty.integer_info())
                .is_none()
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "overflow operands require ordinary integer types",
                );
            }
        }
        let Some(target) = pointer
            .dest_type()
            .filter(|ty| ty.integer_info().is_some() && *ty.unqualified() != Type::Bool)
        else {
            self.error_logger.print_error_position(
                token.pos,
                "overflow result requires a pointer to a nonboolean integer",
            );
            return Node::default();
        };
        if target.qualifiers().constant {
            self.error_logger
                .print_error_position(token.pos, "overflow result must be writable");
        }
        Node {
            nt: operation,
            token: Some(token),
            cty: Some(Type::Bool),
            args: vec![lhs, rhs, pointer],
            volatile_access: target.qualifiers().volatile,
            ..Node::default()
        }
    }
}

pub(super) fn lower(node: &mut Node, offset: &mut usize) {
    for child in [
        &mut node.lhs,
        &mut node.rhs,
        &mut node.cond,
        &mut node.then,
        &mut node.els,
        &mut node.ini,
        &mut node.upd,
    ]
    .into_iter()
    .flatten()
    {
        lower(child, offset);
    }
    for child in node.children.iter_mut().chain(&mut node.args) {
        lower(child, offset);
    }
    if !matches!(node.nt, NodeType::AddOverflow | NodeType::SubOverflow) {
        return;
    }
    let subtract = node.nt == NodeType::SubOverflow;
    let operation = if subtract {
        NodeType::Sub
    } else {
        NodeType::Add
    };
    let token = node.token.as_ref().unwrap().clone();
    let mut args = std::mem::take(&mut node.args).into_iter();
    let lhs = args.next().unwrap();
    let rhs = args.next().unwrap();
    let pointer = args.next().unwrap();
    let lhs_signed = lhs.resolve_type().unwrap().integer_info().unwrap().1;
    let rhs_signed = rhs.resolve_type().unwrap().integer_info().unwrap().1;
    let target = pointer.dest_type().unwrap().unqualified().clone();
    let target_signed = target.integer_info().unwrap().1;
    let unsigned = Type::integer(64, false);
    let signed = Type::integer(64, true);
    let left = temporary(unsigned.clone(), offset, &token);
    let right = temporary(unsigned.clone(), offset, &token);
    let result_pointer = temporary(Type::Ptr(Rc::new(target.clone())), offset, &token);
    let low = temporary(unsigned.clone(), offset, &token);
    let high = temporary(signed.clone(), offset, &token);
    let stored = temporary(target.clone(), offset, &token);
    let assign = |lhs, rhs| binary(NodeType::Assign, lhs, rhs, &token);
    let mut children = vec![
        assign(left.clone(), cast(lhs, unsigned.clone(), &token)),
        assign(right.clone(), cast(rhs, unsigned.clone(), &token)),
        assign(result_pointer.clone(), pointer),
        assign(
            low.clone(),
            binary(operation.clone(), left.clone(), right.clone(), &token),
        ),
    ];
    // A signed high word and unsigned low word represent the exact mathematical result.
    // Correct the sign extensions for the carry from addition or borrow from subtraction.
    let carry = cast(
        if subtract {
            binary(NodeType::Lt, left.clone(), right.clone(), &token)
        } else {
            binary(NodeType::Lt, low.clone(), left.clone(), &token)
        },
        signed,
        &token,
    );
    let signs = binary(
        operation.clone(),
        sign_high(left, lhs_signed, &token),
        sign_high(right, rhs_signed, &token),
        &token,
    );
    children.push(assign(
        high.clone(),
        binary(operation, signs, carry, &token),
    ));
    children.push(assign(stored.clone(), cast(low.clone(), target, &token)));
    let mut destination =
        Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, result_pointer);
    destination.volatile_access = node.volatile_access;
    children.push(assign(destination, stored.clone()));
    let low_changed = binary(
        NodeType::Ne,
        low,
        cast(stored.clone(), unsigned, &token),
        &token,
    );
    let high_changed = binary(
        NodeType::Ne,
        high,
        sign_high(stored, target_signed, &token),
        &token,
    );
    children.push(cast(
        binary(NodeType::BitOr, low_changed, high_changed, &token),
        Type::Bool,
        &token,
    ));
    *node = Node {
        nt: NodeType::Sequence,
        token: Some(token),
        children,
        ..Node::default()
    };
}

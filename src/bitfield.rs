use crate::{
    ctype::{BitField, Type},
    node::{Node, NodeType},
};
use std::rc::Rc;

fn cast(value: Node, ty: Type) -> Node {
    Node {
        nt: NodeType::Cast,
        token: value.token.clone(),
        cty: Some(ty),
        lhs: Some(Box::new(value)),
        ..Node::default()
    }
}
fn binary(operation: NodeType, lhs: Node, rhs: Node) -> Node {
    Node::new_with_op(lhs.token.clone(), operation, lhs, rhs)
}
fn number(value: u64) -> Node {
    Node {
        cty: Some(Type::integer(64, false)),
        ..Node::new_with_num(None, value as usize)
    }
}
fn temporary(ty: Type, offset: &mut usize, source: &Node) -> Node {
    *offset = (*offset + ty.size_of()).div_ceil(ty.align_of()) * ty.align_of();
    Node {
        nt: NodeType::LocalVar,
        token: source.token.clone(),
        cty: Some(ty),
        offset: Some(*offset),
        ..Node::default()
    }
}
fn sequence(children: Vec<Node>, source: &Node) -> Node {
    Node {
        nt: NodeType::Sequence,
        token: source.token.clone(),
        children,
        ..Node::default()
    }
}
fn mask(field: BitField) -> u64 {
    u64::MAX >> (64 - field.width)
}
fn normalize(value: Node, field: BitField, ty: &Type) -> Node {
    let value = binary(
        NodeType::BitAnd,
        cast(value, Type::integer(64, false)),
        number(mask(field)),
    );
    if ty.integer_info().unwrap().1 {
        let shift = number(u64::from(64 - field.width));
        let value = binary(NodeType::BitLeft, value, shift.clone());
        cast(
            binary(
                NodeType::BitRight,
                cast(value, Type::integer(64, true)),
                shift,
            ),
            ty.clone(),
        )
    } else {
        cast(value, ty.clone())
    }
}
fn packed_chunks(field: BitField) -> impl Iterator<Item = (usize, usize, usize, usize)> {
    let start = usize::from(field.shift);
    let end = start + usize::from(field.width);
    (0..end.div_ceil(8)).map(move |byte| {
        let low = start.saturating_sub(byte * 8);
        let width = (end - byte * 8).min(8) - low;
        (byte, low, width, byte * 8 + low - start)
    })
}
fn packed_byte(pointer: &Node, index: usize, volatile: bool) -> Node {
    let address = binary(NodeType::Add, pointer.clone(), number(index as u64));
    Node {
        volatile_access: volatile,
        ..Node::new_with_op_and_lhs(pointer.token.clone(), NodeType::Deref, address)
    }
}
fn packed_read(mut node: Node, offset: &mut usize) -> Node {
    let field = node.bit_field.take().unwrap();
    let ty = node.object_type().unwrap();
    let pointer_type = Type::Ptr(Rc::new(Type::integer(8, false)));
    let pointer = temporary(pointer_type.clone(), offset, &node);
    let save_pointer = binary(
        NodeType::Assign,
        pointer.clone(),
        cast(*node.lhs.take().unwrap(), pointer_type),
    );
    let mut value = number(0);
    for (byte, low, width, shift) in packed_chunks(field) {
        let part = binary(
            NodeType::BitAnd,
            binary(
                NodeType::BitRight,
                cast(
                    packed_byte(&pointer, byte, node.volatile_access),
                    Type::integer(64, false),
                ),
                number(low as u64),
            ),
            number((1u64 << width) - 1),
        );
        value = binary(
            NodeType::BitOr,
            value,
            binary(NodeType::BitLeft, part, number(shift as u64)),
        );
    }
    sequence(vec![save_pointer, normalize(value, field, &ty)], &node)
}
fn packed_write(mut target: Node, value: Node, offset: &mut usize) -> Node {
    let source = target.clone();
    let field = target.bit_field.take().unwrap();
    let ty = target.object_type().unwrap();
    let pointer_type = Type::Ptr(Rc::new(Type::integer(8, false)));
    let pointer = temporary(pointer_type.clone(), offset, &source);
    let save_pointer = binary(
        NodeType::Assign,
        pointer.clone(),
        cast(*target.lhs.take().unwrap(), pointer_type),
    );
    let value = if ty == Type::Bool {
        cast(value, Type::Bool)
    } else {
        value
    };
    let result = temporary(ty.clone(), offset, &source);
    let mut children = vec![
        save_pointer,
        binary(
            NodeType::Assign,
            result.clone(),
            normalize(value, field, &ty),
        ),
    ];
    for (byte, low, width, shift) in packed_chunks(field) {
        let byte_mask = ((1u64 << width) - 1) << low;
        let target = packed_byte(&pointer, byte, source.volatile_access);
        let inserted = binary(
            NodeType::BitAnd,
            binary(
                NodeType::BitLeft,
                binary(
                    NodeType::BitRight,
                    cast(result.clone(), Type::integer(64, false)),
                    number(shift as u64),
                ),
                number(low as u64),
            ),
            number(byte_mask),
        );
        let value = if byte_mask == 255 {
            inserted
        } else {
            binary(
                NodeType::BitOr,
                binary(
                    NodeType::BitAnd,
                    cast(target.clone(), Type::integer(64, false)),
                    number(!byte_mask),
                ),
                inserted,
            )
        };
        children.push(binary(
            NodeType::Assign,
            target,
            cast(value, Type::integer(8, false)),
        ));
    }
    children.push(result);
    sequence(children, &source)
}
fn read(mut node: Node, offset: &mut usize) -> Node {
    if node.bit_field.unwrap().bytewise {
        return packed_read(node, offset);
    }
    let field = node.bit_field.take().unwrap();
    let ty = node.object_type().unwrap();
    let value = binary(
        NodeType::BitRight,
        cast(node, Type::integer(64, false)),
        number(u64::from(field.shift)),
    );
    normalize(value, field, &ty)
}
fn write(mut target: Node, value: Node, offset: &mut usize) -> Node {
    if target.bit_field.unwrap().bytewise {
        return packed_write(target, value, offset);
    }
    let source = target.clone();
    let field = target.bit_field.take().unwrap();
    let ty = target.object_type().unwrap();
    let storage = Type::integer(ty.integer_info().unwrap().0, false);
    let pointer = temporary(Type::Ptr(Rc::new(storage.clone())), offset, &source);
    let save = binary(
        NodeType::Assign,
        pointer.clone(),
        *target.lhs.take().unwrap(),
    );
    target.lhs = Some(Box::new(pointer));
    let value = if ty == Type::Bool {
        cast(value, Type::Bool)
    } else {
        value
    };
    let value = normalize(value, field, &ty);
    let result = temporary(ty, offset, &source);
    let save_value = binary(NodeType::Assign, result.clone(), value);
    let retained = binary(
        NodeType::BitAnd,
        cast(target.clone(), Type::integer(64, false)),
        number(!(mask(field) << field.shift)),
    );
    let inserted = binary(
        NodeType::BitLeft,
        binary(
            NodeType::BitAnd,
            cast(result.clone(), Type::integer(64, false)),
            number(mask(field)),
        ),
        number(u64::from(field.shift)),
    );
    let store = binary(
        NodeType::Assign,
        target,
        cast(binary(NodeType::BitOr, retained, inserted), storage),
    );
    sequence(vec![save, save_value, store, result], &source)
}

/// Lower bit-field access to scalar expressions shared by all output backends.
pub fn lower(node: &mut Node, offset: &mut usize) {
    lower_with_type_changes(node, offset);
}

// Bit-field reads can change promotions, so refresh types along rewritten paths.
fn lower_with_type_changes(node: &mut Node, offset: &mut usize) -> bool {
    if node.nt == NodeType::Addr && node.lhs.as_ref().is_some_and(|lhs| lhs.bit_field.is_some()) {
        // Only internal compound-assignment address captures reach this path.
        *node = *node.lhs.take().unwrap().lhs.unwrap();
        lower_with_type_changes(node, offset);
        return true;
    }
    if node.nt == NodeType::Assign && node.lhs.as_ref().is_some_and(|lhs| lhs.bit_field.is_some()) {
        let mut target = *node.lhs.take().unwrap();
        lower_with_type_changes(target.lhs.as_mut().unwrap(), offset);
        let mut value = *node.rhs.take().unwrap();
        lower_with_type_changes(&mut value, offset);
        *node = write(target, value, offset);
        return true;
    }
    if matches!(node.nt, NodeType::SuffixIncr | NodeType::SuffixDecr)
        && node.lhs.as_ref().is_some_and(|lhs| lhs.bit_field.is_some())
    {
        let source = node.clone();
        let mut target = *node.lhs.take().unwrap();
        lower_with_type_changes(target.lhs.as_mut().unwrap(), offset);
        let pointer = temporary(
            target.lhs.as_ref().unwrap().resolve_type().unwrap(),
            offset,
            &source,
        );
        let save_pointer = binary(
            NodeType::Assign,
            pointer.clone(),
            *target.lhs.take().unwrap(),
        );
        target.lhs = Some(Box::new(pointer));
        let previous = temporary(target.resolve_type().unwrap(), offset, &source);
        let save_value = binary(
            NodeType::Assign,
            previous.clone(),
            read(target.clone(), offset),
        );
        let operation = if source.nt == NodeType::SuffixIncr {
            NodeType::Add
        } else {
            NodeType::Sub
        };
        let update = write(
            target,
            binary(operation, previous.clone(), number(1)),
            offset,
        );
        *node = sequence(vec![save_pointer, save_value, update, previous], &source);
        return true;
    }
    let mut changed = false;
    for child in node.child_nodes_mut() {
        changed |= lower_with_type_changes(child, offset);
    }
    if node.bit_field.is_some() {
        *node = read(std::mem::take(node), offset);
        return true;
    }
    if changed {
        node.refresh_arithmetic_type();
    }
    changed
}

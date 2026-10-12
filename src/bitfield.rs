use crate::{
    ctype::{BitField, Type},
    node::{Node, NodeType},
};

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
        cty: Some(ty),
        offset: Some(*offset),
        token: source.token.clone(),
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
fn read(mut node: Node) -> Node {
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
    let source = target.clone();
    let field = target.bit_field.take().unwrap();
    let ty = target.object_type().unwrap();
    let storage = Type::integer(ty.integer_info().unwrap().0, false);
    let pointer = temporary(Type::Ptr(Box::new(storage.clone())), offset, &source);
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
    let result = temporary(ty.clone(), offset, &source);
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
    if node.nt == NodeType::Addr && node.lhs.as_ref().is_some_and(|lhs| lhs.bit_field.is_some()) {
        // Only internal compound-assignment address captures reach this path.
        *node = *node.lhs.take().unwrap().lhs.unwrap();
        lower(node, offset);
        return;
    }
    if node.nt == NodeType::Assign && node.lhs.as_ref().is_some_and(|lhs| lhs.bit_field.is_some()) {
        let mut target = *node.lhs.take().unwrap();
        lower(target.lhs.as_mut().unwrap(), offset);
        let mut value = *node.rhs.take().unwrap();
        lower(&mut value, offset);
        *node = write(target, value, offset);
        return;
    }
    if matches!(node.nt, NodeType::SuffixIncr | NodeType::SuffixDecr)
        && node.lhs.as_ref().is_some_and(|lhs| lhs.bit_field.is_some())
    {
        let source = node.clone();
        let mut target = *node.lhs.take().unwrap();
        lower(target.lhs.as_mut().unwrap(), offset);
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
        let save_value = binary(NodeType::Assign, previous.clone(), read(target.clone()));
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
        return;
    }
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
    if node.bit_field.is_some() {
        *node = read(std::mem::take(node));
    }
}

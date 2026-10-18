//! Normalize source types and insert value conversions for code generation.

use super::AstBuilderImpl;
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};
use std::borrow::Cow;

impl AstBuilderImpl<'_> {
    pub(super) fn lower_source_types(node: &mut Node) {
        Self::lower_transparent_call(node);
        node.volatile_access |= node.is_volatile_object();
        if let Some(ty) = &mut node.cty {
            ty.lower_for_codegen();
        }
        for ty in &mut node.parameter_types {
            ty.lower_parameter_for_codegen();
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
            Self::lower_source_types(child);
        }
        for child in node.children.iter_mut().chain(&mut node.args) {
            Self::lower_source_types(child);
        }
    }

    pub(super) fn cast_integer(node: &mut Node, target: &Type) {
        if target.is_arithmetic() && node.resolve_type_view().as_deref() != Some(target) {
            let operand = std::mem::take(node);
            *node = Node {
                nt: NodeType::Cast,
                cty: Some(target.clone()),
                token: operand.token.clone(),
                lhs: Some(Box::new(operand)),
                ..Node::default()
            };
        }
    }

    pub(super) fn insert_integer_conversions(node: &mut Node) {
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
            Self::insert_integer_conversions(child);
        }
        for child in node.children.iter_mut().chain(&mut node.args) {
            Self::insert_integer_conversions(child);
        }
        if matches!(
            node.nt,
            NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le
        ) {
            let lhs = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
            let rhs = node.rhs.as_ref().unwrap().resolve_type_view().unwrap();
            if lhs.float_bits().is_some() || rhs.float_bits().is_some() {
                let common = Type::arithmetic_type(&lhs, &rhs);
                Self::cast_integer(node.lhs.as_mut().unwrap(), &common);
                Self::cast_integer(node.rhs.as_mut().unwrap(), &common);
            }
        }
        if matches!(
            node.nt,
            NodeType::If
                | NodeType::Conditional
                | NodeType::While
                | NodeType::DoWhile
                | NodeType::For
        ) {
            if let Some(condition) = &mut node.cond {
                if condition
                    .resolve_type_view()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    Self::cast_integer(condition, &Type::Bool);
                }
            }
        }
        if matches!(node.nt, NodeType::LogicalAnd | NodeType::LogicalOr) {
            for operand in [&mut node.lhs, &mut node.rhs].into_iter().flatten() {
                if operand
                    .resolve_type_view()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    Self::cast_integer(operand, &Type::Bool);
                }
            }
        }
        if node.nt == NodeType::Conditional
            && node
                .resolve_type_view()
                .is_some_and(|ty| ty.float_bits().is_some())
        {
            let common = node.resolve_type_view().unwrap().into_owned();
            Self::cast_integer(node.then.as_mut().unwrap(), &common);
            Self::cast_integer(node.els.as_mut().unwrap(), &common);
        }
        match node.nt {
            NodeType::Assign => {
                let target = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
                Self::cast_integer(node.rhs.as_mut().unwrap(), &target);
            }
            NodeType::CallFunc => {
                for (index, arg) in node.args.iter_mut().rev().enumerate() {
                    let target = node
                        .parameter_types
                        .get(index)
                        .map(Cow::Borrowed)
                        .unwrap_or_else(|| {
                            let ty = arg.resolve_type_view().unwrap().promoted();
                            Cow::Owned(if ty == Type::Float(32) {
                                Type::Float(64)
                            } else {
                                ty
                            })
                        });
                    Self::cast_integer(arg, &target);
                }
            }
            _ => {}
        }
    }
}

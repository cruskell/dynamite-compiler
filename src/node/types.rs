use super::{Node, NodeType};
use crate::ctype::Type;
use std::{borrow::Cow, rc::Rc};

impl Node {
    /// Recompute an arithmetic result after changing its operands or their types.
    /// Refresh children first when changing an existing expression tree.
    pub fn refresh_arithmetic_type(&mut self) {
        // Reuse cty so queries need neither a subtree walk nor a separate cache.
        if matches!(
            self.nt,
            NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Mod
                | NodeType::BitLeft
                | NodeType::BitRight
                | NodeType::BitAnd
                | NodeType::BitXor
                | NodeType::BitOr
                | NodeType::BitNot
        ) {
            self.cty = self
                .infer_arithmetic_type_view()
                .filter(|ty| ty.is_arithmetic())
                .map(Cow::into_owned);
        }
    }

    fn infer_arithmetic_type_view(&self) -> Option<Cow<'_, Type>> {
        let lhs = self.lhs.as_ref()?.resolve_type_view()?;
        match self.nt {
            NodeType::Add | NodeType::Sub => {
                let rhs = self.rhs.as_ref()?;
                if self.nt == NodeType::Sub && rhs.dest_type_view().is_some() {
                    Some(Cow::Owned(Type::I32))
                } else if lhs.dest_type_ref().is_some() {
                    Some(lhs)
                } else {
                    Some(Cow::Owned(Type::arithmetic_type(
                        &lhs,
                        rhs.resolve_type_view()?.as_ref(),
                    )))
                }
            }
            NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitAnd
            | NodeType::BitXor
            | NodeType::BitOr => Some(Cow::Owned(Type::arithmetic_type(
                &lhs,
                self.rhs.as_ref()?.resolve_type_view()?.as_ref(),
            ))),
            NodeType::BitLeft | NodeType::BitRight | NodeType::BitNot => {
                Some(Cow::Owned(lhs.promoted()))
            }
            _ => None,
        }
    }

    pub fn object_type(&self) -> Option<Type> {
        self.object_type_view().map(Cow::into_owned)
    }

    pub(crate) fn object_type_view(&self) -> Option<Cow<'_, Type>> {
        match self.nt {
            NodeType::LocalVar | NodeType::GlobalVar => self.cty.as_ref().map(Cow::Borrowed),
            NodeType::Deref => self.lhs.as_ref()?.dest_type_view(),
            NodeType::SuffixIncr | NodeType::SuffixDecr => self.lhs.as_ref()?.object_type_view(),
            _ => self.resolve_type_view(),
        }
    }

    pub(crate) fn is_volatile_object(&self) -> bool {
        match self.nt {
            NodeType::LocalVar | NodeType::GlobalVar => {
                self.cty.as_ref().is_some_and(|ty| ty.qualifiers().volatile)
            }
            NodeType::Deref => self
                .object_type_view()
                .is_some_and(|ty| ty.qualifiers().volatile),
            NodeType::SuffixIncr | NodeType::SuffixDecr => self
                .lhs
                .as_ref()
                .is_some_and(|node| node.is_volatile_object()),
            // Value expressions discard outer object qualifiers in resolve_type.
            _ => false,
        }
    }

    pub fn resolve_type(&self) -> Option<Type> {
        self.resolve_type_view().map(Cow::into_owned)
    }

    /// Borrow stored expression types; own types constructed by value conversion.
    pub(crate) fn resolve_type_view(&self) -> Option<Cow<'_, Type>> {
        if self.bit_field.is_some_and(|field| field.width < 32) {
            return Some(Cow::Owned(Type::I32));
        }
        self.resolve_qualified_type_view().map(|ty| {
            if let Type::Qualified(child, _) = ty.as_ref() {
                if let Type::Arr(element, size) = child.unqualified() {
                    return Cow::Owned(Type::Arr(
                        Rc::new(Type::Qualified(Rc::clone(element), ty.qualifiers())),
                        *size,
                    ));
                }
                if let Type::Vla(element, offset) = child.unqualified() {
                    return Cow::Owned(Type::Vla(
                        Rc::new(Type::Qualified(Rc::clone(element), ty.qualifiers())),
                        *offset,
                    ));
                }
            }
            Type::unqualified_view(ty)
        })
    }

    pub(crate) fn resolve_qualified_type(&self) -> Option<Type> {
        self.resolve_qualified_type_view().map(Cow::into_owned)
    }

    pub(crate) fn resolve_qualified_type_view(&self) -> Option<Cow<'_, Type>> {
        match self.nt {
            NodeType::Num => Some(
                self.cty
                    .as_ref()
                    .map_or(Cow::Owned(Type::I32), Cow::Borrowed),
            ),
            NodeType::LocalVar
            | NodeType::GlobalVar
            | NodeType::CallFunc
            | NodeType::AddOverflow
            | NodeType::SubOverflow
            | NodeType::BitScan { .. }
            | NodeType::VaStart
            | NodeType::VaEnd
            | NodeType::StackAlloc
            | NodeType::Prefetch(..)
            | NodeType::Atomic(_)
            | NodeType::ZeroFill { .. }
            | NodeType::CopyBytes { .. } => self.cty.as_ref().map(Cow::Borrowed),
            NodeType::Addr => {
                if let Some(ty) = self.lhs.as_ref().unwrap().object_type_view() {
                    Some(Cow::Owned(Type::Ptr(Rc::new(ty.into_owned()))))
                } else {
                    self.cty.as_ref().map(Cow::Borrowed)
                }
            }
            NodeType::Deref => self.lhs.as_ref().unwrap().dest_type_view(),
            NodeType::Cast | NodeType::FloatAbs | NodeType::InlineAsm(_, _) => {
                self.cty.as_ref().map(Cow::Borrowed)
            }
            NodeType::Add
            | NodeType::Sub
            | NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitLeft
            | NodeType::BitRight
            | NodeType::BitAnd
            | NodeType::BitXor
            | NodeType::BitOr
            | NodeType::BitNot => self
                .cty
                .as_ref()
                .map(Cow::Borrowed)
                .or_else(|| self.infer_arithmetic_type_view()),
            NodeType::Eq
            | NodeType::Ne
            | NodeType::Lt
            | NodeType::Le
            | NodeType::LogicalAnd
            | NodeType::LogicalOr => Some(Cow::Owned(Type::I32)),
            NodeType::Conditional => {
                let then = self
                    .then
                    .as_ref()
                    .unwrap()
                    .resolve_type_view()
                    .map(Type::decay_view);
                let otherwise = self
                    .els
                    .as_ref()
                    .unwrap()
                    .resolve_type_view()
                    .map(Type::decay_view);
                if then.as_ref().is_some_and(|ty| ty.is_arithmetic())
                    && otherwise.as_ref().is_some_and(|ty| ty.is_arithmetic())
                {
                    Some(Cow::Owned(Type::arithmetic_type(
                        then.as_ref().unwrap(),
                        otherwise.as_ref().unwrap(),
                    )))
                } else if then.as_ref().is_some_and(|ty| matches!(**ty, Type::Ptr(_))) {
                    then
                } else if otherwise
                    .as_ref()
                    .is_some_and(|ty| matches!(**ty, Type::Ptr(_)))
                {
                    otherwise
                } else {
                    then
                }
            }
            NodeType::Sequence => self.children.last().and_then(Node::resolve_type_view),
            _ => {
                if let Some(node) = self.lhs.as_ref() {
                    if let Some(ty) = node.resolve_type_view() {
                        return Some(ty);
                    }
                }
                if let Some(node) = self.rhs.as_ref() {
                    if let Some(ty) = node.resolve_type_view() {
                        return Some(ty);
                    }
                }
                if let Some(node) = self.then.as_ref() {
                    if let Some(ty) = node.resolve_type_view() {
                        return Some(ty);
                    }
                }
                None
            }
        }
    }

    pub fn dest_type(&self) -> Option<Type> {
        self.dest_type_view().map(Cow::into_owned)
    }

    pub(crate) fn dest_type_view(&self) -> Option<Cow<'_, Type>> {
        match Type::unqualified_view(self.resolve_type_view()?) {
            Cow::Borrowed(ty) => ty.dest_type_ref().map(Cow::Borrowed),
            Cow::Owned(Type::Ptr(child) | Type::Arr(child, _) | Type::Vla(child, _)) => {
                Some(Cow::Owned(Rc::unwrap_or_clone(child)))
            }
            Cow::Owned(_) => None,
        }
    }
}

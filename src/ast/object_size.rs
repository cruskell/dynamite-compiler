use super::*;
use std::rc::Rc;

#[derive(Clone, Copy)]
struct ObjectRegion {
    whole_size: usize,
    whole_offset: i128,
    subobject_size: usize,
    subobject_offset: i128,
}

impl ObjectRegion {
    fn remaining(self, subobject: bool) -> Option<usize> {
        let (size, offset) = if subobject {
            (self.subobject_size, self.subobject_offset)
        } else {
            (self.whole_size, self.whole_offset)
        };
        (offset >= 0).then(|| (size as i128 - offset).max(0) as usize)
    }
}

impl AstBuilderImpl<'_> {
    fn object_region_lvalue(&mut self, node: &Node) -> Option<ObjectRegion> {
        let ty = node.object_type()?;
        match node.nt {
            NodeType::LocalVar | NodeType::GlobalVar
                if ty.is_complete_object()
                    && !ty.is_variably_modified()
                    && !matches!(ty.unqualified(), Type::Func(..) | Type::VariadicFunc(..)) =>
            {
                Some(ObjectRegion {
                    whole_size: ty.size_of(),
                    whole_offset: 0,
                    subobject_size: ty.size_of(),
                    subobject_offset: 0,
                })
            }
            NodeType::Deref => {
                let mut region = self.object_region_pointer(node.lhs.as_deref()?)?;
                if node.member_access {
                    if !ty.is_complete_object()
                        || ty.is_variably_modified()
                        || region.remaining(false)? < ty.size_of()
                    {
                        return None;
                    }
                    region.subobject_size = ty.size_of();
                    region.subobject_offset = 0;
                }
                Some(region)
            }
            _ => None,
        }
    }

    fn object_region_pointer(&mut self, node: &Node) -> Option<ObjectRegion> {
        let ty = node.resolve_type()?;
        match node.nt {
            NodeType::Addr => self.object_region_lvalue(node.lhs.as_deref()?),
            NodeType::Cast
                if node.dest_type().is_some() && node.lhs.as_deref()?.dest_type().is_some() =>
            {
                self.object_region_pointer(node.lhs.as_deref()?)
            }
            NodeType::Add | NodeType::Sub => {
                let lhs = node.lhs.as_deref()?;
                let rhs = node.rhs.as_deref()?;
                if !Self::integer_constant(rhs) {
                    return None;
                }
                let value = self.eval(rhs);
                if value < 0
                    && rhs
                        .resolve_type()?
                        .integer_info()
                        .is_some_and(|(_, signed)| !signed)
                {
                    return None;
                }
                let stride = lhs.dest_type()?.size_of();
                if stride == 0 {
                    return None;
                }
                let mut offset = i128::from(value).checked_mul(stride as i128)?;
                if node.nt == NodeType::Sub {
                    offset = offset.checked_neg()?;
                }
                let mut region = self.object_region_pointer(lhs)?;
                region.whole_offset = region.whole_offset.checked_add(offset)?;
                region.subobject_offset = region.subobject_offset.checked_add(offset)?;
                Some(region)
            }
            _ if matches!(ty.unqualified(), Type::Arr(..)) => {
                let mut region = self.object_region_lvalue(node)?;
                // Decaying an inner array establishes that row's subobject bounds.
                if node.nt == NodeType::Deref && !node.member_access {
                    region.subobject_size = ty.size_of();
                    region.subobject_offset = 0;
                }
                Some(region)
            }
            _ => None,
        }
    }

    fn object_size_value(&mut self, node: &Node, mode: i64) -> Option<usize> {
        if node.nt == NodeType::Conditional {
            let condition = node.cond.as_deref()?;
            if Self::integer_constant(condition) {
                let branch = if self.eval(condition) != 0 {
                    node.then.as_deref()?
                } else {
                    node.els.as_deref()?
                };
                return self.object_size_value(branch, mode);
            }
            let then = self.object_size_value(node.then.as_deref()?, mode)?;
            let els = self.object_size_value(node.els.as_deref()?, mode)?;
            return Some(if mode & 2 == 0 {
                then.max(els)
            } else {
                then.min(els)
            });
        }
        if node.nt == NodeType::Cast {
            let child = node.lhs.as_deref()?;
            if node.dest_type().is_none() || child.dest_type().is_none() {
                return None;
            }
            return self.object_size_value(child, mode);
        }
        self.object_region_pointer(node)?.remaining(mode & 1 != 0)
    }

    pub(super) fn object_size_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let bounds_start = self.pending_bounds.len();
        let pointer = self.assign();
        self.expect_reserved(",");
        let mode = self.assign();
        self.expect_reserved(")");
        let has_bounds = self.pending_bounds.len() != bounds_start;
        self.pending_bounds.truncate(bounds_start);
        let const_void = Type::Qualified(
            Rc::new(Type::Void),
            Qualifiers {
                constant: true,
                volatile: true,
                ..Qualifiers::default()
            },
        );
        self.expect_conversion(&Type::Ptr(Rc::new(const_void)), &pointer);
        if !Self::integer_constant(&mode) {
            self.error_logger.print_error_position(
                token.pos,
                "object-size mode must be an integer constant from 0 to 3",
            );
        }
        let mode = self.eval(&mode);
        if !(0..=3).contains(&mode) {
            self.error_logger.print_error_position(
                token.pos,
                "object-size mode must be an integer constant from 0 to 3",
            );
        }
        let size = if has_bounds || pointer.has_side_effects() {
            None
        } else {
            self.object_size_value(&pointer, mode)
        };
        Node {
            cty: Some(Type::integer(64, false)),
            ..Node::new_with_num(
                Some(token),
                size.unwrap_or(if mode & 2 == 0 { usize::MAX } else { 0 }),
            )
        }
    }
}

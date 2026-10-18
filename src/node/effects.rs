use super::{Node, NodeType};
use crate::ctype::Type;

impl Node {
    /// Whether evaluating this expression can call, write, or read a volatile value.
    pub(crate) fn has_side_effects(&self) -> bool {
        Self::has_effects(self, true)
    }

    fn has_effects(node: &Self, value: bool) -> bool {
        if matches!(
            node.nt,
            NodeType::Assign
                | NodeType::SuffixIncr
                | NodeType::SuffixDecr
                | NodeType::CallFunc
                | NodeType::AddOverflow
                | NodeType::SubOverflow
                | NodeType::VaStart
                | NodeType::VaEnd
                | NodeType::Prefetch(..)
                | NodeType::Atomic(_)
                | NodeType::InlineAsm(_, _)
                | NodeType::ZeroFill { .. }
                | NodeType::CopyBytes { .. }
        ) || (value
            && (node.volatile_access
                || node.object_type().is_some_and(|ty| {
                    ty.qualifiers().volatile
                        && !matches!(ty.unqualified(), Type::Arr(..) | Type::Vla(..))
                })))
        {
            return true;
        }
        if node.nt == NodeType::Addr {
            return node
                .lhs
                .as_deref()
                .is_some_and(|child| Self::has_effects(child, false));
        }
        node.child_nodes()
            .any(|child| Self::has_effects(child, true))
    }
}

//! Parse advisory data-prefetch operations and their constant hints.

use std::rc::Rc;

use super::*;

impl AstBuilderImpl<'_> {
    pub(super) fn prefetch_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let pointer = self.assign();
        let pointer_type = Type::Ptr(Rc::new(Type::Qualified(
            Rc::new(Type::Void),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        self.expect_conversion(&pointer_type, &pointer);
        let pointer = Node {
            nt: NodeType::Cast,
            token: pointer.token.clone(),
            cty: Some(pointer_type),
            lhs: Some(Box::new(pointer)),
            ..Node::default()
        };
        let mut hints = [0u8, 3];
        for (index, maximum) in [(0, 1), (1, 3)] {
            if self.attempt_reserved(",").is_none() {
                break;
            }
            let hint = self.assign();
            if hint
                .resolve_type()
                .is_none_or(|ty| ty.integer_info().is_none())
                || !Self::integer_constant(&hint)
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "prefetch hints require integer constant expressions",
                );
            }
            let value = self.eval(&hint);
            if !(0..=maximum).contains(&value) {
                self.error_logger.print_error_position(
                    token.pos,
                    "prefetch write intent must be 0 or 1 and locality must be 0 through 3",
                );
            }
            hints[index] = value as u8;
        }
        self.expect_reserved(")");
        Node {
            nt: NodeType::Prefetch(hints[0] != 0, hints[1]),
            token: Some(token),
            cty: Some(Type::Void),
            lhs: Some(Box::new(pointer)),
            ..Node::default()
        }
    }
}

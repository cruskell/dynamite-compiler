//! Parse unevaluated GNU type compatibility queries.

use super::*;

impl AstBuilderImpl<'_> {
    pub(super) fn types_compatible_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let left = self.type_query_operand(&token);
        self.expect_reserved(",");
        let right = self.type_query_operand(&token);
        self.expect_reserved(")");
        Node::new_with_num(Some(token), usize::from(left.compatible(&right)))
    }

    fn type_query_operand(&mut self, token: &Token) -> Type {
        let bounds_start = self.pending_bounds.len();
        let base = self.expect_type();
        let (name, ty, _, _, _) = self.declarator().resolve(base);
        if name.is_some() {
            self.error_logger.print_error_position(
                token.pos,
                "type compatibility operands require type names without identifiers",
            );
        }
        self.validate_type_layout(&ty, token);
        // VLA and typeof bounds are type information here, never runtime evaluations.
        self.pending_bounds.truncate(bounds_start);
        ty.unqualified_for_type_query()
    }
}

//! Parse GNU atomic operations independently of target instruction selection.

use super::*;

impl AstBuilderImpl<'_> {
    pub(super) fn atomic_builtin(&mut self, token: Token, operation: AtomicOperation) -> Node {
        self.expect_reserved("(");
        let pointer = self.assign();
        let object = pointer.dest_type().unwrap_or_else(|| {
            self.error_logger
                .print_error_position(token.pos, "atomic operation requires an object pointer");
            unreachable!()
        });
        let ty = object.unqualified().clone();
        if !(ty.integer_info().is_some() || matches!(ty, Type::Ptr(_)))
            || ![1, 2, 4, 8].contains(&ty.size_of())
        {
            self.error_logger.print_error_position(
                token.pos,
                "atomic operation requires a 1-, 2-, 4- or 8-byte integer or pointer object",
            );
        }
        let mut args = vec![pointer];
        if operation == AtomicOperation::Store {
            if object.qualifiers().constant {
                self.error_logger
                    .print_error_position(token.pos, "atomic store requires a writable object");
            }
            self.expect_reserved(",");
            let mut value = self.assign();
            self.expect_conversion(&ty, &value);
            Self::cast_integer(&mut value, &ty);
            args.push(value);
        }
        self.expect_reserved(",");
        let mut order = self.assign();
        self.expect_conversion(&Type::I32, &order);
        Self::cast_integer(&mut order, &Type::I32);
        self.expect_reserved(")");
        if Self::integer_constant(&order) {
            let value = self.eval(&order);
            let valid = match operation {
                AtomicOperation::Load => [0, 1, 2, 5].contains(&value),
                AtomicOperation::Store => [0, 3, 5].contains(&value),
            };
            if !valid {
                self.error_logger
                    .print_error_position(token.pos, "invalid memory order for atomic operation");
            }
        }
        args.push(order);
        self.postfix(Node {
            nt: NodeType::Atomic(operation),
            cty: Some(if operation == AtomicOperation::Store {
                Type::Void
            } else {
                ty
            }),
            volatile_access: object.qualifiers().volatile,
            token: Some(token),
            args,
            ..Node::default()
        })
    }
}

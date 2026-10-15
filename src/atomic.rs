//! Parse GNU atomic operations independently of target instruction selection.

use super::*;
use crate::node::AtomicRmwOperation;

impl AstBuilderImpl<'_> {
    pub(super) fn atomic_operation(name: &str) -> Option<AtomicOperation> {
        Some(match name {
            "__atomic_load_n" => AtomicOperation::Load,
            "__atomic_store_n" => AtomicOperation::Store,
            "__atomic_exchange_n" => AtomicOperation::Update(AtomicRmwOperation::Exchange, false),
            "__atomic_fetch_add" => AtomicOperation::Update(AtomicRmwOperation::Add, false),
            "__atomic_add_fetch" => AtomicOperation::Update(AtomicRmwOperation::Add, true),
            "__atomic_fetch_sub" => AtomicOperation::Update(AtomicRmwOperation::Sub, false),
            "__atomic_sub_fetch" => AtomicOperation::Update(AtomicRmwOperation::Sub, true),
            "__atomic_fetch_and" => AtomicOperation::Update(AtomicRmwOperation::And, false),
            "__atomic_and_fetch" => AtomicOperation::Update(AtomicRmwOperation::And, true),
            "__atomic_fetch_or" => AtomicOperation::Update(AtomicRmwOperation::Or, false),
            "__atomic_or_fetch" => AtomicOperation::Update(AtomicRmwOperation::Or, true),
            "__atomic_fetch_xor" => AtomicOperation::Update(AtomicRmwOperation::Xor, false),
            "__atomic_xor_fetch" => AtomicOperation::Update(AtomicRmwOperation::Xor, true),
            "__atomic_fetch_nand" => AtomicOperation::Update(AtomicRmwOperation::Nand, false),
            "__atomic_nand_fetch" => AtomicOperation::Update(AtomicRmwOperation::Nand, true),
            _ => return None,
        })
    }

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
        if operation != AtomicOperation::Load {
            if object.qualifiers().constant {
                self.error_logger
                    .print_error_position(token.pos, "atomic update requires a writable object");
            }
            self.expect_reserved(",");
            let mut value = self.assign();
            let value_type = if matches!(operation, AtomicOperation::Update(op, _) if op != AtomicRmwOperation::Exchange)
            {
                if ty == Type::Bool {
                    self.error_logger.print_error_position(
                        token.pos,
                        "atomic arithmetic does not support boolean objects",
                    );
                }
                if matches!(ty, Type::Ptr(_)) {
                    Type::integer(64, false)
                } else {
                    ty.clone()
                }
            } else {
                ty.clone()
            };
            self.expect_conversion(&value_type, &value);
            Self::cast_integer(&mut value, &value_type);
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
                AtomicOperation::Update(..) => (0..=5).contains(&value),
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

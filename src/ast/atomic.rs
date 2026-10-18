//! Parse GNU atomic operations independently of target instruction selection.

use super::*;
use crate::node::AtomicRmwOperation;

impl AstBuilderImpl<'_> {
    pub(super) fn atomic_operation(name: &str) -> Option<AtomicOperation> {
        Some(match name {
            "__atomic_compare_exchange_n" => AtomicOperation::CompareExchange(false),
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
        if matches!(operation, AtomicOperation::CompareExchange(_)) {
            return self.atomic_compare_exchange_builtin(token);
        }
        self.expect_reserved("(");
        let pointer = self.assign();
        let object = self.atomic_object(&pointer, &token);
        let ty = object.unqualified().clone();
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
                    &Type::integer(64, false)
                } else {
                    &ty
                }
            } else {
                &ty
            };
            value = self.atomic_value(value, value_type);
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
                AtomicOperation::CompareExchange(_) => unreachable!(),
            };
            if !valid {
                self.error_logger
                    .print_error_position(token.pos, "invalid memory order for atomic operation");
            }
        }
        args.push(order);
        self.postfix(Node {
            nt: NodeType::Atomic(operation),
            token: Some(token),
            cty: Some(if operation == AtomicOperation::Store {
                Type::Void
            } else {
                ty
            }),
            args,
            volatile_access: object.qualifiers().volatile,
            ..Node::default()
        })
    }
    fn atomic_object(&self, pointer: &Node, token: &Token) -> Type {
        let object = pointer.dest_type().unwrap_or_else(|| {
            self.error_logger
                .print_error_position(token.pos, "atomic operation requires an object pointer");
            unreachable!()
        });
        let ty = object.unqualified();
        if !(ty.integer_info().is_some() || matches!(ty, Type::Ptr(_)))
            || ![1, 2, 4, 8].contains(&ty.size_of())
        {
            self.error_logger.print_error_position(
                token.pos,
                "atomic operation requires a 1-, 2-, 4- or 8-byte integer or pointer object",
            );
        }
        object
    }

    fn atomic_value(&self, mut value: Node, ty: &Type) -> Node {
        self.expect_conversion(ty, &value);
        if matches!(ty, Type::Ptr(_)) {
            Node {
                nt: NodeType::Cast,
                token: value.token.clone(),
                cty: Some(ty.clone()),
                lhs: Some(Box::new(value)),
                ..Node::default()
            }
        } else {
            Self::cast_integer(&mut value, ty);
            value
        }
    }

    fn atomic_compare_exchange_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let pointer = self.assign();
        let object = self.atomic_object(&pointer, &token);
        if object.qualifiers().constant {
            self.error_logger
                .print_error_position(token.pos, "atomic update requires a writable object");
        }
        self.expect_reserved(",");
        let expected = self.assign();
        let expected_object = expected.dest_type().unwrap_or_else(|| {
            self.error_logger.print_error_position(
                token.pos,
                "compare-exchange requires an expected-value pointer",
            );
            unreachable!()
        });
        if expected_object.qualifiers().constant || !expected_object.compatible(&object) {
            self.error_logger.print_error_position(
                token.pos,
                "compare-exchange requires a writable compatible expected value",
            );
        }
        self.expect_reserved(",");
        let desired = self.assign();
        let desired = self.atomic_value(desired, object.unqualified());
        self.expect_reserved(",");
        let weak = self.assign();
        let weak = self.atomic_value(weak, &Type::Bool);
        self.expect_reserved(",");
        let success = self.assign();
        let success = self.atomic_value(success, &Type::I32);
        self.expect_reserved(",");
        let failure = self.assign();
        let failure = self.atomic_value(failure, &Type::I32);
        self.expect_reserved(")");
        let success_order = Self::integer_constant(&success).then(|| self.eval(&success));
        let failure_order = Self::integer_constant(&failure).then(|| self.eval(&failure));
        if success_order.is_some_and(|order| !(0..=5).contains(&order))
            || failure_order.is_some_and(|order| ![0, 1, 2, 5].contains(&order))
        {
            self.error_logger
                .print_error_position(token.pos, "invalid compare-exchange memory order");
        }
        if let (Some(success), Some(failure)) = (success_order, failure_order) {
            let allowed = match success {
                0 | 3 => failure == 0,
                1 => failure <= 1,
                2 | 4 => failure <= 2,
                5 => true,
                _ => unreachable!(),
            };
            if !allowed {
                self.error_logger.print_error_position(
                    token.pos,
                    "compare-exchange failure order is stronger than success order",
                );
            }
        }
        self.postfix(Node {
            nt: NodeType::Atomic(AtomicOperation::CompareExchange(
                expected_object.qualifiers().volatile,
            )),
            token: Some(token),
            cty: Some(Type::Bool),
            args: vec![pointer, expected, desired, weak, success, failure],
            volatile_access: object.qualifiers().volatile,
            ..Node::default()
        })
    }
}

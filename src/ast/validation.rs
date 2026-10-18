//! Validate source types and expressions before backend lowering.

use super::AstBuilderImpl;
use crate::{
    ctype::Type,
    global::GlobalVariableData,
    node::{Node, NodeType},
    token::Token,
};

impl AstBuilderImpl<'_> {
    pub(super) fn validate_type_layout(&self, ty: &Type, token: &Token) {
        match ty {
            Type::Ptr(child) => self.validate_type_layout(child, token),
            Type::Arr(child, _) | Type::Vla(child, _) => {
                if child.size_of() % child.align_of() != 0 {
                    self.error_logger.print_error_position(
                        token.pos,
                        "array element size must be a multiple of its alignment",
                    );
                }
                if child.has_flexible_array() {
                    self.error_logger.print_error_position(
                        token.pos,
                        "array element cannot contain a flexible array member",
                    );
                }
                if matches!(
                    **child,
                    Type::Void | Type::Func(..) | Type::VariadicFunc(..)
                ) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid array element type");
                }
                if !child.is_complete_object() {
                    self.error_logger.print_error_position(
                        token.pos,
                        "array element requires a complete object type",
                    );
                }
                self.validate_type_layout(child, token);
            }
            Type::Func(args, result) | Type::VariadicFunc(args, result) => {
                if matches!(
                    **result,
                    Type::Arr(..) | Type::Vla(..) | Type::Func(..) | Type::VariadicFunc(..)
                ) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid function result type");
                }
                for ty in args.iter().chain(std::iter::once(result.as_ref())) {
                    if matches!(ty.unqualified(), Type::Struct(_))
                        && ty.contains_typedef_alignment()
                    {
                        self.error_logger.print_error_position(token.pos, "records containing non-natural-alignment typedefs cannot be passed or returned by value");
                    }
                    if ty.contains_vector() {
                        self.error_logger.print_error_position(token.pos, "vectors and records containing vectors cannot be passed or returned by value");
                    }
                    if matches!(ty.unqualified(), Type::Integer128 { .. }) {
                        self.error_logger.print_error_position(
                            token.pos,
                            "128-bit integer parameters and results are not supported",
                        );
                    }
                    if matches!(ty.unqualified(), Type::Struct(_)) && ty.align_of() > 16 {
                        self.error_logger.print_error_position(
                            token.pos,
                            "records aligned above 16 bytes cannot be passed or returned by value",
                        );
                    }
                    if matches!(ty.unqualified(), Type::Struct(_)) && ty.contains_float16() {
                        self.error_logger.print_error_position(
                            token.pos,
                            "records containing _Float16 cannot be passed or returned by value",
                        );
                    }
                    if matches!(ty.unqualified(), Type::Struct(_))
                        && ty.contains_extended_long_double()
                    {
                        self.error_logger.print_error_position(token.pos, "records containing extended long double cannot be passed or returned by value");
                    }
                    self.validate_type_layout(ty, token);
                }
            }
            Type::Qualified(child, q) => {
                if q.restricted
                    && !matches!(child.unqualified(), Type::Ptr(pointee) if pointee.function_signature_ref().is_none())
                {
                    self.error_logger.print_error_position(
                        token.pos,
                        "restrict requires a pointer to an object or void",
                    );
                }
                self.validate_type_layout(child, token);
            }
            _ => {}
        }
        if ty.checked_size_of().is_none() {
            self.error_logger
                .print_error_position(token.pos, "array size is too large");
        }
    }

    pub(super) fn expect_object_type(&self, ty: &Type, token: &Token) {
        if !ty.is_complete_object()
            || matches!(ty, Type::Void)
            || matches!(ty, Type::Arr(child, _) if **child == Type::Void)
        {
            self.error_logger
                .print_error_position(token.pos, "object requires a complete non-void type");
        }
    }

    pub(super) fn validate_global_initializer(
        &self,
        ty: &Type,
        data: &GlobalVariableData,
        token: &Token,
    ) {
        let ty = ty.unqualified();
        match (ty, data) {
            (_, GlobalVariableData::Zero) => {}
            (Type::Integer128 { .. }, _) => {
                self.error_logger.print_error_position(
                    token.pos,
                    "128-bit integer value initialization is not supported",
                );
            }
            (Type::Float16, _) => {
                self.error_logger.print_error_position(
                    token.pos,
                    "_Float16 value initialization is not supported",
                );
            }
            (Type::LongDouble(128), _) => {
                self.error_logger.print_error_position(
                    token.pos,
                    "extended long double value initialization is not supported",
                );
            }
            (Type::Arr(child, size), GlobalVariableData::Arr(elements)) => {
                if size.is_some_and(|size| elements.len() > size) {
                    self.error_logger
                        .print_error_position(token.pos, "too many array initializer elements");
                }
                for element in elements {
                    self.validate_global_initializer(child, element, token);
                }
            }
            (Type::Arr(..), _) => self
                .error_logger
                .print_error_position(token.pos, "array initializer requires braces"),
            (Type::Struct(ty), GlobalVariableData::Union { index, value }) => {
                self.validate_global_initializer(&ty.member(*index).ty, value, token);
            }
            (Type::Struct(ty), GlobalVariableData::Arr(elements)) => {
                let members = ty.members();
                let capacity = if ty.definition().borrow().is_union {
                    1
                } else {
                    members
                        .iter()
                        .take_while(|member| !matches!(member.ty.unqualified(), Type::Arr(_, None)))
                        .count()
                };
                if elements.len() > capacity {
                    self.error_logger
                        .print_error_position(token.pos, "too many struct initializer elements");
                }
                for (member, element) in members.iter().zip(elements) {
                    self.validate_global_initializer(&member.ty, element, token);
                }
            }
            (Type::Struct(_), _) => {
                self.error_logger
                    .print_error_position(token.pos, "global struct initializer requires braces");
            }
            (_, GlobalVariableData::Arr(elements)) => {
                if elements.len() != 1 {
                    self.error_logger
                        .print_error_position(token.pos, "scalar initializer requires one element");
                }
                for element in elements {
                    self.validate_global_initializer(ty, element, token);
                }
            }
            (ty, GlobalVariableData::Address { .. })
                if ty.is_arithmetic() && ty.integer_info().is_none_or(|(bits, _)| bits != 64) =>
            {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be an address");
            }
            (ty, GlobalVariableData::Elem(value))
                if ty.is_arithmetic() && value.starts_with('@') =>
            {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be a string");
            }
            _ => {}
        }
    }

    pub(super) fn can_convert(expected: &Type, operand: &Node) -> bool {
        let actual = operand.resolve_type_view();
        match (
            expected.unqualified(),
            actual.as_deref().map(Type::unqualified),
        ) {
            (target, Some(source)) if target.is_arithmetic() && source.is_arithmetic() => true,
            (Type::Bool, Some(source)) if source.is_pointer_value() => true,
            (
                Type::Ptr(target),
                Some(Type::Ptr(source) | Type::Arr(source, _) | Type::Vla(source, _)),
            ) => {
                (!source.qualifiers().constant || target.qualifiers().constant)
                    && (!source.qualifiers().volatile || target.qualifiers().volatile)
                    && (target.compatible(source)
                        || *target.unqualified() == Type::Void
                        || *source.unqualified() == Type::Void)
            }
            (Type::Ptr(target), Some(source @ (Type::Func(..) | Type::VariadicFunc(..)))) => {
                target.compatible(source)
            }
            (Type::Ptr(_), Some(Type::I32)) => {
                matches!(operand.nt, NodeType::Num) && operand.value == Some(0)
            }
            (target, Some(source)) => target == source,
            _ => false,
        }
    }

    pub(super) fn expect_conversion(&self, expected: &Type, operand: &Node) {
        if !Self::can_convert(expected, operand) {
            self.error_logger.print_error_position(
                operand.token.as_ref().map_or(0, |token| token.pos),
                "incompatible value type",
            );
        }
    }

    pub(super) fn expect_assignable(&self, node: &Node) {
        if node
            .object_type()
            .is_some_and(|ty| ty.qualifiers().constant)
        {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "cannot modify a const object",
            );
        }
        if !(node.compound_literal
            || matches!(
                node.nt,
                NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref
            ))
            || matches!(
                node.resolve_type(),
                Some(Type::Arr(..) | Type::Vla(..) | Type::Func(..) | Type::VariadicFunc(..))
                    | None
            )
        {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "expression is not assignable",
            );
        }
    }

    pub(super) fn validate_inline_body(&self, node: &Node) {
        let internal_object = self
            .global_variables
            .get(&node.global_name)
            .is_some_and(|object| {
                object.internal
                    && !(node.global_name.starts_with(".static.") && object.ty.is_const_object())
            });
        let internal_function = self
            .functions
            .get(&node.global_name)
            .is_some_and(|function| function.internal);
        if internal_object || internal_function {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "external inline definition cannot refer to an internal identifier",
            );
        }
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            self.validate_inline_body(child);
        }
        for child in node.children.iter().chain(&node.args) {
            self.validate_inline_body(child);
        }
    }

    pub(super) fn validate_storage_only_expressions(&self, node: &Node, address_only: bool) {
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| matches!(ty.unqualified(), Type::LongDouble(128)))
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "extended long double cannot be passed or returned by value",
            );
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| {
                    matches!(ty.unqualified(), Type::Struct(_)) && ty.contains_typedef_alignment()
                })
        {
            self.error_logger.print_error_position(node.token.as_ref().map_or(0, |token| token.pos), "records containing non-natural-alignment typedefs cannot be passed or returned by value");
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| matches!(ty.unqualified(), Type::Struct(_)) && ty.contains_float16())
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "records containing _Float16 cannot be passed or returned by value",
            );
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| matches!(ty.unqualified(), Type::Struct(_)) && ty.align_of() > 16)
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "records aligned above 16 bytes cannot be passed or returned by value",
            );
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| !ty.is_pointer_value() && ty.contains_vector())
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "vectors and records containing vectors cannot be passed or returned by value",
            );
        }
        if !address_only {
            let message = match node.resolve_type_view().as_deref().map(Type::unqualified) {
                Some(Type::Integer128 { .. }) => {
                    Some("128-bit integer value operations are not supported")
                }
                Some(Type::Float16) => Some("_Float16 value operations are not supported"),
                Some(Type::LongDouble(128)) => {
                    Some("extended long double value operations are not supported")
                }
                Some(Type::Vector(..)) => Some("vector value operations are not supported"),
                _ => None,
            };
            if let Some(message) = message {
                self.error_logger.print_error_position(
                    node.token.as_ref().map_or(0, |token| token.pos),
                    message,
                );
            }
        }
        if let Some(lhs) = &node.lhs {
            self.validate_storage_only_expressions(lhs, node.nt == NodeType::Addr);
        }
        for child in [
            &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        .map(Box::as_ref)
        .chain(node.children.iter())
        .chain(node.args.iter())
        {
            self.validate_storage_only_expressions(child, false);
        }
    }

    pub(super) fn validate_void_expressions(&self, node: &Node) {
        if matches!(
            node.nt,
            NodeType::Mod
                | NodeType::BitLeft
                | NodeType::BitRight
                | NodeType::BitAnd
                | NodeType::BitXor
                | NodeType::BitOr
                | NodeType::BitNot
        ) {
            for operand in [&node.lhs, &node.rhs].into_iter().flatten() {
                if operand
                    .resolve_type_view()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    self.error_logger.print_error_position(
                        node.token.as_ref().unwrap().pos,
                        "operation requires integer operands",
                    );
                }
            }
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
            if (lhs.float_bits().is_some() || rhs.float_bits().is_some())
                && !(lhs.is_arithmetic() && rhs.is_arithmetic())
            {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "floating operation requires arithmetic operands",
                );
            }
        }
        let no_value = |operand: &Node| {
            if operand.resolve_type_view().as_deref() == Some(&Type::Void) {
                self.error_logger.print_error_position(
                    operand.token.as_ref().unwrap().pos,
                    "void expression has no value",
                );
            }
        };
        if let Some(condition) = &node.cond {
            no_value(condition);
        }
        if matches!(
            node.nt,
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
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le
                | NodeType::LogicalAnd
                | NodeType::LogicalOr
                | NodeType::Assign
        ) {
            for operand in [&node.lhs, &node.rhs].into_iter().flatten() {
                no_value(operand);
            }
        }
        if matches!(node.nt, NodeType::Assign) {
            self.expect_conversion(
                &node.lhs.as_ref().unwrap().resolve_type_view().unwrap(),
                node.rhs.as_ref().unwrap(),
            );
        }
        if matches!(node.nt, NodeType::CallFunc) {
            for (argument, parameter) in node.args.iter().rev().zip(&node.parameter_types) {
                self.expect_conversion(parameter, argument);
            }
            for argument in &node.args {
                no_value(argument);
            }
        }
        if matches!(node.nt, NodeType::Conditional) {
            let then_void =
                node.then.as_ref().unwrap().resolve_type_view().as_deref() == Some(&Type::Void);
            let else_void =
                node.els.as_ref().unwrap().resolve_type_view().as_deref() == Some(&Type::Void);
            if then_void != else_void {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "conditional branches have incompatible void types",
                );
            }
        }
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            self.validate_void_expressions(child);
        }
        for child in node.children.iter().chain(&node.args) {
            self.validate_void_expressions(child);
        }
    }
}

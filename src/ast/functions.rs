use super::AstBuilderImpl;
use crate::{ctype::Type, node::Node, token::Token};

impl AstBuilderImpl<'_> {
    pub(super) fn parse_function_body(
        &mut self,
        token: &Token,
        ty: &Type,
        args: &[Node],
    ) -> Option<Node> {
        let (_, result, _) = ty.function_signature_ref().unwrap();
        if result.unqualified() != &Type::Void {
            self.expect_object_type(result, token);
        }
        self.return_pointer = if matches!(result, Type::Struct(_)) {
            self.offset_size = self.offset_size.div_ceil(8) * 8 + 8;
            Some(self.offset_size)
        } else {
            None
        };
        self.variadic_last = args.last().and_then(|arg| arg.offset);
        let (parameters, _, variadic) = ty.function_signature_ref().unwrap();
        self.variadic_parameters = variadic.then(|| parameters.to_vec());
        self.return_type = Some(result.clone());
        self.current_function = Some(token.clone());
        self.labels.clear();
        self.defined_labels.clear();
        let mut bounds = self.take_bounds();
        Self::resolve_parameter_bounds(&mut bounds, args);
        self.mutable_static = None;
        let mut body = self.consume_block();
        if body.is_some() {
            let (parameters, result, _) = ty.function_signature_ref().unwrap();
            if parameters
                .iter()
                .chain(std::iter::once(result))
                .any(|ty| matches!(ty.unqualified(), Type::LongDouble(128)))
            {
                self.error_logger.print_error_position(token.pos, "extended long double parameters and results in function definitions are not supported");
            }
            if parameters
                .iter()
                .chain(std::iter::once(result))
                .any(|ty| matches!(ty.unqualified(), Type::Float16))
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "_Float16 parameters and results in function definitions are not supported",
                );
            }
        }
        if let Some(body) = &mut body {
            bounds.append(&mut body.children);
            body.children = bounds;
            self.resolve_vla_gotos(body);
        }
        self.vla_active.clear();
        for (name, position) in &self.labels {
            if !self.defined_labels.contains(name) {
                self.error_logger
                    .print_error_position(*position, "goto target is not defined");
            }
        }
        self.return_type = None;
        self.current_function = None;
        self.variadic_last = args.last().and_then(|arg| arg.offset);
        self.variadic_parameters = None;
        self.scope_stack.pop();
        if body.is_none() {
            self.expect_reserved(";");
        }

        body
    }
}

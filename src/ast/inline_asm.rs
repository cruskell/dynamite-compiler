//! Parse the numbered register-operand subset of GNU extended assembly.

use std::rc::Rc;

use super::*;
use crate::node::render_inline_assembly;

impl AstBuilderImpl<'_> {
    pub(super) fn inline_assembly(&mut self, token: Token) -> Node {
        let volatile = matches!(
            self.tokens[self.cur].s_value.as_str(),
            "volatile" | "__volatile" | "__volatile__"
        );
        if volatile {
            self.cur += 1;
        }
        self.expect_reserved("(");
        let template = self.inline_assembly_string();
        self.expect_reserved(":");
        let mut outputs = vec![];
        let mut read_write = 0;
        while !matches!(self.tokens[self.cur].s_value.as_str(), ":" | ")") {
            if outputs.len() == 7 {
                self.error_logger.print_error_position(
                    token.pos,
                    "asm supports at most seven register operands",
                );
            }
            let constraint = self.inline_assembly_string();
            if !matches!(constraint.as_str(), "=r" | "+r") {
                self.error_logger
                    .print_error_position(token.pos, "asm requires =r or +r outputs");
            }
            self.expect_reserved("(");
            let output = self.assign();
            self.expect_reserved(")");
            self.expect_assignable(&output);
            let ty = output.resolve_type().unwrap();
            if output.bit_field.is_some()
                || !(ty
                    .integer_info()
                    .is_some_and(|(bits, _)| matches!(bits, 32 | 64))
                    || matches!(ty, Type::Ptr(_)))
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "asm outputs require writable 32-bit or 64-bit integer or pointer objects",
                );
            }
            if constraint == "+r" {
                read_write |= 1 << outputs.len();
            }
            outputs.push(output);
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        let mut inputs = vec![];
        let mut memory = false;
        if self.attempt_reserved(":").is_some() {
            if !matches!(self.tokens[self.cur].s_value.as_str(), ":" | ")") {
                loop {
                    if inputs.len() == 6
                        || outputs.len() + inputs.len() == 7
                        || self.inline_assembly_string() != "r"
                    {
                        self.error_logger.print_error_position(
                            token.pos,
                            "asm supports at most six r inputs and seven total register operands",
                        );
                    }
                    self.expect_reserved("(");
                    let mut input = self.expr();
                    self.expect_reserved(")");
                    let input_type = input.resolve_type().map(|ty| ty.decayed()).unwrap();
                    if !(input_type
                        .integer_info()
                        .is_some_and(|(bits, _)| matches!(bits, 32 | 64))
                        || matches!(input_type, Type::Ptr(_)))
                    {
                        self.error_logger.print_error_position(
                            token.pos,
                            "asm inputs require 32-bit or 64-bit integers or pointers",
                        );
                    }
                    if matches!(input_type, Type::Ptr(_)) {
                        input = Node {
                            nt: NodeType::Cast,
                            token: input.token.clone(),
                            cty: Some(input_type),
                            lhs: Some(Box::new(input)),
                            ..Node::default()
                        };
                    } else {
                        Self::cast_integer(&mut input, &input_type);
                    }
                    inputs.push(input);
                    if self.attempt_reserved(",").is_none() {
                        break;
                    }
                }
            }
            if self.attempt_reserved(":").is_some() && self.tokens[self.cur].s_value != ")" {
                loop {
                    if self.inline_assembly_string() != "memory" {
                        self.error_logger.print_error_position(
                            token.pos,
                            "asm supports only the memory clobber",
                        );
                    }
                    memory = true;
                    if self.attempt_reserved(",").is_none() {
                        break;
                    }
                }
            }
        }
        if render_inline_assembly(&template, |index, _| {
            (index < outputs.len() + inputs.len()).then(String::new)
        })
        .is_none()
        {
            self.error_logger
                .print_error_position(token.pos, "invalid or out-of-range asm register operand");
        }
        self.expect_reserved(")");
        self.expect_reserved(";");
        let mut children = vec![];
        let mut destinations = vec![];
        for (index, output) in outputs.into_iter().enumerate() {
            let alignment = output.alignment;
            let temporary = self.register_local_variable(
                Token {
                    s_value: format!(".asm.output.{}.{index}", token.pos).into(),
                    ..token.clone()
                },
                Type::Ptr(Rc::new(output.object_type().unwrap())),
            );
            let address = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, output);
            children.push(Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                temporary.clone(),
                address,
            ));
            let mut destination =
                Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, temporary);
            destination.alignment = alignment;
            destinations.push(destination);
        }
        let volatile = volatile || destinations.is_empty();
        children.push(Node {
            nt: NodeType::InlineAsm(volatile, memory),
            token: Some(token.clone()),
            cty: Some(Type::Void),
            children: destinations,
            value: Some(read_write),
            dest: template,
            args: inputs,
            ..Node::default()
        });
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }

    fn inline_assembly_string(&mut self) -> String {
        let token = self.attempt(TokenType::Str).unwrap_or_else(|| {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "asm requires a narrow string literal",
            );
            unreachable!()
        });
        if token.string_encoding != crate::token::StringEncoding::Narrow
            || token.string_bytes.contains(&0)
            || !token.string_bytes.is_ascii()
        {
            self.error_logger.print_error_position(
                token.pos,
                "asm requires a narrow ASCII string without embedded nulls",
            );
        }
        String::from_utf8(token.string_bytes.into_vec()).unwrap()
    }
}

//! Parse the register-output subset of GNU extended assembly.

use super::*;
use crate::node::{InlineAsmPart, InlineAssembly};

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
        let mut parts = vec![];
        let mut text = String::new();
        let mut chars = template.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                text.push(ch);
                continue;
            }
            match chars.next() {
                Some('%') => text.push('%'),
                Some('0') => {
                    parts.push(InlineAsmPart::Text(std::mem::take(&mut text)));
                    parts.push(InlineAsmPart::Output);
                }
                _ => self.error_logger.print_error_position(
                    token.pos,
                    "supported asm template escapes are %0 and %%",
                ),
            }
        }
        parts.push(InlineAsmPart::Text(text));
        self.expect_reserved(":");
        if self.inline_assembly_string() != "=r" {
            self.error_logger.print_error_position(
                token.pos,
                "asm requires one =r output and no input operands or clobbers",
            );
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
                "asm output requires a writable 32-bit or 64-bit integer or pointer object",
            );
        }
        // Empty optional sections are accepted; constraints requiring allocation are not.
        for _ in 0..2 {
            if self.attempt_reserved(":").is_none() {
                break;
            }
            if !matches!(self.tokens[self.cur].s_value.as_str(), ":" | ")") {
                self.error_logger.print_error_position(
                    token.pos,
                    "asm input operands and clobbers are not supported",
                );
            }
        }
        self.expect_reserved(")");
        self.expect_reserved(";");
        let alignment = output.alignment;
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".asm.output.{}", token.pos),
                ..token.clone()
            },
            Type::Ptr(Box::new(output.object_type().unwrap())),
        );
        let address = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, output);
        let save = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            temporary.clone(),
            address,
        );
        let mut destination =
            Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, temporary);
        destination.alignment = alignment;
        let value = Node {
            nt: NodeType::InlineAsm(Box::new(InlineAssembly { parts, volatile })),
            token: Some(token.clone()),
            cty: Some(ty),
            ..Node::default()
        };
        let update = Node::new_with_op(Some(token.clone()), NodeType::Assign, destination, value);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![save, update],
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
        String::from_utf8(token.string_bytes).unwrap()
    }
}

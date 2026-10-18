use super::{AstBuilderImpl, AutomaticStorage};
use crate::{
    ctype::Type,
    node::{Node, NodeType},
    token::{Token, TokenType},
};
use std::collections::HashMap;

impl<'a> AstBuilderImpl<'a> {
    pub(super) fn stmt(&mut self) -> Node {
        while self.attempt_reserved("__extension__").is_some() {}
        self.declaration_attributes(false, None, true);
        if matches!(
            self.tokens[self.cur].tt,
            TokenType::Reserved | TokenType::Ident
        ) && matches!(
            self.tokens[self.cur].s_value.as_str(),
            "asm" | "__asm" | "__asm__"
        ) {
            let token = self.tokens[self.cur].clone();
            self.cur += 1;
            return self.inline_assembly(token);
        }
        if self.attempt_static_assert() {
            return Node {
                nt: NodeType::Block,
                ..Node::default()
            };
        }
        if self.tokens[self.cur].tt == TokenType::Ident && self.tokens[self.cur + 1].s_value == ":"
        {
            return self.label_statement();
        }
        if let Some(token) = self.attempt_reserved("goto") {
            return self.goto_statement(token);
        }
        if let Some(token) = self.attempt_reserved("typedef") {
            self.typedef_definition();
            self.expect_reserved(";");
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: self.take_bounds(),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved(";") {
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                ..Node::default()
            };
        }
        if let Some(t) = self.attempt_reserved("switch") {
            return self.switch_statement(t);
        }
        if let Some(t) = self
            .attempt_reserved("case")
            .or_else(|| self.attempt_reserved("default"))
        {
            return self.case_statement(t);
        }
        if let Some(t) = self.attempt_reserved("if") {
            return self.if_statement(t);
        }
        if let Some(t) = self.attempt_reserved("do") {
            return self.do_statement(t);
        }
        if let Some(t) = self.attempt_reserved("while") {
            return self.while_statement(t);
        }
        if let Some(t) = self.attempt_reserved("for") {
            return self.for_statement(t);
        }
        if let Some(node) = self.consume_block() {
            return node;
        }
        self.declaration_or_expression_statement()
    }

    pub fn consume_block(&mut self) -> Option<Node> {
        let (token, mut children, cleanup) = self.consume_block_parts()?;
        children.extend(cleanup);
        Some(Node {
            nt: NodeType::Block,
            token: Some(token),
            children,
            ..Node::default()
        })
    }

    pub(super) fn consume_block_parts(&mut self) -> Option<(Token, Vec<Node>, Vec<Node>)> {
        if let Some(t) = self.attempt_reserved("{") {
            self.scope_stack.push(HashMap::new());
            let object_depth = self.dynamic_objects.len();
            let bound_depth = self.vla_active.len();
            let mut children: Vec<Node> = Vec::new();
            while self.attempt_reserved("}").is_none() {
                children.push(self.stmt());
            }
            let cleanup = self.cleanup_arrays(object_depth);
            self.dynamic_objects.truncate(object_depth);
            self.vla_active.truncate(bound_depth);
            self.scope_stack.pop();
            Some((t.clone(), children, cleanup))
        } else {
            None
        }
    }

    fn goto_statement(&mut self, token: &'a Token) -> Node {
        let name = self.attempt_ident().unwrap_or_else(|| {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "goto requires a label");
            unreachable!()
        });
        let id = *self
            .labels
            .entry(name.s_value.to_string())
            .or_insert(name.pos);
        self.expect_reserved(";");
        Node {
            nt: NodeType::Goto,
            token: Some(token.clone()),
            value: Some(id),
            args: self.dynamic_objects.clone(),
            vla_scope: self.vla_active.clone(),
            ..Node::default()
        }
    }

    fn switch_statement(&mut self, t: &'a Token) -> Node {
        self.expect_reserved("(");
        let cond = self.expr();
        self.expect_reserved(")");
        if cond
            .resolve_type()
            .is_none_or(|ty| ty.integer_info().is_none())
        {
            self.error_logger
                .print_error_position(t.pos, "switch requires an integer expression");
        }
        self.switch_vla_depths.push(self.vla_active.len());
        self.jump_depths.push(self.dynamic_objects.len());
        self.switch_stack.push(vec![]);
        self.switch_types
            .push(cond.resolve_type().unwrap().promoted());
        let body = self.stmt();
        let cases = self.switch_stack.pop().unwrap();
        self.switch_types.pop();
        self.switch_vla_depths.pop();
        self.jump_depths.pop();
        Node {
            nt: NodeType::Switch,
            token: Some(t.clone()),
            cond: Some(Box::new(cond)),
            then: Some(Box::new(body)),
            children: cases,
            ..Node::default()
        }
    }

    fn if_statement(&mut self, t: &'a Token) -> Node {
        self.expect_reserved("(");
        let cond = self.expr();
        self.expect_reserved(")");
        let then = self.stmt();
        let mut els: Option<Node> = None;
        if self.attempt_reserved("else").is_some() {
            els = Some(self.stmt());
        }
        Node::new_if_node(Some(t.clone()), cond, then, els)
    }

    fn do_statement(&mut self, t: &'a Token) -> Node {
        self.loop_stack.push(t.pos);
        self.continue_depths
            .insert(t.pos, self.dynamic_objects.len());
        self.jump_depths.push(self.dynamic_objects.len());
        let body = self.stmt();
        self.loop_stack.pop();
        self.jump_depths.pop();
        self.expect_reserved("while");
        self.expect_reserved("(");
        let cond = self.expr();
        self.expect_reserved(")");
        self.expect_reserved(";");
        let mut node = Node::new_while_node(Some(t.clone()), cond, body);
        node.nt = NodeType::DoWhile;
        node
    }

    fn while_statement(&mut self, t: &'a Token) -> Node {
        self.expect_reserved("(");
        let cond = self.expr();
        self.expect_reserved(")");
        self.loop_stack.push(t.pos);
        self.continue_depths
            .insert(t.pos, self.dynamic_objects.len());
        self.jump_depths.push(self.dynamic_objects.len());
        let body = self.stmt();
        self.loop_stack.pop();
        self.jump_depths.pop();
        Node::new_while_node(Some(t.clone()), cond, body)
    }

    fn for_statement(&mut self, t: &'a Token) -> Node {
        let object_depth = self.dynamic_objects.len();
        let bound_depth = self.vla_active.len();
        self.scope_stack.push(HashMap::new());
        self.expect_reserved("(");
        let mut ini: Option<Node> = None;
        let mut cond: Option<Node> = None;
        let mut upd: Option<Node> = None;
        if self.attempt_reserved(";").is_none() {
            ini = Some(
                if let Some((ty, storage, alignment)) = self.attempt_automatic_type(false) {
                    self.local_variable_definition_with_storage(ty, storage, alignment)
                } else {
                    self.expr()
                },
            );
            self.expect_reserved(";");
        }
        if self.attempt_reserved(";").is_none() {
            cond = Some(self.expr());
            self.expect_reserved(";");
        }
        if self.attempt_reserved(")").is_none() {
            upd = Some(self.expr());
            self.expect_reserved(")");
        }
        self.loop_stack.push(t.pos);
        self.continue_depths
            .insert(t.pos, self.dynamic_objects.len());
        self.jump_depths.push(self.dynamic_objects.len());
        let body = self.stmt();
        self.loop_stack.pop();
        self.jump_depths.pop();
        self.scope_stack.pop();
        let loop_node = Node::new_for_node(Some(t.clone()), ini, cond, upd, body);
        let mut children = vec![loop_node];
        children.extend(self.cleanup_arrays(object_depth));
        self.dynamic_objects.truncate(object_depth);
        self.vla_active.truncate(bound_depth);
        Node {
            nt: NodeType::Block,
            children,
            ..Node::default()
        }
    }

    fn case_statement(&mut self, t: &'a Token) -> Node {
        if self.switch_stack.is_empty() {
            self.error_logger
                .print_error_position(t.pos, "case or default requires an enclosing switch");
        }
        if self.vla_active.len() > *self.switch_vla_depths.last().unwrap() {
            self.error_logger.print_error_position(
                t.pos,
                "switch label enters the scope of a variably modified declaration",
            );
        }
        let value = if t.s_value == "case" {
            let expr = self.ternary();
            let value = self.eval(&expr);
            Some(self.switch_types.last().unwrap().convert_constant(value) as usize)
        } else {
            None
        };
        self.expect_reserved(":");
        if self
            .switch_stack
            .last()
            .unwrap()
            .iter()
            .any(|case| case.value == value)
        {
            self.error_logger
                .print_error_position(t.pos, "duplicate switch label");
        }
        let mut node = Node {
            nt: NodeType::Case,
            token: Some(t.clone()),
            value,
            ..Node::default()
        };
        self.switch_stack.last_mut().unwrap().push(node.clone());
        node.lhs = Some(Box::new(self.stmt()));
        node
    }

    fn label_statement(&mut self) -> Node {
        let token = self.attempt_ident().unwrap();
        self.expect_reserved(":");
        if !self.defined_labels.insert(token.s_value.to_string()) {
            self.error_logger
                .print_error_position(token.pos, "duplicate label");
        }
        let id = *self
            .labels
            .entry(token.s_value.to_string())
            .or_insert(token.pos);
        let vla_scope = self.vla_active.clone();
        let lhs = Some(Box::new(self.stmt()));
        Node {
            nt: NodeType::Label,
            token: Some(token),
            lhs,
            value: Some(id),
            vla_scope,
            ..Node::default()
        }
    }

    fn declaration_or_expression_statement(&mut self) -> Node {
        let mut requested_alignment = self.object_alignment_annotations();
        let mut thread_local = false;
        while let Some(value) = self.attempt_alignment_specifier() {
            requested_alignment = requested_alignment.max(value);
        }
        let inline = self.attempt_inline();
        let storage = self
            .attempt_reserved("static")
            .or_else(|| self.attempt_reserved("extern"));
        let inline = inline.or_else(|| self.attempt_inline());
        let mut storage = storage;
        loop {
            requested_alignment = requested_alignment.max(self.object_alignment_annotations());
            if self.attempt_thread_local() {
                thread_local = true;
            } else if let Some(token) = self
                .attempt_reserved("static")
                .or_else(|| self.attempt_reserved("extern"))
            {
                if storage.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting storage classes");
                } else {
                    storage = Some(token);
                }
            } else {
                break;
            }
        }
        if thread_local && storage.is_none() {
            self.error_logger.print_error_position(
                self.tokens[self.cur - 1].pos,
                "block thread-local declaration requires static or extern storage",
            );
        }
        if storage.is_some() || inline.is_some() || thread_local {
            let mut storage = storage.cloned().unwrap_or_else(|| {
                inline.cloned().unwrap_or_else(|| Token {
                    s_value: "static".to_owned().into(),
                    ..self.tokens[self.cur - 1].clone()
                })
            });
            if matches!(
                storage.s_value.as_str(),
                "inline" | "__inline" | "__inline__"
            ) {
                storage.s_value = "extern".to_owned().into();
            }
            requested_alignment = requested_alignment.max(self.object_alignment_annotations());
            let ty = self.expect_type();
            requested_alignment = requested_alignment.max(self.object_alignment_annotations());
            let declaration = self.local_static_definition(
                ty,
                &storage,
                inline.is_some(),
                requested_alignment,
                thread_local,
            );
            self.expect_reserved(";");
            return declaration;
        }
        let node = if let Some((ty, storage, alignment)) = self.attempt_automatic_type(false) {
            if storage == AutomaticStorage::Register && requested_alignment != 0 {
                self.error_logger.print_error_position(
                    self.tokens[self.cur].pos,
                    "alignment cannot be applied to a register object",
                );
            }
            if self.tokens[self.cur].s_value == ";" {
                if alignment.max(requested_alignment) != 0 {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "alignment requires an object declaration",
                    );
                }
                Node {
                    nt: NodeType::Block,
                    ..Node::default()
                }
            } else {
                self.local_variable_definition_with_storage(
                    ty,
                    storage,
                    alignment.max(requested_alignment),
                )
            }
        } else if requested_alignment != 0 {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "alignment requires an object declaration",
            );
            unreachable!()
        } else if let Some(t) = self.attempt_reserved("continue") {
            let target = self.loop_stack.last().copied().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(t.pos, "continue requires an enclosing loop");
                unreachable!()
            });
            Node {
                nt: NodeType::Continue,
                token: Some(t.clone()),
                value: Some(target),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_reserved("break") {
            Node {
                nt: NodeType::Break,
                token: Some(t.clone()),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_reserved("return") {
            let mut operand = if self.tokens[self.cur].s_value == ";" {
                None
            } else {
                Some(self.expr())
            };
            let expected_void = self.return_type == Some(Type::Void);
            let actual_void = operand
                .as_ref()
                .is_none_or(|operand| operand.resolve_type() == Some(Type::Void));
            if expected_void != actual_void {
                self.error_logger.print_error_position(
                    t.pos,
                    "return value does not match function return type",
                );
            }
            if let Some(value) = &operand {
                if !expected_void {
                    self.expect_conversion(self.return_type.as_ref().unwrap(), value);
                }
            }
            if !expected_void {
                if let Some(value) = &mut operand {
                    Self::cast_integer(value, self.return_type.as_ref().unwrap());
                }
            }
            Node {
                nt: NodeType::Return,
                token: Some(t.clone()),
                lhs: operand.map(Box::new),
                offset: self.return_pointer,
                ..Node::default()
            }
        } else {
            self.expr()
        };
        self.expect_reserved(";");
        self.cleanup_jump(node)
    }
}

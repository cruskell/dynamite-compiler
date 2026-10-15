//! Parse GNU statement expressions using ordinary scoped statements and value temporaries.

use super::*;

impl AstBuilderImpl<'_> {
    pub(super) fn statement_expression(&mut self, token: Token) -> Node {
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "statement expression requires function scope");
        }
        let (_, mut statements, cleanup) = self.consume_block_parts().unwrap();
        self.expect_reserved(")");
        for statement in &statements {
            self.validate_statement_expression_jumps(statement, 0, 0);
        }
        let has_value = statements.last().is_some_and(|node| {
            !matches!(
                node.nt,
                NodeType::Block
                    | NodeType::DefVar
                    | NodeType::Return
                    | NodeType::If
                    | NodeType::While
                    | NodeType::DoWhile
                    | NodeType::For
                    | NodeType::Switch
                    | NodeType::Case
                    | NodeType::Break
                    | NodeType::Continue
                    | NodeType::Goto
                    | NodeType::Label
                    | NodeType::InlineAsm(_)
            )
        });
        let expression = if has_value { statements.pop() } else { None };
        let mut save = None;
        let result = if let Some(expression) = expression {
            let ty = expression.resolve_type().unwrap().decayed();
            if ty == Type::Void {
                statements.push(expression);
                None
            } else {
                let temporary = self.register_local_variable(
                    Token {
                        s_value: format!(".statement_expression.{}", token.pos),
                        ..token.clone()
                    },
                    ty,
                );
                // Capture the value before leaving a scope that may own variable-length arrays.
                save = Some(self.local_expression_initializer(&temporary, expression, &token));
                Some(temporary)
            }
        } else {
            None
        };
        let mut children = vec![Node {
            nt: NodeType::Block,
            token: Some(token.clone()),
            children: statements,
            ..Node::default()
        }];
        if let Some(save) = save {
            children.push(save);
        }
        children.push(Node {
            nt: NodeType::Block,
            token: Some(token.clone()),
            children: cleanup,
            ..Node::default()
        });
        children.push(result.unwrap_or_else(|| Node {
            nt: NodeType::Cast,
            token: Some(token.clone()),
            cty: Some(Type::Void),
            lhs: Some(Box::new(Node::new_with_num(Some(token.clone()), 0))),
            ..Node::default()
        }));
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }

    fn validate_statement_expression_jumps(&self, node: &Node, loops: usize, switches: usize) {
        let escaping = match node.nt {
            NodeType::Return | NodeType::Goto | NodeType::Label => true,
            NodeType::Break => loops == 0 && switches == 0,
            NodeType::Continue => loops == 0,
            NodeType::Case => switches == 0,
            _ => false,
        };
        if escaping {
            self.error_logger.print_error_position(node.token.as_ref().unwrap().pos,
                "labels, goto, return, and escaping control transfers in statement expressions are not supported");
        }
        let loops = loops
            + usize::from(matches!(
                node.nt,
                NodeType::While | NodeType::DoWhile | NodeType::For
            ));
        let switches = switches + usize::from(node.nt == NodeType::Switch);
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            self.validate_statement_expression_jumps(child, loops, switches);
        }
        for child in node.children.iter().chain(&node.args) {
            self.validate_statement_expression_jumps(child, loops, switches);
        }
    }
}

use super::*;

fn cast(node: Node, ty: Type) -> Node {
    Node {
        nt: NodeType::Cast,
        token: node.token.clone(),
        lhs: Some(Box::new(node)),
        cty: Some(ty),
        ..Node::default()
    }
}
fn number(value: usize) -> Node {
    Node {
        value: Some(value),
        cty: Some(Type::integer(64, false)),
        ..Node::default()
    }
}
fn binary(nt: NodeType, lhs: Node, rhs: Node) -> Node {
    Node::new_with_op(
        lhs.token.clone().or_else(|| rhs.token.clone()),
        nt,
        lhs,
        rhs,
    )
}
fn block(children: Vec<Node>) -> Node {
    Node {
        nt: NodeType::Block,
        children,
        ..Node::default()
    }
}
fn walk(node: &Node, visitor: &mut impl FnMut(&Node)) {
    visitor(node);
    for child in [
        &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
    ]
    .into_iter()
    .flatten()
    {
        walk(child, visitor);
    }
    for child in node.children.iter().chain(&node.args) {
        walk(child, visitor);
    }
}
fn walk_mut(node: &mut Node, visitor: &mut impl FnMut(&mut Node)) {
    for child in [
        &mut node.lhs,
        &mut node.rhs,
        &mut node.cond,
        &mut node.then,
        &mut node.els,
        &mut node.ini,
        &mut node.upd,
    ]
    .into_iter()
    .flatten()
    {
        walk_mut(child, visitor);
    }
    for child in node.children.iter_mut().chain(&mut node.args) {
        walk_mut(child, visitor);
    }
    visitor(node);
}

impl AstBuilderImpl<'_> {
    pub(super) fn resolve_parameter_bounds(bounds: &mut [Node], args: &[Node]) {
        for bound in bounds {
            walk_mut(bound, &mut |node| {
                if node.nt == NodeType::LocalVar
                    && node.offset.is_some_and(|offset| offset > usize::MAX / 2)
                {
                    let name = &node.token.as_ref().unwrap().s_value;
                    node.offset = args
                        .iter()
                        .find(|arg| &arg.token.as_ref().unwrap().s_value == name)
                        .and_then(|arg| arg.offset);
                }
            });
        }
    }
    pub(super) fn integer_constant(node: &Node) -> bool {
        match node.nt {
            NodeType::Num => node
                .resolve_type()
                .is_some_and(|ty| ty.integer_info().is_some()),
            NodeType::Cast
                if node
                    .resolve_type()
                    .is_some_and(|ty| ty.integer_info().is_some())
                    && node
                        .lhs
                        .as_ref()
                        .is_some_and(|child| child.nt == NodeType::Num) =>
            {
                true
            }
            NodeType::Cast | NodeType::BitNot => node
                .lhs
                .as_ref()
                .is_some_and(|child| Self::integer_constant(child)),
            NodeType::Add
            | NodeType::Sub
            | NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::Eq
            | NodeType::Ne
            | NodeType::Lt
            | NodeType::Le
            | NodeType::BitAnd
            | NodeType::BitOr
            | NodeType::BitXor
            | NodeType::BitLeft
            | NodeType::BitRight
            | NodeType::LogicalAnd
            | NodeType::LogicalOr => {
                node.lhs
                    .as_ref()
                    .is_some_and(|child| Self::integer_constant(child))
                    && node
                        .rhs
                        .as_ref()
                        .is_some_and(|child| Self::integer_constant(child))
            }
            NodeType::Conditional => [&node.cond, &node.then, &node.els]
                .into_iter()
                .flatten()
                .all(|child| Self::integer_constant(child)),
            _ => false,
        }
    }
    pub(super) fn vla_temporary(&mut self, token: &Token, ty: Type) -> Node {
        self.register_local_variable(
            Token {
                s_value: format!(".vla.{}.{}", token.pos, self.offset_size),
                ..token.clone()
            },
            ty,
        )
    }
    pub(super) fn dynamic_node(ty: Type, offset: usize, token: Token) -> Node {
        let pointer = Node {
            nt: NodeType::LocalVar,
            cty: Some(Type::Ptr(Box::new(ty))),
            offset: Some(offset),
            token: Some(token.clone()),
            ..Node::default()
        };
        Node::new_with_op_and_lhs(Some(token), NodeType::Deref, pointer)
    }
    pub(super) fn take_bounds(&mut self) -> Vec<Node> {
        let bounds = std::mem::take(&mut self.pending_bounds);
        self.vla_active.extend(
            bounds
                .iter()
                .filter_map(|bound| bound.lhs.as_ref().and_then(|lhs| lhs.offset)),
        );
        bounds
    }
    pub(super) fn runtime_size(ty: &Type) -> Node {
        match ty.unqualified() {
            Type::Vla(child, offset) => binary(
                NodeType::Mul,
                Node {
                    nt: NodeType::LocalVar,
                    offset: Some(*offset),
                    cty: Some(Type::integer(64, false)),
                    ..Node::default()
                },
                Self::runtime_size(child),
            ),
            Type::Arr(child, size) if child.is_variable_array() => {
                binary(NodeType::Mul, number(*size), Self::runtime_size(child))
            }
            _ => number(ty.size_of()),
        }
    }
    fn allocation_call(name: &str, value: Node, result: Type, parameter: Type) -> Node {
        Node {
            nt: NodeType::CallFunc,
            global_name: name.into(),
            args: vec![value],
            cty: Some(result),
            parameter_types: vec![parameter],
            ..Node::default()
        }
    }
    pub(super) fn allocate_variable_array(&mut self, node: &Node) -> Node {
        let pointer = node.lhs.as_ref().unwrap().as_ref().clone();
        let ty = node.resolve_type().unwrap();
        let size = Self::runtime_size(&ty);
        let call = Self::allocation_call(
            "malloc",
            size,
            Type::Ptr(Box::new(Type::Void)),
            Type::integer(64, false),
        );
        self.dynamic_objects.push(pointer.clone());
        self.vla_active.push(pointer.offset.unwrap());
        binary(
            NodeType::Assign,
            pointer,
            cast(call, Type::Ptr(Box::new(ty))),
        )
    }
    fn free_array(pointer: Node) -> Node {
        let void_pointer = Type::Ptr(Box::new(Type::Void));
        Self::allocation_call(
            "free",
            cast(pointer, void_pointer.clone()),
            Type::Void,
            void_pointer,
        )
    }
    pub(super) fn cleanup_arrays(&self, depth: usize) -> Vec<Node> {
        self.dynamic_objects[depth..]
            .iter()
            .rev()
            .cloned()
            .map(Self::free_array)
            .collect()
    }
    pub(super) fn cleanup_jump(&mut self, mut node: Node) -> Node {
        let depth = match node.nt {
            NodeType::Return => 0,
            NodeType::Break => self
                .jump_depths
                .last()
                .copied()
                .unwrap_or(self.dynamic_objects.len()),
            NodeType::Continue => self.continue_depths[&node.value.unwrap()],
            _ => return node,
        };
        if depth == self.dynamic_objects.len() {
            return node;
        }
        let mut children = vec![];
        if node.nt == NodeType::Return {
            if let Some(value) = node.lhs.take() {
                let ty = value.resolve_type().unwrap();
                if ty == Type::Void {
                    children.push(*value);
                } else {
                    let token = node.token.as_ref().unwrap();
                    let temporary = self.vla_temporary(token, ty.clone());
                    children.push(if matches!(ty, Type::Struct(_)) {
                        self.struct_assignment(temporary.clone(), *value, token.clone())
                    } else {
                        binary(NodeType::Assign, temporary.clone(), *value)
                    });
                    node.lhs = Some(Box::new(temporary));
                }
            }
        }
        children.extend(self.cleanup_arrays(depth));
        children.push(node);
        block(children)
    }
    pub(super) fn resolve_vla_gotos(&self, body: &mut Node) {
        let mut labels = HashMap::new();
        walk(body, &mut |node| {
            if node.nt == NodeType::Label {
                labels.insert(node.value.unwrap(), node.vla_scope.clone());
            }
        });
        walk_mut(body, &mut |node| {
            if node.nt != NodeType::Goto {
                return;
            }
            let Some(target) = labels.get(&node.value.unwrap()) else {
                return;
            };
            if target.iter().any(|id| !node.vla_scope.contains(id)) {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "goto enters the scope of a variably modified declaration",
                );
            }
            let allocations = std::mem::take(&mut node.args);
            let mut cleanup = vec![];
            for pointer in allocations.into_iter().rev() {
                if !target.contains(&pointer.offset.unwrap()) {
                    cleanup.push(Self::free_array(pointer));
                }
            }
            if !cleanup.is_empty() {
                cleanup.push(std::mem::take(node));
                *node = block(cleanup);
            }
        });
    }
    pub(super) fn lower_variable_arithmetic(node: &mut Node) {
        walk_mut(node, &mut |node| {
            if !matches!(node.nt, NodeType::Add | NodeType::Sub) {
                return;
            }
            let Some(element) = node.lhs.as_ref().and_then(|lhs| lhs.dest_type()) else {
                return;
            };
            if !element.is_variable_array() {
                return;
            }
            let original = std::mem::take(node);
            let result_type = match original.resolve_type().unwrap() {
                Type::Arr(child, _) | Type::Vla(child, _) => Type::Ptr(child),
                ty => ty,
            };
            let token = original.token.clone();
            let lhs = *original.lhs.unwrap();
            let rhs = *original.rhs.unwrap();
            let bytes = Type::Ptr(Box::new(Type::I8));
            *node = if original.nt == NodeType::Sub && rhs.dest_type().is_some() {
                let difference = binary(
                    NodeType::Sub,
                    cast(lhs, Type::integer(64, true)),
                    cast(rhs, Type::integer(64, true)),
                );
                cast(
                    binary(
                        NodeType::Div,
                        difference,
                        cast(Self::runtime_size(&element), Type::integer(64, true)),
                    ),
                    Type::integer(64, true),
                )
            } else {
                let scaled = binary(
                    NodeType::Mul,
                    cast(rhs, Type::integer(64, true)),
                    Self::runtime_size(&element),
                );
                cast(binary(original.nt, cast(lhs, bytes), scaled), result_type)
            };
            node.token = token;
        });
    }
}

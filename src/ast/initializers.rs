use super::AstBuilderImpl;
use super::Identifier;
use crate::ctype::{Qualifiers, StructMember, Type};
use crate::global::GlobalVariableData;
use crate::node::{Node, NodeType};
use crate::token::{Token, TokenType};
use rustc_apfloat::Float;
use std::collections::HashMap;
use std::rc::Rc;

impl AstBuilderImpl<'_> {
    pub(super) fn register_incomplete_local_array(
        &mut self,
        token: Token,
        ty: Type,
        requested_alignment: usize,
    ) -> Node {
        let alignment = ty.align_of().max(requested_alignment);
        if alignment > 16 {
            self.error_logger.print_error_position(
                token.pos,
                "requested local alignment exceeds the supported stack alignment",
            );
        }
        if self
            .scope_stack
            .last()
            .unwrap()
            .contains_key(token.s_value.as_str())
        {
            self.error_logger
                .print_error_position(token.pos, "invalid redeclaration");
        }
        // Reserve an identity while expressions are parsed; allocate the completed object later.
        let offset = self.next_deferred_local;
        self.next_deferred_local -= 1;
        self.scope_stack.last_mut().unwrap().insert(
            token.s_value.to_string(),
            Identifier::Local(ty.clone(), offset, false, alignment),
        );
        Node {
            nt: NodeType::LocalVar,
            token: Some(token),
            cty: Some(ty),
            offset: Some(offset),
            alignment,
            ..Node::default()
        }
    }

    pub(super) fn resolve_deferred_local(node: &mut Node, offset: usize, object: &Node) {
        if node.nt == NodeType::LocalVar && node.offset == Some(offset) {
            node.offset = object.offset;
            node.cty = object.cty.clone();
        }
        for child in node.child_nodes_mut() {
            Self::resolve_deferred_local(child, offset, object);
        }
        node.refresh_arithmetic_type();
    }

    pub(super) fn inferred_local_array_initializer(
        &mut self,
        object: &Node,
        token: &Token,
    ) -> (Node, Node) {
        if self.tokens[self.cur].s_value != "{"
            && self
                .string_initializer_index(&object.resolve_type().unwrap())
                .is_none()
        {
            self.error_logger.print_error_position(
                token.pos,
                "array initializer requires braces or a string literal",
            );
        }
        let mut count = 0;
        let mut initializer =
            self.local_variable_initialization_with_count(object, token, Some(&mut count));
        let ty = self.infer_array_size(object.cty.clone().unwrap(), count, token);
        let name = object.token.clone().unwrap();
        self.scope_stack
            .last_mut()
            .unwrap()
            .remove(name.s_value.as_str());
        let mut completed = self.register_local_variable_aligned(name, ty, object.alignment);
        if object.registered {
            self.mark_registered(&mut completed);
        }
        Self::resolve_deferred_local(&mut initializer, object.offset.unwrap(), &completed);
        // Zero omitted elements after the final bound is known, before evaluating any initializer.
        initializer
            .children
            .insert(0, Self::zero_initializer(&completed, token));
        (completed, initializer)
    }

    pub(super) fn infer_array_size(&self, ty: Type, count: usize, token: &Token) -> Type {
        if let Type::Arr(child, None) = ty {
            if count == 0 {
                self.error_logger.print_error_position(
                    token.pos,
                    "incomplete array requires a nonempty initializer",
                );
            }
            if child.size_of().checked_mul(count).is_none() {
                self.error_logger
                    .print_error_position(token.pos, "array size is too large");
            }
            Type::Arr(child, Some(count))
        } else {
            ty
        }
    }

    pub(super) fn string_initializer_index(&self, ty: &Type) -> Option<usize> {
        let index = self.cur + usize::from(self.tokens[self.cur].s_value == "{");
        let token = self.tokens.get(index)?;
        if token.tt == TokenType::Str
            && matches!(ty.unqualified(), Type::Arr(child, _)
                if child.integer_info().is_some()
                    && child.size_of() == token.string_encoding.unit_size())
        {
            Some(index)
        } else {
            None
        }
    }

    pub(super) fn attempt_string_initializer(&mut self, ty: &Type) -> Option<Token> {
        let index = self.string_initializer_index(ty)?;
        let braced = index != self.cur;
        self.cur = index;
        let token = self.attempt(TokenType::Str).unwrap();
        if braced {
            self.attempt_reserved(",");
            self.expect_reserved("}");
        }
        Some(token)
    }

    pub(super) fn aggregate_type(ty: &Type, index: usize) -> Option<Type> {
        match ty.unqualified() {
            Type::Arr(child, size) if size.is_none_or(|size| index < size) => {
                Some(child.as_ref().clone())
            }
            Type::Struct(st) => st
                .definition()
                .borrow()
                .members
                .get(index)
                .filter(|member| !matches!(member.ty.unqualified(), Type::Arr(_, None)))
                .map(|member| member.ty.clone()),
            _ => None,
        }
    }

    pub(super) fn initializer_subobject(ty: &Type, path: &[usize]) -> Option<Type> {
        path.iter()
            .try_fold(ty.clone(), |ty, index| Self::aggregate_type(&ty, *index))
    }

    pub(super) fn next_initializer_path(ty: &Type, mut path: Vec<usize>) -> Option<Vec<usize>> {
        while let Some(index) = path.pop() {
            let parent = Self::initializer_subobject(ty, &path).unwrap();
            if matches!(parent.unqualified(), Type::Struct(st) if st.definition().borrow().is_union)
            {
                continue;
            }
            if Self::aggregate_type(&parent, index + 1).is_some() {
                path.push(index + 1);
                return Some(path);
            }
        }
        None
    }

    pub(super) fn designation(&mut self, ty: &Type) -> Vec<usize> {
        let mut current = ty.clone();
        let mut path = vec![];
        loop {
            let indices = if let Some(token) = self.attempt_reserved("[") {
                let expression = self.ternary();
                let index = self.eval(&expression);
                self.expect_reserved("]");
                if index < 0 || !matches!(current.unqualified(), Type::Arr(..)) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid array designator");
                }
                vec![index as usize]
            } else if let Some(token) = self.attempt_reserved(".") {
                let name = self.attempt_ident().unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(token.pos, "member designator expected");
                    unreachable!()
                });
                let Type::Struct(st) = current.unqualified() else {
                    self.error_logger
                        .print_error_position(token.pos, "member designator requires an aggregate");
                    unreachable!()
                };
                st.member_path(&name.s_value).unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(name.pos, "unknown designated member");
                    unreachable!()
                })
            } else {
                break;
            };
            for index in indices {
                current = Self::aggregate_type(&current, index).unwrap_or_else(|| {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "designator is out of bounds",
                    );
                    unreachable!()
                });
                path.push(index);
            }
        }
        if !path.is_empty() {
            self.expect_reserved("=");
        }
        path
    }

    pub(super) fn set_global_element(
        ty: &Type,
        data: &mut GlobalVariableData,
        path: &[usize],
        value: GlobalVariableData,
    ) {
        if path.is_empty() {
            *data = value;
            return;
        }
        let child = Self::aggregate_type(ty, path[0]).unwrap();
        if matches!(ty.unqualified(), Type::Struct(st) if st.definition().borrow().is_union) {
            if !matches!(data, GlobalVariableData::Union { index, .. } if *index == path[0]) {
                *data = GlobalVariableData::Union {
                    index: path[0],
                    value: Box::new(GlobalVariableData::Zero),
                };
            }
            let GlobalVariableData::Union {
                value: selected, ..
            } = data
            else {
                unreachable!()
            };
            Self::set_global_element(&child, selected, &path[1..], value);
            return;
        }
        if !matches!(data, GlobalVariableData::Arr(_)) {
            *data = GlobalVariableData::Arr(vec![]);
        }
        let GlobalVariableData::Arr(elements) = data else {
            unreachable!()
        };
        while elements.len() <= path[0] {
            elements.push(GlobalVariableData::Zero);
        }
        Self::set_global_element(&child, &mut elements[path[0]], &path[1..], value);
    }

    pub(super) fn global_initializer(&mut self, ty: &Type) -> GlobalVariableData {
        if ty.contains_vector() {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "explicit vector and vector-containing aggregate initializers are not supported",
            );
        }
        if let Some(token) = self.attempt_string_initializer(ty) {
            let mut bytes = token.decoded_string();
            let unit_size = token.string_encoding.unit_size();
            let Type::Arr(_, size) = ty.unqualified() else {
                unreachable!()
            };
            if size.is_some_and(|size| bytes.len() / unit_size > size) {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            if size.is_none_or(|size| bytes.len() / unit_size < size) {
                bytes.extend(vec![0; unit_size]);
            }
            return GlobalVariableData::Arr(
                bytes
                    .chunks(unit_size)
                    .map(|unit| {
                        GlobalVariableData::Elem(
                            unit.iter()
                                .enumerate()
                                .map(|(index, byte)| usize::from(*byte) << (index * 8))
                                .sum::<usize>()
                                .to_string(),
                        )
                    })
                    .collect(),
            );
        }
        if matches!(ty.unqualified(), Type::Arr(..) | Type::Struct(_))
            && self.attempt_reserved("{").is_some()
        {
            let mut data = GlobalVariableData::Arr(vec![]);
            let mut cursor = Some(vec![0]);
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(ty);
                    if path.is_empty() {
                        path = cursor.clone().unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                match ty.unqualified() {
                                    Type::Struct(st) if st.definition().borrow().is_union => {
                                        "too many union initializer elements"
                                    }
                                    Type::Struct(_) => "too many struct initializer elements",
                                    _ => "too many array initializer elements",
                                },
                            );
                            unreachable!()
                        });
                    }
                    let mut child = ty.clone();
                    for index in &path {
                        child = Self::aggregate_type(&child, *index).unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                if matches!(ty, Type::Struct(_)) {
                                    "too many struct initializer elements"
                                } else {
                                    "too many array initializer elements"
                                },
                            );
                            unreachable!()
                        });
                    }
                    if self.tokens[self.cur].s_value != "{" {
                        while self.string_initializer_index(&child).is_none() {
                            let Some(first) = Self::aggregate_type(&child, 0) else {
                                break;
                            };
                            path.push(0);
                            child = first;
                        }
                    }
                    let value = self.global_initializer(&child);
                    cursor = Self::next_initializer_path(ty, path.clone());
                    Self::set_global_element(ty, &mut data, &path, value);
                    if self.attempt_reserved(",").is_none() {
                        self.expect_reserved("}");
                        break;
                    }
                    if self.attempt_reserved("}").is_some() {
                        break;
                    }
                }
            }
            return data;
        }
        if self.attempt_reserved("{").is_some() {
            let value = self.global_initializer(ty);
            self.attempt_reserved(",");
            self.expect_reserved("}");
            return GlobalVariableData::Arr(vec![value]);
        }
        let expression = self.ternary();
        self.validate_storage_only_expressions(&expression, false);
        if let Some((symbol, offset)) = self.constant_address(&expression) {
            self.expect_conversion(ty, &expression);
            return GlobalVariableData::Address { symbol, offset };
        }
        if ty.float_bits() == Some(80) {
            return GlobalVariableData::Elem(self.eval_extended(&expression).to_bits().to_string());
        }
        if expression.resolve_type_view().unwrap().float_bits() == Some(80) {
            let value = self.eval_extended(&expression);
            if *ty.unqualified() == Type::Bool {
                return GlobalVariableData::Elem(i64::from(!value.is_zero()).to_string());
            }
            if let Some((bits, signed)) = ty.integer_info() {
                let integer = if signed {
                    value.to_i128(bits.into()).value as i64
                } else {
                    value.to_u128(bits.into()).value as i64
                };
                return GlobalVariableData::Elem(integer.to_string());
            }
            if let Some(bits) = ty.float_bits() {
                let value = crate::floating::to_double(crate::floating::round(value, bits));
                return GlobalVariableData::Elem((ty.float_constant(value) as i64).to_string());
            }
        }
        let value = if ty.float_bits().is_some() {
            ty.float_constant(self.eval_float(&expression)) as i64
        } else if expression
            .resolve_type()
            .is_some_and(|ty| ty.float_bits().is_some())
        {
            self.eval_float(&expression) as i64
        } else {
            self.eval(&expression)
        };
        GlobalVariableData::Elem(value.to_string())
    }

    pub(super) fn member_node(node: Node, member: &StructMember, token: &Token) -> Node {
        let registered = node.registered;
        let qualifiers = node
            .object_type()
            .map_or(Qualifiers::default(), |ty| ty.qualifiers());
        let mut member = member.clone();
        if qualifiers != Qualifiers::default() {
            member.ty = Type::Qualified(Rc::new(member.ty), qualifiers);
        }
        let address = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, node);
        let bytes = Node {
            nt: NodeType::Cast,
            token: Some(token.clone()),
            cty: Some(Type::Ptr(Rc::new(Type::I8))),
            lhs: Some(Box::new(address)),
            ..Node::default()
        };
        let offset = Node::new_with_op(
            Some(token.clone()),
            NodeType::Add,
            bytes,
            Node::new_with_num(Some(token.clone()), member.offset),
        );
        let pointer = Node {
            nt: NodeType::Cast,
            token: Some(token.clone()),
            cty: Some(Type::Ptr(Rc::new(member.ty.clone()))),
            lhs: Some(Box::new(offset)),
            ..Node::default()
        };
        let mut result = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, pointer);
        result.member_access = true;
        result.bit_field = member.bit_field;
        result.registered = registered;
        result
    }

    pub(super) fn aggregate_elements(node: &Node, token: &Token) -> Option<Vec<Node>> {
        match node.resolve_type()? {
            Type::Arr(_, size) => Some(
                (0..size.unwrap_or(0))
                    .map(|index| Self::array_element(node, index, token))
                    .collect(),
            ),
            Type::Struct(ty) => Some(
                ty.definition()
                    .borrow()
                    .members
                    .iter()
                    .filter(|member| !matches!(member.ty.unqualified(), Type::Arr(_, None)))
                    .map(|member| Self::member_node(node.clone(), member, token))
                    .collect(),
            ),
            _ => None,
        }
    }

    pub(super) fn array_element(node: &Node, index: usize, token: &Token) -> Node {
        Node::new_with_op_and_lhs(
            Some(token.clone()),
            NodeType::Deref,
            Node::new_with_op(
                Some(token.clone()),
                NodeType::Add,
                node.clone(),
                Node::new_with_num(Some(token.clone()), index),
            ),
        )
    }

    pub(super) fn zero_initializer(node: &Node, token: &Token) -> Node {
        let ty = node.object_type().unwrap();
        if matches!(ty.unqualified(), Type::Arr(..) | Type::Struct(_))
            && !ty.is_variably_modified()
            && !ty.contains_volatile_subobject()
            && ty.size_of() != 0
        {
            return Node {
                nt: NodeType::ZeroFill {
                    bytes: ty.size_of(),
                },
                token: Some(token.clone()),
                cty: Some(Type::Void),
                lhs: Some(Box::new(node.clone())),
                ..Node::default()
            };
        }
        let mut padding = vec![];
        if let Type::Struct(record) = ty.unqualified() {
            let is_union = record.definition().borrow().is_union;
            let address =
                Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, node.clone());
            let bytes = Node {
                nt: NodeType::Cast,
                token: Some(token.clone()),
                cty: Some(Type::Ptr(Rc::new(Type::I8))),
                lhs: Some(Box::new(address)),
                ..Node::default()
            };
            // Clear gaps and bit-field storage before member-wise initialization.
            // Typed member stores retain their normal volatile access widths.
            let mut ranges = if is_union {
                vec![]
            } else {
                record
                    .definition()
                    .borrow()
                    .members
                    .iter()
                    .filter(|member| member.bit_field.is_none())
                    .map(|member| (member.offset, member.offset + member.ty.size_of()))
                    .collect::<Vec<_>>()
            };
            ranges.sort_unstable();
            let mut cursor = 0;
            for (start, end) in ranges
                .into_iter()
                .chain(std::iter::once((ty.size_of(), ty.size_of())))
            {
                for index in cursor..start {
                    padding.push(Node::new_with_op(
                        Some(token.clone()),
                        NodeType::Assign,
                        Self::array_element(&bytes, index, token),
                        Node::new_with_num(Some(token.clone()), 0),
                    ));
                }
                cursor = cursor.max(end);
            }
            if is_union {
                return Node {
                    nt: NodeType::Block,
                    token: Some(token.clone()),
                    children: padding,
                    ..Node::default()
                };
            }
        }
        if let Some(elements) = Self::aggregate_elements(node, token) {
            padding.extend(
                elements
                    .iter()
                    .map(|element| Self::zero_initializer(element, token)),
            );
            Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: padding,
                ..Node::default()
            }
        } else {
            Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                node.clone(),
                Node::new_with_num(Some(token.clone()), 0),
            )
        }
    }

    pub(super) fn local_variable_initialization(
        &mut self,
        node: &Node,
        assign_token: &Token,
    ) -> Node {
        self.local_variable_initialization_with_count(node, assign_token, None)
    }

    pub(super) fn local_expression_initializer(
        &mut self,
        node: &Node,
        rhs: Node,
        token: &Token,
    ) -> Node {
        if matches!(node.resolve_type(), Some(Type::Struct(_))) {
            self.struct_assignment(node.clone(), rhs, token.clone())
        } else {
            Node::new_with_op(Some(token.clone()), NodeType::Assign, node.clone(), rhs)
        }
    }

    pub(super) fn local_variable_initialization_with_count(
        &mut self,
        node: &Node,
        assign_token: &Token,
        mut outer_count: Option<&mut usize>,
    ) -> Node {
        if node.resolve_type().is_some_and(|ty| ty.contains_vector()) {
            self.error_logger.print_error_position(
                assign_token.pos,
                "explicit vector and vector-containing aggregate initializers are not supported",
            );
        }
        if let Some(token) = self.attempt_string_initializer(&node.resolve_type().unwrap()) {
            let bytes = token.decoded_string();
            let unit_size = token.string_encoding.unit_size();
            let Type::Arr(_, size) = node.resolve_type().unwrap() else {
                unreachable!()
            };
            let size = size.unwrap_or(bytes.len() / unit_size + 1);
            if let Some(count) = outer_count.as_deref_mut() {
                *count = size;
            }
            if bytes.len() / unit_size > size {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            if size != 0 && !node.object_type().unwrap().contains_volatile_subobject() {
                let Type::Arr(element, _) = node.resolve_type().unwrap() else {
                    unreachable!()
                };
                let literal_size = bytes.len() / unit_size + 1;
                let literal = Node {
                    nt: NodeType::GlobalVar,
                    token: Some(token.clone()),
                    cty: Some(Type::Arr(element, Some(literal_size))),
                    dest: self.new_string_literal(&token),
                    ..Node::default()
                };
                let mut children = vec![];
                if size > literal_size {
                    children.push(Node {
                        nt: NodeType::ZeroFill {
                            bytes: size * unit_size,
                        },
                        token: Some(token.clone()),
                        cty: Some(Type::Void),
                        lhs: Some(Box::new(node.clone())),
                        ..Node::default()
                    });
                }
                children.push(Node {
                    nt: NodeType::CopyBytes {
                        bytes: size.min(literal_size) * unit_size,
                    },
                    token: Some(token.clone()),
                    cty: Some(Type::Void),
                    lhs: Some(Box::new(node.clone())),
                    rhs: Some(Box::new(literal)),
                    ..Node::default()
                });
                return Node {
                    nt: NodeType::Block,
                    token: Some(token),
                    children,
                    ..Node::default()
                };
            }
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: (0..size)
                    .map(|index| {
                        Node::new_with_op(
                            Some(token.clone()),
                            NodeType::Assign,
                            Self::array_element(node, index, &token),
                            Node::new_with_num(
                                Some(token.clone()),
                                (0..unit_size)
                                    .map(|byte| {
                                        usize::from(
                                            bytes
                                                .get(index * unit_size + byte)
                                                .copied()
                                                .unwrap_or(0),
                                        ) << (byte * 8)
                                    })
                                    .sum(),
                            ),
                        )
                    })
                    .collect(),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("{") {
            if !matches!(node.resolve_type(), Some(Type::Arr(..) | Type::Struct(_))) {
                let initializer = self.local_variable_initialization(node, assign_token);
                self.attempt_reserved(",");
                self.expect_reserved("}");
                return initializer;
            }
            let mut children = vec![Self::zero_initializer(node, assign_token)];
            let mut selected_unions = HashMap::new();
            let ty = node.resolve_type().unwrap();
            let mut cursor = Some(vec![0]);
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(&node.resolve_type().unwrap());
                    if path.is_empty() {
                        path = cursor.clone().unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                token.pos,
                                "too many aggregate initializer elements",
                            );
                            unreachable!()
                        });
                    }
                    let mut child = Self::initializer_subobject(&ty, &path).unwrap_or_else(|| {
                        self.error_logger.print_error_position(
                            token.pos,
                            "too many aggregate initializer elements",
                        );
                        unreachable!()
                    });
                    let rhs = if self.tokens[self.cur].s_value == "{"
                        || self.string_initializer_index(&child).is_some()
                    {
                        None
                    } else {
                        let rhs = self.assign();
                        while !(matches!(child.unqualified(), Type::Struct(_))
                            && rhs
                                .resolve_type()
                                .is_some_and(|source| child.compatible(&source)))
                        {
                            let Some(first) = Self::aggregate_type(&child, 0) else {
                                break;
                            };
                            path.push(0);
                            child = first;
                        }
                        Some(rhs)
                    };
                    if let Some(count) = outer_count.as_deref_mut() {
                        *count = (*count).max(path[0] + 1);
                    }
                    cursor = Self::next_initializer_path(&ty, path.clone());
                    let mut target = node.clone();
                    let mut prefix = vec![];
                    for index in path {
                        let union = matches!(target.resolve_type(), Some(Type::Struct(st)) if st.definition().borrow().is_union);
                        let selected = match target.resolve_type().unwrap() {
                            Type::Arr(_, size) if size.is_none_or(|size| index < size) => {
                                Some(Self::array_element(&target, index, token))
                            }
                            _ => Self::aggregate_elements(&target, token)
                                .and_then(|targets| targets.get(index).cloned()),
                        };
                        target = selected.unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                token.pos,
                                "too many aggregate initializer elements",
                            );
                            unreachable!()
                        });
                        if union && selected_unions.insert(prefix.clone(), index) != Some(index) {
                            children.push(Self::zero_initializer(&target, assign_token));
                        }
                        prefix.push(index);
                    }
                    children.push(if let Some(rhs) = rhs {
                        self.local_expression_initializer(&target, rhs, assign_token)
                    } else {
                        self.local_variable_initialization(&target, assign_token)
                    });
                    if self.attempt_reserved(",").is_none() {
                        self.expect_reserved("}");
                        break;
                    }
                    if self.attempt_reserved("}").is_some() {
                        break;
                    }
                }
            }
            Node {
                nt: NodeType::Block,
                token: Some(assign_token.clone()),
                children,
                ..Node::default()
            }
        } else {
            let rhs = self.assign();
            self.local_expression_initializer(node, rhs, assign_token)
        }
    }

    pub(super) fn struct_assignment(&mut self, lhs: Node, rhs: Node, token: Token) -> Node {
        let ty = lhs.resolve_type().unwrap();
        self.expect_object_type(&ty, &token);
        if rhs.resolve_type() != Some(ty.clone()) {
            self.error_logger
                .print_error_position(token.pos, "incompatible struct assignment");
        }
        if ty.size_of() != 0
            && !lhs.object_type().unwrap().contains_volatile_subobject()
            && !rhs.object_type().unwrap().contains_volatile_subobject()
        {
            return Node {
                nt: NodeType::CopyBytes {
                    bytes: ty.size_of(),
                },
                token: Some(token),
                cty: Some(ty),
                lhs: Some(Box::new(lhs)),
                rhs: Some(Box::new(rhs)),
                ..Node::default()
            };
        }
        let mut children = vec![];
        let mut addresses = vec![];
        for operand in [lhs, rhs] {
            self.offset_size = (self.offset_size + 8).div_ceil(8) * 8;
            let temporary = Node {
                nt: NodeType::LocalVar,
                token: Some(token.clone()),
                cty: Some(Type::Ptr(Rc::new(operand.object_type().unwrap()))),
                offset: Some(self.offset_size),
                ..Node::default()
            };
            children.push(Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                temporary.clone(),
                Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, operand),
            ));
            addresses.push(temporary);
        }
        for index in 0..ty.size_of() {
            let bytes = addresses
                .iter()
                .map(|address| {
                    let qualifiers = address.dest_type().unwrap().qualifiers();
                    let byte_type = if qualifiers == Qualifiers::default() {
                        Type::I8
                    } else {
                        Type::Qualified(Rc::new(Type::I8), qualifiers)
                    };
                    let pointer = Node {
                        nt: NodeType::Cast,
                        token: Some(token.clone()),
                        cty: Some(Type::Ptr(Rc::new(byte_type))),
                        lhs: Some(Box::new(address.clone())),
                        ..Node::default()
                    };
                    Self::array_element(&pointer, index, &token)
                })
                .collect::<Vec<_>>();
            children.push(Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                bytes[0].clone(),
                bytes[1].clone(),
            ));
        }
        children.push(Node::new_with_op_and_lhs(
            Some(token.clone()),
            NodeType::Deref,
            addresses.remove(0),
        ));
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }

    pub(super) fn initializer_expressions(node: Node, expressions: &mut Vec<Node>) {
        if matches!(node.nt, NodeType::DefVar | NodeType::Block) {
            for child in node.children {
                Self::initializer_expressions(child, expressions);
            }
        } else {
            expressions.push(node);
        }
    }
}

use super::AstBuilderImpl;
use super::{AutomaticStorage, DeclarationAttributes, Identifier};
use crate::ctype::{Qualifiers, Type};
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use crate::token::{Token, TokenType};
use std::rc::Rc;

struct GlobalDeclaration {
    base: Type,
    alignment: usize,
    internal: bool,
    external: bool,
    inline: bool,
    thread_local: bool,
    attributes: DeclarationAttributes,
}

impl AstBuilderImpl<'_> {
    pub(super) fn new_local_variable(&mut self, ty: Type, alignment: usize) -> (Node, bool) {
        let mut alignments = vec![alignment];
        let (token, ty) = self.expect_ident_with_type(ty, Some(&mut alignments));
        let alignment = alignments.into_iter().max().unwrap_or(0);
        if matches!(ty, Type::Arr(_, None)) {
            if self.tokens[self.cur].s_value != "=" {
                self.error_logger.print_error_position(
                    token.pos,
                    "incomplete automatic array requires an initializer",
                );
            }
            (
                self.register_incomplete_local_array(token, ty, alignment),
                true,
            )
        } else {
            (
                self.register_local_variable_aligned(token, ty, alignment),
                false,
            )
        }
    }

    pub(super) fn register_local_variable(&mut self, t: Token, ty: Type) -> Node {
        self.register_local_variable_aligned(t, ty, 0)
    }

    pub(super) fn register_local_variable_aligned(
        &mut self,
        t: Token,
        ty: Type,
        requested_alignment: usize,
    ) -> Node {
        let alignment = ty.align_of().max(requested_alignment);
        if alignment > 16 {
            self.error_logger.print_error_position(
                t.pos,
                "requested local alignment exceeds the supported stack alignment",
            );
        }
        if ty.is_variable_array() {
            self.expect_object_type(&ty, &t);
            self.offset_size = self.offset_size.div_ceil(8) * 8 + 8;
            if self
                .scope_stack
                .last()
                .unwrap()
                .contains_key(t.s_value.as_str())
            {
                self.error_logger
                    .print_error_position(t.pos, "invalid redeclaration");
            }
            self.scope_stack.last_mut().unwrap().insert(
                t.s_value.to_string(),
                Identifier::Dynamic(ty.clone(), self.offset_size, false),
            );
            return Self::dynamic_node(ty, self.offset_size, t);
        }
        self.expect_object_type(&ty, &t);
        // Distinct internal slots preserve each zero-sized object's type and alignment.
        self.offset_size += ty.size_of().max(1);
        self.offset_size = self.offset_size.div_ceil(alignment) * alignment;
        if self
            .scope_stack
            .last()
            .unwrap()
            .contains_key(t.s_value.as_str())
        {
            self.error_logger
                .print_error_position(t.pos, "invalid redeclaration");
        }
        self.scope_stack.last_mut().unwrap().insert(
            t.s_value.to_string(),
            Identifier::Local(ty.clone(), self.offset_size, false, alignment),
        );
        Node {
            nt: NodeType::LocalVar,
            token: Some(t),
            cty: Some(ty),
            offset: Some(self.offset_size),
            alignment,
            ..Node::default()
        }
    }

    pub(super) fn mark_registered(&mut self, node: &mut Node) {
        let name = &node.token.as_ref().unwrap().s_value;
        match self
            .scope_stack
            .last_mut()
            .unwrap()
            .get_mut(name.as_str())
            .unwrap()
        {
            Identifier::Local(_, _, registered, _) | Identifier::Dynamic(_, _, registered) => {
                *registered = true
            }
            _ => unreachable!(),
        }
        node.registered = true;
    }

    pub(super) fn typedef_definition(&mut self) {
        let saved_attributes = self.declaration_attributes_state.take();
        self.declaration_attributes_state = Some(DeclarationAttributes {
            typedef_declarator: true,
            ..DeclarationAttributes::default()
        });
        let prefix_alignments = self.declaration_attributes(true, None, false);
        let prefix_attributes = self.declaration_attributes_state.take().unwrap();
        let base = self.expect_type();
        loop {
            self.declaration_attributes_state = Some(prefix_attributes.clone());
            let mut alignments = prefix_alignments.clone();
            let (name, mut ty, _, _, _) = self
                .declarator_with_alignment(Some(&mut alignments))
                .resolve(base.clone());
            let token = name.unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "ident expected");
                unreachable!()
            });
            let attributes = self.declaration_attributes_state.take().unwrap();
            if attributes.word_mode {
                let Some((_, signed)) = ty
                    .integer_info()
                    .filter(|_| *ty.unqualified() != Type::Bool)
                else {
                    self.error_logger
                        .print_error_position(token.pos, "word mode requires an integer typedef");
                    unreachable!()
                };
                let qualifiers = ty.qualifiers();
                ty = Type::integer(64, signed);
                if qualifiers != Qualifiers::default() {
                    ty = Type::Qualified(Rc::new(ty), qualifiers);
                }
            }
            if attributes.transparent_union {
                self.apply_transparent_union(&ty, &token);
            }
            if let Some(size) = attributes.vector_size {
                let element = ty.unqualified();
                if !(element.integer_info().is_some() && *element != Type::Bool
                    || matches!(element, Type::Float(32 | 64)))
                    || size % element.size_of().max(1) != 0
                {
                    self.error_logger.print_error_position(token.pos, "vector_size requires an ordinary integer or float scalar typedef with a matching element size");
                }
                let qualifiers = ty.qualifiers();
                let vector = Type::Vector(Rc::new(element.clone()), size / element.size_of());
                ty = if qualifiers == Qualifiers::default() {
                    vector
                } else {
                    Type::Qualified(Rc::new(vector), qualifiers)
                };
            }
            self.validate_type_layout(&ty, &token);
            if let Some(alignment) = alignments.into_iter().max() {
                if !ty.is_complete_object() || ty.function_signature_ref().is_some() {
                    self.error_logger.print_error_position(
                        token.pos,
                        "typedef alignment requires a complete object type",
                    );
                }
                if alignment != ty.align_of() {
                    let raised_record = matches!(ty.unqualified(), Type::Struct(_))
                        && alignment > ty.natural_align_of()
                        && alignment <= 16;
                    if !raised_record
                        && (alignment > ty.natural_align_of()
                            || !(ty.integer_info().is_some()
                                || matches!(ty.unqualified(), Type::Float(32 | 64) | Type::Ptr(_))))
                    {
                        self.error_logger.print_error_position(token.pos, "non-natural typedef alignment is supported only for scalar alignment reductions");
                    }
                    ty = Type::Aligned(Rc::new(ty), alignment);
                }
            }
            if let Some(previous) = self.scope_stack.last().unwrap().get(token.s_value.as_str()) {
                let valid = matches!(previous, Identifier::TypeDef(previous)
                    if !previous.is_variably_modified()
                        && !ty.is_variably_modified()
                        && previous.qualifiers() == ty.qualifiers()
                        && previous.align_of() == ty.align_of()
                        && previous.identical(&ty));
                if !valid {
                    self.error_logger
                        .print_error_position(token.pos, "incompatible typedef redeclaration");
                }
            }
            if self.return_type.is_none() && ty.is_variably_modified() {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified typedef requires block scope",
                );
            }
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value.to_string(), Identifier::TypeDef(ty));
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        self.declaration_attributes_state = saved_attributes;
    }

    pub(super) fn attempt_static_assert(&mut self) -> bool {
        let Some(token) = self.attempt_reserved("_Static_assert") else {
            return false;
        };
        self.expect_reserved("(");
        let expression = self.ternary();
        if !Self::integer_constant(&expression)
            || expression
                .resolve_type()
                .is_none_or(|ty| ty.integer_info().is_none())
        {
            self.error_logger.print_error_position(
                token.pos,
                "static assertion requires an integer constant expression",
            );
        }
        self.expect_reserved(",");
        let message = self.attempt(TokenType::Str).unwrap_or_else(|| {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "static assertion requires a string literal message",
            );
            unreachable!()
        });
        self.expect_reserved(")");
        self.expect_reserved(";");
        if self.eval(&expression) == 0 {
            let bytes = message.decoded_string();
            self.error_logger.print_error_position(
                token.pos,
                &format!(
                    "static assertion failed: {}",
                    String::from_utf8_lossy(&bytes)
                ),
            );
        }
        true
    }

    pub(super) fn attempt_thread_local(&mut self) -> bool {
        self.attempt_reserved("_Thread_local")
            .or_else(|| self.attempt_reserved("__thread"))
            .is_some()
    }

    fn global_declaration_specifiers(&mut self) -> Option<GlobalDeclaration> {
        while self.attempt_reserved("__extension__").is_some() {}
        if self.attempt_static_assert() {
            return None;
        }
        // Attribute collection belongs to this declaration header, not its body or parameters.
        self.declaration_attributes_state = Some(DeclarationAttributes::default());
        self.offset_size = 0;
        self.pending_bounds.clear();
        self.vla_active.clear();
        if let Some(token) = self
            .attempt_reserved("auto")
            .or_else(|| self.attempt_reserved("register"))
        {
            self.error_logger.print_error_position(
                token.pos,
                "automatic storage class requires block or parameter scope",
            );
        }
        let mut storage: Option<Token> = None;
        let mut inline = false;
        let mut qualifiers = Qualifiers::default();
        let mut alignment = 0;
        let mut thread_local = false;
        loop {
            alignment = alignment.max(self.object_alignment_annotations());
            if self.attempt_thread_local() {
                thread_local = true;
            } else if self.attempt_inline().is_some() {
                inline = true;
            } else if let Some(value) = self.attempt_alignment_specifier() {
                alignment = alignment.max(value);
            } else if let Some(token) = self
                .attempt_reserved("static")
                .or_else(|| self.attempt_reserved("extern"))
            {
                if storage.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting storage classes");
                } else {
                    storage = Some(token.clone());
                }
            } else {
                let q = self.attempt_qualifiers();
                if q == Qualifiers::default() {
                    break;
                }
                qualifiers.constant |= q.constant;
                qualifiers.volatile |= q.volatile;
                qualifiers.restricted |= q.restricted;
            }
        }
        let internal = storage.as_ref().is_some_and(|s| s.s_value == "static");
        let external = storage.as_ref().is_some_and(|s| s.s_value == "extern");
        if self.attempt_reserved("typedef").is_some() {
            if self
                .declaration_attributes_state
                .take()
                .is_some_and(|a| a.gnu_inline || a.noinline || a.weak_import || a.common)
            {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "declaration linkage attributes cannot be applied to a typedef",
                );
            }
            if thread_local {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "thread-local storage cannot be applied to a typedef",
                );
            }
            if alignment != 0 {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "_Alignas cannot be applied to a typedef",
                );
            }
            self.typedef_definition();
            self.expect_reserved(";");
            return None;
        }
        if storage
            .as_ref()
            .is_some_and(|storage| storage.s_value == "static")
            && external
        {
            self.error_logger
                .print_error_position(self.tokens[self.cur - 1].pos, "conflicting storage classes");
        }
        let mut base = self.expect_type();
        alignment = alignment.max(self.object_alignment_annotations());
        if qualifiers != Qualifiers::default() {
            base = Type::Qualified(Rc::new(base), qualifiers);
        }
        inline |= self.attempt_inline().is_some();
        if self.attempt_reserved(";").is_some() {
            if alignment != 0 {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "alignment requires an object declaration",
                );
            }
            if self
                .declaration_attributes_state
                .take()
                .is_some_and(|a| a.gnu_inline || a.noinline || a.weak_import || a.common)
            {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "declaration attributes require a declarator",
                );
            }
            if thread_local {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "thread-local storage requires an object declaration",
                );
            }
            if inline {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "inline requires a function declaration",
                );
            }
            return None;
        }
        let attributes = self.declaration_attributes_state.take().unwrap_or_default();
        Some(GlobalDeclaration {
            base,
            alignment,
            internal,
            external,
            inline,
            thread_local,
            attributes,
        })
    }

    pub(super) fn global_definition(&mut self) {
        let Some(GlobalDeclaration {
            base,
            alignment,
            internal,
            external,
            inline,
            thread_local,
            attributes: inherited_attributes,
        }) = self.global_declaration_specifiers()
        else {
            return;
        };
        loop {
            self.declaration_attributes_state = Some(inherited_attributes.clone());
            let mut alignments = vec![alignment];
            let (name, mut ty, parameters, parameter_scope, stars) = self
                .declarator_with_alignment(Some(&mut alignments))
                .resolve(base.clone());
            let alignment = alignments.into_iter().max().unwrap_or(0);
            let token = name.unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "ident expected");
                unreachable!()
            });
            self.assembler_label(&token);
            let attributes = self.declaration_attributes_state.take().unwrap_or_default();
            if attributes.noinline && ty.function_signature_ref().is_none() {
                self.error_logger
                    .print_error_position(token.pos, "noinline requires a function declaration");
            }
            if attributes.gnu_inline && (!inline || ty.function_signature_ref().is_none()) {
                self.error_logger.print_error_position(
                    token.pos,
                    "gnu_inline requires an inline function declaration",
                );
            }
            if attributes.common
                && (ty.function_signature_ref().is_some() || attributes.weak_import)
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "common requires an object declaration without weak_import",
                );
            }
            self.validate_type_layout(&ty, &token);
            if ty.is_variably_modified() {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified declaration requires block scope",
                );
            }
            if inline && ty.function_signature_ref().is_none() {
                self.error_logger
                    .print_error_position(token.pos, "inline requires a function declaration");
            }
            if ty.function_signature_ref().is_some() {
                if alignment != 0 {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Alignas cannot be applied to a function",
                    );
                }
                if thread_local {
                    self.error_logger.print_error_position(
                        token.pos,
                        "thread-local storage cannot be applied to a function",
                    );
                }
                if self.tokens[self.cur].s_value == "{" && !stars.is_empty() {
                    self.error_logger.print_error_position(
                        stars[0].pos,
                        "unspecified array bound is only allowed in a function prototype",
                    );
                }
                if self.global_variables.contains_key(token.s_value.as_str()) {
                    self.error_logger
                        .print_error_position(token.pos, "function conflicts with object");
                }
                let definition = self.tokens[self.cur].s_value == "{";
                self.scope_stack.push(parameter_scope);
                let mut args = vec![];
                if definition {
                    for (i, (name, ty, registered)) in parameters.into_iter().enumerate() {
                        let name = name.unwrap_or_else(|| Token {
                            s_value: format!(".parameter.{i}").into(),
                            ..token.clone()
                        });
                        let mut argument = self.register_local_variable(name, ty);
                        if registered {
                            self.mark_registered(&mut argument);
                        }
                        args.push(argument);
                    }
                }
                self.scope_stack
                    .first_mut()
                    .unwrap()
                    .insert(token.s_value.to_string(), Identifier::Static(ty.clone()));
                if let Some(existing) = self.functions.get(token.s_value.as_str()) {
                    if !existing.cty.compatible(&ty) {
                        self.error_logger
                            .print_error_position(token.pos, "incompatible function redeclaration");
                    }
                }
                self.functions
                    .entry(token.s_value.to_string())
                    .or_insert_with(|| Func {
                        token: Some(token.clone()),
                        cty: ty.clone(),
                        ..Func::default()
                    });
                let internal = internal
                    || self
                        .functions
                        .get(token.s_value.as_str())
                        .is_some_and(|f| f.internal);
                let previous = &self.functions[token.s_value.as_str()];
                if attributes.gnu_inline && previous.body.is_some() && !previous.gnu_inline {
                    self.error_logger.print_error_position(
                        token.pos,
                        "gnu_inline must be declared before the function definition",
                    );
                }
                let gnu_inline = attributes.gnu_inline || previous.gnu_inline;
                let noinline = attributes.noinline || previous.noinline;
                let replaceable_inline_body = previous.gnu_inline && previous.inline_only();
                let weak_import = attributes.weak_import || previous.weak_import;
                let hidden = attributes.hidden || previous.hidden;
                if weak_import && (internal || definition || previous.body.is_some()) {
                    self.error_logger.print_error_position(token.pos, "weak_import requires an external function declaration without a definition");
                }
                let external_definition = if gnu_inline {
                    if !definition && previous.body.is_some() {
                        previous.external_definition
                    } else {
                        internal || !inline || !external
                    }
                } else {
                    internal || external || !inline || previous.external_definition
                };
                let function = self.functions.get_mut(token.s_value.as_str()).unwrap();
                function.external_definition = external_definition;
                function.noinline = noinline;
                function.hidden = hidden;
                if !definition {
                    if function.body.is_none() {
                        *function = Func {
                            token: Some(token),
                            cty: ty,
                            offset_size: self.offset_size,
                            internal,
                            external_definition,
                            weak_import,
                            hidden,
                            gnu_inline,
                            noinline,
                            ..Func::default()
                        };
                    }
                    self.pending_bounds.clear();
                    self.vla_active.clear();
                    self.scope_stack.pop();
                    self.expect_reserved(";");
                    return;
                }
                let body = self.parse_function_body(&token, &ty, &args);
                if self.functions[token.s_value.as_str()].body.is_some() {
                    if body.is_some() {
                        if !replaceable_inline_body || !external_definition {
                            self.error_logger
                                .print_error_position(token.pos, "function is already defined");
                        }
                    } else {
                        return;
                    }
                }
                self.functions.insert(token.s_value.to_string(), {
                    let inline_definition = inline && body.is_some();
                    let mutable_static = self.mutable_static.take();
                    Func {
                        token: Some(token),
                        cty: ty,
                        args,
                        body,
                        mutable_static,
                        offset_size: self.offset_size,
                        return_pointer: self.return_pointer,
                        internal,
                        external_definition,
                        weak_import,
                        hidden,
                        inline_definition,
                        gnu_inline,
                        noinline,
                    }
                });
                return;
            }
            if let Some(previous) = self.global_variables.get(token.s_value.as_str()) {
                ty = self.composite_object_declaration(&previous.ty, ty, &token);
            }
            if self.functions.contains_key(token.s_value.as_str()) {
                self.error_logger
                    .print_error_position(token.pos, "object conflicts with function");
            }
            // The object's scope begins after its declarator, before its initializer.
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value.to_string(), Identifier::Static(ty.clone()));
            self.global_variables
                .entry(token.s_value.to_string())
                .or_insert_with(|| GlobalVariable {
                    ty: ty.clone(),
                    alignment,
                    data: None,
                    internal,
                    declaration_only: external,
                    common: attributes.common,
                    weak_import: attributes.weak_import,
                    hidden: attributes.hidden,
                    thread_local,
                });
            let data = if self.attempt_reserved("=").is_some() {
                Some(self.global_initializer(&ty))
            } else {
                None
            };
            let common = attributes.common
                || self
                    .global_variables
                    .get(token.s_value.as_str())
                    .is_some_and(|object| object.common);
            let weak_import = attributes.weak_import
                || self
                    .global_variables
                    .get(token.s_value.as_str())
                    .is_some_and(|object| object.weak_import);
            if weak_import
                && (common
                    || !external
                    || internal
                    || thread_local
                    || data.is_some()
                    || self
                        .global_variables
                        .get(token.s_value.as_str())
                        .is_some_and(|object| !object.declaration_only))
            {
                self.error_logger.print_error_position(token.pos, "weak_import requires an external non-TLS object declaration without a definition");
            }
            let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                elements.len()
            } else {
                0
            };
            let ty = if external && data.is_none() && Self::incomplete_external_object(&ty) {
                ty
            } else {
                let ty = self.infer_array_size(ty, count, &token);
                self.expect_object_type(&ty, &token);
                ty
            };
            if let Some(data) = &data {
                self.validate_global_initializer(&ty, data, &token);
            }
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value.to_string(), Identifier::Static(ty.clone()));
            {
                let existing = self
                    .global_variables
                    .get_mut(token.s_value.as_str())
                    .unwrap();
                if existing.internal != internal && !external {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting object linkage");
                }
                if !external || data.is_some() {
                    existing.declaration_only = false;
                }
                if existing.ty != ty {
                    existing.ty = ty.clone();
                }
                existing.alignment = existing.alignment.max(alignment);
                existing.weak_import = weak_import;
                existing.hidden |= attributes.hidden;
                existing.common = common;
                if existing.thread_local != thread_local {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting thread storage duration");
                }
                if data.is_some() && existing.data.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "object is already initialized");
                }
                if data.is_some() {
                    existing.data = data;
                }
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        self.expect_reserved(";");
    }

    pub(super) fn incomplete_external_object(ty: &Type) -> bool {
        match ty.unqualified() {
            Type::Arr(child, None) => child.is_complete_object(),
            Type::Struct(record) => !record.definition().borrow().complete,
            _ => false,
        }
    }

    pub(super) fn composite_object_declaration(
        &self,
        previous: &Type,
        declared: Type,
        token: &Token,
    ) -> Type {
        if previous == &declared {
            return declared;
        }
        if previous.qualifiers() == declared.qualifiers() {
            if let (Type::Arr(left, left_count), Type::Arr(right, right_count)) =
                (previous.unqualified(), declared.unqualified())
            {
                if left.qualifiers() == right.qualifiers()
                    && left.compatible(right)
                    && (left_count.is_none() || right_count.is_none())
                {
                    return if right_count.is_none() {
                        previous.clone()
                    } else {
                        declared
                    };
                }
            }
        }
        if previous.qualifiers() == declared.qualifiers() && previous.compatible(&declared) {
            return declared;
        }
        self.error_logger
            .print_error_position(token.pos, "incompatible object redeclaration");
        unreachable!()
    }

    pub(super) fn resolve_name(&self, s: &str) -> Option<&Identifier> {
        self.scope_stack.iter().rev().find_map(|map| map.get(s))
    }

    pub(super) fn local_static_definition(
        &mut self,
        base: Type,
        storage: &Token,
        inline: bool,
        requested_alignment: usize,
        thread_local: bool,
    ) -> Node {
        let mut bounds = vec![];
        loop {
            let mut alignments = vec![requested_alignment];
            let (token, mut ty) = self.expect_ident_with_type(base.clone(), Some(&mut alignments));
            let requested_alignment = alignments.into_iter().max().unwrap_or(0);
            if ty.is_variable_array() || (storage.s_value == "extern" && ty.is_variably_modified())
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified static or extern declaration is invalid",
                );
            }
            bounds.extend(self.take_bounds());
            let external = storage.s_value == "extern";
            let name = if external {
                token.s_value.to_string()
            } else {
                format!(".static.{}.{}", token.pos, token.s_value)
            };
            if ty.function_signature_ref().is_some() {
                if thread_local {
                    self.error_logger.print_error_position(
                        token.pos,
                        "thread-local storage cannot be applied to a function",
                    );
                }
                if requested_alignment != 0 {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Alignas cannot be applied to a function",
                    );
                }
                if !external {
                    self.error_logger.print_error_position(
                        token.pos,
                        "block static function declaration is invalid",
                    );
                }
                if let Some(existing) = self.functions.get(&name) {
                    if !existing.cty.compatible(&ty) {
                        self.error_logger
                            .print_error_position(token.pos, "incompatible function redeclaration");
                    }
                } else {
                    self.functions.insert(
                        name.clone(),
                        Func {
                            token: Some(token.clone()),
                            cty: ty.clone(),
                            ..Func::default()
                        },
                    );
                }
                self.scope_stack.last_mut().unwrap().insert(
                    token.s_value.to_string(),
                    Identifier::StaticObject(ty, name),
                );
                if self.attempt_reserved(",").is_none() {
                    break;
                }
                continue;
            }
            if inline {
                self.error_logger
                    .print_error_position(token.pos, "inline requires a function declaration");
            }
            if !external && !ty.is_const_object() && self.mutable_static.is_none() {
                self.mutable_static = Some(token.clone());
            }
            if external {
                if let Some(previous) = self.global_variables.get(&name) {
                    ty = self.composite_object_declaration(&previous.ty, ty, &token);
                }
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
            self.scope_stack.last_mut().unwrap().insert(
                token.s_value.to_string(),
                Identifier::StaticObject(ty.clone(), name.clone()),
            );
            let new_object = !self.global_variables.contains_key(&name);
            self.global_variables
                .entry(name.clone())
                .or_insert_with(|| GlobalVariable {
                    ty: ty.clone(),
                    alignment: requested_alignment,
                    data: None,
                    internal: !external,
                    declaration_only: external,
                    common: false,
                    weak_import: false,
                    hidden: false,
                    thread_local,
                });
            let data = if self.attempt_reserved("=").is_some() {
                if external {
                    self.error_logger
                        .print_error_position(token.pos, "block extern cannot have an initializer");
                }
                Some(self.global_initializer(&ty))
            } else {
                None
            };
            let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                elements.len()
            } else {
                0
            };
            if !(external && data.is_none() && Self::incomplete_external_object(&ty)) {
                ty = self.infer_array_size(ty, count, &token);
                self.expect_object_type(&ty, &token);
            }
            if let Some(data) = &data {
                self.validate_global_initializer(&ty, data, &token);
            }
            self.scope_stack.last_mut().unwrap().insert(
                token.s_value.to_string(),
                Identifier::StaticObject(ty.clone(), name.clone()),
            );
            {
                let existing = self.global_variables.get_mut(&name).unwrap();
                existing.alignment = existing.alignment.max(requested_alignment);
                if existing.ty != ty {
                    if external || new_object {
                        existing.ty = ty.clone();
                    } else {
                        self.error_logger
                            .print_error_position(token.pos, "incompatible object redeclaration");
                    }
                }
                if existing.thread_local != thread_local {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting thread storage duration");
                }
                if new_object {
                    existing.data = data;
                }
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        Node {
            nt: NodeType::Block,
            children: bounds,
            ..Node::default()
        }
    }

    pub fn local_variable_definition(&mut self, ty: Type) -> Node {
        self.local_variable_definition_with_storage(ty, AutomaticStorage::Default, 0)
    }

    pub(super) fn local_variable_definition_with_storage(
        &mut self,
        ty: Type,
        storage: AutomaticStorage,
        alignment: usize,
    ) -> Node {
        let mut vec = Vec::new();
        loop {
            let (mut node, inferred) = self.new_local_variable(ty.clone(), alignment);
            if storage != AutomaticStorage::Default
                && node
                    .resolve_type()
                    .unwrap()
                    .function_signature_ref()
                    .is_some()
            {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "automatic storage class requires an object declaration",
                );
            }
            if storage == AutomaticStorage::Register {
                self.mark_registered(&mut node);
            }
            vec.extend(self.take_bounds());
            if node.resolve_type().unwrap().is_variable_array() {
                if self.tokens[self.cur].s_value == "=" {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "variable length array cannot have an initializer",
                    );
                }
                vec.push(self.allocate_variable_array(&node));
            }
            if let Some(token) = self.attempt_reserved("=") {
                // if initializer element exists, push into AST
                vec.push(if inferred {
                    self.inferred_local_array_initializer(&node, token).1
                } else {
                    self.local_variable_initialization(&node, token)
                });
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        Node {
            nt: NodeType::DefVar,
            children: vec,
            ..Node::default()
        }
    }
}

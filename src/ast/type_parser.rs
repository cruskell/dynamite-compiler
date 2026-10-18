use super::AstBuilderImpl;
use super::{AutomaticStorage, Identifier};
use crate::ctype::{
    BitField, EnumType, Qualifiers, StructDefinition, StructMember, StructType, Type,
};
use crate::token::{Token, TokenType};
use std::cell::RefCell;
use std::rc::Rc;

impl AstBuilderImpl<'_> {
    pub(super) fn attempt_automatic_type(
        &mut self,
        parameter: bool,
    ) -> Option<(Type, AutomaticStorage, usize)> {
        let saved = self.cur;
        let mut storage: Option<Token> = None;
        let mut qualifiers = Qualifiers::default();
        let mut alignment = 0;
        loop {
            if !parameter {
                alignment = alignment.max(self.object_alignment_annotations());
            }
            if let Some(value) = self.attempt_alignment_specifier() {
                alignment = alignment.max(value);
            } else if let Some(token) = self
                .attempt_reserved("auto")
                .or_else(|| self.attempt_reserved("register"))
            {
                if storage.is_some() || (parameter && token.s_value == "auto") {
                    self.error_logger
                        .print_error_position(token.pos, "invalid automatic storage class");
                }
                storage = Some(token.clone());
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
        let Some(mut ty) = self.attempt_type() else {
            if storage.is_some() {
                self.error_logger.print_error_position(
                    self.tokens[self.cur].pos,
                    "type expected after storage class",
                );
            }
            self.cur = saved;
            return None;
        };
        if !parameter {
            alignment = alignment.max(self.object_alignment_annotations());
        }
        while let Some(value) = self.attempt_alignment_specifier() {
            alignment = alignment.max(value);
        }
        if let Some(token) = self
            .attempt_reserved("auto")
            .or_else(|| self.attempt_reserved("register"))
        {
            if storage.is_some() || (parameter && token.s_value == "auto") {
                self.error_logger
                    .print_error_position(token.pos, "invalid automatic storage class");
            }
            storage = Some(token.clone());
        }
        if qualifiers != Qualifiers::default() {
            ty = Type::Qualified(Rc::new(ty), qualifiers);
        }
        if parameter && *ty.unqualified() == Type::Void {
            if let Some(token) = &storage {
                self.error_logger
                    .print_error_position(token.pos, "storage class requires an object parameter");
            }
        }
        let storage = match storage.as_ref().map(|token| token.s_value.as_str()) {
            Some("register") => AutomaticStorage::Register,
            Some("auto") => AutomaticStorage::Auto,
            _ => AutomaticStorage::Default,
        };
        if storage == AutomaticStorage::Register && alignment != 0 {
            self.error_logger.print_error_position(
                self.tokens[saved].pos,
                "_Alignas cannot be applied to a register object",
            );
        }
        if parameter && alignment != 0 {
            self.error_logger.print_error_position(
                self.tokens[saved].pos,
                "_Alignas cannot be applied to a parameter",
            );
        }
        Some((ty, storage, alignment))
    }

    pub(super) fn attempt_type(&mut self) -> Option<Type> {
        let saved = self.cur;
        while self.attempt_reserved("__extension__").is_some() {}
        let mut nullability = None;
        let mut q = self.qualifiers_with_nullability(&mut nullability);
        let Some(ty) = self.attempt_unqualified_type(&mut q) else {
            self.cur = saved;
            return None;
        };
        let suffix = if matches!(ty.unqualified(), Type::Ptr(_)) {
            self.qualifiers_with_nullability(&mut nullability)
        } else {
            let qualifiers = self.attempt_qualifiers();
            if nullability.is_some() || self.nullability().is_some() {
                let position = if nullability.is_some() {
                    saved
                } else {
                    self.cur
                };
                self.error_logger.print_error_position(
                    self.tokens[position].pos,
                    "nullability requires a pointer type",
                );
            }
            qualifiers
        };
        q.constant |= suffix.constant;
        q.volatile |= suffix.volatile;
        q.restricted |= suffix.restricted;
        if q.restricted
            && !matches!(ty.unqualified(), Type::Ptr(pointee) if pointee.function_signature_ref().is_none())
        {
            self.error_logger.print_error_position(
                self.tokens[saved].pos,
                "restrict requires a pointer to an object or void",
            );
        }
        Some(if q == Qualifiers::default() {
            ty
        } else {
            Type::Qualified(Rc::new(ty), q)
        })
    }

    pub(super) fn long_double_type(&self, token: &Token) -> Type {
        if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            return Type::LongDouble(64);
        }
        if cfg!(target_arch = "x86_64") {
            return Type::LongDouble(80);
        }
        if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
            return Type::LongDouble(128);
        }
        self.error_logger
            .print_error_position(token.pos, "unsupported long double storage format");
        unreachable!()
    }

    pub(super) fn typeof_type(&mut self, token: Token) -> Type {
        self.expect_reserved("(");
        let pending_start = self.pending_bounds.len();
        let (ty, expression) = if let Some(base) = self.attempt_type() {
            let (name, ty, _, _, _) = self.declarator().resolve(base);
            if name.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "typeof requires a type name or expression");
            }
            (ty, None)
        } else {
            let expression = self.expr();
            self.validate_void_expressions(&expression);
            if expression.bit_field.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "typeof cannot be applied to a bit-field");
            }
            (
                expression.resolve_qualified_type().unwrap(),
                Some(expression),
            )
        };
        self.expect_reserved(")");
        if ty.is_variably_modified() {
            if let Some(expression) = expression {
                self.pending_bounds.push(expression);
            }
        } else {
            self.pending_bounds.truncate(pending_start);
        }
        ty
    }

    pub(super) fn attempt_unqualified_type(&mut self, qualifiers: &mut Qualifiers) -> Option<Type> {
        if self.tokens[self.cur].tt != TokenType::Reserved {
            if self.tokens[self.cur].tt == TokenType::Ident {
                let name = self.tokens[self.cur].s_value.clone();
                if let Some(Identifier::TypeDef(ty)) = self.resolve_name(&name) {
                    let ty = ty.clone();
                    self.cur += 1;
                    return Some(ty);
                }
            }
            return None;
        }
        if let Some(token) = self
            .attempt_reserved("typeof")
            .or_else(|| self.attempt_reserved("__typeof"))
            .or_else(|| self.attempt_reserved("__typeof__"))
        {
            return Some(self.typeof_type(token.clone()));
        }
        if let Some(token) = self.attempt_reserved("enum") {
            return Some(self.enum_type(token.clone()));
        }
        if let Some(token) = self
            .attempt_reserved("struct")
            .or_else(|| self.attempt_reserved("union"))
        {
            return Some(self.struct_type(token.clone()));
        }
        let mut specifiers = vec![];
        loop {
            let q = self.attempt_qualifiers();
            qualifiers.constant |= q.constant;
            qualifiers.volatile |= q.volatile;
            qualifiers.restricted |= q.restricted;
            let token = &self.tokens[self.cur];
            if token.tt != TokenType::Reserved
                || !crate::tokenizer::TYPES.contains(&token.s_value.as_str())
                || matches!(token.s_value.as_str(), "typeof" | "__typeof" | "__typeof__")
            {
                break;
            }
            specifiers.push(token.clone());
            self.cur += 1;
        }
        if specifiers.is_empty() {
            return None;
        }
        let count = |name: &str| {
            specifiers
                .iter()
                .filter(|token| token.s_value == name)
                .count()
        };
        let signs = count("signed") + count("unsigned");
        if signs > 1 {
            self.error_logger
                .print_error_position(specifiers[0].pos, "duplicate signedness");
        }
        let signed = count("unsigned") == 0;
        let ints = count("int");
        let longs = count("long");
        let others = specifiers.len() - signs - ints - longs;
        let ty = if others == 0 && ints <= 1 && longs <= 2 {
            Some(if longs == 2 {
                Type::LongLong { signed }
            } else {
                Type::integer(if longs == 1 { 64 } else { 32 }, signed)
            })
        } else if others == 1 && count("char") == 1 && ints == 0 && longs == 0 {
            Some(if signed && signs != 0 {
                Type::SignedChar
            } else {
                Type::integer(8, signed)
            })
        } else if others == 1 && count("short") == 1 && ints <= 1 && longs == 0 {
            Some(Type::integer(16, signed))
        } else if others == 1 && count("__int128") == 1 && ints == 0 && longs == 0 {
            Some(Type::Integer128 { signed })
        } else if others == 1 && count("double") == 1 && ints == 0 && signs == 0 && longs <= 1 {
            Some(if longs == 1 {
                self.long_double_type(&specifiers[0])
            } else {
                Type::Float(64)
            })
        } else if specifiers.len() == 1 {
            match specifiers[0].s_value.as_str() {
                "void" => Some(Type::Void),
                "bool" | "_Bool" => Some(Type::Bool),
                "float" => Some(Type::Float(32)),
                "_Float16" => Some(Type::Float16),
                "__int128_t" => Some(Type::Integer128 { signed: true }),
                "__uint128_t" => Some(Type::Integer128 { signed: false }),
                _ => None,
            }
        } else {
            None
        };
        if ty.is_none() {
            self.error_logger
                .print_error_position(specifiers[0].pos, "invalid combination of type specifiers");
        }
        ty
    }

    pub(super) fn enum_integer_type(&self, minimum: i128, maximum: i128, token: &Token) -> Type {
        if minimum >= i128::from(i32::MIN) && maximum <= i128::from(i32::MAX) {
            Type::I32
        } else if minimum >= 0 && maximum <= i128::from(u32::MAX) {
            Type::integer(32, false)
        } else if minimum >= i128::from(i64::MIN) && maximum <= i128::from(i64::MAX) {
            Type::integer(64, minimum < 0)
        } else if minimum >= 0 && maximum <= i128::from(u64::MAX) {
            Type::integer(64, false)
        } else {
            self.error_logger.print_error_position(
                token.pos,
                "enumerator range exceeds supported integer types",
            );
            unreachable!()
        }
    }

    pub(super) fn enum_type(&mut self, token: Token) -> Type {
        let attributes = self.declaration_attributes_state.take();
        let result = self.enum_type_body(token);
        self.declaration_attributes_state = attributes;
        result
    }

    pub(super) fn enum_type_body(&mut self, token: Token) -> Type {
        self.declaration_annotations();
        let name = self.attempt_ident();
        self.declaration_annotations();
        let key = name.as_ref().map(|name| format!(".enum.{}", name.s_value));
        // A generic association also places ':' immediately after an enum type name.
        let underlying_follows = self.tokens.get(self.cur + 1).is_some_and(|token| {
            crate::tokenizer::TYPES.contains(&token.s_value.as_str())
                || matches!(
                    token.s_value.as_str(),
                    "struct"
                        | "union"
                        | "enum"
                        | "const"
                        | "volatile"
                        | "restrict"
                        | "__restrict"
                        | "__restrict__"
                        | "__attribute__"
                )
                || matches!(
                    self.resolve_name(&token.s_value),
                    Some(Identifier::TypeDef(_))
                )
        });
        let underlying = if self.tokens[self.cur].s_value == ":" && underlying_follows {
            self.cur += 1;
            let position = self.tokens[self.cur].pos;
            let ty = self.expect_type().unqualified().clone();
            if ty.integer_info().is_none() || matches!(ty, Type::Enum(_)) {
                self.error_logger.print_error_position(
                    position,
                    "enum underlying type must be a supported integer type",
                );
            }
            Some(ty)
        } else {
            None
        };
        let defining = self.tokens[self.cur].s_value == "{";
        let previous = key.as_ref().and_then(|key| {
            let binding = if defining || underlying.is_some() {
                self.scope_stack.last().unwrap().get(key)
            } else {
                self.resolve_name(key)
            };
            if let Some(Identifier::EnumTag { ty, fixed, defined }) = binding {
                Some((ty.clone(), *fixed, *defined))
            } else {
                None
            }
        });
        if !defining && underlying.is_none() {
            return previous.map(|(ty, _, _)| ty).unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(token.pos, "enum requires a known tag or definition");
                unreachable!()
            });
        }
        if !defining && name.is_none() {
            self.error_logger
                .print_error_position(token.pos, "enum declaration requires a tag or definition");
        }
        let fixed = underlying.is_some();
        let previously_defined = previous.as_ref().is_some_and(|(_, _, defined)| *defined);
        let ty = underlying.unwrap_or(Type::I32);
        let enum_type = if let Some((previous, previous_fixed, defined)) = previous {
            if defining && defined {
                self.error_logger
                    .print_error_position(token.pos, "enum tag is already defined");
            }
            if previous_fixed != fixed {
                self.error_logger.print_error_position(
                    token.pos,
                    "enum redeclaration changes whether its underlying type is fixed",
                );
            }
            let Type::Enum(value) = &previous else {
                unreachable!()
            };
            if *value.underlying.borrow() != ty {
                self.error_logger.print_error_position(
                    token.pos,
                    "enum redeclaration has a different underlying type",
                );
            }
            previous
        } else {
            let value = Type::Enum(EnumType {
                id: self.next_enum_id,
                underlying: Rc::new(RefCell::new(ty.clone())),
            });
            self.next_enum_id += 1;
            value
        };
        if let Some(key) = &key {
            self.scope_stack.last_mut().unwrap().insert(
                key.clone(),
                Identifier::EnumTag {
                    ty: enum_type.clone(),
                    fixed,
                    defined: defining || previously_defined,
                },
            );
        }
        if !defining {
            return enum_type;
        }
        self.expect_reserved("{");
        let (bits, signed) = ty.integer_info().unwrap();
        let (minimum, maximum) = if !fixed {
            (i128::from(i64::MIN), i128::from(u64::MAX))
        } else if ty == Type::Bool {
            (0, 1)
        } else if signed {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        } else {
            (0, (1i128 << bits) - 1)
        };
        let mut value = 0i128;
        let mut smallest = i128::MAX;
        let mut largest = i128::MIN;
        let mut enumerators = vec![];
        loop {
            let name = self.attempt_ident().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "enumerator expected");
                unreachable!()
            });
            self.declaration_annotations();
            if self.attempt_reserved("=").is_some() {
                let expression = self.ternary();
                if !Self::integer_constant(&expression) {
                    self.error_logger.print_error_position(
                        name.pos,
                        "enumerator requires an integer constant expression",
                    );
                }
                let result = self.eval(&expression);
                value = if expression
                    .resolve_type()
                    .unwrap()
                    .integer_info()
                    .is_some_and(|(_, signed)| !signed)
                {
                    i128::from(result as u64)
                } else {
                    i128::from(result)
                };
            }
            if value < minimum || value > maximum {
                self.error_logger.print_error_position(
                    name.pos,
                    "enumerator is outside the underlying type range",
                );
            }
            if self
                .scope_stack
                .last()
                .unwrap()
                .contains_key(name.s_value.as_str())
            {
                self.error_logger
                    .print_error_position(name.pos, "duplicate enumerator");
            }
            smallest = smallest.min(value);
            largest = largest.max(value);
            let constant_type = if fixed {
                enum_type.clone()
            } else {
                self.enum_integer_type(value, value, &name)
            };
            enumerators.push((name.s_value.clone(), value as i64));
            self.scope_stack.last_mut().unwrap().insert(
                name.s_value.to_string(),
                Identifier::Constant(value as i64, constant_type),
            );
            value += 1;
            if self.attempt_reserved("}").is_some() {
                break;
            }
            self.expect_reserved(",");
            if self.attempt_reserved("}").is_some() {
                break;
            }
        }
        if fixed {
            return enum_type;
        }
        let ty = self.enum_integer_type(smallest, largest, &token);
        let scope = self.scope_stack.last_mut().unwrap();
        for (name, value) in enumerators {
            scope.insert(name.to_string(), Identifier::Constant(value, ty.clone()));
        }
        if let Some(key) = key {
            scope.insert(
                key,
                Identifier::EnumTag {
                    ty: enum_type.clone(),
                    fixed: false,
                    defined: true,
                },
            );
        }
        let Type::Enum(value) = &enum_type else {
            unreachable!()
        };
        // The supported LP64 ABIs use unsigned int for nonnegative 32-bit enums.
        // Enumerator constants keep their separately selected source-language type.
        *value.underlying.borrow_mut() = if smallest >= 0 && largest <= i128::from(u32::MAX) {
            Type::integer(32, false)
        } else {
            ty
        };
        enum_type
    }

    pub(super) fn struct_type(&mut self, token: Token) -> Type {
        let attributes = self.declaration_attributes_state.take();
        let result = self.struct_type_body(token);
        self.declaration_attributes_state = attributes;
        result
    }

    pub(super) fn struct_type_body(&mut self, token: Token) -> Type {
        let is_union = token.s_value == "union";
        let packing = token.packing.map(usize::from);
        let (mut requested_record_alignment, mut packed, mut transparent) =
            self.record_attributes();
        let name = self.attempt_ident();
        self.declaration_attributes(false, None, true);
        let key = format!(
            ".struct.{}",
            name.as_ref()
                .map(|name| name.s_value.clone())
                .unwrap_or_else(|| format!("anonymous.{}", token.pos).into())
        );
        let defining = self.tokens[self.cur].s_value == "{";
        let existing = if defining {
            self.scope_stack.last().unwrap().get(&key)
        } else {
            self.scope_stack
                .iter()
                .rev()
                .find_map(|scope| scope.get(&key))
        };
        let ty = if let Some(Identifier::TypeDef(Type::Struct(ty))) = existing {
            ty.clone()
        } else {
            if name.is_none() && !defining {
                self.error_logger
                    .print_error_position(token.pos, "struct requires a tag or definition");
            }
            let (ty, definition) = StructType::new(self.struct_definitions.len());
            definition.borrow_mut().is_union = is_union;
            self.struct_definitions.push(definition);
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(key, Identifier::TypeDef(Type::Struct(ty.clone())));
            ty
        };
        if ty.definition().borrow().complete && requested_record_alignment != 0 {
            self.error_logger.print_error_position(
                token.pos,
                "alignment on an existing record reference is not supported",
            );
        }
        if ty.definition().borrow().is_union != is_union {
            self.error_logger
                .print_error_position(token.pos, "aggregate tag has a different kind");
        }
        let inherited_record_alignment = ty.definition().borrow().align;
        if self.attempt_reserved("{").is_some() {
            if ty.definition().borrow().complete {
                self.error_logger
                    .print_error_position(token.pos, "struct tag is already defined");
            }
            let mut members: Vec<StructMember> = vec![];
            let mut size = 0usize;
            let mut bit_address = 0usize;
            let mut has_bit_fields = false;
            let mut has_flexible_array = false;
            let mut align = ty.definition().borrow().align.max(1);
            let mut natural_align = 1;
            while self.attempt_reserved("}").is_none() {
                if self.attempt_static_assert() {
                    continue;
                }
                if has_flexible_array && !is_union {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "flexible array member must be last",
                    );
                }
                let mut requested_alignment = 0;
                while let Some(value) = self.attempt_alignment_specifier() {
                    requested_alignment = requested_alignment.max(value);
                }
                let base = self.expect_type();
                while let Some(value) = self.attempt_alignment_specifier() {
                    requested_alignment = requested_alignment.max(value);
                }
                loop {
                    let mut member_alignments = vec![];
                    let (name, member_type, _, _, _) = self
                        .declarator_with_alignment(Some(&mut member_alignments))
                        .resolve(base.clone());
                    let requested_alignment = requested_alignment
                        .max(member_alignments.iter().copied().max().unwrap_or(0));
                    let position = name.as_ref().unwrap_or(&token);
                    self.validate_type_layout(&member_type, position);
                    if member_type.is_variably_modified() {
                        self.error_logger.print_error_position(
                            position.pos,
                            "struct member cannot have a variably modified type",
                        );
                    }
                    let flexible = matches!(member_type.unqualified(), Type::Arr(_, None));
                    let member_alignment = member_type
                        .align_of()
                        .max(requested_alignment)
                        .min(packing.unwrap_or(usize::MAX));
                    if flexible {
                        if is_union || members.is_empty() || name.is_none() {
                            self.error_logger.print_error_position(position.pos,
                                "flexible array requires a named final member after another struct member");
                        }
                        self.validate_type_layout(&member_type, position);
                        has_flexible_array = true;
                    } else {
                        self.expect_object_type(&member_type, position);
                        if member_type.has_flexible_array() {
                            if !is_union {
                                self.error_logger.print_error_position(
                                    position.pos,
                                    "struct member cannot contain a flexible array member",
                                );
                            }
                            has_flexible_array = true;
                        }
                    }
                    let (field, offset) = if self.attempt_reserved(":").is_some() {
                        if !member_alignments.is_empty() {
                            self.error_logger.print_error_position(
                                position.pos,
                                "alignment attributes on bit-fields are not supported",
                            );
                        }
                        has_bit_fields = true;
                        let expression = self.ternary();
                        let width = self.eval(&expression);
                        let Some((bits, _)) = member_type.integer_info() else {
                            self.error_logger.print_error_position(
                                position.pos,
                                "bit-field requires an integer type",
                            );
                            unreachable!()
                        };
                        if width < 0
                            || width > i64::from(bits)
                            || (width == 0 && name.is_some())
                            || (*member_type.unqualified() == Type::Bool && width > 1)
                        {
                            self.error_logger
                                .print_error_position(position.pos, "invalid bit-field width");
                        }
                        let width = width as u8;
                        let container = usize::from(bits);
                        if is_union {
                            bit_address = 0;
                        }
                        if requested_alignment > member_type.align_of() {
                            bit_address =
                                bit_address.div_ceil(member_alignment * 8) * member_alignment * 8;
                            align = align.max(member_alignment);
                            natural_align = natural_align.max(member_alignment);
                        }
                        if width == 0
                            || (packing.is_none()
                                && usize::from(width) > container - bit_address % container)
                        {
                            bit_address = bit_address.div_ceil(container) * container;
                        }
                        // AAPCS64 zero-width fields retain natural alignment on Linux.
                        if name.is_some() || cfg!(target_arch = "aarch64") {
                            let field_alignment = if width == 0
                                && cfg!(all(target_arch = "aarch64", target_os = "linux"))
                            {
                                member_type.align_of()
                            } else {
                                member_alignment
                            };
                            align = align.max(field_alignment);
                            natural_align = natural_align.max(member_type.align_of());
                        }
                        let bytewise = packing.is_some();
                        let field = BitField {
                            width,
                            shift: (bit_address % if bytewise { 8 } else { container }) as u8,
                            bytewise,
                        };
                        let offset = if bytewise {
                            bit_address / 8
                        } else {
                            bit_address / container * member_type.size_of()
                        };
                        bit_address += usize::from(width);
                        size = size.max(bit_address.div_ceil(8));
                        (field, offset)
                    } else {
                        if name.is_none() && !matches!(member_type.unqualified(), Type::Struct(_)) {
                            self.error_logger
                                .print_error_position(token.pos, "member name expected");
                        }
                        size = size.div_ceil(member_alignment) * member_alignment;
                        align = align.max(member_alignment);
                        natural_align = natural_align.max(member_alignment);
                        let offset = if is_union { 0 } else { size };
                        size = if is_union {
                            size.max(member_type.size_of())
                        } else {
                            size + member_type.size_of()
                        };
                        bit_address = size * 8;
                        (
                            BitField {
                                width: 0,
                                shift: 0,
                                bytewise: false,
                            },
                            offset,
                        )
                    };
                    if name.is_some() || matches!(member_type.unqualified(), Type::Struct(_)) {
                        let anonymous = name.is_none();
                        let visible = if anonymous {
                            let Type::Struct(child) = member_type.unqualified() else {
                                unreachable!()
                            };
                            child
                                .visible_members()
                                .into_iter()
                                .map(|(name, _)| name)
                                .collect::<Vec<_>>()
                        } else {
                            vec![name.as_ref().unwrap().s_value.to_string()]
                        };
                        for existing in &members {
                            let existing_names = if existing.anonymous {
                                let Type::Struct(child) = existing.ty.unqualified() else {
                                    unreachable!()
                                };
                                child
                                    .visible_members()
                                    .into_iter()
                                    .map(|(name, _)| name)
                                    .collect::<Vec<_>>()
                            } else {
                                vec![existing.name.clone()]
                            };
                            if visible.iter().any(|name| existing_names.contains(name)) {
                                self.error_logger
                                    .print_error_position(position.pos, "duplicate struct member");
                            }
                        }
                        members.push(StructMember {
                            name: name.map_or_else(
                                || format!(".anonymous.{}", members.len()),
                                |name| name.s_value.to_string(),
                            ),
                            anonymous,
                            requested_alignment,
                            ty: member_type,
                            offset,
                            bit_field: (field.width != 0).then_some(field),
                        });
                    }
                    if self.attempt_reserved(",").is_none() {
                        break;
                    }
                    if flexible {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur].pos,
                            "flexible array member must be last",
                        );
                    }
                }
                self.expect_reserved(";");
            }
            if members.is_empty() && size == 0 {
                self.error_logger
                    .print_error_position(token.pos, "struct requires at least one member");
            }
            *ty.definition().borrow_mut() = StructDefinition {
                members,
                is_union,
                transparent_union: false,
                has_bit_fields,
                has_flexible_array,
                size: size.div_ceil(align) * align,
                align,
                natural_align,
                packing,
                complete: true,
            };
        }
        if defining {
            let (alignment, trailing_packed, trailing_transparent) = self.record_attributes();
            transparent |= trailing_transparent;
            requested_record_alignment = requested_record_alignment.max(alignment);
            packed |= trailing_packed;
        } else {
            self.declaration_attributes(false, None, true);
        }
        if packed {
            if !defining {
                self.error_logger.print_error_position(
                    token.pos,
                    "packed attribute requires a record definition",
                );
            }
            let definition = ty.definition();
            let mut definition = definition.borrow_mut();
            if definition.has_bit_fields {
                self.error_logger.print_error_position(
                    token.pos,
                    "GNU packed records with bit-fields are not supported",
                );
            }
            let mut size = 0usize;
            let mut align = 1;
            for member in &mut definition.members {
                // GNU packing lowers natural alignment; explicit member alignment may raise it.
                let alignment = member
                    .requested_alignment
                    .max(1)
                    .min(packing.unwrap_or(usize::MAX));
                align = align.max(alignment);
                member.offset = if is_union {
                    0
                } else {
                    size.div_ceil(alignment) * alignment
                };
                size = size.max(member.offset + member.ty.size_of());
            }
            definition.natural_align = align;
            definition.align = align.max(inherited_record_alignment);
            definition.size = size.div_ceil(definition.align) * definition.align;
            definition.packing = Some(1);
        }
        if requested_record_alignment != 0 {
            let definition = ty.definition();
            let mut definition = definition.borrow_mut();
            definition.align = definition.align.max(requested_record_alignment);
            definition.size = definition.size.div_ceil(definition.align) * definition.align;
        }
        let ty = Type::Struct(ty);
        if transparent {
            self.apply_transparent_union(&ty, &token);
        }
        ty
    }

    pub(super) fn expect_type(&mut self) -> Type {
        if let Some(ty) = self.attempt_type() {
            ty
        } else {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "type expected");
            unreachable!()
        }
    }
}

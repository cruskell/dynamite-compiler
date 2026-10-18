use super::AstBuilderImpl;
use super::METADATA_ATTRIBUTES;
use super::{DeclarationAttributes, Nullability};
use crate::ctype::{Qualifiers, Type};
use crate::token::{Token, TokenType};

impl AstBuilderImpl<'_> {
    pub(super) fn assembler_label(&mut self, name: &Token) {
        if !matches!(
            self.tokens[self.cur].s_value.as_str(),
            "asm" | "__asm" | "__asm__"
        ) {
            return;
        }
        self.cur += 1;
        self.expect_reserved("(");
        let label = self.attempt(TokenType::Str).unwrap_or_else(|| {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "assembler label requires a string literal",
            );
            unreachable!()
        });
        let value = std::str::from_utf8(&label.string_bytes).unwrap_or("");
        if label.string_encoding != crate::token::StringEncoding::Narrow
            || value.is_empty()
            || value == "."
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.$".contains(&byte))
        {
            self.error_logger.print_error_position(
                label.pos,
                "assembler label requires an identifier-like symbol",
            );
        }
        self.expect_reserved(")");
        if let Some(previous) = self.symbol_names.get(name.s_value.as_str()) {
            if previous != value {
                self.error_logger
                    .print_error_position(label.pos, "conflicting assembler labels");
            }
        }
        self.symbol_names
            .insert(name.s_value.to_string(), value.to_owned());
        self.declaration_annotations();
    }

    pub(super) fn declaration_annotations(&mut self) {
        self.declaration_attributes(false, None, false);
    }

    pub(super) fn declaration_attributes(
        &mut self,
        allow_alignment: bool,
        mut packed: Option<&mut bool>,
        stop_before_alignment: bool,
    ) -> Vec<usize> {
        let mut alignments = vec![];
        while matches!(self.tokens[self.cur].tt, TokenType::Ident)
            && matches!(
                self.tokens[self.cur].s_value.as_str(),
                "__attribute__" | "__attribute"
            )
        {
            // Qualifier parsing leaves semantic alignment attributes to the declaration owner.
            if stop_before_alignment {
                let mut depth = 0usize;
                let mut aligned = false;
                for token in &self.tokens[self.cur + 1..] {
                    match token.s_value.as_str() {
                        "(" => depth += 1,
                        ")" if depth > 0 => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ if depth == 2 && token.s_value.trim_matches('_') == "aligned" => {
                            aligned = true
                        }
                        _ => {}
                    }
                }
                if aligned {
                    break;
                }
            }
            let token = self.tokens[self.cur].clone();
            self.cur += 1;
            self.expect_reserved("(");
            self.expect_reserved("(");
            while self.attempt_reserved(")").is_none() {
                if self.attempt_reserved(",").is_some() {
                    continue;
                }
                let name = self.tokens[self.cur].clone();
                if !matches!(name.tt, TokenType::Reserved | TokenType::Ident) {
                    self.error_logger
                        .print_error_position(name.pos, "attribute name expected");
                }
                if name.s_value.trim_matches('_') == "noinline" {
                    if self.parameter_depth != 0
                        || self.declaration_attributes_state.is_none()
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|a| a.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "noinline requires a file-scope function declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "noinline does not take arguments");
                    }
                    self.declaration_attributes_state.as_mut().unwrap().noinline = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "gnu_inline" {
                    if self.parameter_depth != 0
                        || self.declaration_attributes_state.is_none()
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "gnu_inline requires a file-scope inline function declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "gnu_inline does not take arguments");
                    }
                    self.declaration_attributes_state
                        .as_mut()
                        .unwrap()
                        .gnu_inline = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if matches!(name.s_value.trim_matches('_'), "weak_import" | "weak") {
                    if self.parameter_depth != 0
                        || self.declaration_attributes_state.is_none()
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "weak_import requires a file-scope declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "weak_import does not take arguments");
                    }
                    self.declaration_attributes_state
                        .as_mut()
                        .unwrap()
                        .weak_import = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "visibility" {
                    self.cur += 1;
                    self.expect_reserved("(");
                    let visibility = self.attempt(TokenType::Str).unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(name.pos, "visibility requires a string literal");
                        unreachable!()
                    });
                    if visibility.string_encoding.unit_size() != 1
                        || !matches!(visibility.string_bytes.as_slice(), b"default" | b"hidden")
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "only default and hidden symbol visibility are supported",
                        );
                    }
                    if visibility.string_bytes.as_slice() == b"hidden" {
                        if self.parameter_depth != 0
                            || self
                                .declaration_attributes_state
                                .as_ref()
                                .is_none_or(|attributes| attributes.typedef_declarator)
                        {
                            self.error_logger.print_error_position(
                                name.pos,
                                "hidden visibility requires a file-scope symbol declaration",
                            );
                        }
                        self.declaration_attributes_state.as_mut().unwrap().hidden = true;
                    }
                    self.expect_reserved(")");
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "transparent_union" {
                    if packed.is_none()
                        && !self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "transparent_union requires a union type or typedef",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger.print_error_position(
                            name.pos,
                            "transparent_union does not take arguments",
                        );
                    }
                    self.declaration_attributes_state
                        .get_or_insert_with(DeclarationAttributes::default)
                        .transparent_union = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "common" {
                    if self.parameter_depth != 0
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_none_or(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "common requires a file-scope object declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "common does not take arguments");
                    }
                    self.declaration_attributes_state.as_mut().unwrap().common = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "mode" {
                    if !self
                        .declaration_attributes_state
                        .as_ref()
                        .is_some_and(|a| a.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "mode is supported only on integer typedefs",
                        );
                    }
                    self.cur += 1;
                    self.expect_reserved("(");
                    let mode = &self.tokens[self.cur];
                    if mode.s_value.trim_matches('_') != "word" {
                        self.error_logger
                            .print_error_position(mode.pos, "only word integer mode is supported");
                    }
                    self.cur += 1;
                    self.expect_reserved(")");
                    self.declaration_attributes_state
                        .as_mut()
                        .unwrap()
                        .word_mode = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "vector_size" {
                    if self.parameter_depth != 0
                        || !self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "vector_size is supported only on scalar typedef declarators",
                        );
                    }
                    self.cur += 1;
                    self.expect_reserved("(");
                    let mut attributes = self.declaration_attributes_state.take().unwrap();
                    let size = self.ternary();
                    if !Self::integer_constant(&size) {
                        self.error_logger.print_error_position(
                            name.pos,
                            "vector size requires an integer constant expression",
                        );
                    }
                    let size = self.eval(&size);
                    if size <= 0 || size > 16 || !(size as usize).is_power_of_two() {
                        self.error_logger.print_error_position(
                            name.pos,
                            "supported vector sizes are powers of two through 16 bytes",
                        );
                    }
                    self.expect_reserved(")");
                    let previous = attributes.vector_size.replace(size as usize);
                    self.declaration_attributes_state = Some(attributes);
                    if previous.is_some_and(|previous| previous != size as usize) {
                        self.error_logger
                            .print_error_position(name.pos, "conflicting vector sizes");
                    }
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "may_alias"
                    && self.tokens[self.cur + 1].s_value == "("
                {
                    self.error_logger
                        .print_error_position(name.pos, "may_alias does not take arguments");
                }
                let supported = METADATA_ATTRIBUTES.contains(&name.s_value.trim_matches('_'));
                if name.s_value.trim_matches('_') == "packed" {
                    if let Some(packed) = packed.as_deref_mut() {
                        self.cur += 1;
                        if self.tokens[self.cur].s_value == "(" {
                            self.error_logger.print_error_position(
                                name.pos,
                                "packed attribute does not take arguments",
                            );
                        }
                        *packed = true;
                        if self.tokens[self.cur].s_value != ")" {
                            self.expect_reserved(",");
                        }
                        continue;
                    }
                }
                if allow_alignment && name.s_value.trim_matches('_') == "aligned" {
                    self.cur += 1;
                    let value = if self.attempt_reserved("(").is_some() {
                        let expression = self.ternary();
                        if !Self::integer_constant(&expression) {
                            self.error_logger.print_error_position(
                                name.pos,
                                "alignment requires an integer constant expression",
                            );
                        }
                        let value = self.eval(&expression);
                        self.expect_reserved(")");
                        value
                    } else {
                        16
                    };
                    if value <= 0 || !(value as usize).is_power_of_two() || value > 4096 {
                        self.error_logger.print_error_position(
                            name.pos,
                            "alignment must be a power of two no greater than 4096",
                        );
                    }
                    alignments.push(value as usize);
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if !supported {
                    self.error_logger
                        .print_error_position(name.pos, "unsupported declaration attribute");
                }
                self.cur += 1;
                if self.attempt_reserved("(").is_some() {
                    let mut depth = 1;
                    while depth != 0 {
                        if self.tokens[self.cur].tt == TokenType::EOF {
                            self.error_logger.print_error_position(
                                token.pos,
                                "unterminated attribute arguments",
                            );
                        }
                        if self.attempt_reserved("(").is_some() {
                            depth += 1;
                        } else if self.attempt_reserved(")").is_some() {
                            depth -= 1;
                        } else {
                            self.cur += 1;
                        }
                    }
                }
                if self.tokens[self.cur].s_value != ")" {
                    self.expect_reserved(",");
                }
            }
            self.expect_reserved(")");
        }
        alignments
    }

    pub(super) fn object_alignment_annotations(&mut self) -> usize {
        let mut alignment = 0;
        loop {
            alignment = alignment.max(
                self.declaration_attributes(true, None, false)
                    .into_iter()
                    .max()
                    .unwrap_or(0),
            );
            let Some(value) = self.attempt_alignment_specifier() else {
                break;
            };
            alignment = alignment.max(value);
        }
        alignment
    }

    pub(super) fn record_attributes(&mut self) -> (usize, bool, bool) {
        let mut packed = false;
        let alignment = self
            .declaration_attributes(true, Some(&mut packed), false)
            .into_iter()
            .max()
            .unwrap_or(0);
        let transparent = self
            .declaration_attributes_state
            .as_mut()
            .is_some_and(|attributes| std::mem::take(&mut attributes.transparent_union));
        (alignment, packed, transparent)
    }

    pub(super) fn nullability(&self) -> Option<Nullability> {
        match self.tokens[self.cur].s_value.as_str() {
            "_Nullable" => Some(Nullability::Nullable),
            "_Nonnull" => Some(Nullability::Nonnull),
            "_Null_unspecified" => Some(Nullability::Unspecified),
            "_Nullable_result" => Some(Nullability::NullableResult),
            _ => None,
        }
    }

    pub(super) fn pointer_qualifiers(&mut self) -> Qualifiers {
        self.qualifiers_with_nullability(&mut None)
    }

    pub(super) fn qualifiers_with_nullability(
        &mut self,
        nullability: &mut Option<Nullability>,
    ) -> Qualifiers {
        let mut qualifiers = Qualifiers::default();
        loop {
            let current = self.attempt_qualifiers();
            qualifiers.constant |= current.constant;
            qualifiers.volatile |= current.volatile;
            qualifiers.restricted |= current.restricted;
            let Some(annotation) = self.nullability() else {
                break;
            };
            if nullability.is_some_and(|previous| previous != annotation) {
                self.error_logger.print_error_position(
                    self.tokens[self.cur].pos,
                    "conflicting pointer nullability annotations",
                );
            }
            *nullability = Some(annotation);
            self.cur += 1;
        }
        qualifiers
    }

    pub(super) fn attempt_qualifiers(&mut self) -> Qualifiers {
        let mut q = Qualifiers::default();
        loop {
            self.declaration_attributes(false, None, true);
            if self.attempt_reserved("const").is_some() {
                q.constant = true;
            } else if self.attempt_reserved("volatile").is_some() {
                q.volatile = true;
            } else if self.attempt_reserved("restrict").is_some() {
                q.restricted = true;
            } else {
                break;
            }
        }
        q
    }

    pub(super) fn attempt_alignment_specifier(&mut self) -> Option<usize> {
        let token = self.attempt_reserved("_Alignas")?;
        self.expect_reserved("(");
        let alignment = if let Some(base) = self.attempt_type() {
            let (name, ty, _, _, _) = self.declarator().resolve(base);
            if name.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "_Alignas requires a type name");
            }
            if matches!(
                ty.unqualified(),
                Type::Void | Type::Func(..) | Type::VariadicFunc(..)
            ) || !ty.is_complete_object()
            {
                self.error_logger
                    .print_error_position(token.pos, "_Alignas requires a complete object type");
            }
            ty.align_of()
        } else {
            let expression = self.ternary();
            if !Self::integer_constant(&expression) {
                self.error_logger.print_error_position(
                    token.pos,
                    "_Alignas requires an integer constant expression",
                );
            }
            let value = self.eval(&expression);
            if value < 0 {
                self.error_logger
                    .print_error_position(token.pos, "_Alignas value cannot be negative");
            }
            value as usize
        };
        self.expect_reserved(")");
        if alignment != 0 && !alignment.is_power_of_two() {
            self.error_logger
                .print_error_position(token.pos, "_Alignas value must be zero or a power of two");
        }
        Some(alignment)
    }
}

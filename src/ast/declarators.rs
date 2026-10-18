use super::AstBuilderImpl;
use super::{AutomaticStorage, Declarator, Identifier};
use crate::ctype::{Qualifiers, Type};
use crate::node::{Node, NodeType};
use crate::token::Token;
use std::collections::HashMap;
use std::rc::Rc;

impl AstBuilderImpl<'_> {
    pub(super) fn declarator(&mut self) -> Declarator {
        self.declarator_with_alignment(None)
    }

    pub(super) fn declarator_with_alignment(
        &mut self,
        mut alignments: Option<&mut Vec<usize>>,
    ) -> Declarator {
        if let Some(alignments) = alignments.as_deref_mut() {
            alignments.extend(self.declaration_attributes(true, None, false));
        } else {
            self.declaration_annotations();
        }
        let mut pointers = vec![];
        while self.attempt_reserved("*").is_some() {
            pointers.push(self.pointer_qualifiers());
        }
        let mut direct = if self.attempt_reserved("(").is_some() {
            let inner = self.declarator_with_alignment(alignments.as_deref_mut());
            self.expect_reserved(")");
            inner
        } else {
            Declarator::Name(self.attempt_ident())
        };
        loop {
            if let Some(alignments) = alignments.as_deref_mut() {
                alignments.extend(self.declaration_attributes(true, None, false));
            } else {
                self.declaration_annotations();
            }
            if self.attempt_reserved("[").is_some() {
                let mut minimum = self.attempt_reserved("static");
                let qualifiers = self.attempt_qualifiers();
                if minimum.is_none() {
                    minimum = self.attempt_reserved("static");
                }
                if minimum.is_some() || qualifiers != Qualifiers::default() {
                    if self.parameter_depth == 0 || !matches!(direct, Declarator::Name(_)) {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur].pos,
                            "array parameter modifiers require an outer parameter array",
                        );
                    }
                    direct = Declarator::ArrayQualifiers(Box::new(direct), qualifiers);
                }
                if let Some(star) = self.attempt_reserved("*") {
                    if self.parameter_depth == 0 || minimum.is_some() {
                        self.error_logger.print_error_position(
                            star.pos,
                            "unspecified array bound requires a function prototype parameter",
                        );
                    }
                    self.expect_reserved("]");
                    direct = Declarator::UnspecifiedArray(Box::new(direct), star.clone());
                    continue;
                }
                if minimum.is_some() && self.tokens[self.cur].s_value == "]" {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "static array parameter requires a bound",
                    );
                }
                if self.attempt_reserved("]").is_some() {
                    if matches!(
                        direct,
                        Declarator::Array(..) | Declarator::VariableArray(..)
                    ) {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur - 1].pos,
                            "only the outer array dimension may be omitted",
                        );
                    }
                    direct = Declarator::Array(Box::new(direct), None);
                } else {
                    let token = self.tokens[self.cur].clone();
                    let expression = self.assign();
                    self.expect_reserved("]");
                    if Self::integer_constant(&expression) {
                        let size = self.eval(&expression);
                        if size < 0 || (size == 0 && minimum.is_some()) {
                            self.error_logger
                                .print_error_position(token.pos, "array size must be positive");
                        }
                        direct = Declarator::Array(Box::new(direct), Some(size as usize));
                    } else {
                        if expression
                            .resolve_type()
                            .is_none_or(|ty| ty.integer_info().is_none())
                        {
                            self.error_logger.print_error_position(
                                token.pos,
                                "array bound requires an integer expression",
                            );
                        }
                        let bound = self.vla_temporary(&token, Type::integer(64, false));
                        self.pending_bounds.push(Node::new_with_op(
                            Some(token),
                            NodeType::Assign,
                            bound.clone(),
                            expression,
                        ));
                        direct = Declarator::VariableArray(Box::new(direct), bound.offset.unwrap());
                    }
                }
            } else if self.attempt_reserved("(").is_some() {
                self.scope_stack.push(HashMap::new());
                let mut parameters = vec![];
                let mut stars = vec![];
                let mut variadic = false;
                let saved = self.cur;
                let empty_void =
                    self.attempt_reserved("void").is_some() && self.attempt_reserved(")").is_some();
                if !empty_void {
                    self.cur = saved;
                }
                if !empty_void && self.attempt_reserved(")").is_none() {
                    loop {
                        if self.attempt_reserved("...").is_some() {
                            if parameters.is_empty() {
                                self.error_logger.print_error_position(
                                    self.tokens[self.cur - 1].pos,
                                    "variadic function requires a fixed parameter",
                                );
                            }
                            variadic = true;
                            self.expect_reserved(")");
                            break;
                        }
                        let (base, storage, _alignment) =
                            self.attempt_automatic_type(true).unwrap_or_else(|| {
                                self.error_logger.print_error_position(
                                    self.tokens[self.cur].pos,
                                    "parameter type expected",
                                );
                                unreachable!()
                            });
                        let registered = storage == AutomaticStorage::Register;
                        if base == Type::Void
                            && parameters.is_empty()
                            && self.attempt_reserved(")").is_some()
                        {
                            break;
                        }
                        self.parameter_depth += 1;
                        let (name, ty, _, _, mut parameter_stars) = self.declarator().resolve(base);
                        self.parameter_depth -= 1;
                        stars.append(&mut parameter_stars);
                        if matches!(ty.unqualified(), Type::Arr(..) | Type::Vla(..)) {
                            self.validate_type_layout(ty.unqualified(), &self.tokens[self.cur]);
                        }
                        let qualifiers = ty.qualifiers();
                        let ty = match ty.unqualified() {
                            Type::Arr(child, _) | Type::Vla(child, _) => {
                                let pointer = Type::Ptr(Rc::clone(child));
                                if qualifiers == Qualifiers::default() {
                                    pointer
                                } else {
                                    Type::Qualified(Rc::new(pointer), qualifiers)
                                }
                            }
                            Type::Func(..) | Type::VariadicFunc(..) => Type::Ptr(Rc::new(ty)),
                            _ => ty,
                        };
                        self.validate_type_layout(&ty, &self.tokens[self.cur]);
                        if let Some(name) = &name {
                            self.scope_stack.last_mut().unwrap().insert(
                                name.s_value.to_string(),
                                Identifier::Local(
                                    ty.clone(),
                                    usize::MAX - parameters.len(),
                                    registered,
                                    ty.align_of(),
                                ),
                            );
                        }
                        parameters.push((name, ty, registered));
                        if self.attempt_reserved(")").is_some() {
                            break;
                        }
                        self.expect_reserved(",");
                    }
                }
                let scope = self
                    .scope_stack
                    .pop()
                    .unwrap()
                    .into_iter()
                    .filter(|(_, identifier)| {
                        matches!(
                            identifier,
                            Identifier::TypeDef(_)
                                | Identifier::Constant(..)
                                | Identifier::EnumTag { .. }
                        )
                    })
                    .collect();
                direct = Declarator::Function(Box::new(direct), parameters, variadic, scope, stars);
            } else {
                break;
            }
        }
        for q in pointers.into_iter().rev() {
            direct = Declarator::Pointer(Box::new(direct), q);
        }
        direct
    }

    pub(super) fn expect_ident_with_type(
        &mut self,
        ty: Type,
        alignments: Option<&mut Vec<usize>>,
    ) -> (Token, Type) {
        let (name, ty, _, _, _) = self.declarator_with_alignment(alignments).resolve(ty);
        let token = name.unwrap_or_else(|| {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "ident expected");
            unreachable!()
        });
        self.validate_type_layout(&ty, &token);
        (token, ty)
    }
}

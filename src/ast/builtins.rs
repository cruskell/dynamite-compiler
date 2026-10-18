use super::AstBuilderImpl;
use super::Identifier;
use crate::ctype::{Qualifiers, Type};
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use crate::token::{Token, TokenType};
use rustc_apfloat::Float;
use std::rc::Rc;

impl AstBuilderImpl<'_> {
    pub(super) fn offsetof_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let base = self.expect_type();
        let (name, mut ty, _, _, _) = self.declarator().resolve(base);
        if name.is_some() || ty.is_variably_modified() {
            self.error_logger
                .print_error_position(token.pos, "offsetof requires a fixed type name");
        }
        self.expect_reserved(",");
        let mut offset = 0;
        loop {
            let member_name = self.attempt_ident().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "offsetof member expected");
                unreachable!()
            });
            let Type::Struct(st) = ty.unqualified() else {
                self.error_logger
                    .print_error_position(member_name.pos, "offsetof member requires an aggregate");
                unreachable!()
            };
            let path = st.member_path(&member_name.s_value).unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(member_name.pos, "unknown offsetof member");
                unreachable!()
            });
            for index in path {
                let Type::Struct(st) = ty.unqualified() else {
                    unreachable!()
                };
                let member = st.member(index);
                if member.bit_field.is_some() {
                    self.error_logger.print_error_position(
                        member_name.pos,
                        "offsetof cannot be applied to a bit-field",
                    );
                }
                offset += member.offset;
                ty = member.ty;
            }
            while let Some(bracket) = self.attempt_reserved("[") {
                let index = self.ternary();
                if !Self::integer_constant(&index) {
                    self.error_logger.print_error_position(
                        bracket.pos,
                        "offsetof index requires an integer constant",
                    );
                }
                let index = self.eval(&index);
                self.expect_reserved("]");
                let Type::Arr(child, _) = ty.unqualified() else {
                    self.error_logger
                        .print_error_position(bracket.pos, "offsetof subscript requires an array");
                    unreachable!()
                };
                if index < 0 {
                    self.error_logger
                        .print_error_position(bracket.pos, "offsetof index cannot be negative");
                }
                offset = (index as usize)
                    .checked_mul(child.size_of())
                    .and_then(|stride| offset.checked_add(stride))
                    .unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(bracket.pos, "offsetof result is too large");
                        unreachable!()
                    });
                ty = child.as_ref().clone();
            }
            if self.attempt_reserved(".").is_none() {
                break;
            }
        }
        self.expect_reserved(")");
        Node {
            token: Some(token),
            cty: Some(Type::integer(64, false)),
            value: Some(offset),
            ..Node::default()
        }
    }

    pub(super) fn predefined_function_name(&mut self, token: Token) -> Node {
        let Some(function) = self.current_function.clone() else {
            self.error_logger
                .print_error_position(token.pos, "__func__ is only available inside a function");
            return Node::default();
        };
        let name = format!(".static.{}.__func__", function.pos);
        let element = Type::Qualified(
            Rc::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        );
        let ty = Type::Arr(Rc::new(element), Some(function.s_value.len() + 1));
        self.global_variables
            .entry(name.clone())
            .or_insert_with(|| GlobalVariable {
                ty: ty.clone(),
                alignment: 1,
                data: Some(GlobalVariableData::Arr(
                    function
                        .s_value
                        .bytes()
                        .chain([0])
                        .map(|byte| GlobalVariableData::Elem(byte.to_string()))
                        .collect(),
                )),
                internal: true,
                declaration_only: false,
                common: false,
                weak_import: false,
                hidden: false,
                thread_local: false,
            });
        Node {
            nt: NodeType::GlobalVar,
            token: Some(token),
            cty: Some(ty),
            global_name: name,
            ..Node::default()
        }
    }

    pub(super) fn branch_expectation_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let value = self.assign();
        self.expect_reserved(",");
        let expected = self.assign();
        self.expect_reserved(")");
        let ty = Type::integer(64, true);
        self.expect_conversion(&ty, &value);
        self.expect_conversion(&ty, &expected);
        let cast = |operand| Node {
            nt: NodeType::Cast,
            token: Some(token.clone()),
            cty: Some(ty.clone()),
            lhs: Some(Box::new(operand)),
            ..Node::default()
        };
        let value = cast(value);
        if !expected.has_side_effects() {
            return value;
        }
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "branch expectation is not a constant expression");
        }
        let expected = cast(expected);
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".expect.{}.{}", token.pos, self.offset_size).into(),
                ..token.clone()
            },
            ty,
        );
        let save = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            temporary.clone(),
            value,
        );
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![save, expected, temporary],
            ..Node::default()
        }
    }

    pub(super) fn floating_infinity_builtin(&mut self, token: Token) -> Node {
        let ty = match token.s_value.as_str() {
            "__builtin_inff" | "__builtin_huge_valf" => Type::Float(32),
            "__builtin_inf" | "__builtin_huge_val" => Type::Float(64),
            "__builtin_infl" | "__builtin_huge_vall" => self.long_double_type(&token),
            _ => unreachable!(),
        };
        self.expect_reserved("(");
        self.expect_reserved(")");
        let value = ty.float_constant(f64::INFINITY);
        Node {
            token: Some(token),
            cty: Some(ty),
            value: Some(value),
            ..Node::default()
        }
    }

    pub(super) fn floating_nan_builtin(&mut self, token: Token) -> Node {
        let (name, ty) = match token.s_value.as_str() {
            "__builtin_nanf" => ("nanf", Type::Float(32)),
            "__builtin_nan" => ("nan", Type::Float(64)),
            "__builtin_nanl" => ("nanl", self.long_double_type(&token)),
            _ => unreachable!(),
        };
        let pointer = Type::Ptr(Rc::new(Type::Qualified(
            Rc::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let call = self.runtime_builtin(
            token.clone(),
            name,
            Type::Func(Rc::new(vec![pointer]), Rc::new(ty.clone())),
        );
        let literal = call
            .args
            .first()
            .filter(|argument| call.nt == NodeType::CallFunc && argument.nt == NodeType::GlobalVar)
            .and_then(|argument| argument.token.as_ref())
            .filter(|literal| {
                literal.tt == TokenType::Str && literal.string_encoding.unit_size() == 1
            });
        if let Some(literal) = literal {
            let payload = Self::nan_payload(&literal.string_bytes);
            let bits = if ty.float_bits() == Some(32) {
                0x7fc0_0000 | (payload & 0x003f_ffff)
            } else {
                0x7ff8_0000_0000_0000 | (payload & 0x0007_ffff_ffff_ffff)
            };
            let extended_float = (ty.float_bits() == Some(80))
                .then(|| Box::new(crate::floating::Extended::qnan(Some(payload.into())).to_bits()));
            return Node {
                token: Some(token),
                cty: Some(ty),
                value: Some(bits as usize),
                extended_float,
                ..Node::default()
            };
        }
        call
    }

    pub(super) fn nan_payload(bytes: &[u8]) -> u64 {
        let bytes = bytes.split(|byte| *byte == 0).next().unwrap();
        let (radix, digits) = if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
            (16, &bytes[2..])
        } else if bytes.starts_with(b"0") {
            (8, &bytes[1..])
        } else {
            (10, bytes)
        };
        let mut value = 0u64;
        for byte in digits {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return 0,
            };
            if u64::from(digit) >= radix {
                return 0;
            }
            // Literal payloads retain their low bits even when their spelling exceeds u64.
            value = value.wrapping_mul(radix).wrapping_add(u64::from(digit));
        }
        value
    }

    pub(super) fn floating_abs_builtin(&mut self, token: Token) -> Node {
        let ty = match token.s_value.as_str() {
            "__builtin_fabsf" => Type::Float(32),
            "__builtin_fabs" => Type::Float(64),
            "__builtin_fabsl" => self.long_double_type(&token),
            _ => unreachable!(),
        };
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        Node {
            nt: NodeType::FloatAbs,
            token: Some(token),
            cty: Some(ty),
            lhs: Some(Box::new(argument)),
            ..Node::default()
        }
    }

    pub(super) fn memory_builtin(&mut self, token: Token) -> Node {
        let pointer = Type::Ptr(Rc::new(Type::Void));
        let const_pointer = Type::Ptr(Rc::new(Type::Qualified(
            Rc::new(Type::Void),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let (name, second, checked) = match token.s_value.as_str() {
            "__builtin___memcpy_chk" => ("__memcpy_chk", const_pointer, true),
            "__builtin___memmove_chk" => ("__memmove_chk", const_pointer, true),
            "__builtin___memset_chk" => ("__memset_chk", Type::I32, true),
            "__builtin_memcpy" => ("memcpy", const_pointer, false),
            "__builtin_memmove" => ("memmove", const_pointer, false),
            "__builtin_memset" => ("memset", Type::I32, false),
            _ => unreachable!(),
        };
        let size = Type::integer(64, false);
        let mut parameters = vec![pointer.clone(), second, size.clone()];
        if checked {
            parameters.push(size);
        }
        let ty = Type::Func(Rc::new(parameters), Rc::new(pointer));
        self.runtime_builtin(token, name, ty)
    }

    pub(super) fn checked_format_builtin(&mut self, token: Token) -> Node {
        let pointer = Type::Ptr(Rc::new(Type::I8));
        let format = Type::Ptr(Rc::new(Type::Qualified(
            Rc::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let size = Type::integer(64, false);
        let (name, parameters) = match token.s_value.as_str() {
            "__builtin___sprintf_chk" => ("__sprintf_chk", vec![pointer, Type::I32, size, format]),
            "__builtin___snprintf_chk" => (
                "__snprintf_chk",
                vec![pointer, size.clone(), Type::I32, size, format],
            ),
            _ => unreachable!(),
        };
        self.runtime_builtin(
            token,
            name,
            Type::VariadicFunc(Rc::new(parameters), Rc::new(Type::I32)),
        )
    }

    pub(super) fn checked_bsd_string_builtin(&mut self, token: Token) -> Node {
        let pointer = Type::Ptr(Rc::new(Type::I8));
        let source = Type::Ptr(Rc::new(Type::Qualified(
            Rc::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let size = Type::integer(64, false);
        let name = match token.s_value.as_str() {
            "__builtin___strlcpy_chk" => "__strlcpy_chk",
            "__builtin___strlcat_chk" => "__strlcat_chk",
            _ => unreachable!(),
        };
        self.runtime_builtin(
            token,
            name,
            Type::Func(
                Rc::new(vec![pointer, source, size.clone(), size.clone()]),
                Rc::new(size),
            ),
        )
    }

    pub(super) fn runtime_builtin(&mut self, token: Token, name: &str, ty: Type) -> Node {
        if self.tokens[self.cur].s_value != "(" {
            self.error_logger
                .print_error_position(token.pos, "runtime builtin must be called directly");
        }
        if self.global_variables.contains_key(name)
            || self.scope_stack[0]
                .get(name)
                .is_some_and(|identifier| !matches!(identifier, Identifier::Static(_)))
        {
            self.error_logger
                .print_error_position(token.pos, "runtime symbol conflicts with an identifier");
        }
        if let Some(function) = self.functions.get(name) {
            if !function.cty.compatible(&ty) {
                self.error_logger
                    .print_error_position(token.pos, "incompatible runtime declaration");
            }
        }
        self.functions.entry(name.into()).or_insert_with(|| Func {
            token: Some(token.clone()),
            cty: ty.clone(),
            ..Func::default()
        });
        self.scope_stack[0]
            .entry(name.into())
            .or_insert_with(|| Identifier::Static(ty.clone()));
        self.postfix(Node {
            nt: NodeType::GlobalVar,
            token: Some(token),
            cty: Some(ty),
            global_name: name.into(),
            ..Node::default()
        })
    }

    pub(super) fn constant_query_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let bounds_start = self.pending_bounds.len();
        let operand = self.assign();
        self.expect_reserved(")");
        let has_bounds = self.pending_bounds.len() != bounds_start;
        self.pending_bounds.truncate(bounds_start);
        let literal = operand.nt == NodeType::Num
            || (operand.nt == NodeType::GlobalVar
                && operand
                    .token
                    .as_ref()
                    .is_some_and(|token| token.tt == TokenType::Str));
        let known = !has_bounds
            && !operand.has_side_effects()
            && (literal || self.foldable_integer_constant(&operand));
        Node::new_with_num(Some(token), usize::from(known))
    }

    pub(super) fn bit_scan_builtin(&mut self, token: Token, bits: u8, leading: bool) -> Node {
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        let ty = Type::integer(bits, false);
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        if Self::integer_constant(&argument) {
            let value = self.eval(&argument) as u64 & (u64::MAX >> (64 - bits));
            let count = if value == 0 {
                u32::from(bits)
            } else if leading {
                value.leading_zeros() - u32::from(64 - bits)
            } else {
                value.trailing_zeros()
            };
            return Node::new_with_num(Some(token), count as usize);
        }
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "bit-scan operand is not a constant expression");
        }
        Node {
            cty: Some(Type::I32),
            ..Node::new_with_op_and_lhs(Some(token), NodeType::BitScan { bits, leading }, argument)
        }
    }

    pub(super) fn population_count_builtin(&mut self, token: Token, bits: u8) -> Node {
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        let ty = Type::integer(bits, false);
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        if Self::integer_constant(&argument) {
            let value = self.eval(&argument) as u64 & (u64::MAX >> (64 - bits));
            return Node::new_with_num(Some(token), value.count_ones() as usize);
        }
        if self.return_type.is_none() {
            self.error_logger.print_error_position(
                token.pos,
                "population-count operand is not a constant expression",
            );
        }
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".popcount.{}.{}", token.pos, self.offset_size).into(),
                ..token.clone()
            },
            ty.clone(),
        );
        let binary = |nt, lhs, rhs| Node::new_with_op(Some(token.clone()), nt, lhs, rhs);
        let number = |value| Node {
            cty: Some(ty.clone()),
            ..Node::new_with_num(Some(token.clone()), value & (usize::MAX >> (64 - bits)))
        };
        let assign = |value| binary(NodeType::Assign, temporary.clone(), value);
        let shift = |amount| binary(NodeType::BitRight, temporary.clone(), number(amount));
        let mut children = vec![assign(argument)];
        // Sum adjacent groups of bits, then accumulate the byte counts.
        children.push(assign(binary(
            NodeType::Sub,
            temporary.clone(),
            binary(NodeType::BitAnd, shift(1), number(0x5555_5555_5555_5555)),
        )));
        children.push(assign(binary(
            NodeType::Add,
            binary(
                NodeType::BitAnd,
                temporary.clone(),
                number(0x3333_3333_3333_3333),
            ),
            binary(NodeType::BitAnd, shift(2), number(0x3333_3333_3333_3333)),
        )));
        children.push(assign(binary(
            NodeType::BitAnd,
            binary(NodeType::Add, temporary.clone(), shift(4)),
            number(0x0f0f_0f0f_0f0f_0f0f),
        )));
        for amount in [8, 16, 32]
            .into_iter()
            .filter(|amount| *amount < usize::from(bits))
        {
            children.push(assign(binary(
                NodeType::Add,
                temporary.clone(),
                shift(amount),
            )));
        }
        let mut result = binary(NodeType::BitAnd, temporary.clone(), number(127));
        Self::cast_integer(&mut result, &Type::I32);
        children.push(result);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }

    pub(super) fn byte_swap_builtin(&mut self, token: Token, bits: u8) -> Node {
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        let ty = if bits == 64 {
            Type::LongLong { signed: false }
        } else {
            Type::integer(bits, false)
        };
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        if Self::integer_constant(&argument) {
            let value = self.eval(&argument) as u64;
            let value = match bits {
                16 => u64::from((value as u16).swap_bytes()),
                32 => u64::from((value as u32).swap_bytes()),
                64 => value.swap_bytes(),
                _ => unreachable!(),
            };
            return Node {
                cty: Some(ty),
                ..Node::new_with_num(Some(token), value as usize)
            };
        }
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "byte-swap operand is not a constant expression");
        }
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".bswap.{}.{}", token.pos, self.offset_size).into(),
                ..token.clone()
            },
            ty.clone(),
        );
        let assignment = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            temporary.clone(),
            argument,
        );
        let mut result = Node::new_with_num(Some(token.clone()), 0);
        for index in 0..usize::from(bits / 8) {
            let shifted = Node::new_with_op(
                Some(token.clone()),
                NodeType::BitRight,
                temporary.clone(),
                Node::new_with_num(Some(token.clone()), index * 8),
            );
            let byte = Node::new_with_op(
                Some(token.clone()),
                NodeType::BitAnd,
                shifted,
                Node::new_with_num(Some(token.clone()), 255),
            );
            let byte = Node::new_with_op(
                Some(token.clone()),
                NodeType::BitLeft,
                byte,
                Node::new_with_num(Some(token.clone()), usize::from(bits) - 8 - index * 8),
            );
            result = Node::new_with_op(Some(token.clone()), NodeType::BitOr, result, byte);
        }
        Self::cast_integer(&mut result, &ty);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![assignment, result],
            ..Node::default()
        }
    }
}

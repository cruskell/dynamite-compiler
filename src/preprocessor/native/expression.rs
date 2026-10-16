//! Integer constant expressions used by conditional directives.

use super::Token;

#[derive(Clone, Copy, Default)]
pub(super) struct Value {
    pub(super) bits: u64,
    unsigned: bool,
}
impl Value {
    fn boolean(value: bool) -> Self {
        Self {
            bits: u64::from(value),
            unsigned: false,
        }
    }
}

pub(super) struct Expression<'a> {
    tokens: &'a [Token],
    pos: usize,
}
impl<'a> Expression<'a> {
    pub(super) fn evaluate(tokens: &'a [Token]) -> Result<Value, String> {
        let mut parser = Self { tokens, pos: 0 };
        let value = parser.expression(0, true)?;
        if parser.pos != tokens.len() {
            return Err(format!(
                "unexpected token in #if: {}",
                tokens[parser.pos].text
            ));
        }
        Ok(value)
    }
    fn take(&mut self, text: &str) -> bool {
        if self.tokens.get(self.pos).is_some_and(|t| t.is(text)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expression(&mut self, minimum: u8, evaluate: bool) -> Result<Value, String> {
        let mut left = self.unary(evaluate)?;
        loop {
            if minimum == 0 && self.take("?") {
                let yes = self.expression(0, evaluate && left.bits != 0)?;
                if !self.take(":") {
                    return Err("missing ':' in #if".into());
                }
                let no = self.expression(0, evaluate && left.bits == 0)?;
                let unsigned = yes.unsigned || no.unsigned;
                left = if left.bits != 0 { yes } else { no };
                left.unsigned = unsigned;
                continue;
            }
            let Some(op) = self.tokens.get(self.pos).map(|t| t.text.as_str()) else {
                break;
            };
            let precedence = match op {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            self.pos += 1;
            let right = self.expression(
                precedence + 1,
                evaluate && !(op == "&&" && left.bits == 0 || op == "||" && left.bits != 0),
            )?;
            let (a, b) = (left.bits, right.bits);
            let unsigned = left.unsigned || right.unsigned;
            let less = if unsigned {
                a < b
            } else {
                (a as i64) < (b as i64)
            };
            left = match op {
                "||" => Value::boolean(a != 0 || b != 0),
                "&&" => Value::boolean(a != 0 && b != 0),
                "==" => Value::boolean(a == b),
                "!=" => Value::boolean(a != b),
                "<" => Value::boolean(less),
                ">=" => Value::boolean(!less),
                ">" => Value::boolean(!less && a != b),
                "<=" => Value::boolean(less || a == b),
                _ => Value {
                    bits: match op {
                        "+" => a.wrapping_add(b),
                        "-" => a.wrapping_sub(b),
                        "*" => a.wrapping_mul(b),
                        "/" | "%" if b == 0 => {
                            if evaluate {
                                return Err("division by zero in #if".into());
                            }
                            0
                        }
                        "/" if unsigned => a / b,
                        "%" if unsigned => a % b,
                        "/" => (a as i64).wrapping_div(b as i64) as u64,
                        "%" => (a as i64).wrapping_rem(b as i64) as u64,
                        "|" => a | b,
                        "^" => a ^ b,
                        "&" => a & b,
                        "<<" => a.wrapping_shl(b as u32),
                        ">>" if left.unsigned => a.wrapping_shr(b as u32),
                        ">>" => (a as i64).wrapping_shr(b as u32) as u64,
                        _ => unreachable!(),
                    },
                    unsigned: if matches!(op, "<<" | ">>") {
                        left.unsigned
                    } else {
                        unsigned
                    },
                },
            };
        }
        Ok(left)
    }
    fn unary(&mut self, evaluate: bool) -> Result<Value, String> {
        if self.take("!") {
            return Ok(Value::boolean(self.unary(evaluate)?.bits == 0));
        }
        if self.take("~") {
            let mut value = self.unary(evaluate)?;
            value.bits = !value.bits;
            return Ok(value);
        }
        if self.take("-") {
            let mut value = self.unary(evaluate)?;
            value.bits = value.bits.wrapping_neg();
            return Ok(value);
        }
        if self.take("+") {
            return self.unary(evaluate);
        }
        if self.take("(") {
            let value = self.expression(0, evaluate)?;
            if !self.take(")") {
                return Err("missing ')' in #if".into());
            }
            return Ok(value);
        }
        let text = self
            .tokens
            .get(self.pos)
            .ok_or("missing operand in #if")?
            .text
            .as_str();
        self.pos += 1;
        if text.starts_with('\'') {
            let body = &text[1..text.len() - 1];
            let bytes = crate::tokenizer::decode_string_literal(body)
                .ok_or("invalid character escape in #if")?;
            if bytes.is_empty() || bytes.len() > 4 {
                return Err("unsupported character constant in #if".into());
            }
            let bits = if bytes.len() == 1 {
                (bytes[0] as i8 as i64) as u64
            } else {
                (bytes
                    .iter()
                    .fold(0u32, |value, byte| (value << 8) | u32::from(*byte))
                    as i32 as i64) as u64
            };
            return Ok(Value {
                bits,
                unsigned: false,
            });
        }
        if !text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return Ok(Value::default());
        }
        let unsigned = text.contains(['u', 'U']);
        let number = text.trim_end_matches(['u', 'U', 'l', 'L']);
        let (number, radix) = if let Some(n) = number
            .strip_prefix("0x")
            .or_else(|| number.strip_prefix("0X"))
        {
            (n, 16)
        } else if let Some(n) = number
            .strip_prefix("0b")
            .or_else(|| number.strip_prefix("0B"))
        {
            (n, 2)
        } else if number.starts_with('0') && number.len() > 1 {
            (&number[1..], 8)
        } else {
            (number, 10)
        };
        let bits = u64::from_str_radix(number, radix)
            .map_err(|_| format!("invalid integer in #if: {text}"))?;
        Ok(Value {
            bits,
            unsigned: unsigned || bits > i64::MAX as u64,
        })
    }
}

use crate::error;
use crate::token::{Token, TokenType};
use crate::trie::Trie;
use std::collections::HashSet;

pub const TYPES: [&str; 11] = [
    "int", "char", "void", "short", "long", "signed", "unsigned", "_Bool", "bool", "float",
    "double",
];
const RESERVED_WORDS: [&str; 30] = [
    "return",
    "if",
    "else",
    "while",
    "for",
    "break",
    "sizeof",
    "continue",
    "do",
    "switch",
    "case",
    "default",
    "typedef",
    "struct",
    "enum",
    "goto",
    "static",
    "extern",
    "union",
    "const",
    "volatile",
    "restrict",
    "inline",
    "_Static_assert",
    "_Generic",
    "auto",
    "register",
    "_Alignof",
    "_Alignas",
    "_Thread_local",
];

const RESERVED_SYMBOLS: [&str; 51] = [
    "=", "+", "-", "*", "/", "%", "<", ">", "==", "!=", "+=", "-=", "*=", "/=", "%=", "<=", ">=",
    "&", "^", "|", "&&", "||", "<<", ">>", "{", "}", "(", ")", "[", "]", ",", ";", "/*", "//",
    "\"", "!", "~", "?", ":", "<<=", ">>=", "&=", "^=", "|=", "++", "--", "#", "'", "...", ".",
    "->",
];

pub(crate) fn decode_string_literal(value: &str) -> Option<Vec<u8>> {
    let mut chars = value.chars().peekable();
    let mut decoded = vec![];
    while let Some(character) = chars.next() {
        if character != '\\' {
            let mut encoded = [0; 4];
            decoded.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            'a' => decoded.push(7),
            'b' => decoded.push(8),
            'f' => decoded.push(12),
            'n' => decoded.push(10),
            'r' => decoded.push(13),
            't' => decoded.push(9),
            'v' => decoded.push(11),
            '\\' | '\'' | '"' | '?' => decoded.push(escaped as u8),
            '0'..='7' => {
                let mut value = escaped.to_digit(8)? as u16;
                for _ in 0..2 {
                    if chars
                        .peek()
                        .is_some_and(|character| matches!(character, '0'..='7'))
                    {
                        value = value * 8 + chars.next()?.to_digit(8)? as u16;
                    } else {
                        break;
                    }
                }
                decoded.push(u8::try_from(value).ok()?);
            }
            'x' => {
                let mut value = 0u16;
                let mut count = 0;
                while chars.peek().is_some_and(char::is_ascii_hexdigit) {
                    value = value
                        .checked_mul(16)?
                        .checked_add(chars.next()?.to_digit(16)? as u16)?;
                    count += 1;
                }
                if count == 0 {
                    return None;
                }
                decoded.push(u8::try_from(value).ok()?);
            }
            'u' | 'U' => {
                let digits = if escaped == 'u' { 4 } else { 8 };
                let mut value = 0u32;
                for _ in 0..digits {
                    value = value
                        .checked_mul(16)?
                        .checked_add(chars.next()?.to_digit(16)?)?;
                }
                let character = char::from_u32(value)?;
                let mut encoded = [0; 4];
                decoded.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            }
            _ => return None,
        }
    }
    Some(decoded)
}

pub struct Tokenizer {}

impl Tokenizer {
    fn reserved_token(pos: usize, s_value: String) -> Token {
        Token {
            tt: TokenType::Reserved,
            pos,
            s_value,
            ..Token::default()
        }
    }
    fn num_token(pos: usize, i_value: usize) -> Token {
        Token {
            tt: TokenType::Num,
            pos,
            i_value,
            ..Token::default()
        }
    }
    fn ident_token(pos: usize, s_value: String) -> Token {
        Token {
            tt: TokenType::Ident,
            pos,
            s_value,
            ..Token::default()
        }
    }
    fn float_token(code: &str, start: usize) -> Result<Option<(Token, usize)>, error::SyntaxError> {
        let bytes = code.as_bytes();
        let hexadecimal = code[start..].starts_with("0x") || code[start..].starts_with("0X");
        let mut pos = start + if hexadecimal { 2 } else { 0 };
        let digits_start = pos;
        while bytes.get(pos).is_some_and(|b| {
            if hexadecimal {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        }) {
            pos += 1;
        }
        let mut fractional = false;
        if bytes.get(pos) == Some(&b'.') {
            fractional = true;
            pos += 1;
            while bytes.get(pos).is_some_and(|b| {
                if hexadecimal {
                    b.is_ascii_hexdigit()
                } else {
                    b.is_ascii_digit()
                }
            }) {
                pos += 1;
            }
        }
        let mantissa_end = pos;
        if !code[digits_start..mantissa_end]
            .bytes()
            .any(|byte| byte.is_ascii_hexdigit())
        {
            return Err(error::SyntaxError::new(
                start,
                "floating literal requires digits",
            ));
        }
        let exponent = bytes.get(pos).is_some_and(|b| {
            if hexadecimal {
                matches!(b, b'p' | b'P')
            } else {
                matches!(b, b'e' | b'E')
            }
        });
        if !fractional && !exponent {
            return Ok(None);
        }
        if hexadecimal && !exponent {
            return Err(error::SyntaxError::new(
                start,
                "hexadecimal float requires an exponent",
            ));
        }
        let mut exponent_value = 0i32;
        if exponent {
            pos += 1;
            let exponent_start = pos;
            if bytes.get(pos).is_some_and(|b| matches!(b, b'+' | b'-')) {
                pos += 1;
            }
            let exponent_digits = pos;
            while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
                pos += 1;
            }
            if pos == exponent_digits {
                return Err(error::SyntaxError::new(
                    start,
                    "floating exponent requires digits",
                ));
            }
            exponent_value = code[exponent_start..pos]
                .parse()
                .map_err(|_| error::SyntaxError::new(start, "floating exponent is out of range"))?;
        }
        let numeric_end = pos;
        let single = bytes.get(pos).is_some_and(|b| matches!(b, b'f' | b'F'));
        if single {
            pos += 1;
        }
        if bytes
            .get(pos)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            return Err(error::SyntaxError::new(start, "invalid floating literal"));
        }
        let value = if hexadecimal {
            let mut value = 0.0f64;
            let mut fractional_digits = 0i32;
            let mut after_point = false;
            for b in code[digits_start..mantissa_end].bytes() {
                if b == b'.' {
                    after_point = true;
                    continue;
                }
                value = value * 16.0 + f64::from((b as char).to_digit(16).unwrap());
                fractional_digits += i32::from(after_point);
            }
            value * 2.0f64.powi(exponent_value.saturating_sub(fractional_digits.saturating_mul(4)))
        } else {
            code[start..numeric_end]
                .parse::<f64>()
                .map_err(|_| error::SyntaxError::new(start, "invalid floating literal"))?
        };
        Ok(Some((
            Token {
                tt: TokenType::Float,
                pos: start,
                i_value: if single {
                    (value as f32).to_bits() as usize
                } else {
                    value.to_bits() as usize
                },
                s_value: if single { "f" } else { "d" }.into(),
            },
            pos,
        )))
    }
    pub fn tokenize(code: &str, is_debug: bool) -> Result<Vec<Token>, error::SyntaxError> {
        let mut tokens = vec![];
        let reserved_words = RESERVED_WORDS.into_iter().collect::<HashSet<_>>();
        let reserved_symbols = Trie::new(&RESERVED_SYMBOLS);
        let bytes = code.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            let start = pos;
            if bytes[pos].is_ascii_digit()
                || (bytes[pos] == b'.' && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit))
            {
                if let Some((token, end)) = Self::float_token(code, pos)? {
                    tokens.push(token);
                    pos = end;
                    continue;
                }
            }
            let utf8_prefix = code[pos..].starts_with("u8\"");
            if utf8_prefix {
                pos += 2;
            }
            match bytes[pos] {
                byte if byte.is_ascii_whitespace() => pos += 1,
                b'0'..=b'9' => {
                    let radix = if code[pos..].starts_with("0x") || code[pos..].starts_with("0X") {
                        pos += 2;
                        16
                    } else if bytes[pos] == b'0' {
                        8
                    } else {
                        10
                    };
                    let digits_start = pos;
                    let mut value = 0usize;
                    while pos < bytes.len() {
                        let Some(digit) = (bytes[pos] as char).to_digit(radix) else {
                            break;
                        };
                        value = value
                            .checked_mul(radix as usize)
                            .and_then(|value| value.checked_add(digit as usize))
                            .ok_or_else(|| {
                                error::SyntaxError::new(start, "integer literal is too large")
                            })?;
                        pos += 1;
                    }
                    let suffix_start = pos;
                    while pos < bytes.len() && matches!(bytes[pos], b'u' | b'U' | b'l' | b'L') {
                        pos += 1;
                    }
                    let suffix = code[suffix_start..pos].to_ascii_lowercase();
                    if !matches!(
                        suffix.as_str(),
                        "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu"
                    ) {
                        return Err(error::SyntaxError::new(start, "invalid integer suffix"));
                    }
                    if suffix_start == digits_start
                        || (pos < bytes.len()
                            && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_'))
                    {
                        return Err(error::SyntaxError::new(start, "invalid integer literal"));
                    }
                    let mut token = Self::num_token(start, value);
                    let wide = suffix.contains('l')
                        || value > u32::MAX as usize
                        || (radix == 10 && !suffix.contains('u') && value > i32::MAX as usize);
                    let unsigned = suffix.contains('u') || (!wide && value > i32::MAX as usize);
                    token.s_value = format!(
                        "{}{}",
                        if wide { "l" } else { "" },
                        if unsigned { "u" } else { "" }
                    );
                    tokens.push(token);
                }
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    pos += 1;
                    while pos < bytes.len()
                        && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                    {
                        pos += 1;
                    }
                    let value = &code[start..pos];
                    tokens.push(
                        if reserved_words.contains(value) || TYPES.contains(&value) {
                            Self::reserved_token(start, value.to_string())
                        } else {
                            Self::ident_token(start, value.to_string())
                        },
                    );
                }
                b'#' => {
                    while pos < bytes.len() && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                }
                b'/' if code[pos..].starts_with("//") => {
                    while pos < bytes.len() && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                }
                b'/' if code[pos..].starts_with("/*") => {
                    let end = code[pos + 2..]
                        .find("*/")
                        .ok_or_else(|| error::SyntaxError::new(start, "unterminated comment"))?;
                    pos += end + 4;
                }
                b'"' | b'\'' => {
                    let quote = bytes[pos];
                    pos += 1;
                    while pos < bytes.len() && bytes[pos] != quote {
                        if bytes[pos] == b'\n' || bytes[pos] == b'\r' {
                            return Err(error::SyntaxError::new(start, "newline in literal"));
                        }
                        if bytes[pos] == b'\\' {
                            pos += 1;
                        }
                        if pos < bytes.len() {
                            pos += 1;
                        }
                    }
                    if pos >= bytes.len() {
                        return Err(error::SyntaxError::new(start, "unterminated literal"));
                    }
                    let value_start = if utf8_prefix { start + 3 } else { start + 1 };
                    let value = &code[value_start..pos];
                    let decoded = decode_string_literal(value)
                        .ok_or_else(|| error::SyntaxError::new(start, "invalid escape sequence"))?;
                    pos += 1;
                    if quote == b'\'' {
                        if decoded.len() != 1 {
                            return Err(error::SyntaxError::new(
                                start,
                                "character constant must contain one byte",
                            ));
                        }
                        tokens.push(Self::num_token(start, decoded[0] as usize));
                    } else {
                        tokens.push(Token {
                            tt: TokenType::Str,
                            pos: start,
                            s_value: value.to_string(),
                            ..Token::default()
                        });
                    }
                }
                _ => {
                    let size = reserved_symbols.matched_length(&code[pos..]);
                    if size == 0 {
                        return Err(error::SyntaxError::new(start, "unexpected character"));
                    }
                    pos += size;
                    tokens.push(Self::reserved_token(start, code[start..pos].to_string()));
                }
            }
        }
        let mut joined: Vec<Token> = Vec::with_capacity(tokens.len());
        for token in tokens {
            if token.tt == TokenType::Str && joined.last().is_some_and(|t| t.tt == TokenType::Str) {
                let previous = joined.last_mut().unwrap();
                let bytes = decode_string_literal(&previous.s_value)
                    .unwrap()
                    .into_iter()
                    .chain(decode_string_literal(&token.s_value).unwrap());
                previous.s_value = bytes.map(|byte| format!("\\{:03o}", byte)).collect();
            } else {
                joined.push(token);
            }
        }
        let mut tokens = joined;
        tokens.push(Token {
            tt: TokenType::EOF,
            pos: code.len(),
            ..Token::default()
        });
        if is_debug {
            Self::print_tokens(&tokens);
        }
        Ok(tokens)
    }
    pub fn print_tokens(tokens: &[Token]) {
        tokens.iter().for_each(|t| t.print())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_literals_and_diagnostics() {
        let tokens = Tokenizer::tokenize("0x10 0XfF 123", false).unwrap();
        assert_eq!(tokens[0].i_value, 16);
        assert_eq!(tokens[1].i_value, 255);
        assert_eq!(tokens[2].i_value, 123);
        for code in ["0x", "0xG", "0x1G", "0xFFFFFFFFFFFFFFFFFFFFFFFF"] {
            assert!(Tokenizer::tokenize(code, false).is_err(), "{code}");
        }
    }

    #[test]
    fn test_tokenize() {
        let code = "int main() { return 0; }";
        let expected = vec![
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                s_value: String::from("int"),
                i_value: 0,
            },
            Token {
                tt: TokenType::Ident,
                pos: 4,
                s_value: String::from("main"),
                i_value: 0,
            },
            Token {
                tt: TokenType::Reserved,
                pos: 8,
                s_value: String::from("("),
                i_value: 0,
            },
            Token {
                tt: TokenType::Reserved,
                pos: 9,
                s_value: String::from(")"),
                i_value: 0,
            },
            Token {
                tt: TokenType::Reserved,
                pos: 11,
                s_value: String::from("{"),
                i_value: 0,
            },
            Token {
                tt: TokenType::Reserved,
                pos: 13,
                s_value: String::from("return"),
                i_value: 0,
            },
            Token {
                tt: TokenType::Num,
                pos: 20,
                s_value: String::new(),
                i_value: 0,
            },
            Token {
                tt: TokenType::Reserved,
                pos: 21,
                s_value: String::from(";"),
                i_value: 0,
            },
            Token {
                tt: TokenType::Reserved,
                pos: 23,
                s_value: String::from("}"),
                i_value: 0,
            },
        ];
        let mut expected = expected;
        expected.push(Token {
            tt: TokenType::EOF,
            pos: code.len(),
            ..Token::default()
        });
        assert_eq!(Tokenizer::tokenize(code, false).unwrap(), expected);
    }

    #[test]
    fn literals_and_comments_preserve_byte_positions() {
        let code = "/* 日本語 \\ */ \"日本語\\t\"; // end";
        let tokens = Tokenizer::tokenize(code, false).unwrap();
        assert_eq!(tokens[0].pos, code.find('"').unwrap());
        assert_eq!(tokens[0].s_value, "日本語\\t");
        assert_eq!(tokens.last().unwrap().tt, TokenType::EOF);
        assert_eq!(tokens.last().unwrap().pos, code.len());
        assert_eq!(
            decode_string_literal("\\x41\\101\\0").unwrap(),
            vec![65, 65, 0]
        );
    }

    #[test]
    fn invalid_literals_return_syntax_errors() {
        for code in [
            "\"日本語",
            "\"abc\\",
            "/* unfinished",
            "''",
            "'ab'",
            "\"\\q\"",
            "99999999999999999999999999",
        ] {
            assert!(Tokenizer::tokenize(code, false).is_err(), "{}", code);
        }
    }

    #[test]
    fn test_tokenize_error() {
        let code = "int main() { char *s = \"hello; return 0; }";
        assert!(Tokenizer::tokenize(code, false).is_err());
    }
}

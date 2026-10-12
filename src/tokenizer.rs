use crate::error;
use crate::token::{Token, TokenType};
use crate::trie::Trie;
use std::collections::HashSet;

pub const TYPES: [&str; 2] = ["int", "char"];
const RESERVED_WORDS: [&str; 9] = [
    "return", "if", "else", "while", "for", "break", "sizeof", "continue", "do",
];
const RESERVED_SYMBOLS: [&str; 48] = [
    "=", "+", "-", "*", "/", "%", "<", ">", "==", "!=", "+=", "-=", "*=", "/=", "%=", "<=", ">=",
    "&", "^", "|", "&&", "||", "<<", ">>", "{", "}", "(", ")", "[", "]", ",", ";", "/*", "//",
    "\"", "!", "~", "?", ":", "<<=", ">>=", "&=", "^=", "|=", "++", "--", "#", "'",
];

pub(crate) fn decode_string_literal(value: &str) -> Option<Vec<u8>> {
    let mut bytes = value.bytes().peekable();
    let mut decoded = vec![];
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let escaped = bytes.next()?;
        decoded.push(match escaped {
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'n' => 10,
            b'r' => 13,
            b't' => 9,
            b'v' => 11,
            b'\\' | b'\'' | b'"' | b'?' => escaped,
            b'0'..=b'7' => {
                let mut value = (escaped - b'0') as u16;
                for _ in 0..2 {
                    if let Some(b'0'..=b'7') = bytes.peek() {
                        value = value * 8 + (bytes.next()? - b'0') as u16;
                    } else {
                        break;
                    }
                }
                u8::try_from(value).ok()?
            }
            b'x' => {
                let mut value = 0u16;
                let mut count = 0;
                while bytes.peek().is_some_and(u8::is_ascii_hexdigit) {
                    value = value
                        .checked_mul(16)?
                        .checked_add((bytes.next()? as char).to_digit(16)? as u16)?;
                    count += 1;
                }
                if count == 0 {
                    return None;
                }
                u8::try_from(value).ok()?
            }
            _ => return None,
        });
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
    pub fn tokenize(code: &str, is_debug: bool) -> Result<Vec<Token>, error::SyntaxError> {
        let mut tokens = vec![];
        let reserved_words = RESERVED_WORDS.into_iter().collect::<HashSet<_>>();
        let reserved_symbols = Trie::new(&RESERVED_SYMBOLS);
        let bytes = code.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            let start = pos;
            match bytes[pos] {
                byte if byte.is_ascii_whitespace() => pos += 1,
                b'0'..=b'9' => {
                    let radix = if code[pos..].starts_with("0x") || code[pos..].starts_with("0X") {
                        pos += 2;
                        16
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
                    if pos == digits_start
                        || (pos < bytes.len()
                            && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_'))
                    {
                        return Err(error::SyntaxError::new(start, "invalid integer literal"));
                    }
                    tokens.push(Self::num_token(start, value));
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
                    let value = &code[start + 1..pos];
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

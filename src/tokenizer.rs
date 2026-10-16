use crate::error;
use crate::token::{StringEncoding, Token, TokenType};
use crate::trie::Trie;

pub const TYPES: [&str; 18] = [
    "int",
    "char",
    "void",
    "short",
    "long",
    "signed",
    "unsigned",
    "_Bool",
    "bool",
    "float",
    "_Float16",
    "typeof",
    "__typeof",
    "__typeof__",
    "double",
    "__int128",
    "__int128_t",
    "__uint128_t",
];
fn is_reserved_word(value: &str) -> bool {
    matches!(
        value,
        "__extension__"
            | "return"
            | "if"
            | "else"
            | "while"
            | "for"
            | "break"
            | "sizeof"
            | "continue"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "typedef"
            | "struct"
            | "enum"
            | "goto"
            | "static"
            | "extern"
            | "union"
            | "const"
            | "volatile"
            | "restrict"
            | "inline"
            | "__inline"
            | "__inline__"
            | "_Static_assert"
            | "_Generic"
            | "auto"
            | "register"
            | "_Alignof"
            | "_Alignas"
            | "_Thread_local"
            | "__thread"
            | "_Nullable"
            | "_Nonnull"
            | "_Null_unspecified"
            | "_Nullable_result"
    )
}

const RESERVED_SYMBOLS: [&str; 51] = [
    "=", "+", "-", "*", "/", "%", "<", ">", "==", "!=", "+=", "-=", "*=", "/=", "%=", "<=", ">=",
    "&", "^", "|", "&&", "||", "<<", ">>", "{", "}", "(", ")", "[", "]", ",", ";", "/*", "//",
    "\"", "!", "~", "?", ":", "<<=", ">>=", "&=", "^=", "|=", "++", "--", "#", "'", "...", ".",
    "->",
];

pub(crate) fn decode_string_literal(value: &str) -> Option<Vec<u8>> {
    decode_literal(value, false)
}

fn decode_literal(value: &str, byte_characters: bool) -> Option<Vec<u8>> {
    let mut chars = value.chars().peekable();
    let mut decoded = vec![];
    while let Some(character) = chars.next() {
        if character != '\\' {
            if byte_characters && !character.is_ascii() {
                return None;
            }
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
                if byte_characters && !character.is_ascii() {
                    return None;
                }
                let mut encoded = [0; 4];
                decoded.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            }
            _ => return None,
        }
    }
    Some(decoded)
}

fn decode_prefixed_character(value: &str, encoding: StringEncoding) -> Option<usize> {
    let scalar = if let Some(digits) = value.strip_prefix("\\x") {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok()?
    } else if let Some(digits) = value.strip_prefix('\\').filter(|digits| {
        (1..=3).contains(&digits.len()) && digits.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
    }) {
        u32::from_str_radix(digits, 8).ok()?
    } else {
        let decoded = String::from_utf8(decode_literal(value, false)?).ok()?;
        let mut chars = decoded.chars();
        let scalar = chars.next()? as u32;
        if chars.next().is_some() {
            return None;
        }
        scalar
    };
    match encoding {
        StringEncoding::Utf16 if scalar <= 0xffff => Some(scalar as usize),
        StringEncoding::Utf32 => Some(scalar as usize),
        StringEncoding::Wide => Some(scalar as usize),
        _ => None,
    }
}

fn encode_string_literal(value: &str, encoding: StringEncoding) -> Vec<u8> {
    let utf8 = decode_string_literal(value).unwrap_or_default();
    if encoding == StringEncoding::Narrow || encoding == StringEncoding::Utf8 {
        return utf8;
    }
    let text = String::from_utf8_lossy(&utf8);
    let mut bytes = Vec::new();
    for character in text.chars() {
        let scalar = character as u32;
        match encoding {
            StringEncoding::Utf16 => {
                let value = scalar as u16;
                if scalar > 0xffff {
                    let value = scalar - 0x10000;
                    for unit in [
                        0xd800 | (value >> 10) as u16,
                        0xdc00 | (value & 0x3ff) as u16,
                    ] {
                        bytes.extend_from_slice(&unit.to_ne_bytes());
                    }
                } else {
                    bytes.extend_from_slice(&value.to_ne_bytes());
                }
            }
            StringEncoding::Utf32 => bytes.extend_from_slice(&scalar.to_ne_bytes()),
            StringEncoding::Wide => bytes.extend_from_slice(&scalar.to_ne_bytes()),
            StringEncoding::Narrow | StringEncoding::Utf8 => unreachable!(),
        }
    }
    bytes
}

#[derive(Default)]
struct Packing {
    current: Option<usize>,
    stack: Vec<(Option<String>, Option<usize>)>,
}

impl Packing {
    fn alignment(value: &str, pos: usize) -> Result<Option<usize>, error::SyntaxError> {
        let digits = value.trim_end_matches(['u', 'U', 'l', 'L']);
        if !matches!(
            value[digits.len()..].to_ascii_lowercase().as_str(),
            "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu"
        ) {
            return Err(error::SyntaxError::new(
                pos,
                "invalid pragma pack alignment",
            ));
        }
        let value = digits;
        let parsed = if let Some(value) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            usize::from_str_radix(value, 16)
        } else if value.len() > 1 && value.starts_with('0') {
            usize::from_str_radix(&value[1..], 8)
        } else {
            value.parse::<usize>()
        };
        match parsed {
            Ok(0) => Ok(None),
            Ok(value @ (1 | 2 | 4 | 8 | 16)) => Ok(Some(value)),
            _ => Err(error::SyntaxError::new(
                pos,
                "invalid pragma pack alignment",
            )),
        }
    }

    fn identifier(value: &str) -> bool {
        !value.is_empty()
            && value.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
            })
    }

    fn directive(&mut self, text: &str, pos: usize) -> Result<(), error::SyntaxError> {
        let Some(text) = text
            .strip_prefix('#')
            .unwrap()
            .trim_start()
            .strip_prefix("pragma")
        else {
            return Ok(());
        };
        if !text.starts_with(char::is_whitespace) {
            return Ok(());
        }
        let Some(text) = text.trim_start().strip_prefix("pack") else {
            return Ok(());
        };
        if !text.is_empty() && !text.starts_with('(') && !text.starts_with(char::is_whitespace) {
            return Ok(());
        }
        let invalid = || error::SyntaxError::new(pos, "invalid pragma pack directive");
        let text = text
            .trim()
            .strip_prefix('(')
            .and_then(|text| text.strip_suffix(')'))
            .ok_or_else(invalid)?;
        if text.trim().is_empty() {
            self.current = None;
            return Ok(());
        }
        let args = text.split(',').map(str::trim).collect::<Vec<_>>();
        if !matches!(args[0], "push" | "pop") {
            if args.len() != 1 {
                return Err(invalid());
            }
            self.current = Self::alignment(args[0], pos)?;
            return Ok(());
        }
        let mut label = None;
        let mut alignment = None;
        let mut next = 1;
        if args.get(next).is_some_and(|value| Self::identifier(value)) {
            label = Some(args[next]);
            next += 1;
        }
        if let Some(value) = args.get(next) {
            alignment = Some(Self::alignment(value, pos)?);
            next += 1;
        }
        if next != args.len() {
            return Err(invalid());
        }
        if args[0] == "push" {
            self.stack.push((label.map(str::to_owned), self.current));
        } else {
            let index = if let Some(label) = label {
                self.stack
                    .iter()
                    .rposition(|(name, _)| name.as_deref() == Some(label))
            } else {
                self.stack.len().checked_sub(1)
            }
            .ok_or_else(invalid)?;
            self.current = self.stack[index].1;
            self.stack.truncate(index);
        }
        if let Some(alignment) = alignment {
            self.current = alignment;
        }
        Ok(())
    }
}

pub struct Tokenizer {}

impl Tokenizer {
    fn reserved_token(pos: usize, s_value: &str) -> Token {
        Token {
            tt: TokenType::Reserved,
            pos,
            s_value: s_value.into(),
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
    fn ident_token(pos: usize, s_value: &str) -> Token {
        Token {
            tt: TokenType::Ident,
            pos,
            s_value: s_value.into(),
            ..Token::default()
        }
    }
    fn version_token(code: &str, start: usize) -> Option<(Token, usize)> {
        let bytes = code.as_bytes();
        if !bytes[start].is_ascii_digit() {
            return None;
        }
        let mut end = start;
        while bytes
            .get(end)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
        {
            end += 1;
        }
        let spelling = &code[start..end];
        if !spelling.contains('_')
            || spelling.split('_').any(str::is_empty)
            || bytes.get(end).is_some_and(u8::is_ascii_alphabetic)
        {
            return None;
        }
        // Attribute versions are opaque metadata, never C integer constants.
        Some((
            Token {
                tt: TokenType::Version,
                pos: start,
                s_value: spelling.into(),
                ..Token::default()
            },
            end,
        ))
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
        let extended = bytes.get(pos).is_some_and(|b| matches!(b, b'l' | b'L'));
        if single || extended {
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
                extended_float: if extended && cfg!(target_arch = "x86_64") {
                    use rustc_apfloat::Float;
                    let value = code[start..numeric_end]
                        .parse::<crate::floating::Extended>()
                        .map_err(|_| {
                            error::SyntaxError::new(start, "invalid long double literal")
                        })?;
                    Some(Box::new(value.to_bits()))
                } else {
                    None
                },
                i_value: if single {
                    (value as f32).to_bits() as usize
                } else {
                    value.to_bits() as usize
                },
                s_value: if single {
                    "f"
                } else if extended {
                    "l"
                } else {
                    "d"
                }
                .into(),
                ..Token::default()
            },
            pos,
        )))
    }
    fn refresh_string_spelling(token: &mut Token) {
        let mut spelling = String::with_capacity(token.string_bytes.len() * 4);
        for &byte in token.string_bytes.iter() {
            spelling.push('\\');
            spelling.push(char::from(b'0' + (byte >> 6)));
            spelling.push(char::from(b'0' + ((byte >> 3) & 7)));
            spelling.push(char::from(b'0' + (byte & 7)));
        }
        token.s_value = spelling.into();
    }

    pub fn tokenize(code: &str, is_debug: bool) -> Result<Vec<Token>, error::SyntaxError> {
        let mut tokens = vec![];
        let mut packing = Packing::default();
        let mut packing_changes = vec![];
        let reserved_symbols = Trie::new(&RESERVED_SYMBOLS);
        let bytes = code.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos].is_ascii_whitespace() {
                pos += 1;
                continue;
            }
            let start = pos;
            if let Some((token, end)) = Self::version_token(code, pos) {
                tokens.push(token);
                pos = end;
                continue;
            }
            if bytes[pos].is_ascii_digit()
                || (bytes[pos] == b'.' && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit))
            {
                if let Some((token, end)) = Self::float_token(code, pos)? {
                    tokens.push(token);
                    pos = end;
                    continue;
                }
            }
            let (string_encoding, prefix_size) = if code[pos..].starts_with("u8\"") {
                (StringEncoding::Utf8, 2)
            } else if matches!(bytes[pos], b'L' | b'u' | b'U')
                && matches!(bytes.get(pos + 1), Some(b'\"' | b'\''))
            {
                (
                    match bytes[pos] {
                        b'L' => StringEncoding::Wide,
                        b'u' => StringEncoding::Utf16,
                        b'U' => StringEncoding::Utf32,
                        _ => unreachable!(),
                    },
                    1,
                )
            } else {
                (StringEncoding::Narrow, 0)
            };
            pos += prefix_size;
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
                    let long_long = suffix.contains("ll")
                        || (radix == 10 && !suffix.contains('u') && value > i64::MAX as usize);
                    let unsigned = suffix.contains('u')
                        || (!wide && value > i32::MAX as usize)
                        || value > i64::MAX as usize;
                    token.s_value = smol_str::format_smolstr!(
                        "{}{}",
                        if long_long {
                            "ll"
                        } else if wide {
                            "l"
                        } else {
                            ""
                        },
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
                    let value = match &code[start..pos] {
                        "__restrict" | "__restrict__" => "restrict",
                        "__const" | "__const__" => "const",
                        "__volatile" | "__volatile__" => "volatile",
                        "__signed" | "__signed__" => "signed",
                        "__alignof" | "__alignof__" => "_Alignof",
                        value => value,
                    };
                    tokens.push(if is_reserved_word(value) || TYPES.contains(&value) {
                        Self::reserved_token(start, value)
                    } else {
                        Self::ident_token(start, value)
                    });
                }
                b'#' => {
                    while pos < bytes.len() && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                    packing.directive(&code[start..pos], start)?;
                    packing_changes.push((pos, packing.current));
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
                    let literal_start = start + prefix_size;
                    let value_start = literal_start + 1;
                    let value = &code[value_start..pos];
                    if quote == b'\'' && string_encoding != StringEncoding::Narrow {
                        let value = decode_prefixed_character(value, string_encoding).ok_or_else(|| error::SyntaxError::new(start, "prefixed character constant requires one representable code unit"))?;
                        let mut token = Self::num_token(start, value);
                        token.string_encoding = string_encoding;
                        tokens.push(token);
                        pos += 1;
                        continue;
                    }
                    let decoded = decode_literal(value, quote == b'\'').ok_or_else(|| {
                        error::SyntaxError::new(
                            start,
                            "invalid literal character or escape sequence",
                        )
                    })?;
                    let encoded = if quote == b'"' {
                        encode_string_literal(value, string_encoding)
                    } else {
                        decoded
                    };
                    pos += 1;
                    if quote == b'\'' {
                        if encoded.is_empty() {
                            return Err(error::SyntaxError::new(start, "empty character constant"));
                        }
                        let value = if encoded.len() == 1 {
                            usize::from(encoded[0])
                        } else {
                            // Pack bytes in source order, retaining the low 32 bits.
                            let packed = encoded
                                .iter()
                                .fold(0u32, |value, byte| (value << 8) | u32::from(*byte));
                            (packed as i32 as isize) as usize
                        };
                        tokens.push(Self::num_token(start, value));
                    } else {
                        tokens.push(Token {
                            tt: TokenType::Str,
                            pos: start,
                            s_value: value.into(),
                            string_encoding,
                            string_bytes: encoded.into(),
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
                    tokens.push(Self::reserved_token(start, &code[start..pos]));
                }
            }
        }
        let mut concatenation_error = None;
        // Rebuild spelling once per run, or before promoting its encoding.
        let mut spelling_dirty = false;
        // Compact adjacent strings in place instead of copying the whole token stream.
        tokens.dedup_by(|token, previous| {
            if concatenation_error.is_none()
                && token.tt == TokenType::Str
                && previous.tt == TokenType::Str
            {
                let encoding = match (previous.string_encoding, token.string_encoding) {
                    (left, right) if left == right => left,
                    (StringEncoding::Narrow, right) => right,
                    (left, StringEncoding::Narrow) => left,
                    _ => {
                        concatenation_error = Some(error::SyntaxError::new(
                            token.pos,
                            "incompatible string literal encodings",
                        ));
                        spelling_dirty = false;
                        return false;
                    }
                };
                let unit_size = encoding.unit_size();
                if previous.string_encoding != encoding {
                    if spelling_dirty {
                        Self::refresh_string_spelling(previous);
                    }
                    previous.string_bytes =
                        encode_string_literal(&previous.s_value, encoding).into();
                }
                let token_bytes = if token.string_encoding != encoding {
                    encode_string_literal(&token.s_value, encoding)
                } else {
                    std::mem::take(&mut token.string_bytes).into_vec()
                };
                previous.string_bytes.extend(token_bytes);
                previous.string_encoding = encoding;
                spelling_dirty = true;
                let aligned_len = previous.string_bytes.len() / unit_size * unit_size;
                previous.string_bytes.truncate(aligned_len);
                true
            } else {
                if spelling_dirty {
                    Self::refresh_string_spelling(previous);
                    spelling_dirty = false;
                }
                false
            }
        });
        if let Some(error) = concatenation_error {
            return Err(error);
        }
        if spelling_dirty {
            Self::refresh_string_spelling(tokens.last_mut().unwrap());
        }
        let mut change = 0;
        let mut current = None;
        for token in &mut tokens {
            while change < packing_changes.len() && packing_changes[change].0 <= token.pos {
                current = packing_changes[change].1;
                change += 1;
            }
            token.packing = current.map(|limit| limit as u8);
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
    fn attribute_versions_preserve_spelling_without_becoming_numbers() {
        let tokens = Tokenizer::tokenize("10_0 12_3_4 123 1.25 0x10", false).unwrap();
        for (token, spelling) in tokens.iter().zip(["10_0", "12_3_4"]) {
            assert_eq!(token.tt, TokenType::Version);
            assert_eq!(token.s_value, spelling);
        }
        assert_eq!(tokens[2].tt, TokenType::Num);
        assert_eq!(tokens[2].i_value, 123);
        assert_eq!(tokens[3].tt, TokenType::Float);
        assert_eq!(tokens[4].i_value, 16);
        for code in ["10__0", "10_", "10_0u", "10_0abc"] {
            assert!(Tokenizer::tokenize(code, false).is_err(), "{code}");
        }
    }

    #[test]
    fn prefixed_character_constants_decode_one_code_unit() {
        for (source, value, encoding) in [
            (r"L'\0'", 0, StringEncoding::Wide),
            (r"L'\xffffffff'", 4294967295, StringEncoding::Wide),
            ("L'Ω'", 937, StringEncoding::Wide),
            (r"u'\u03a9'", 937, StringEncoding::Utf16),
            (r"u'\xffff'", 65535, StringEncoding::Utf16),
            (r"U'\U0001f600'", 128512, StringEncoding::Utf32),
            (r"U'\xffffffff'", 4294967295, StringEncoding::Utf32),
        ] {
            let tokens = Tokenizer::tokenize(source, false).unwrap();
            assert_eq!(tokens[0].tt, TokenType::Num);
            assert_eq!(tokens[0].pos, 0);
            assert_eq!(tokens[0].i_value, value, "{source}");
            assert_eq!(tokens[0].string_encoding, encoding);
        }
        for source in [
            "L''",
            "L'ab'",
            r"u'\U0001f600'",
            r"u'\x10000'",
            r"U'\x100000000'",
            r"L'\uD800'",
            r"L'\x'",
            r"L'\x+1'",
            r"L'\7777'",
            "u'😀'",
        ] {
            assert!(Tokenizer::tokenize(source, false).is_err(), "{source}");
        }
    }

    #[test]
    fn multi_character_constants_pack_bytes_as_signed_ints() {
        for (code, expected) in [
            ("'ab'", 0x6162i32),
            ("'drag'", 0x64726167),
            (r"'A\0B'", 0x410042),
            (r"'\377ABC'", -12500413),
            (r"'\xff\xff\xff\xff'", -1),
            ("'abcdef'", 0x63646566),
            (r"'\\u'", 0x5c75),
        ] {
            let tokens = Tokenizer::tokenize(code, false).unwrap();
            assert_eq!(tokens[0].tt, TokenType::Num);
            assert_eq!(tokens[0].i_value as isize, expected as isize, "{code}");
            assert_eq!(tokens[0].s_value, "");
        }
    }

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
                s_value: String::from("int").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Ident,
                pos: 4,
                s_value: String::from("main").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 8,
                s_value: String::from("(").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 9,
                s_value: String::from(")").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 11,
                s_value: String::from("{").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 13,
                s_value: String::from("return").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Num,
                pos: 20,
                s_value: String::new().into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 21,
                s_value: String::from(";").into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 23,
                s_value: String::from("}").into(),
                i_value: 0,
                ..Token::default()
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
    fn wide_string_prefixes_encode_code_units_and_concatenate() {
        let tokens = Tokenizer::tokenize("u\"A\" \"B\"", false).unwrap();
        assert_eq!(tokens[0].string_encoding, StringEncoding::Utf16);
        assert_eq!(tokens[0].string_bytes.as_slice(), vec![65, 0, 66, 0]);
    }

    #[test]
    fn concatenated_strings_preserve_following_tokens_and_packing() {
        let code = "#pragma pack(push,1)\nu\"A\" /* gap */ \"B\" u\"C\";\n#pragma pack(pop)\n\"x\" \"y\";tail";
        let tokens = Tokenizer::tokenize(code, false).unwrap();
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].pos, code.find("u\"A\"").unwrap());
        assert_eq!(tokens[0].packing, Some(1));
        assert_eq!(tokens[0].string_encoding, StringEncoding::Utf16);
        assert_eq!(tokens[0].string_bytes.as_slice(), vec![65, 0, 66, 0, 67, 0]);
        assert_eq!(tokens[1].s_value, ";");
        assert_eq!(tokens[1].packing, Some(1));
        assert_eq!(tokens[2].string_bytes.as_slice(), b"xy");
        assert_eq!(tokens[2].packing, None);
        assert_eq!(tokens[3].s_value, ";");
        assert_eq!(tokens[4].s_value, "tail");
        assert_eq!(tokens[4].pos, code.find("tail").unwrap());
        assert_eq!(tokens[5].tt, TokenType::EOF);
        assert_eq!(tokens[5].pos, code.len());
    }

    #[test]
    fn concatenated_runs_finalize_spelling_and_promote_all_fragments() {
        let code = r#""" "A" "B" u"C" "D"; "\000" "\377" """#;
        let tokens = Tokenizer::tokenize(code, false).unwrap();
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].string_encoding, StringEncoding::Utf16);
        assert_eq!(tokens[0].decoded_string(), b"A\0B\0C\0D\0");
        assert_eq!(
            tokens[0].s_value,
            "\\101\\000\\102\\000\\103\\000\\104\\000"
        );
        assert_eq!(tokens[2].decoded_string(), [0, 255]);
        assert_eq!(tokens[2].s_value, "\\000\\377");
        assert_eq!(tokens[2].encoded_string(), [0, 255, 0]);
        assert_eq!(tokens[3].tt, TokenType::EOF);

        let incompatible = r#""A" "B" u"C" U"D""#;
        assert!(Tokenizer::tokenize(incompatible, false).is_err());
    }

    #[test]
    fn utf32_and_wide_strings_encode_scalar_units() {
        assert!(Tokenizer::tokenize("U\"😀\" L\"x\"", false).is_err());
        let utf32 = Tokenizer::tokenize("U\"😀\"", false).unwrap();
        assert_eq!(utf32[0].string_encoding, StringEncoding::Utf32);
        assert_eq!(
            utf32[0].string_bytes.as_slice(),
            vec![0x00, 0xf6, 0x01, 0x00]
        );
        let wide = Tokenizer::tokenize("L\"x\"", false).unwrap();
        assert_eq!(wide[0].string_encoding, StringEncoding::Wide);
        assert_eq!(wide[0].string_bytes.as_slice(), vec![120, 0, 0, 0]);
    }

    #[test]
    fn invalid_literals_return_syntax_errors() {
        for code in [
            "\"日本語",
            "\"abc\\",
            "/* unfinished",
            "''",
            "'あ'",
            "'\\u00e9'",
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

#[cfg(test)]
mod packing_tests {
    use super::*;

    #[test]
    fn packing_tracks_nested_named_frames_and_resets() {
        let code = "a;\n#pragma pack(push,outer,4)\nb;\n#pragma pack(push,inner,1)\nc;\n#pragma pack(pop,outer,2)\nd;\n#pragma pack(0)\ne;\n#pragma pack(push,8)\nf;\n#pragma pack()\ng;\n#pragma pack(pop)\nh;";
        let tokens = Tokenizer::tokenize(code, false).unwrap();
        let packing = tokens
            .iter()
            .filter(|token| token.tt == TokenType::Ident)
            .map(|token| token.packing)
            .collect::<Vec<_>>();
        assert_eq!(
            packing,
            vec![None, Some(4), Some(1), Some(2), None, Some(8), None, None]
        );
    }

    #[test]
    fn malformed_packing_is_diagnosed() {
        for code in [
            "#pragma pack(3)",
            "#pragma pack(4uuu)",
            "#pragma pack(32)",
            "#pragma pack(-1)",
            "#pragma pack(pop)",
            "#pragma pack(push,4,1)",
            "#pragma pack(push,)",
            "#pragma pack(push,label,2,4)",
            "#pragma pack(push,label,2)\n#pragma pack(pop,missing)",
        ] {
            assert!(Tokenizer::tokenize(code, false).is_err(), "{code}");
        }
    }
}

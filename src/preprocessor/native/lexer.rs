//! Tokenize preprocessing input and preserve token boundaries when spelling it.

use super::Token;

pub(super) fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 128
}

/// Preserve physical line numbers while performing line splicing and comment removal.
pub(super) fn lex(code: &str) -> Result<Vec<Token>, String> {
    let bytes = code.as_bytes();
    let mut spliced = Vec::new();
    let mut splices = vec![];
    let mut line = 1;
    let code = if bytes.contains(&b'\\') {
        spliced.reserve(bytes.len());
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos] == b'\\'
                && (bytes.get(pos + 1) == Some(&b'\n')
                    || bytes.get(pos + 1..pos + 3) == Some(b"\r\n"))
            {
                pos += if bytes[pos + 1] == b'\r' { 3 } else { 2 };
                splices.push(spliced.len());
            } else {
                spliced.push(bytes[pos]);
                pos += 1;
            }
        }
        std::str::from_utf8(&spliced).map_err(|e| e.to_string())?
    } else {
        code
    };
    let bytes = code.as_bytes();
    let (mut pos, mut space) = (0, false);
    let mut out = Vec::with_capacity((bytes.len() / 4 + 1).min(256));
    while pos < bytes.len() {
        let start = pos;
        let physical_line = line + splices.partition_point(|offset| *offset <= pos);
        let b = bytes[pos];
        if b == b'\n' {
            out.push(Token::new("\n", false, physical_line));
            pos += 1;
            line += 1;
            space = true;
            continue;
        }
        if b.is_ascii_whitespace() {
            pos += 1;
            space = true;
            continue;
        }
        if bytes.get(pos..pos + 2) == Some(b"//") {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            space = true;
            continue;
        }
        if bytes.get(pos..pos + 2) == Some(b"/*") {
            pos += 2;
            while pos < bytes.len() && bytes.get(pos..pos + 2) != Some(b"*/") {
                if bytes[pos] == b'\n' {
                    out.push(Token::new(
                        "\n",
                        false,
                        line + splices.partition_point(|offset| *offset <= pos),
                    ));
                    line += 1;
                }
                pos += 1;
            }
            if pos == bytes.len() {
                return Err(format!("line {line}: unterminated comment"));
            }
            pos += 2;
            space = true;
            continue;
        }
        let prefix = if bytes.get(pos..pos + 2) == Some(b"u8") && bytes.get(pos + 2) == Some(&b'"')
        {
            2
        } else if matches!(b, b'L' | b'u' | b'U')
            && matches!(bytes.get(pos + 1), Some(b'"' | b'\''))
        {
            1
        } else {
            0
        };
        let quote = bytes[pos + prefix];
        if quote == b'"' || quote == b'\'' {
            pos += prefix + 1;
            while pos < bytes.len() && bytes[pos] != quote && bytes[pos] != b'\n' {
                pos += if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
                    2
                } else {
                    1
                };
            }
            if bytes.get(pos) != Some(&quote) {
                return Err(format!("line {line}: unterminated literal"));
            }
            pos += 1;
        } else if b.is_ascii_digit()
            || b == b'.' && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit)
        {
            pos += 1;
            while pos < bytes.len()
                && (identifier(bytes[pos])
                    || bytes[pos] == b'.'
                    || matches!(bytes[pos], b'+' | b'-')
                        && matches!(bytes[pos - 1], b'e' | b'E' | b'p' | b'P'))
            {
                pos += 1;
            }
        } else if identifier(b) {
            pos += 1;
            while pos < bytes.len() && identifier(bytes[pos]) {
                pos += 1;
            }
        } else {
            let n = if bytes.get(pos..pos + 4) == Some(b"%:%:") {
                4
            } else if matches!(bytes.get(pos..pos + 3), Some(b"..." | b"<<=" | b">>=")) {
                3
            } else if matches!(
                bytes.get(pos..pos + 2),
                Some(
                    b"##"
                        | b"&&"
                        | b"||"
                        | b"<<"
                        | b">>"
                        | b"<="
                        | b">="
                        | b"=="
                        | b"!="
                        | b"++"
                        | b"--"
                        | b"->"
                        | b"+="
                        | b"-="
                        | b"*="
                        | b"/="
                        | b"%="
                        | b"&="
                        | b"|="
                        | b"^="
                        | b"<:"
                        | b":>"
                        | b"<%"
                        | b"%>"
                        | b"%:"
                )
            ) {
                2
            } else {
                1
            };
            pos += n;
        }
        let text = match &code[start..pos] {
            "%:" => "#",
            "%:%:" => "##",
            "<:" => "[",
            ":>" => "]",
            "<%" => "{",
            "%>" => "}",
            text => text,
        };
        out.push(Token::new(text, space, physical_line));
        space = false;
    }
    Ok(out)
}

pub(super) fn spelling(tokens: &[Token]) -> String {
    let mut result = String::new();
    for token in tokens {
        if token.is("\n") {
            continue;
        }
        if token.space && !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&token.text);
    }
    result
}

pub(super) fn needs_space(left: &str, right: &str) -> bool {
    let (Some(&a), Some(&b)) = (left.as_bytes().last(), right.as_bytes().first()) else {
        return false;
    };
    identifier(a) && identifier(b)
        || a == b'.' && b.is_ascii_digit()
        || left.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && (b == b'.' || matches!(a, b'e' | b'E' | b'p' | b'P') && matches!(b, b'+' | b'-'))
        || matches!(
            (a, b),
            (b'#', b'#')
                | (b'&', b'&' | b'=')
                | (b'|', b'|' | b'=')
                | (b'<', b'<' | b'=' | b':' | b'%')
                | (b'>', b'>' | b'=')
                | (b'=', b'=')
                | (b'!', b'=')
                | (b'+', b'+' | b'=')
                | (b'-', b'-' | b'=' | b'>')
                | (b'*', b'=')
                | (b'/', b'=' | b'/' | b'*')
                | (b'%', b'=' | b'>' | b':')
                | (b'^', b'=')
                | (b':', b'>')
                | (b'.', b'.')
        )
        || matches!(left, "L" | "u" | "U" | "u8") && matches!(b, b'\'' | b'"')
}

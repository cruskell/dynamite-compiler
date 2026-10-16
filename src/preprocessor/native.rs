//! C preprocessing without a compiler SDK or a subprocess.

mod expression;
mod profile;

use expression::Expression;
use smol_str::SmolStr;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Clone, Debug)]
struct Token {
    text: SmolStr,
    space: bool,
    line: usize,
    hidden: Option<Rc<Hidden>>,
}

/// Share disabled macro names across a replacement instead of copying a set per token.
#[derive(Debug)]
enum Hidden {
    Name(SmolStr, Option<Rc<Hidden>>),
    Union(Rc<Hidden>, Rc<Hidden>),
}

impl Hidden {
    fn contains(&self, name: &str) -> bool {
        match self {
            Self::Name(value, parent) => {
                value == name || parent.as_ref().is_some_and(|p| p.contains(name))
            }
            Self::Union(left, right) => left.contains(name) || right.contains(name),
        }
    }
}

impl Token {
    fn new(text: &str, space: bool, line: usize) -> Self {
        Self {
            text: text.into(),
            space,
            line,
            hidden: None,
        }
    }
    fn is(&self, text: &str) -> bool {
        self.text == text
    }
}

fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 128
}

/// Preserve physical line numbers while performing line splicing and comment removal.
fn lex(code: &str) -> Result<Vec<Token>, String> {
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

#[derive(Clone)]
enum Query {
    Builtin,
    Attribute,
    Zero,
}

#[derive(Clone)]
struct Macro {
    parameters: Option<Vec<SmolStr>>,
    variadic: bool,
    body: Vec<Token>,
    parameter_indices: Vec<Option<usize>>,
    has_paste: bool,
    query: Option<Query>,
}

struct Conditional {
    parent: bool,
    active: bool,
    taken: bool,
    had_else: bool,
}
struct Header {
    source: Rc<CleanSource>,
    guard: Option<SmolStr>,
}

struct CleanSource {
    text: String,
    lines: Vec<(usize, usize)>,
    directives: std::cell::RefCell<Vec<Option<Rc<Vec<Token>>>>>,
}

impl CleanSource {
    /// Scan comments and splices once; tokenize only the selected conditional branches.
    fn new(code: &str) -> Result<Self, String> {
        let bytes = code.as_bytes();
        let mut spliced = Vec::with_capacity(bytes.len());
        let mut splices = vec![];
        let (mut pos, mut run) = (0, 0);
        while pos < bytes.len() {
            if bytes[pos] == b'\\'
                && (bytes.get(pos + 1) == Some(&b'\n')
                    || bytes.get(pos + 1..pos + 3) == Some(b"\r\n"))
            {
                spliced.extend_from_slice(&bytes[run..pos]);
                pos += if bytes[pos + 1] == b'\r' { 3 } else { 2 };
                splices.push(spliced.len());
                run = pos;
            } else {
                pos += 1;
            }
        }
        spliced.extend_from_slice(&bytes[run..]);
        let bytes = spliced.as_slice();
        let mut output = Vec::with_capacity(bytes.len());
        let mut lines = vec![(0, 1)];
        let (mut pos, mut line, mut splice) = (0, 1, 0);
        let mut comment = false;
        let mut quote = None;
        while pos < bytes.len() {
            while splices.get(splice).is_some_and(|p| *p <= pos) {
                line += 1;
                splice += 1;
                output.extend_from_slice(b"\\\n");
            }
            if bytes[pos] == b'\n' {
                output.push(b'\n');
                line += 1;
                pos += 1;
                lines.push((output.len(), line));
                continue;
            }
            if comment {
                if bytes.get(pos..pos + 2) == Some(b"*/") {
                    comment = false;
                    pos += 2;
                } else {
                    pos += 1;
                }
            } else if let Some(delimiter) = quote {
                let byte = bytes[pos];
                output.push(byte);
                pos += 1;
                if byte == b'\\' {
                    if let Some(byte) = bytes.get(pos) {
                        output.push(*byte);
                        pos += 1;
                    }
                } else if byte == delimiter {
                    quote = None;
                }
            } else if bytes.get(pos..pos + 2) == Some(b"/*") {
                comment = true;
                output.push(b' ');
                pos += 2;
            } else if bytes.get(pos..pos + 2) == Some(b"//") {
                output.push(b' ');
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
            } else {
                let start = pos;
                // Stop before a splice so its physical-line marker stays in place.
                let limit = splices.get(splice).copied().unwrap_or(bytes.len());
                while pos < limit && !matches!(bytes[pos], b'\n' | b'/' | b'\'' | b'"') {
                    pos += 1;
                }
                if pos > start {
                    output.extend_from_slice(&bytes[start..pos]);
                    continue;
                }
                let byte = bytes[pos];
                if byte == b'\'' || byte == b'"' {
                    quote = Some(byte);
                }
                output.push(byte);
                pos += 1;
            }
        }
        if comment {
            return Err("unterminated comment".into());
        }
        let text = String::from_utf8(output).map_err(|e| e.to_string())?;
        let directives = std::cell::RefCell::new(vec![None; lines.len()]);
        Ok(Self {
            text,
            lines,
            directives,
        })
    }
    fn lines(&self) -> impl Iterator<Item = (usize, &str)> {
        self.lines.iter().enumerate().map(|(index, (start, line))| {
            let end = self
                .lines
                .get(index + 1)
                .map_or(self.text.len(), |(start, _)| start.saturating_sub(1));
            (*line, &self.text[*start..end])
        })
    }

    fn directive_tokens(&self, index: usize, text: &str) -> Result<Vec<Token>, String> {
        let mut cache = self.directives.borrow_mut();
        if cache[index].is_none() {
            cache[index] = Some(Rc::new(lex(text)?));
        }
        Ok(cache[index].as_ref().unwrap().as_ref().clone())
    }
}

fn directive_words(line: &str) -> impl Iterator<Item = &str> {
    line.trim_start()
        .strip_prefix('#')
        .unwrap_or("")
        .split_ascii_whitespace()
}

fn include_guard(source: &CleanSource) -> Option<SmolStr> {
    let mut lines = source
        .text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .peekable();
    let mut first = directive_words(lines.next()?);
    if first.next()? != "ifndef" {
        return None;
    }
    let guard = SmolStr::new(first.next()?);
    if first.next().is_some() {
        return None;
    }
    let mut second = directive_words(lines.next()?);
    if second.next()? != "define" || second.next()? != guard {
        return None;
    }
    let mut depth = 1usize;
    for line in lines.by_ref() {
        match directive_words(line).next() {
            Some("if" | "ifdef" | "ifndef") => depth += 1,
            Some("elif" | "else") if depth == 1 => return None,
            Some("endif") => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return lines.next().is_none().then_some(guard);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) struct Preprocessor {
    macros: HashMap<SmolStr, Rc<Macro>>,
    includes: Vec<PathBuf>,
    quotes: Vec<PathBuf>,
    include_cache: HashMap<(PathBuf, SmolStr, bool, bool), Option<PathBuf>>,
    search_origins: HashMap<PathBuf, usize>,
    headers: HashMap<PathBuf, Header>,
    once: HashSet<PathBuf>,
    counter: usize,
    depth: usize,
    output: String,
    base_file: String,
    pub(super) dependencies: Vec<PathBuf>,
    seen_dependencies: HashSet<PathBuf>,
    warnings_as_errors: bool,
    dependency_only: bool,
    stateful: HashSet<SmolStr>,
    // Keep historical edges after redefinition: extra expansion is safe; missing effects is not.
    dependents: HashMap<SmolStr, HashSet<SmolStr>>,
    builtin_names: HashSet<SmolStr>,
    include_context: PathBuf,
}

fn spelling(tokens: &[Token]) -> String {
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

fn needs_space(left: &str, right: &str) -> bool {
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

impl Preprocessor {
    pub(super) fn new(includes: Vec<PathBuf>, quotes: Vec<PathBuf>) -> Self {
        Self {
            macros: HashMap::new(),
            includes,
            quotes,
            include_cache: HashMap::new(),
            search_origins: HashMap::new(),
            headers: HashMap::new(),
            once: HashSet::new(),
            counter: 0,
            depth: 0,
            output: String::new(),
            base_file: String::new(),
            dependencies: vec![],
            seen_dependencies: HashSet::new(),
            warnings_as_errors: false,
            dependency_only: false,
            stateful: [SmolStr::new("__COUNTER__"), SmolStr::new("_Pragma")]
                .into_iter()
                .collect(),
            dependents: HashMap::new(),
            builtin_names: [
                "__LINE__",
                "__FILE__",
                "__BASE_FILE__",
                "__INCLUDE_LEVEL__",
                "__COUNTER__",
                "__has_include",
                "__has_include_next",
            ]
            .into_iter()
            .map(SmolStr::new)
            .collect(),
            include_context: PathBuf::new(),
        }
    }

    pub(super) fn define(&mut self, definition: &str) -> Result<(), String> {
        self.define_tokens(&lex(definition)?)
    }

    pub(super) fn undefine(&mut self, name: &str) {
        self.macros.remove(name);
        self.builtin_names.remove(name);
    }

    fn is_defined(&self, name: &str) -> bool {
        self.macros.contains_key(name) || self.builtin_names.contains(name)
    }

    fn define_tokens(&mut self, tokens: &[Token]) -> Result<(), String> {
        let Some(name) = tokens.first() else {
            return Err("missing macro name".into());
        };
        let mut index = 1;
        let mut parameters = None;
        let mut variadic = false;
        if tokens.get(index).is_some_and(|t| t.is("(") && !t.space) {
            index += 1;
            let mut names = vec![];
            while !tokens.get(index).is_some_and(|t| t.is(")")) {
                let token = tokens.get(index).ok_or("unterminated macro parameters")?;
                if token.is("...") {
                    variadic = true;
                    names.push("__VA_ARGS__".into());
                    index += 1;
                    break;
                }
                if !token
                    .text
                    .as_bytes()
                    .first()
                    .is_some_and(|b| identifier(*b) && !b.is_ascii_digit())
                {
                    return Err("invalid macro parameter".into());
                }
                names.push(token.text.clone());
                index += 1;
                if tokens.get(index).is_some_and(|t| t.is("...")) {
                    variadic = true;
                    index += 1;
                    break;
                }
                if !tokens.get(index).is_some_and(|t| t.is(",")) {
                    break;
                }
                index += 1;
            }
            if !tokens.get(index).is_some_and(|t| t.is(")")) {
                return Err("unterminated macro parameters".into());
            }
            index += 1;
            parameters = Some(names);
        }
        let body = tokens[index..].to_vec();
        let parameter_indices = body
            .iter()
            .map(|t| {
                parameters
                    .as_ref()
                    .and_then(|names| names.iter().position(|p| p == &t.text))
            })
            .collect();
        let has_paste = body.iter().any(|t| t.is("##"));
        if self.dependency_only {
            let mut effectful = has_paste;
            for token in &body {
                if token
                    .text
                    .as_bytes()
                    .first()
                    .is_some_and(|b| identifier(*b) && !b.is_ascii_digit())
                {
                    self.dependents
                        .entry(token.text.clone())
                        .or_default()
                        .insert(name.text.clone());
                    effectful |= self.stateful.contains(&token.text);
                }
            }
            if effectful {
                let mut pending = vec![name.text.clone()];
                while let Some(name) = pending.pop() {
                    if self.stateful.insert(name.clone()) {
                        if let Some(names) = self.dependents.get(&name) {
                            pending.extend(names.iter().cloned());
                        }
                    }
                }
            }
        }
        self.macros.insert(
            name.text.clone(),
            Rc::new(Macro {
                parameters,
                variadic,
                body,
                parameter_indices,
                has_paste,
                query: None,
            }),
        );
        Ok(())
    }

    fn expand(
        &mut self,
        tokens: Vec<Token>,
        file: &str,
        depth: usize,
    ) -> Result<Vec<Token>, String> {
        self.expand_inner(tokens, file, depth, false)
    }

    fn expand_inner(
        &mut self,
        tokens: Vec<Token>,
        file: &str,
        depth: usize,
        discard: bool,
    ) -> Result<Vec<Token>, String> {
        if depth > 200 {
            return Err("macro expansion nesting limit exceeded".into());
        }
        let mut out = if discard {
            Vec::new()
        } else {
            Vec::with_capacity(tokens.len())
        };
        let mut pending: VecDeque<_> = tokens.into();
        let mut steps = 0usize;
        while let Some(mut token) = pending.pop_front() {
            steps += 1;
            if steps > 10_000_000 {
                return Err("macro expansion token limit exceeded".into());
            }
            let name = token.text.clone();
            if !name
                .as_bytes()
                .first()
                .is_some_and(|b| identifier(*b) && !b.is_ascii_digit())
            {
                if !discard {
                    out.push(token);
                }
                continue;
            }
            if token
                .hidden
                .as_ref()
                .is_some_and(|hidden| hidden.contains(&name))
            {
                if !discard {
                    out.push(token);
                }
                continue;
            }
            if name == "_Pragma" {
                let open = pending.pop_front().ok_or("_Pragma requires parentheses")?;
                let value = pending
                    .pop_front()
                    .ok_or("_Pragma requires a string literal")?;
                let close = pending.pop_front().ok_or("_Pragma requires parentheses")?;
                if !open.is("(") || !close.is(")") || !value.text.starts_with('"') {
                    return Err("invalid _Pragma expression".into());
                }
                let text = value.text[1..value.text.len() - 1]
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\");
                if text.trim() == "once" {
                    self.once.insert(self.include_context.clone());
                }
                if !discard {
                    out.push(Token::new(&format!("#pragma {text}"), false, token.line));
                }
                continue;
            }
            if matches!(name.as_str(), "__has_include" | "__has_include_next")
                && self.builtin_names.contains(&name)
                && !self.macros.contains_key(&name)
            {
                if !pending.pop_front().is_some_and(|t| t.is("(")) {
                    return Err("__has_include requires parentheses".into());
                }
                let mut header = vec![];
                let mut nesting = 0usize;
                loop {
                    let value = pending.pop_front().ok_or("unterminated __has_include")?;
                    if value.is("(") {
                        nesting += 1;
                    }
                    if value.is(")") {
                        if nesting == 0 {
                            break;
                        }
                        nesting -= 1;
                    }
                    header.push(value);
                }
                let header = if header
                    .first()
                    .is_some_and(|t| t.is("<") || t.text.starts_with('"'))
                {
                    header
                } else {
                    self.expand(header, file, depth + 1)?
                };
                let text = header.iter().map(|t| t.text.as_str()).collect::<String>();
                let quoted = text.starts_with('"');
                let value = if quoted {
                    text.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                } else {
                    text.strip_prefix('<').and_then(|s| s.strip_suffix('>'))
                }
                .ok_or("invalid __has_include header")?;
                let current = self.include_context.clone();
                let found = self
                    .find_include(value, quoted, &current, name == "__has_include_next")
                    .is_some();
                token.text = if found { "1" } else { "0" }.into();
                if !discard {
                    out.push(token);
                }
                continue;
            }
            let builtin = if matches!(
                name.as_str(),
                "__LINE__" | "__FILE__" | "__BASE_FILE__" | "__INCLUDE_LEVEL__" | "__COUNTER__"
            ) && self.builtin_names.contains(&name)
                && !self.macros.contains_key(&name)
            {
                match name.as_str() {
                    "__LINE__" => Some(token.line.to_string()),
                    "__FILE__" => Some(format!("{file:?}")),
                    "__BASE_FILE__" => Some(format!("{:?}", self.base_file)),
                    "__INCLUDE_LEVEL__" => Some(self.depth.saturating_sub(1).to_string()),
                    "__COUNTER__" => {
                        let value = self.counter;
                        self.counter += 1;
                        Some(value.to_string())
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(value) = builtin {
                token.text = value.into();
                if !discard {
                    out.push(token);
                }
                continue;
            }
            let Some(definition) = self.macros.get(&name).cloned() else {
                if !discard {
                    out.push(token);
                }
                continue;
            };
            if definition.parameters.is_none() && !definition.has_paste {
                if discard && !self.stateful.contains(&name) {
                    continue;
                }
                if let [value] = definition.body.as_slice() {
                    if !value
                        .text
                        .as_bytes()
                        .first()
                        .is_some_and(|b| identifier(*b) && !b.is_ascii_digit())
                    {
                        let mut value = value.clone();
                        value.line = token.line;
                        value.space = token.space;
                        if !discard {
                            out.push(value);
                        }
                        continue;
                    }
                }
                let hidden = Rc::new(Hidden::Name(name, token.hidden.clone()));
                for (index, value) in definition.body.iter().enumerate().rev() {
                    let mut value = value.clone();
                    value.line = token.line;
                    value.hidden = Some(Rc::clone(&hidden));
                    if index == 0 {
                        value.space = token.space;
                    }
                    pending.push_front(value);
                }
                continue;
            }
            let mut omitted_variadic = false;
            let arguments = if let Some(parameters) = &definition.parameters {
                let n = pending.iter().take_while(|t| t.is("\n")).count();
                if !pending.get(n).is_some_and(|t| t.is("(")) {
                    if !discard {
                        out.push(token);
                    }
                    continue;
                }
                for _ in 0..=n {
                    pending.pop_front();
                }
                let mut arguments = vec![vec![]];
                let mut nesting = 0usize;
                loop {
                    let mut next = pending
                        .pop_front()
                        .ok_or_else(|| format!("unterminated invocation of {name}"))?;
                    if next.is("(") {
                        nesting += 1;
                    } else if next.is(")") {
                        if nesting == 0 {
                            break;
                        }
                        nesting -= 1;
                    } else if next.is(",")
                        && nesting == 0
                        && !(definition.variadic && arguments.len() >= parameters.len())
                    {
                        arguments.push(vec![]);
                        continue;
                    }
                    if next.is("\n") {
                        continue;
                    }
                    next.line = token.line;
                    arguments.last_mut().unwrap().push(next);
                }
                if parameters.is_empty() && arguments.len() == 1 && arguments[0].is_empty() {
                    arguments.clear();
                }
                if definition.variadic && arguments.len() + 1 == parameters.len() {
                    omitted_variadic = true;
                    arguments.push(vec![]);
                }
                if definition.variadic && parameters.len() == 1 && arguments[0].is_empty() {
                    omitted_variadic = true;
                }
                if arguments.len() != parameters.len() {
                    return Err(format!(
                        "macro {name} expects {} arguments, got {}",
                        parameters.len(),
                        arguments.len()
                    ));
                }
                Some(arguments)
            } else {
                None
            };
            if discard
                && !self.stateful.contains(&name)
                && arguments.as_ref().is_none_or(|args| {
                    args.iter()
                        .flatten()
                        .all(|t| !self.stateful.contains(&t.text))
                })
            {
                continue;
            }
            if let Some(query) = &definition.query {
                let args = &arguments.as_ref().unwrap()[0];
                let value = if args.len() != 1 {
                    false
                } else {
                    match query {
                        Query::Builtin => self
                            .macros
                            .contains_key(format!("__DYNAMITE_BUILTIN_{}", args[0].text).as_str()),
                        Query::Attribute => self.macros.contains_key(
                            format!("__DYNAMITE_ATTRIBUTE_{}", args[0].text).as_str(),
                        ),
                        Query::Zero => false,
                    }
                };
                token.text = if value { "1" } else { "0" }.into();
                if !discard {
                    out.push(token);
                }
                continue;
            }
            let mut substituted: Vec<Token> = vec![];
            let body = &definition.body;
            let mut index = 0;
            let mut expanded_arguments: Vec<Option<Vec<Token>>> =
                vec![None; arguments.as_ref().map_or(0, Vec::len)];
            while index < body.len() {
                let current = &body[index];
                if current.is("#") && arguments.is_some() {
                    index += 1;
                    let arg = definition
                        .parameter_indices
                        .get(index)
                        .copied()
                        .flatten()
                        .ok_or("# must precede a macro parameter")?;
                    let string = spelling(&arguments.as_ref().unwrap()[arg])
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"");
                    substituted.push(Token::new(
                        &format!("\"{string}\""),
                        current.space,
                        token.line,
                    ));
                } else if current.is("##")
                    && definition.variadic
                    && index > 0
                    && body[index - 1].is(",")
                    && definition
                        .parameter_indices
                        .get(index + 1)
                        .copied()
                        .flatten()
                        == definition.parameters.as_ref().map(|p| p.len() - 1)
                {
                    substituted.push(Token::new("##gnu", current.space, token.line));
                } else if let Some(arg) = definition.parameter_indices[index] {
                    let pasted = index > 0 && body[index - 1].is("##")
                        || body.get(index + 1).is_some_and(|t| t.is("##"));
                    let raw = &arguments.as_ref().unwrap()[arg];
                    let values = if pasted {
                        raw
                    } else {
                        if expanded_arguments[arg].is_none() {
                            expanded_arguments[arg] =
                                Some(self.expand(raw.clone(), file, depth + 1)?);
                        }
                        expanded_arguments[arg].as_ref().unwrap()
                    };
                    if values.is_empty() && pasted {
                        substituted.push(Token::new("", current.space, token.line));
                    } else {
                        let start = substituted.len();
                        substituted.extend_from_slice(values);
                        if let Some(first) = substituted.get_mut(start) {
                            first.space = current.space;
                        }
                    }
                } else {
                    substituted.push(current.clone());
                }
                index += 1;
            }
            let mut replacement = if !definition.has_paste {
                substituted
            } else {
                let mut replacement: Vec<Token> = Vec::with_capacity(substituted.len());
                let mut index = 0;
                while index < substituted.len() {
                    if substituted[index].is("##") || substituted[index].is("##gnu") {
                        let gnu = substituted[index].is("##gnu");
                        let left = replacement
                            .pop()
                            .ok_or("## cannot start a macro replacement")?;
                        index += 1;
                        let right = substituted
                            .get(index)
                            .ok_or("## cannot end a macro replacement")?;
                        // GNU variadic macros permit comma elision for an omitted tail.
                        if gnu {
                            if !omitted_variadic {
                                replacement.extend([left, right.clone()]);
                            }
                        } else {
                            let joined = format!("{}{}", left.text, right.text);
                            if !joined.is_empty() {
                                let mut pasted = lex(&joined)?;
                                if pasted.len() != 1 {
                                    return Err(format!("invalid token paste: {joined}"));
                                }
                                let mut value = pasted.pop().unwrap();
                                value.space = left.space;
                                value.hidden = left.hidden;
                                replacement.push(value);
                            } else {
                                replacement.push(left);
                            }
                        }
                    } else {
                        replacement.push(substituted[index].clone());
                    }
                    index += 1;
                }
                replacement.retain(|t| !t.text.is_empty());
                replacement
            };
            let hidden = Rc::new(Hidden::Name(name, token.hidden.clone()));
            for value in &mut replacement {
                value.line = token.line;
                value.hidden = Some(match value.hidden.take() {
                    None => Rc::clone(&hidden),
                    Some(existing) => Rc::new(Hidden::Union(existing, Rc::clone(&hidden))),
                });
            }
            if let Some(first) = replacement.first_mut() {
                first.space = token.space;
            }
            for value in replacement.into_iter().rev() {
                pending.push_front(value);
            }
        }
        Ok(out)
    }

    fn condition(&mut self, tokens: &[Token], file: &str) -> Result<bool, String> {
        let mut replaced = vec![];
        let mut index = 0;
        while index < tokens.len() {
            if tokens[index].is("defined") {
                index += 1;
                let parens = tokens.get(index).is_some_and(|t| t.is("("));
                if parens {
                    index += 1;
                }
                let name = tokens.get(index).ok_or("defined requires an identifier")?;
                let defined = self.is_defined(&name.text);
                replaced.push(Token::new(if defined { "1" } else { "0" }, true, name.line));
                index += 1;
                if parens {
                    if !tokens.get(index).is_some_and(|t| t.is(")")) {
                        return Err("missing ')' after defined".into());
                    }
                    index += 1;
                }
            } else {
                replaced.push(tokens[index].clone());
                index += 1;
            }
        }
        let expanded = self.expand(replaced, file, 0)?;
        Expression::evaluate(&expanded).map(|v| v.bits != 0)
    }

    fn find_include(
        &mut self,
        name: &str,
        quoted: bool,
        current: &Path,
        next: bool,
    ) -> Option<PathBuf> {
        let parent = current.parent().unwrap_or(Path::new("."));
        let origin = self.search_origins.get(current).copied();
        let key = (
            if next {
                current.to_path_buf()
            } else {
                parent.to_path_buf()
            },
            SmolStr::new(name),
            quoted,
            next,
        );
        if let Some(result) = self.include_cache.get(&key) {
            return result.clone();
        }
        let paths: Vec<_> = if quoted && !next {
            std::iter::once(parent.to_path_buf())
                .chain(self.quotes.iter().cloned())
                .chain(self.includes.iter().cloned())
                .collect()
        } else {
            self.includes.clone()
        };
        let skip = if next { origin.map_or(0, |i| i + 1) } else { 0 };
        let result = paths.into_iter().skip(skip).find_map(|directory| {
            let path = directory.join(name);
            if !path.is_file() {
                return None;
            }
            if let Some(index) = self.includes.iter().position(|p| p == &directory) {
                self.search_origins.insert(path.clone(), index);
            }
            Some(path)
        });
        self.include_cache.insert(key, result.clone());
        result
    }

    fn flush(&mut self, pending: &mut Vec<Token>, file: &str) -> Result<(), String> {
        if pending.is_empty() {
            return Ok(());
        }
        let line = pending[0].line;
        self.output.push_str(&format!("# {line} {file:?}\n"));
        let expanded = self.expand(std::mem::take(pending), file, 0)?;
        if self.dependency_only {
            return Ok(());
        }
        let mut current_line = line;
        let mut previous = SmolStr::default();
        for token in expanded {
            if token.text.starts_with("#pragma ") {
                if !self.output.ends_with('\n') {
                    self.output.push('\n');
                }
                self.output.push_str(&token.text);
                self.output.push('\n');
                self.output
                    .push_str(&format!("# {current_line} {file:?}\n"));
                previous = SmolStr::default();
            } else if token.is("\n") {
                self.output.push('\n');
                current_line += 1;
                previous = SmolStr::default();
            } else {
                if token.line != current_line {
                    if !self.output.ends_with('\n') {
                        self.output.push('\n');
                    }
                    self.output
                        .push_str(&format!("# {} {file:?}\n", token.line));
                    current_line = token.line;
                    previous = SmolStr::default();
                }
                if !previous.is_empty() && (token.space || needs_space(&previous, &token.text)) {
                    self.output.push(' ');
                }
                self.output.push_str(&token.text);
                previous = token.text;
            }
        }
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        Ok(())
    }

    fn flush_dependency(
        &mut self,
        lines: &mut Vec<(usize, &str)>,
        file: &str,
    ) -> Result<(), String> {
        if lines.iter().any(|(_, text)| {
            text.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|word| self.stateful.contains(word))
        }) {
            let mut tokens = vec![];
            for (line, text) in lines.iter() {
                let mut values = lex(text)?;
                for value in &mut values {
                    value.line += line - 1;
                }
                if let Some(first) = values.first_mut() {
                    first.space = true;
                }
                tokens.extend(values);
                tokens.push(Token::new("\n", false, *line));
            }
            self.expand_inner(tokens, file, 0, true)?;
        }
        lines.clear();
        Ok(())
    }

    pub(super) fn process(&mut self, code: &str, path: &Path) -> Result<String, String> {
        self.base_file = path.to_string_lossy().into_owned();
        self.file(code, path)?;
        Ok(std::mem::take(&mut self.output))
    }

    fn file(&mut self, code: &str, path: &Path) -> Result<(), String> {
        if self.depth >= 200 {
            return Err("include nesting limit exceeded".into());
        }
        if self.once.contains(path) {
            return Ok(());
        }
        if let Some(header) = self.headers.get(path) {
            if header
                .guard
                .as_ref()
                .is_some_and(|guard| self.macros.contains_key(guard))
            {
                return Ok(());
            }
        }
        let source = if let Some(header) = self.headers.get(path) {
            Rc::clone(&header.source)
        } else {
            let source = Rc::new(CleanSource::new(code)?);
            let guard = include_guard(&source);
            self.headers.insert(
                path.to_path_buf(),
                Header {
                    source: Rc::clone(&source),
                    guard,
                },
            );
            source
        };
        if self.seen_dependencies.insert(path.to_path_buf()) {
            self.dependencies.push(path.to_path_buf());
        }
        self.depth += 1;
        let previous_context = std::mem::replace(&mut self.include_context, path.to_path_buf());
        let mut file = path.to_string_lossy().into_owned();
        let mut line_offset: isize = 0;
        let mut conditions: Vec<Conditional> = vec![];
        let mut pending = vec![];
        let mut dependency_lines = vec![];
        for (line_index, (physical_line, text)) in source.lines().enumerate() {
            let active = conditions.last().is_none_or(|c| c.active);
            let text = text.trim_start();
            if text.is_empty() {
                continue;
            }
            if !active {
                let directive = text
                    .strip_prefix('#')
                    .or_else(|| text.strip_prefix("%:"))
                    .map(str::trim_start)
                    .unwrap_or("");
                let name = directive
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .next()
                    .unwrap_or("");
                if !matches!(name, "if" | "ifdef" | "ifndef" | "elif" | "else" | "endif") {
                    continue;
                }
            }
            if self.dependency_only && !text.starts_with('#') && !text.starts_with("%:") {
                dependency_lines.push((physical_line.saturating_add_signed(line_offset), text));
                continue;
            }
            let mut line = if self.dependency_only {
                source.directive_tokens(line_index, text)?
            } else {
                lex(text)?
            };
            if let Some(first) = line.first_mut() {
                first.space = true;
            }
            for token in &mut line {
                token.line = (physical_line + token.line - 1).saturating_add_signed(line_offset);
            }
            if line.first().is_some_and(|t| t.is("#")) {
                self.flush_dependency(&mut dependency_lines, &file)?;
                self.flush(&mut pending, &file)?;
                let Some(directive) = line.get(1) else {
                    continue;
                };
                let args = &line[2..];
                let name = directive.text.as_str();
                match name {
                    "if" | "ifdef" | "ifndef" => {
                        let value = active
                            && if name == "if" {
                                self.condition(args, &file)?
                            } else {
                                let defined =
                                    args.first().is_some_and(|t| self.is_defined(&t.text));
                                if name == "ifdef" { defined } else { !defined }
                            };
                        conditions.push(Conditional {
                            parent: active,
                            active: value,
                            taken: value,
                            had_else: false,
                        });
                    }
                    "elif" => {
                        let condition = conditions.last().ok_or("#elif without #if")?;
                        if condition.had_else {
                            return Err("#elif after #else".into());
                        }
                        let value =
                            condition.parent && !condition.taken && self.condition(args, &file)?;
                        let condition = conditions.last_mut().unwrap();
                        condition.active = value;
                        condition.taken |= value;
                    }
                    "else" => {
                        let condition = conditions.last_mut().ok_or("#else without #if")?;
                        if condition.had_else {
                            return Err("duplicate #else".into());
                        }
                        condition.had_else = true;
                        condition.active = condition.parent && !condition.taken;
                        condition.taken = true;
                    }
                    "endif" => {
                        conditions.pop().ok_or("#endif without #if")?;
                    }
                    _ if !active => {}
                    "define" => self.define_tokens(args)?,
                    "undef" => {
                        self.undefine(&args.first().ok_or("missing macro name")?.text);
                    }
                    "include" | "include_next" => {
                        let expanded = if args
                            .first()
                            .is_some_and(|t| t.is("<") || t.text.starts_with('"'))
                        {
                            args.to_vec()
                        } else {
                            self.expand(args.to_vec(), &file, 0)?
                        };
                        let spelling = spelling(&expanded);
                        let (header, quoted) = if let Some(value) =
                            spelling.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                        {
                            (value.to_string(), true)
                        } else if expanded.first().is_some_and(|t| t.is("<"))
                            && expanded.last().is_some_and(|t| t.is(">"))
                        {
                            (
                                expanded[1..expanded.len() - 1]
                                    .iter()
                                    .map(|t| t.text.as_str())
                                    .collect::<String>(),
                                false,
                            )
                        } else {
                            return Err(format!("{file}: invalid include: {spelling}"));
                        };
                        let included = self
                            .find_include(&header, quoted, path, name == "include_next")
                            .ok_or_else(|| format!("{file}: header not found: {header}"))?;
                        if !self.once.contains(&included)
                            && !self
                                .headers
                                .get(&included)
                                .and_then(|h| h.guard.as_ref())
                                .is_some_and(|g| self.macros.contains_key(g))
                        {
                            let contents = if self.headers.contains_key(&included) {
                                String::new()
                            } else {
                                fs::read_to_string(&included)
                                    .map_err(|e| format!("{}: {e}", included.display()))?
                            };
                            self.file(&contents, &included)?;
                        }
                    }
                    "line" => {
                        let expanded = self.expand(args.to_vec(), &file, 0)?;
                        let number: usize = expanded
                            .first()
                            .ok_or("missing line number")?
                            .text
                            .parse()
                            .map_err(|_| "invalid line number")?;

                        line_offset = number as isize - physical_line as isize - 1;
                        if let Some(name) = expanded.get(1) {
                            file = name.text.trim_matches('"').to_string();
                        }
                    }
                    _ if name.bytes().all(|b| b.is_ascii_digit()) => {
                        let number: usize = name.parse().map_err(|_| "invalid line number")?;
                        line_offset = number as isize - physical_line as isize - 1;
                        if let Some(name) = args.first() {
                            file = name.text.trim_matches('"').to_string();
                        }
                    }
                    "error" => {
                        return Err(format!(
                            "{file}:{}: #error {}",
                            directive.line,
                            spelling(args)
                        ));
                    }
                    "warning" => {
                        if self.warnings_as_errors {
                            return Err(format!(
                                "{file}:{}: #warning {} [-Werror]",
                                directive.line,
                                spelling(args)
                            ));
                        }
                        eprintln!("{file}:{}: warning: {}", directive.line, spelling(args))
                    }
                    "pragma" if args.first().is_some_and(|t| t.is("once")) => {
                        self.once.insert(path.to_path_buf());
                    }
                    "pragma" => {
                        self.output.push_str(&format!(
                            "# {} {file:?}\n#pragma {}\n",
                            directive.line,
                            spelling(args)
                        ));
                    }
                    "ident" => {}
                    _ => {
                        return Err(format!(
                            "{file}:{}: unsupported directive #{name}",
                            directive.line
                        ));
                    }
                }
            } else if active {
                pending.extend(line);
                pending.push(Token::new(
                    "\n",
                    false,
                    physical_line.saturating_add_signed(line_offset),
                ));
            }
        }
        self.flush_dependency(&mut dependency_lines, &file)?;
        self.flush(&mut pending, &file)?;
        self.depth -= 1;
        self.include_context = previous_context;
        if !conditions.is_empty() {
            return Err(format!("{file}: unterminated conditional directive"));
        }
        Ok(())
    }
}

fn make_quote(text: &str) -> String {
    text.replace('$', "$$")
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace('\t', "\\\t")
        .replace('#', "\\#")
}

pub(super) fn preprocess(
    code: &str,
    path: &Path,
    options: &super::PreprocessorOptions,
    arguments: &[String],
) -> Result<String, String> {
    preprocess_cached(code, path, options, arguments, &mut HashMap::new())
}

pub(super) fn preprocess_many(paths: &[PathBuf], arguments: &[String]) -> Result<String, String> {
    let dependencies = arguments
        .iter()
        .any(|arg| matches!(arg.as_str(), "-M" | "-MM"));
    let mut dependency_file = None;
    let mut filtered = vec![];
    let mut index = 0;
    while index < arguments.len() {
        let arg = &arguments[index];
        if dependencies && arg == "-MF" {
            index += 1;
            dependency_file = Some(
                arguments
                    .get(index)
                    .ok_or("-MF requires an argument")?
                    .clone(),
            );
        } else if dependencies && arg.starts_with("-MF") {
            dependency_file = Some(arg[3..].to_string());
        } else {
            filtered.push(arg.clone());
        }
        index += 1;
    }
    let mut headers = HashMap::new();
    let mut result = String::new();
    for path in paths {
        let code = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        result.push_str(&preprocess_cached(
            &code,
            path,
            &super::PreprocessorOptions {
                system_headers: true,
                ..Default::default()
            },
            &filtered,
            &mut headers,
        )?);
    }
    if let Some(file) = dependency_file.filter(|file| file != "-") {
        fs::write(file, &result).map_err(|e| e.to_string())?;
        Ok(String::new())
    } else {
        Ok(result)
    }
}

fn preprocess_cached(
    code: &str,
    path: &Path,
    options: &super::PreprocessorOptions,
    arguments: &[String],
    headers: &mut HashMap<PathBuf, Header>,
) -> Result<String, String> {
    if !cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        return Err("native preprocessing currently supports Linux x86_64".into());
    }
    let bundled = if options.system_headers {
        Some(super::IncludeDirectory::native()?)
    } else {
        Some(super::IncludeDirectory::new()?)
    };
    let mut includes = options.include_directories.clone();
    let mut quotes = vec![];
    let mut after = vec![];
    let mut system_includes = vec![];
    let mut definitions = vec![];
    let mut forced = vec![];
    let mut dependency_file = None;
    let mut dependency_targets = vec![];
    let mut phony = false;
    let mut dependency_only = false;
    let mut omit_system = false;
    let mut no_markers = false;
    let mut optimization = "0";
    let mut standard = "gnu99";
    let mut warnings_as_errors = false;
    let mut index = 0;
    while index < arguments.len() {
        let arg = &arguments[index];
        let mut value = |prefix: &str| -> Result<String, String> {
            if arg.len() > prefix.len() {
                Ok(arg[prefix.len()..].to_string())
            } else {
                index += 1;
                arguments
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("{arg} requires an argument"))
            }
        };
        match arg.as_str() {
            "-MD" => {}
            "-MMD" => omit_system = true,
            "-M" | "-MM" => {
                dependency_only = true;
                omit_system = arg == "-MM";
            }
            "-MP" => phony = true,
            "-P" => no_markers = true,
            "-E" => {}
            "-Werror" | "-Werror=cpp" => warnings_as_errors = true,
            "-Wno-error" | "-Wno-error=cpp" => warnings_as_errors = false,
            "-pthread" => definitions.push((true, "_REENTRANT=1".into())),
            _ if arg.starts_with("-MF") => dependency_file = Some(value("-MF")?),
            _ if arg.starts_with("-MT") || arg.starts_with("-MQ") => {
                let text = value(&arg[..3])?;
                dependency_targets.push(if arg.starts_with("-MQ") {
                    make_quote(&text)
                } else {
                    text
                });
            }
            _ if arg.starts_with("-isystem") => {
                system_includes.push(PathBuf::from(value("-isystem")?))
            }
            _ if arg.starts_with("-iquote") => quotes.push(value("-iquote")?.into()),
            _ if arg.starts_with("-idirafter") => after.push(PathBuf::from(value("-idirafter")?)),
            _ if arg.starts_with("-include") => forced.push((false, value("-include")?)),
            _ if arg.starts_with("-imacros") => forced.push((true, value("-imacros")?)),
            _ if arg.starts_with("-I") => includes.push(value("-I")?.into()),
            _ if arg.starts_with("-D") => definitions.push((true, value("-D")?)),
            _ if arg.starts_with("-U") => definitions.push((false, value("-U")?)),
            _ if arg.starts_with("-O") => optimization = &arg[2..],
            _ if arg.starts_with("-std=") => standard = &arg[5..],
            _ if arg.starts_with("-W") => {}
            _ => return Err(format!("unsupported native preprocessing option: {arg}")),
        }
        index += 1;
    }
    if let Some(paths) = std::env::var_os("CPATH").filter(|p| !p.is_empty()) {
        includes.extend(std::env::split_paths(&paths).map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p
            }
        }));
    }
    if let Some(paths) = std::env::var_os("C_INCLUDE_PATH").filter(|p| !p.is_empty()) {
        system_includes.extend(std::env::split_paths(&paths).map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p
            }
        }));
    }
    let mut system_paths = system_includes;
    let builtin_path = bundled.as_ref().unwrap().0.clone();
    system_paths.push(builtin_path.clone());
    if options.system_headers {
        system_paths.extend([
            PathBuf::from("/usr/local/include"),
            PathBuf::from("/usr/include/x86_64-linux-gnu"),
            PathBuf::from("/usr/include"),
        ]);
    }
    includes.extend(system_paths.iter().cloned());
    includes.extend(after);
    let mut pp = Preprocessor::new(includes, quotes);
    pp.headers = std::mem::take(headers);
    pp.dependency_only = dependency_only;
    pp.warnings_as_errors = warnings_as_errors;
    profile::define(&mut pp)?;
    if optimization != "0" {
        pp.define("__OPTIMIZE__ 1")?;
        pp.undefine("__NO_INLINE__");
        if matches!(optimization, "s" | "z") {
            pp.define("__OPTIMIZE_SIZE__ 1")?;
        }
    }
    let version = match standard {
        "gnu89" | "gnu90" | "c89" | "c90" => None,
        "gnu99" | "c99" => Some("199901L"),
        "gnu11" | "c11" => Some("201112L"),
        "gnu17" | "c17" | "gnu18" | "c18" => Some("201710L"),
        _ => {
            return Err(format!(
                "unsupported native preprocessing standard: {standard}"
            ));
        }
    };
    if let Some(version) = version {
        pp.define(&format!("__STDC_VERSION__ {version}"))?;
    } else {
        pp.undefine("__STDC_VERSION__");
        pp.undefine("__GNUC_STDC_INLINE__");
        pp.define("__GNUC_GNU_INLINE__ 1")?;
    }
    if !standard.starts_with("gnu") {
        pp.define("__STRICT_ANSI__ 1")?;
        pp.undefine("linux");
        pp.undefine("unix");
    }
    for arg in super::compiler_macro_arguments() {
        if let Some(name) = arg.strip_prefix("-U") {
            pp.undefine(name);
        }
        if let Some(definition) = arg.strip_prefix("-D") {
            let (name, value) = definition.split_once('=').unwrap_or((definition, "1"));
            pp.define(&format!("{name} {value}"))?;
        }
    }
    for (name, query) in [
        ("__has_builtin", Query::Builtin),
        ("__has_attribute", Query::Attribute),
        ("__has_feature", Query::Zero),
        ("__has_extension", Query::Zero),
        ("__has_c_attribute", Query::Zero),
        ("__has_cpp_attribute", Query::Zero),
        ("__has_declspec_attribute", Query::Zero),
        ("__has_warning", Query::Zero),
        ("__has_constexpr_builtin", Query::Zero),
    ] {
        Rc::get_mut(pp.macros.get_mut(name).unwrap()).unwrap().query = Some(query);
    }
    for definition in &options.definitions {
        definitions.push((true, definition.clone()));
    }
    for name in &options.undefinitions {
        definitions.push((false, name.clone()));
    }
    for (define, text) in definitions {
        if define {
            let (name, value) = text.split_once('=').unwrap_or((&text, "1"));
            pp.define(&format!("{name} {value}"))?;
        } else {
            pp.undefine(&text);
        }
    }
    pp.base_file = path.to_string_lossy().into_owned();
    for (macros_only, file) in forced {
        let included = if Path::new(&file).is_file() {
            PathBuf::from(file)
        } else {
            pp.find_include(&file, true, path, false)
                .ok_or_else(|| format!("header not found: {file}"))?
        };
        let contents = fs::read_to_string(&included).map_err(|e| e.to_string())?;
        pp.file(&contents, &included)?;
        if macros_only {
            pp.output.clear();
        }
    }
    let output = pp.process(code, path)?;
    *headers = std::mem::take(&mut pp.headers);
    if dependency_file.is_some() || dependency_only {
        let escape = make_quote;
        let target = if dependency_targets.is_empty() {
            escape(
                &Path::new(path.file_name().unwrap_or(path.as_os_str()))
                    .with_extension("o")
                    .to_string_lossy(),
            )
        } else {
            dependency_targets.join(" ")
        };
        let deps: Vec<_> = pp
            .dependencies
            .iter()
            .filter(|p| {
                !p.starts_with(&builtin_path)
                    && (!omit_system || !system_paths.iter().any(|base| p.starts_with(base)))
            })
            .collect();
        let mut text = format!("{target}:");
        for dependency in &deps {
            text.push(' ');
            text.push_str(&escape(&dependency.to_string_lossy()));
        }
        text.push('\n');
        if phony {
            for dependency in deps.iter().filter(|p| p.as_path() != path) {
                text.push_str(&format!("{}:\n", escape(&dependency.to_string_lossy())));
            }
        }
        if let Some(file) = dependency_file.as_deref().filter(|file| *file != "-") {
            fs::write(file, &text).map_err(|e| e.to_string())?;
        }
        if dependency_only {
            return Ok(
                if dependency_file.as_deref().is_none_or(|file| file == "-") {
                    text
                } else {
                    String::new()
                },
            );
        }
    }
    if no_markers {
        Ok(output
            .lines()
            .filter(|line| !line.starts_with("# "))
            .map(|l| format!("{l}\n"))
            .collect())
    } else {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn tokens(code: &str) -> Vec<String> {
        lex(&code
            .lines()
            .filter(|line| !line.starts_with("# "))
            .collect::<Vec<_>>()
            .join("\n"))
        .unwrap()
        .into_iter()
        .filter(|t| !t.is("\n"))
        .map(|t| t.text.to_string())
        .collect()
    }

    #[test]
    fn macro_expansion_matches_c_preprocessing() {
        let cases = [
            "#define F(x) x+x\n#define A F\nA(3) F(F(2))\n",
            "#define A B\n#define B A\nA B\n",
            "#define f(x) x f\nf(1)(2)\n",
            "#define cat(a,b) a##b\n#define X 42\ncat(X,) cat(,X) cat(,)\n",
            "#define str(x) #x\nstr(hello   world) str(\"a\\\\b\")\n",
            "#define arg(x) x\narg(1\n+2)\n",
            "#define V(x,...) f(x,##__VA_ARGS__)\nV(1) V(1,2,3)\n",
            "#define A 9\n#define S(x) #x\n#define E(x) S(x)\nS(A) E(A)\n",
            "#define F(x) x\n#define G F\nG(G)(2)\n",
            "#define F(x) x\n#define H(x) F(x)\nH(F(1))\n",
            "#define F(x) x\nF(F)(2)\n",
            "#define F(x) x\nF\n(3)\n",
            "#define V(args...) args\nV(1,2)\n",
            "#define V(x,...) f(x,##__VA_ARGS__)\nV(1,) V(1)\n",
            "#define S(x) #x\nS(a\n b)\n",
            "#define x 3\n#define f(a) f(x * (a))\n#undef x\n#define x 2\n#define g f\n#define z z[0]\n#define h g(~\n#define m(a) a(w)\n#define w 0,1\n#define t(a) a\nf(y+1)+ f(f(z)) % t(t(g)(0)+t)(1);\ng(x+(3,4)-w)| h 5) & m(f)^m(m);\n",
            "#define x 7\n#if defined(x) && !defined missing\nx\n#else\n#error wrong\n#endif\n",
            "#if 0 && 1/0\n#error wrong\n#elif 1 || 1/0\nyes\n#endif\n",
            "#if (-1 < 1U) || ((1 ? -1 : 0U) < 1)\n#error wrong\n#endif\n",
            "#define X 4\\\n2\nX\n/* a\nb */ X\n",
        ];
        for code in cases {
            let mut pp = Preprocessor::new(vec![], vec![]);
            let own = pp.process(code, Path::new("test.c")).unwrap();
            let mut child = Command::new("clang")
                .args(["-E", "-P", "-x", "c", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(code.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                tokens(&own),
                tokens(&String::from_utf8(output.stdout).unwrap()),
                "{code}"
            );
        }
    }

    #[test]
    fn conditional_arithmetic_obeys_signedness_and_precedence() {
        for (code, expected) in [
            ("1+2*3==7", true),
            ("-1 < 1U", false),
            ("~0ULL > 0", true),
            ("0 ? 1/0 : 7", true),
            ("1 ? 7 : 1/0", true),
            ("(1 ? -1 : 0U) < 1", false),
            ("'a' == 97", true),
            ("0x10 + 010 == 24", true),
            ("0 && 1/0", false),
        ] {
            let value = Expression::evaluate(&lex(code).unwrap()).unwrap();
            assert_eq!(value.bits != 0, expected, "{code}");
        }
        assert!(Expression::evaluate(&lex("1/0").unwrap()).is_err());
        assert!(Expression::evaluate(&lex("(1").unwrap()).is_err());
    }

    #[test]
    fn builtins_keep_physical_and_explicit_line_numbers() {
        let mut pp = Preprocessor::new(vec![], vec![]);
        let output = pp
            .process(
                "__LINE__\n#line 100 \"other.c\"\n__FILE__ __LINE__ __COUNTER__ __COUNTER__\n",
                Path::new("test.c"),
            )
            .unwrap();
        assert_eq!(tokens(&output), ["1", "\"other.c\"", "100", "0", "1"]);
    }

    #[test]
    fn builtins_can_be_redefined_and_undefined() {
        let mut pp = Preprocessor::new(vec![], vec![]);
        let output = pp
            .process(
                "ordinary __LINE__ __COUNTER__\n\
                 #define __LINE__ 73\n\
                 #define __COUNTER__ replacement\n\
                 __LINE__ __COUNTER__\n\
                 #undef __LINE__\n\
                 #undef __COUNTER__\n\
                 __LINE__ __COUNTER__\n\
                 #define __LINE__ 91\n\
                 #define __COUNTER__ 5\n\
                 #define __has_include(x) 42\n\
                 __LINE__ __COUNTER__ __has_include(\"missing.h\")\n\
                 #undef __has_include\n\
                 __has_include\n",
                Path::new("test.c"),
            )
            .unwrap();
        assert_eq!(
            tokens(&output),
            [
                "ordinary",
                "1",
                "0",
                "73",
                "replacement",
                "__LINE__",
                "__COUNTER__",
                "91",
                "5",
                "42",
                "__has_include"
            ]
        );
    }

    #[test]
    fn source_runs_preserve_literals_comments_and_splice_boundaries() {
        for (code, expected) in [
            (
                "alpha beta / gamma \"/* λ */\" 'x' // ignored\n__LINE__\n",
                vec!["alpha", "beta", "/", "gamma", "\"/* λ */\"", "'x'", "2"],
            ),
            ("left\\\n\\\r\nright __LINE__\n", vec!["leftright", "3"]),
            (
                "a /\\\n* ignored *\\\r\n/ b __LINE__\n",
                vec!["a", "b", "3"],
            ),
            (
                "\"a\\\"/*b*/\" '\\'' __LINE__\n",
                vec!["\"a\\\"/*b*/\"", "'\\''", "1"],
            ),
        ] {
            let mut pp = Preprocessor::new(vec![], vec![]);
            let output = pp.process(code, Path::new("test.c")).unwrap();
            assert_eq!(tokens(&output), expected, "{code}");
        }
    }

    #[test]
    fn dependency_scan_keeps_stateful_macro_effects_and_guard_redefinitions() {
        let directory = super::super::IncludeDirectory::new().unwrap();
        fs::write(directory.0.join("selected.h"), "#pragma once\n").unwrap();
        fs::write(
            directory.0.join("guarded.h"),
            "#ifndef G\n#define G\n#endif\n#include \"selected.h\"\n",
        )
        .unwrap();
        let main = directory.0.join("main.c");
        let code = "#define FORWARD LATER\n#define LATER __COUNTER__\nFORWARD\n#define CAT(a,b) a##b\nCAT(__COUNT,ER__)\n#if __COUNTER__ != 2\n#error lost counter effects\n#endif\n#include \"guarded.h\"\n#include \"guarded.h\"\n";
        let mut pp = Preprocessor::new(vec![directory.0.clone()], vec![]);
        pp.dependency_only = true;
        pp.process(code, &main).unwrap();
        assert!(pp.dependencies.contains(&directory.0.join("selected.h")));
        assert_eq!(pp.counter, 3);
        assert_eq!(pp.dependencies.len(), 3);
        let header = pp.headers.get(&directory.0.join("guarded.h")).unwrap();
        assert!(
            header.guard.is_none(),
            "trailing includes must prevent guard-based skipping"
        );
    }

    #[test]
    fn source_splices_preserve_builtin_lines() {
        for code in [
            "\\\n__LINE__\n",
            "__LINE__ \\\n__LINE__\n",
            "/*a\\\nb*/ __LINE__\n",
        ] {
            let mut pp = Preprocessor::new(vec![], vec![]);
            let output = pp.process(code, Path::new("test.c")).unwrap();
            let expected = if code.starts_with("__LINE__") {
                vec!["1", "2"]
            } else {
                vec!["2"]
            };
            assert_eq!(tokens(&output), expected, "{code}");
        }
    }

    #[test]
    fn header_queries_preserve_literal_header_names() {
        let directory = super::super::IncludeDirectory::new().unwrap();
        fs::create_dir(directory.0.join("linux")).unwrap();
        fs::write(directory.0.join("linux/value.h"), "").unwrap();
        let mut pp = Preprocessor::new(vec![directory.0.clone()], vec![]);
        pp.define("linux 1").unwrap();
        pp.process("#if !defined(__has_include) || !__has_include(<linux/value.h>)\n#error literal header expanded\n#endif\n#define HEADER \"linux/value.h\"\n#if !__has_include(HEADER)\n#error header macro not expanded\n#endif\n",&directory.0.join("main.c")).unwrap();
    }
}

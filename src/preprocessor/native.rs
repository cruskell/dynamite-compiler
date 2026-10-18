//! C preprocessing without a compiler SDK or a subprocess.

mod expansion;
mod expression;
mod lexer;
mod options;
mod profile;

use expression::Expression;
use lexer::{lex, needs_space, spelling};
pub(super) use options::{preprocess, preprocess_many};
use smol_str::SmolStr;
use std::{
    collections::{HashMap, HashSet},
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
    // Search configuration and diagnostics.
    includes: Vec<PathBuf>,
    quotes: Vec<PathBuf>,
    frameworks: Vec<PathBuf>,
    warnings_as_errors: bool,
    dependency_only: bool,

    // Macro definitions and expansion dependencies.
    macros: HashMap<SmolStr, Rc<Macro>>,
    builtin_names: HashSet<SmolStr>,
    stateful: HashSet<SmolStr>,
    // Keep historical edges after redefinition: extra expansion is safe; missing effects is not.
    dependents: HashMap<SmolStr, HashSet<SmolStr>>,

    // Header lookup and source traversal.
    include_context: PathBuf,
    search_origins: HashMap<PathBuf, usize>,
    include_cache: HashMap<(PathBuf, SmolStr, bool, bool), Option<PathBuf>>,
    headers: HashMap<PathBuf, Header>,
    once: HashSet<PathBuf>,
    depth: usize,
    base_file: String,

    // Expansion state.
    counter: usize,
    condition_expansion: bool,

    // Generated source and dependency output.
    output: String,
    pub(super) dependencies: Vec<PathBuf>,
    seen_dependencies: HashSet<PathBuf>,
}

impl Preprocessor {
    pub(super) fn new(includes: Vec<PathBuf>, quotes: Vec<PathBuf>) -> Self {
        Self {
            includes,
            quotes,
            frameworks: vec![],
            warnings_as_errors: false,
            dependency_only: false,
            macros: HashMap::new(),
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
            stateful: [SmolStr::new("__COUNTER__"), SmolStr::new("_Pragma")]
                .into_iter()
                .collect(),
            dependents: HashMap::new(),
            include_context: PathBuf::new(),
            search_origins: HashMap::new(),
            include_cache: HashMap::new(),
            headers: HashMap::new(),
            once: HashSet::new(),
            depth: 0,
            base_file: String::new(),
            counter: 0,
            condition_expansion: false,
            output: String::new(),
            dependencies: vec![],
            seen_dependencies: HashSet::new(),
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
        let previous = std::mem::replace(&mut self.condition_expansion, true);
        let expanded = self.expand(replaced, file, 0);
        self.condition_expansion = previous;
        let expanded = expanded?;
        Expression::evaluate(&expanded)
            .map(|v| v.bits != 0)
            .map_err(|error| format!("{file}:{}: {error}", tokens.first().map_or(1, |t| t.line)))
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
        let result = result.or_else(|| {
            if next {
                return None;
            }
            let (framework, header) = name.split_once('/')?;
            self.frameworks
                .iter()
                .map(|directory| directory.join(format!("{framework}.framework/Headers/{header}")))
                .find(|path| path.is_file())
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
    fn condition_macros_can_produce_defined_without_expanding_its_operand() {
        let mut pp = Preprocessor::new(vec![], vec![]);
        let output = pp.process("#define ENABLED 7\n#define CHECK() defined(ENABLED) && !defined(MISSING)\n#if CHECK()\nselected\n#else\n#error wrong conditional\n#endif\ndefined(ENABLED)\n", Path::new("test.c")).unwrap();
        assert_eq!(tokens(&output), ["selected", "defined", "(", "7", ")"]);
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

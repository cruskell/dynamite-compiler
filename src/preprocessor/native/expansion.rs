//! Define and expand macros while preserving hide sets and stateful effects.

use super::lexer::identifier;
use super::{Hidden, Macro, Preprocessor, Query, Token, lex, spelling};
use std::{collections::VecDeque, rc::Rc};

impl Preprocessor {
    pub(super) fn define_tokens(&mut self, tokens: &[Token]) -> Result<(), String> {
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

    pub(super) fn expand(
        &mut self,
        tokens: Vec<Token>,
        file: &str,
        depth: usize,
    ) -> Result<Vec<Token>, String> {
        self.expand_inner(tokens, file, depth, false)
    }

    pub(super) fn expand_inner(
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
            // Darwin headers use the GNU extension that produces defined via expansion.
            if self.condition_expansion && token.is("defined") {
                let parens = pending.front().is_some_and(|t| t.is("("));
                if parens {
                    pending.pop_front();
                }
                let name = pending
                    .pop_front()
                    .ok_or("defined requires an identifier")?;
                if !name
                    .text
                    .as_bytes()
                    .first()
                    .is_some_and(|b| identifier(*b) && !b.is_ascii_digit())
                {
                    return Err("defined requires an identifier".into());
                }
                if parens && !pending.pop_front().is_some_and(|t| t.is(")")) {
                    return Err("missing ')' after defined".into());
                }
                out.push(Token::new(
                    if self.is_defined(&name.text) {
                        "1"
                    } else {
                        "0"
                    },
                    token.space,
                    token.line,
                ));
                continue;
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
}

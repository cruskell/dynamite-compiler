use crate::ctype::{StructDefinition, StructMember, StructType, Type};
use crate::error;
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use crate::token::{Token, TokenType};
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

type Parameters = Vec<(Option<Token>, Type)>;

enum Declarator {
    Name(Option<Token>),
    Pointer(Box<Declarator>),
    Array(Box<Declarator>, usize),
    Function(Box<Declarator>, Parameters, bool),
}

impl Declarator {
    fn resolve(self, base: Type) -> (Option<Token>, Type, Parameters) {
        match self {
            Self::Name(token) => (token, base, vec![]),
            Self::Pointer(inner) => inner.resolve(Type::Ptr(Box::new(base))),
            Self::Array(inner, size) => inner.resolve(Type::Arr(Box::new(base), size)),
            Self::Function(inner, parameters, variadic) => {
                let types = parameters.iter().map(|(_, ty)| ty.clone()).collect();
                let ty = if variadic {
                    Type::VariadicFunc(types, Box::new(base))
                } else {
                    Type::Func(types, Box::new(base))
                };
                let function_type = ty.clone();
                let (name, ty, nested) = inner.resolve(ty);
                let parameters = if ty == function_type {
                    parameters
                } else {
                    nested
                };
                (name, ty, parameters)
            }
        }
    }
}

pub struct AstBuilderImpl<'a> {
    error_logger: &'a dyn error::ErrorLogger,
    tokens: &'a Vec<Token>,
    cur: usize,
    offset_size: usize,
    loop_stack: Vec<usize>,
    labels: HashMap<String, usize>,
    defined_labels: HashSet<String>,
    switch_stack: Vec<Vec<Node>>,
    switch_types: Vec<Type>,
    return_type: Option<Type>,
    struct_definitions: Vec<Rc<RefCell<StructDefinition>>>,
    scope_stack: Vec<HashMap<String, Identifier>>,
    pub functions: HashMap<String, Func>,
    pub global_variables: HashMap<String, GlobalVariable>,
    pub string_literals: Vec<String>,
}

pub struct ProgramAst<'a> {
    pub functions: &'a HashMap<String, Func>,
    pub global_variables: &'a HashMap<String, GlobalVariable>,
    pub string_literals: &'a Vec<String>,
}

pub enum Identifier {
    TypeDef(Type),
    Constant(i64),
    StaticObject(Type, String),
    Local(Type, usize),
    Static(Type),
}

pub trait AstBuilder {
    fn build(&mut self, is_debug: bool) -> ProgramAst;
}

impl AstBuilder for AstBuilderImpl<'_> {
    fn build(&mut self, is_debug: bool) -> ProgramAst {
        while !self.at_eof() {
            self.global_definition()
        }
        for function in self.functions.values() {
            if let Some(body) = &function.body {
                self.validate_void_expressions(body);
            }
        }
        for function in self.functions.values_mut() {
            if let Some(body) = &mut function.body {
                Self::insert_integer_conversions(body);
            }
        }
        if is_debug {
            self.print_functions();
        }
        ProgramAst {
            functions: &self.functions,
            global_variables: &self.global_variables,
            string_literals: &self.string_literals,
        }
    }
}

pub fn reserved_functions() -> HashMap<String, Identifier> {
    [
        (
            String::from("printf"),
            Identifier::Static(Type::VariadicFunc(
                vec![Type::Ptr(Box::new(Type::I8))],
                Box::new(Type::I32),
            )),
        ),
        (
            String::from("puts"),
            Identifier::Static(Type::Func(
                vec![Type::Ptr(Box::new(Type::I8))],
                Box::new(Type::I32),
            )),
        ),
        (
            String::from("putchar"),
            Identifier::Static(Type::Func(vec![Type::I32], Box::new(Type::I32))),
        ),
        (
            String::from("exit"),
            Identifier::Static(Type::Func(vec![Type::I32], Box::new(Type::I32))),
        ),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>()
}

impl<'a> AstBuilderImpl<'a> {
    pub fn new(error_logger: &'a dyn error::ErrorLogger, tokens: &'a Vec<Token>) -> Self {
        Self {
            error_logger,
            tokens,
            cur: 0,
            offset_size: 0,
            loop_stack: vec![],
            labels: HashMap::new(),
            defined_labels: HashSet::new(),
            switch_stack: vec![],
            switch_types: vec![],
            return_type: None,
            struct_definitions: vec![],
            scope_stack: vec![reserved_functions()],
            functions: HashMap::new(),
            global_variables: HashMap::new(),
            string_literals: Vec::new(),
        }
    }
    fn attempt_reserved(&mut self, s_value: &str) -> Option<Token> {
        if let TokenType::Reserved = self.tokens[self.cur].tt {
            if self.tokens[self.cur].s_value == s_value {
                self.cur += 1;
                return Some(self.tokens[self.cur - 1].clone());
            }
        }
        None
    }
    fn attempt_ident(&mut self) -> Option<Token> {
        if let TokenType::Ident = self.tokens[self.cur].tt {
            self.cur += 1;
            Some(self.tokens[self.cur - 1].clone())
        } else {
            None
        }
    }
    fn expect_reserved(&mut self, s_value: &str) -> Token {
        if let TokenType::Reserved = self.tokens[self.cur].tt {
            if self.tokens[self.cur].s_value == s_value {
                self.cur += 1;
                return self.tokens[self.cur - 1].clone();
            }
        }
        self.error_logger.print_error_position(
            self.tokens[self.cur].pos,
            &format!("`{}` expected", s_value),
        );
        unreachable!()
    }
    fn attempt(&mut self, tt: TokenType) -> Option<Token> {
        if tt == self.tokens[self.cur].tt {
            self.cur += 1;
            Some(self.tokens[self.cur - 1].clone())
        } else {
            None
        }
    }
    fn at_eof(&self) -> bool {
        self.cur >= self.tokens.len() || matches!(self.tokens[self.cur].tt, TokenType::EOF)
    }
    fn constant_array_size(&mut self) -> Token {
        let token = self.tokens[self.cur].clone();
        let expression = self.ternary();
        let value = self.eval(&expression);
        if value <= 0 {
            self.error_logger
                .print_error_position(token.pos, "array size must be positive");
        }
        Token {
            i_value: value as usize,
            ..token
        }
    }

    fn validate_type_layout(&self, ty: &Type, token: &Token) {
        match ty {
            Type::Ptr(child) => self.validate_type_layout(child, token),
            Type::Arr(child, _) => {
                if matches!(
                    **child,
                    Type::Void | Type::Func(..) | Type::VariadicFunc(..)
                ) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid array element type");
                }
                self.validate_type_layout(child, token);
            }
            Type::Func(args, result) | Type::VariadicFunc(args, result) => {
                if matches!(
                    **result,
                    Type::Arr(..) | Type::Func(..) | Type::VariadicFunc(..)
                ) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid function result type");
                }
                for arg in args {
                    self.validate_type_layout(arg, token);
                }
                self.validate_type_layout(result, token);
            }
            _ => {}
        }
        if ty.checked_size_of().is_none() {
            self.error_logger
                .print_error_position(token.pos, "array size is too large");
        }
    }

    fn declarator(&mut self) -> Declarator {
        let mut pointers = 0;
        while self.attempt_reserved("*").is_some() {
            pointers += 1;
        }
        let mut direct = if self.attempt_reserved("(").is_some() {
            let inner = self.declarator();
            self.expect_reserved(")");
            inner
        } else {
            Declarator::Name(self.attempt_ident())
        };
        loop {
            if self.attempt_reserved("[").is_some() {
                let size = if self.attempt_reserved("]").is_some() {
                    if matches!(direct, Declarator::Array(..)) {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur - 1].pos,
                            "only the outer array dimension may be omitted",
                        );
                    }
                    0
                } else {
                    let size = self.constant_array_size();
                    self.expect_reserved("]");
                    size.i_value
                };
                direct = Declarator::Array(Box::new(direct), size);
            } else if self.attempt_reserved("(").is_some() {
                let mut parameters = vec![];
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
                        let base = self.expect_type();
                        if base == Type::Void
                            && parameters.is_empty()
                            && self.attempt_reserved(")").is_some()
                        {
                            break;
                        }
                        let (name, ty, _) = self.declarator().resolve(base);
                        self.validate_type_layout(&ty, &self.tokens[self.cur]);
                        let ty = match ty {
                            Type::Arr(child, _) => Type::Ptr(child),
                            Type::Func(..) | Type::VariadicFunc(..) => Type::Ptr(Box::new(ty)),
                            _ => ty,
                        };
                        parameters.push((name, ty));
                        if self.attempt_reserved(")").is_some() {
                            break;
                        }
                        self.expect_reserved(",");
                    }
                }
                direct = Declarator::Function(Box::new(direct), parameters, variadic);
            } else {
                break;
            }
        }
        for _ in 0..pointers {
            direct = Declarator::Pointer(Box::new(direct));
        }
        direct
    }

    fn expect_ident_with_type(&mut self, ty: Type) -> (Token, Type) {
        let (name, ty, _) = self.declarator().resolve(ty);
        let token = name.unwrap_or_else(|| {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "ident expected");
            unreachable!()
        });
        self.validate_type_layout(&ty, &token);
        (token, ty)
    }
    fn new_local_variable(&mut self, ty: Type) -> Node {
        let (t, ty) = self.expect_ident_with_type(ty);
        let ty = if matches!(ty, Type::Arr(_, 0)) {
            let saved = self.cur;
            self.expect_reserved("=");
            let count = self.initializer_count();
            self.cur = saved;
            self.infer_array_size(ty, count, &t)
        } else {
            ty
        };
        self.register_local_variable(t, ty)
    }

    fn infer_array_size(&self, ty: Type, count: usize, token: &Token) -> Type {
        if let Type::Arr(child, 0) = ty {
            if count == 0 {
                self.error_logger.print_error_position(
                    token.pos,
                    "incomplete array requires a nonempty initializer",
                );
            }
            if child.size_of().checked_mul(count).is_none() {
                self.error_logger
                    .print_error_position(token.pos, "array size is too large");
            }
            Type::Arr(child, count)
        } else {
            ty
        }
    }

    fn initializer_count(&mut self) -> usize {
        if let Some(token) = self.attempt(TokenType::Str) {
            return crate::tokenizer::decode_string_literal(&token.s_value)
                .unwrap()
                .len()
                + 1;
        }
        self.expect_reserved("{");
        let mut depth = 0;
        let mut count = 0;
        let mut maximum = 0;
        let mut element = false;
        loop {
            if depth == 0 && !element && self.tokens[self.cur].s_value == "[" {
                self.cur += 1;
                let expression = self.ternary();
                let index = self.eval(&expression);
                if index < 0 {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "negative array designator",
                    );
                }
                count = index as usize;
                self.expect_reserved("]");
                element = true;
                continue;
            }
            let token = &self.tokens[self.cur];
            if token.tt == TokenType::EOF {
                self.error_logger
                    .print_error_position(token.pos, "unterminated initializer");
            }
            match token.s_value.as_str() {
                "}" if depth == 0 => {
                    return maximum.max(count + usize::from(element));
                }
                "," if depth == 0 => {
                    count += usize::from(element);
                    maximum = maximum.max(count);
                    element = false;
                }
                "{" | "(" | "[" => {
                    depth += 1;
                    element = true;
                }
                "}" | ")" | "]" => {
                    depth -= 1;
                }
                _ => {
                    element = true;
                }
            }
            self.cur += 1;
        }
    }

    fn register_local_variable(&mut self, t: Token, ty: Type) -> Node {
        self.expect_object_type(&ty, &t);
        self.offset_size += ty.size_of();
        self.offset_size = self.offset_size.div_ceil(ty.align_of()) * ty.align_of();
        if self.scope_stack.last().unwrap().contains_key(&t.s_value) {
            self.error_logger
                .print_error_position(t.pos, "invalid redeclaration");
        }
        self.scope_stack.last_mut().unwrap().insert(
            t.s_value.clone(),
            Identifier::Local(ty.clone(), self.offset_size),
        );
        Node {
            token: Some(t),
            nt: NodeType::LocalVar,
            cty: Some(ty),
            offset: Some(self.offset_size),
            ..Node::default()
        }
    }
    fn attempt_type(&mut self) -> Option<Type> {
        if let Some(token) = self.attempt_reserved("enum") {
            return Some(self.enum_type(token));
        }
        if let Some(token) = self
            .attempt_reserved("struct")
            .or_else(|| self.attempt_reserved("union"))
        {
            return Some(self.struct_type(token));
        }
        if self.attempt_reserved("void").is_some() {
            return Some(Type::Void);
        }
        if self.attempt_reserved("_Bool").is_some() || self.attempt_reserved("bool").is_some() {
            return Some(Type::Bool);
        }
        let mut bits = 32;
        let mut signed = true;
        let mut found = false;
        let mut width = false;
        let mut sign = false;
        let mut longs = 0;
        loop {
            let word = self.tokens[self.cur].s_value.as_str();
            match word {
                "unsigned" | "signed" => {
                    if sign {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur].pos,
                            "duplicate signedness",
                        );
                    }
                    signed = word == "signed";
                    sign = true;
                }
                "char" | "short" | "long" | "int" => {
                    if word == "long" {
                        longs += 1;
                        if longs > 2 {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                "invalid integer type",
                            );
                        }
                        bits = 64;
                    } else if word != "int" {
                        if width {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                "conflicting integer widths",
                            );
                        }
                        bits = if word == "char" { 8 } else { 16 };
                    }
                    width = width || word != "int";
                }
                _ => break,
            }
            found = true;
            self.cur += 1;
        }
        if found {
            return Some(Type::integer(bits, signed));
        }
        if let TokenType::Ident = self.tokens[self.cur].tt {
            let name = self.tokens[self.cur].s_value.clone();
            if let Some(Identifier::TypeDef(ty)) = self.resolve_name(&name) {
                let ty = ty.clone();
                self.cur += 1;
                return Some(ty);
            }
        }
        None
    }
    fn enum_type(&mut self, token: Token) -> Type {
        let name = self.attempt_ident();
        let key = name.as_ref().map(|name| format!(".enum.{}", name.s_value));
        if self.attempt_reserved("{").is_some() {
            if let Some(key) = &key {
                if self.scope_stack.last().unwrap().contains_key(key) {
                    self.error_logger
                        .print_error_position(token.pos, "enum tag is already defined");
                }
                self.scope_stack
                    .last_mut()
                    .unwrap()
                    .insert(key.clone(), Identifier::TypeDef(Type::I32));
            }
            let mut value = 0i64;
            loop {
                let name = self.attempt_ident().unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(self.tokens[self.cur].pos, "enumerator expected");
                    unreachable!()
                });
                if self.attempt_reserved("=").is_some() {
                    let expression = self.ternary();
                    value = self.eval(&expression);
                }
                if i32::try_from(value).is_err() {
                    self.error_logger
                        .print_error_position(name.pos, "enumerator is outside int range");
                }
                if self.scope_stack.last().unwrap().contains_key(&name.s_value) {
                    self.error_logger
                        .print_error_position(name.pos, "duplicate enumerator");
                }
                self.scope_stack
                    .last_mut()
                    .unwrap()
                    .insert(name.s_value, Identifier::Constant(value));
                value += 1;
                if self.attempt_reserved("}").is_some() {
                    break;
                }
                self.expect_reserved(",");
                if self.attempt_reserved("}").is_some() {
                    break;
                }
            }
        } else if key
            .as_ref()
            .is_none_or(|key| self.resolve_name(key).is_none())
        {
            self.error_logger
                .print_error_position(token.pos, "enum requires a known tag or definition");
        }
        Type::I32
    }

    fn struct_type(&mut self, token: Token) -> Type {
        let is_union = token.s_value == "union";
        let name = self.attempt_ident();
        let key = format!(
            ".struct.{}",
            name.as_ref()
                .map(|name| name.s_value.clone())
                .unwrap_or_else(|| format!("anonymous.{}", token.pos))
        );
        let defining = self.tokens[self.cur].s_value == "{";
        let existing = if defining {
            self.scope_stack.last().unwrap().get(&key)
        } else {
            self.scope_stack
                .iter()
                .rev()
                .find_map(|scope| scope.get(&key))
        };
        let ty = if let Some(Identifier::TypeDef(Type::Struct(ty))) = existing {
            ty.clone()
        } else {
            if name.is_none() && !defining {
                self.error_logger
                    .print_error_position(token.pos, "struct requires a tag or definition");
            }
            let (ty, definition) = StructType::new(self.struct_definitions.len());
            definition.borrow_mut().is_union = is_union;
            self.struct_definitions.push(definition);
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(key, Identifier::TypeDef(Type::Struct(ty.clone())));
            ty
        };
        if ty.definition().borrow().is_union != is_union {
            self.error_logger
                .print_error_position(token.pos, "aggregate tag has a different kind");
        }
        if self.attempt_reserved("{").is_some() {
            if ty.definition().borrow().complete {
                self.error_logger
                    .print_error_position(token.pos, "struct tag is already defined");
            }
            let mut members: Vec<StructMember> = vec![];
            let mut size = 0usize;
            let mut align = 1;
            while self.attempt_reserved("}").is_none() {
                let base = self.expect_type();
                loop {
                    let (name, member_type) = self.expect_ident_with_type(base.clone());
                    self.expect_object_type(&member_type, &name);
                    if member_type.size_of() == 0 {
                        self.error_logger.print_error_position(
                            name.pos,
                            "member requires a complete object type",
                        );
                    }
                    if members.iter().any(|member| member.name == name.s_value) {
                        self.error_logger
                            .print_error_position(name.pos, "duplicate struct member");
                    }
                    size = size.div_ceil(member_type.align_of()) * member_type.align_of();
                    align = align.max(member_type.align_of());
                    members.push(StructMember {
                        name: name.s_value,
                        ty: member_type.clone(),
                        offset: if is_union { 0 } else { size },
                    });
                    size = if is_union {
                        size.max(member_type.size_of())
                    } else {
                        size + member_type.size_of()
                    };
                    if self.attempt_reserved(",").is_none() {
                        break;
                    }
                }
                self.expect_reserved(";");
            }
            if members.is_empty() {
                self.error_logger
                    .print_error_position(token.pos, "struct requires at least one member");
            }
            *ty.definition().borrow_mut() = StructDefinition {
                members,
                is_union,
                size: size.div_ceil(align) * align,
                align,
                complete: true,
            };
        }
        Type::Struct(ty)
    }

    fn expect_object_type(&self, ty: &Type, token: &Token) {
        if ty.size_of() == 0
            || matches!(ty, Type::Void)
            || matches!(ty, Type::Arr(child, _) if **child == Type::Void)
        {
            self.error_logger
                .print_error_position(token.pos, "object requires a complete non-void type");
        }
    }
    fn expect_type(&mut self) -> Type {
        if let Some(ty) = self.attempt_type() {
            ty
        } else {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "type expected");
            unreachable!()
        }
    }
    fn typedef_definition(&mut self) {
        let base = self.expect_type();
        loop {
            let (token, ty) = self.expect_ident_with_type(base.clone());
            if self
                .scope_stack
                .last()
                .unwrap()
                .contains_key(&token.s_value)
            {
                self.error_logger
                    .print_error_position(token.pos, "invalid redeclaration");
            }
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value, Identifier::TypeDef(ty));
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
    }

    fn global_definition(&mut self) {
        let storage = self
            .attempt_reserved("static")
            .or_else(|| self.attempt_reserved("extern"));
        let internal = storage.as_ref().is_some_and(|s| s.s_value == "static");
        let external = storage.as_ref().is_some_and(|s| s.s_value == "extern");
        if self.attempt_reserved("typedef").is_some() {
            self.typedef_definition();
            self.expect_reserved(";");
            return;
        }
        let base = self.expect_type();
        if self.attempt_reserved(";").is_some() {
            return;
        }
        loop {
            let (name, ty, parameters) = self.declarator().resolve(base.clone());
            let token = name.unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "ident expected");
                unreachable!()
            });
            self.validate_type_layout(&ty, &token);
            if ty.function_signature().is_some() {
                if self.global_variables.contains_key(&token.s_value) {
                    self.error_logger
                        .print_error_position(token.pos, "function conflicts with object");
                }
                self.offset_size = 0;
                self.scope_stack.push(HashMap::new());
                let mut args = vec![];
                for (i, (name, ty)) in parameters.into_iter().enumerate() {
                    let name = name.unwrap_or_else(|| Token {
                        s_value: format!(".parameter.{i}"),
                        ..token.clone()
                    });
                    args.push(self.register_local_variable(name, ty));
                }
                self.scope_stack
                    .first_mut()
                    .unwrap()
                    .insert(token.s_value.clone(), Identifier::Static(ty.clone()));
                if let Some(existing) = self.functions.get(&token.s_value) {
                    if existing.cty != ty {
                        self.error_logger
                            .print_error_position(token.pos, "incompatible function redeclaration");
                    }
                }
                self.functions
                    .entry(token.s_value.clone())
                    .or_insert_with(|| Func {
                        cty: ty.clone(),
                        token: Some(token.clone()),
                        ..Func::default()
                    });
                let internal = internal
                    || self
                        .functions
                        .get(&token.s_value)
                        .is_some_and(|f| f.internal);
                let (parameters, result, _) = ty.function_signature().unwrap();
                if matches!(result, Type::Struct(_))
                    || parameters.iter().any(|ty| matches!(ty, Type::Struct(_)))
                {
                    self.error_logger.print_error_position(
                        token.pos,
                        "struct function arguments and results must use pointers",
                    );
                }
                self.return_type = Some(result);
                self.labels.clear();
                self.defined_labels.clear();
                let body = self.consume_block();
                for (name, position) in &self.labels {
                    if !self.defined_labels.contains(name) {
                        self.error_logger
                            .print_error_position(*position, "goto target is not defined");
                    }
                }
                self.return_type = None;
                self.scope_stack.pop();
                if body.is_none() {
                    self.expect_reserved(";");
                }
                if self.functions[&token.s_value].body.is_some() {
                    if body.is_some() {
                        self.error_logger
                            .print_error_position(token.pos, "function is already defined");
                    }
                    return;
                }
                self.functions.insert(
                    token.s_value.clone(),
                    Func {
                        body,
                        cty: ty,
                        token: Some(token),
                        args,
                        offset_size: self.offset_size,
                        internal,
                    },
                );
                return;
            }
            let data = if self.attempt_reserved("=").is_some() {
                Some(self.global_initializer(&ty))
            } else {
                None
            };
            let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                elements.len()
            } else {
                0
            };
            let ty = self.infer_array_size(ty, count, &token);
            self.expect_object_type(&ty, &token);
            if let Some(data) = &data {
                self.validate_global_initializer(&ty, data, &token);
            }
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value.clone(), Identifier::Static(ty.clone()));
            if let Some(existing) = self.global_variables.get_mut(&token.s_value) {
                if existing.internal != internal && !external {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting object linkage");
                }
                if !external || data.is_some() {
                    existing.declaration_only = false;
                }
                if existing.ty != ty {
                    self.error_logger
                        .print_error_position(token.pos, "incompatible object redeclaration");
                }
                if data.is_some() && existing.data.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "object is already initialized");
                }
                if data.is_some() {
                    existing.data = data;
                }
            } else {
                if self.functions.contains_key(&token.s_value) {
                    self.error_logger
                        .print_error_position(token.pos, "object conflicts with function");
                }
                self.global_variables.insert(
                    token.s_value,
                    GlobalVariable {
                        ty,
                        declaration_only: external && data.is_none(),
                        internal,
                        data,
                    },
                );
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        self.expect_reserved(";");
    }
    fn resolve_name(&mut self, s: &str) -> Option<&Identifier> {
        self.scope_stack.iter().rev().find_map(|map| map.get(s))
    }
    fn validate_global_initializer(&self, ty: &Type, data: &GlobalVariableData, token: &Token) {
        match (ty, data) {
            (_, GlobalVariableData::Zero) => {}
            (Type::Struct(ty), GlobalVariableData::Arr(elements)) => {
                let members = ty.members();
                let capacity = if ty.definition().borrow().is_union {
                    1
                } else {
                    members.len()
                };
                if elements.len() > capacity {
                    self.error_logger
                        .print_error_position(token.pos, "too many struct initializer elements");
                }
                for (member, element) in members.iter().zip(elements) {
                    self.validate_global_initializer(&member.ty, element, token);
                }
            }
            (Type::Struct(_), _) => {
                self.error_logger
                    .print_error_position(token.pos, "global struct initializer requires braces");
            }
            (Type::Arr(child, size), GlobalVariableData::Arr(elements)) => {
                if elements.len() > *size {
                    self.error_logger
                        .print_error_position(token.pos, "too many array initializer elements");
                }
                for element in elements {
                    self.validate_global_initializer(child, element, token);
                }
            }
            (Type::Arr(..), _) => self
                .error_logger
                .print_error_position(token.pos, "array initializer requires braces"),
            (_, GlobalVariableData::Arr(elements)) => {
                if elements.len() != 1 {
                    self.error_logger
                        .print_error_position(token.pos, "scalar initializer requires one element");
                }
                for element in elements {
                    self.validate_global_initializer(ty, element, token);
                }
            }
            (Type::I8 | Type::I32, GlobalVariableData::Address { .. }) => {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be an address");
            }
            (Type::I8 | Type::I32, GlobalVariableData::Elem(value)) if value.starts_with('@') => {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be a string");
            }
            _ => {}
        }
    }

    fn aggregate_type(ty: &Type, index: usize) -> Option<Type> {
        match ty {
            Type::Arr(child, size) if *size == 0 || index < *size => Some(*child.clone()),
            Type::Struct(st) => st.members().get(index).map(|member| member.ty.clone()),
            _ => None,
        }
    }
    fn designation(&mut self, ty: &Type) -> Vec<usize> {
        let mut current = ty.clone();
        let mut path = vec![];
        loop {
            let index = if let Some(token) = self.attempt_reserved("[") {
                let expression = self.ternary();
                let index = self.eval(&expression);
                self.expect_reserved("]");
                if index < 0 || !matches!(current, Type::Arr(..)) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid array designator");
                }
                index as usize
            } else if let Some(token) = self.attempt_reserved(".") {
                let name = self.attempt_ident().unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(token.pos, "member designator expected");
                    unreachable!()
                });
                let Type::Struct(st) = &current else {
                    self.error_logger
                        .print_error_position(token.pos, "member designator requires an aggregate");
                    unreachable!()
                };
                st.members()
                    .iter()
                    .position(|m| m.name == name.s_value)
                    .unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(name.pos, "unknown designated member");
                        unreachable!()
                    })
            } else {
                break;
            };
            current = Self::aggregate_type(&current, index).unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "designator is out of bounds");
                unreachable!()
            });
            path.push(index);
        }
        if !path.is_empty() {
            self.expect_reserved("=");
        }
        path
    }
    fn set_global_element(
        data: &mut GlobalVariableData,
        path: &[usize],
        value: GlobalVariableData,
    ) {
        if path.is_empty() {
            *data = value;
            return;
        }
        if !matches!(data, GlobalVariableData::Arr(_)) {
            *data = GlobalVariableData::Arr(vec![]);
        }
        let GlobalVariableData::Arr(elements) = data else {
            unreachable!()
        };
        while elements.len() <= path[0] {
            elements.push(GlobalVariableData::Zero);
        }
        Self::set_global_element(&mut elements[path[0]], &path[1..], value);
    }
    fn global_initializer(&mut self, ty: &Type) -> GlobalVariableData {
        if matches!(ty,Type::Arr(child,_) if **child==Type::I8)
            && self.tokens[self.cur].tt == TokenType::Str
        {
            let token = self.attempt(TokenType::Str).unwrap();
            let mut bytes = crate::tokenizer::decode_string_literal(&token.s_value).unwrap();
            let Type::Arr(_, size) = ty else {
                unreachable!()
            };
            if *size != 0 && bytes.len() > *size {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            if *size == 0 || bytes.len() < *size {
                bytes.push(0);
            }
            return GlobalVariableData::Arr(
                bytes
                    .into_iter()
                    .map(|b| GlobalVariableData::Elem(b.to_string()))
                    .collect(),
            );
        }
        if matches!(ty, Type::Arr(..) | Type::Struct(_)) && self.attempt_reserved("{").is_some() {
            let mut data = GlobalVariableData::Arr(vec![]);
            let mut cursor = 0;
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(ty);
                    if path.is_empty() {
                        path.push(cursor);
                    }
                    let mut child = ty.clone();
                    for index in &path {
                        child = Self::aggregate_type(&child, *index).unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                if matches!(ty, Type::Struct(_)) {
                                    "too many struct initializer elements"
                                } else {
                                    "too many array initializer elements"
                                },
                            );
                            unreachable!()
                        });
                    }
                    let value = self.global_initializer(&child);
                    cursor = path[0] + 1;
                    Self::set_global_element(&mut data, &path, value);
                    if self.attempt_reserved(",").is_none() {
                        self.expect_reserved("}");
                        break;
                    }
                    if self.attempt_reserved("}").is_some() {
                        break;
                    }
                }
            }
            return data;
        }
        self.global_data()
    }

    fn global_data(&mut self) -> GlobalVariableData {
        if self.attempt_reserved("{").is_some() {
            let mut vec = Vec::new();
            if self.attempt_reserved("}").is_none() {
                loop {
                    vec.push(self.global_data());
                    if self.attempt_reserved(",").is_none() {
                        self.expect_reserved("}");
                        break;
                    }
                    if self.attempt_reserved("}").is_some() {
                        break;
                    }
                }
            }
            GlobalVariableData::Arr(vec)
        } else {
            let expression = self.ternary();
            if let Some((symbol, offset)) = self.constant_address(&expression) {
                GlobalVariableData::Address { symbol, offset }
            } else {
                GlobalVariableData::Elem(format!("{}", self.eval(&expression)))
            }
        }
    }
    fn constant_address(&mut self, node: &Node) -> Option<(String, i64)> {
        match node.nt {
            NodeType::GlobalVar
                if node.resolve_type().is_some_and(|ty| {
                    matches!(ty, Type::Arr(..) | Type::Func(..) | Type::VariadicFunc(..))
                }) =>
            {
                Some((node.global_name.clone(), 0))
            }
            NodeType::Addr => {
                let child = node.lhs.as_ref().unwrap();
                if matches!(child.nt, NodeType::GlobalVar) {
                    Some((
                        if child.dest.is_empty() {
                            child.global_name.clone()
                        } else {
                            child.dest.trim_start_matches('@').to_string()
                        },
                        0,
                    ))
                } else if matches!(child.nt, NodeType::Deref) {
                    self.constant_address(child.lhs.as_ref().unwrap())
                } else {
                    None
                }
            }
            NodeType::Cast => self.constant_address(node.lhs.as_ref().unwrap()),
            NodeType::Add | NodeType::Sub => {
                let lhs = node.lhs.as_ref().unwrap();
                let (symbol, offset) = self.constant_address(lhs)?;
                let stride = lhs.dest_type()?.size_of() as i64;
                let delta = self.eval(node.rhs.as_ref().unwrap()).checked_mul(stride)?;
                Some((
                    symbol,
                    if node.nt == NodeType::Add {
                        offset.checked_add(delta)?
                    } else {
                        offset.checked_sub(delta)?
                    },
                ))
            }
            _ => None,
        }
    }

    fn eval(&mut self, node: &Node) -> i64 {
        match node.nt {
            NodeType::Num => return node.value.unwrap() as i64,
            NodeType::Cast => {
                let value = self.eval(node.lhs.as_ref().unwrap());
                return match node.cty.as_ref().unwrap() {
                    ty if ty.integer_info().is_some() => ty.convert_constant(value),
                    Type::Ptr(_) => value,
                    _ => {
                        self.error_logger.print_error_position(
                            node.token.as_ref().unwrap().pos,
                            "cast is not a constant initializer",
                        );
                        unreachable!()
                    }
                };
            }

            NodeType::LogicalAnd => {
                return (self.eval(node.lhs.as_ref().unwrap()) != 0
                    && self.eval(node.rhs.as_ref().unwrap()) != 0) as i64;
            }
            NodeType::LogicalOr => {
                return (self.eval(node.lhs.as_ref().unwrap()) != 0
                    || self.eval(node.rhs.as_ref().unwrap()) != 0) as i64;
            }
            NodeType::Conditional => {
                return if self.eval(node.cond.as_ref().unwrap()) != 0 {
                    self.eval(node.then.as_ref().unwrap())
                } else {
                    self.eval(node.els.as_ref().unwrap())
                };
            }
            NodeType::BitNot => return !self.eval(node.lhs.as_ref().unwrap()),
            NodeType::Eq
            | NodeType::Ne
            | NodeType::Le
            | NodeType::Lt
            | NodeType::Add
            | NodeType::Sub
            | NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitAnd
            | NodeType::BitOr
            | NodeType::BitXor
            | NodeType::BitLeft
            | NodeType::BitRight => {}
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "initializer element is not a compile-time constant",
                );
                unreachable!()
            }
        }
        let lhs_node = node.lhs.as_ref().unwrap();
        let rhs_node = node.rhs.as_ref().unwrap();
        let lhs_ty = lhs_node.resolve_type().unwrap();
        let rhs_ty = rhs_node.resolve_type().unwrap();
        let ty = Type::arithmetic_type(&lhs_ty, &rhs_ty);
        let lhs = ty.convert_constant(self.eval(lhs_node));
        let rhs = ty.convert_constant(self.eval(rhs_node));
        let unsigned = ty.integer_info().is_some_and(|(_, signed)| !signed);
        if unsigned {
            let (lhs, rhs) = (lhs as u64, rhs as u64);
            let value = match node.nt {
                NodeType::Div if rhs != 0 => Some(lhs / rhs),
                NodeType::Mod if rhs != 0 => Some(lhs % rhs),
                NodeType::Lt => Some(u64::from(lhs < rhs)),
                NodeType::Le => Some(u64::from(lhs <= rhs)),
                NodeType::Add => Some(lhs.wrapping_add(rhs)),
                NodeType::Sub => Some(lhs.wrapping_sub(rhs)),
                NodeType::Mul => Some(lhs.wrapping_mul(rhs)),
                NodeType::BitRight if rhs < ty.integer_info().unwrap().0 as u64 => Some(lhs >> rhs),
                _ => None,
            };
            if let Some(value) = value {
                return ty.convert_constant(value as i64);
            }
        }
        let result = match node.nt {
            NodeType::Eq => Some((lhs == rhs) as i64),
            NodeType::Ne => Some((lhs != rhs) as i64),
            NodeType::Le => Some((lhs <= rhs) as i64),
            NodeType::Lt => Some((lhs < rhs) as i64),
            NodeType::Add => lhs.checked_add(rhs),
            NodeType::Sub => lhs.checked_sub(rhs),
            NodeType::Mul => lhs.checked_mul(rhs),
            NodeType::Div => lhs.checked_div(rhs),
            NodeType::Mod => lhs.checked_rem(rhs),
            NodeType::BitAnd => Some(lhs & rhs),
            NodeType::BitOr => Some(lhs | rhs),
            NodeType::BitXor => Some(lhs ^ rhs),
            NodeType::BitLeft
                if (0..lhs_ty.promoted().integer_info().unwrap().0 as i64).contains(&rhs) =>
            {
                lhs.checked_shl(rhs as u32)
            }
            NodeType::BitRight
                if (0..lhs_ty.promoted().integer_info().unwrap().0 as i64).contains(&rhs) =>
            {
                lhs.checked_shr(rhs as u32)
            }
            _ => None,
        };
        let value = result.unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "invalid arithmetic in constant initializer",
            );
            unreachable!()
        });
        if matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        ) {
            value
        } else {
            ty.convert_constant(value)
        }
    }
    fn cast_integer(node: &mut Node, target: &Type) {
        if target.integer_info().is_some() && node.resolve_type().as_ref() != Some(target) {
            let operand = std::mem::take(node);
            *node = Node {
                nt: NodeType::Cast,
                cty: Some(target.clone()),
                token: operand.token.clone(),
                lhs: Some(Box::new(operand)),
                ..Node::default()
            };
        }
    }
    fn insert_integer_conversions(node: &mut Node) {
        for child in [
            &mut node.lhs,
            &mut node.rhs,
            &mut node.cond,
            &mut node.then,
            &mut node.els,
            &mut node.ini,
            &mut node.upd,
        ]
        .into_iter()
        .flatten()
        {
            Self::insert_integer_conversions(child);
        }
        for child in node.children.iter_mut().chain(&mut node.args) {
            Self::insert_integer_conversions(child);
        }
        match node.nt {
            NodeType::Assign => {
                let target = node.lhs.as_ref().unwrap().resolve_type().unwrap();
                Self::cast_integer(node.rhs.as_mut().unwrap(), &target);
            }
            NodeType::CallFunc => {
                for (index, arg) in node.args.iter_mut().rev().enumerate() {
                    let target = node
                        .parameter_types
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| arg.resolve_type().unwrap().promoted());
                    Self::cast_integer(arg, &target);
                }
            }
            _ => {}
        }
    }

    fn expect_conversion(&self, expected: &Type, operand: &Node) {
        let actual = operand.resolve_type();
        let compatible = match (expected, actual.as_ref()) {
            (target, Some(source))
                if target.integer_info().is_some() && source.integer_info().is_some() =>
            {
                true
            }
            (Type::Bool, Some(Type::Ptr(_) | Type::Arr(..))) => true,
            (Type::Ptr(target), Some(Type::Ptr(source) | Type::Arr(source, _))) => {
                target == source || **target == Type::Void || **source == Type::Void
            }
            (Type::Ptr(target), Some(source @ (Type::Func(..) | Type::VariadicFunc(..)))) => {
                **target == *source
            }
            (Type::Ptr(_), Some(Type::I32)) => {
                matches!(operand.nt, NodeType::Num) && operand.value == Some(0)
            }
            (_, Some(actual)) => expected == actual,
            _ => false,
        };
        if !compatible {
            self.error_logger.print_error_position(
                operand.token.as_ref().map_or(0, |t| t.pos),
                "incompatible value type",
            );
        }
    }

    fn validate_void_expressions(&self, node: &Node) {
        let no_value = |operand: &Node| {
            if operand.resolve_type() == Some(Type::Void) {
                self.error_logger.print_error_position(
                    operand.token.as_ref().unwrap().pos,
                    "void expression has no value",
                );
            }
        };
        if let Some(condition) = &node.cond {
            no_value(condition);
        }
        if matches!(
            node.nt,
            NodeType::Assign
                | NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Mod
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le
                | NodeType::BitLeft
                | NodeType::BitRight
                | NodeType::BitAnd
                | NodeType::BitOr
                | NodeType::BitXor
                | NodeType::BitNot
                | NodeType::LogicalAnd
                | NodeType::LogicalOr
        ) {
            for operand in [&node.lhs, &node.rhs].into_iter().flatten() {
                no_value(operand);
            }
        }
        if matches!(node.nt, NodeType::Assign) {
            self.expect_conversion(
                &node.lhs.as_ref().unwrap().resolve_type().unwrap(),
                node.rhs.as_ref().unwrap(),
            );
        }
        if matches!(node.nt, NodeType::CallFunc) {
            for (argument, parameter) in node.args.iter().rev().zip(&node.parameter_types) {
                self.expect_conversion(parameter, argument);
            }
            for argument in &node.args {
                no_value(argument);
            }
        }
        if matches!(node.nt, NodeType::Conditional) {
            let then_void = node.then.as_ref().unwrap().resolve_type() == Some(Type::Void);
            let else_void = node.els.as_ref().unwrap().resolve_type() == Some(Type::Void);
            if then_void != else_void {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "conditional branches have incompatible void types",
                );
            }
        }
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            self.validate_void_expressions(child);
        }
        for child in node.children.iter().chain(&node.args) {
            self.validate_void_expressions(child);
        }
    }

    fn stmt(&mut self) -> Node {
        if self.tokens[self.cur].tt == TokenType::Ident && self.tokens[self.cur + 1].s_value == ":"
        {
            let token = self.attempt_ident().unwrap();
            self.expect_reserved(":");
            if !self.defined_labels.insert(token.s_value.clone()) {
                self.error_logger
                    .print_error_position(token.pos, "duplicate label");
            }
            let id = *self
                .labels
                .entry(token.s_value.clone())
                .or_insert(token.pos);
            return Node {
                nt: NodeType::Label,
                value: Some(id),
                token: Some(token),
                lhs: Some(Box::new(self.stmt())),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("goto") {
            let name = self.attempt_ident().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "goto requires a label");
                unreachable!()
            });
            let id = *self.labels.entry(name.s_value).or_insert(name.pos);
            self.expect_reserved(";");
            return Node {
                nt: NodeType::Goto,
                value: Some(id),
                token: Some(token),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("typedef") {
            self.typedef_definition();
            self.expect_reserved(";");
            return Node {
                nt: NodeType::Block,
                token: Some(token),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved(";") {
            return Node {
                nt: NodeType::Block,
                token: Some(token),
                ..Node::default()
            };
        }
        if let Some(t) = self.attempt_reserved("switch") {
            self.expect_reserved("(");
            let cond = self.expr();
            self.expect_reserved(")");
            if cond
                .resolve_type()
                .is_none_or(|ty| ty.integer_info().is_none())
            {
                self.error_logger
                    .print_error_position(t.pos, "switch requires an integer expression");
            }
            self.switch_stack.push(vec![]);
            self.switch_types
                .push(cond.resolve_type().unwrap().promoted());
            let body = self.stmt();
            let cases = self.switch_stack.pop().unwrap();
            self.switch_types.pop();
            return Node {
                token: Some(t),
                nt: NodeType::Switch,
                cond: Some(Box::new(cond)),
                then: Some(Box::new(body)),
                children: cases,
                ..Node::default()
            };
        }
        if let Some(t) = self
            .attempt_reserved("case")
            .or_else(|| self.attempt_reserved("default"))
        {
            if self.switch_stack.is_empty() {
                self.error_logger
                    .print_error_position(t.pos, "case or default requires an enclosing switch");
            }
            let value = if t.s_value == "case" {
                let expr = self.ternary();
                let value = self.eval(&expr);
                Some(self.switch_types.last().unwrap().convert_constant(value) as usize)
            } else {
                None
            };
            self.expect_reserved(":");
            if self
                .switch_stack
                .last()
                .unwrap()
                .iter()
                .any(|case| case.value == value)
            {
                self.error_logger
                    .print_error_position(t.pos, "duplicate switch label");
            }
            let mut node = Node {
                nt: NodeType::Case,
                token: Some(t),
                value,
                ..Node::default()
            };
            self.switch_stack.last_mut().unwrap().push(node.clone());
            node.lhs = Some(Box::new(self.stmt()));
            return node;
        }
        if let Some(t) = self.attempt_reserved("if") {
            self.expect_reserved("(");
            let cond = self.expr();
            self.expect_reserved(")");
            let then = self.stmt();
            let mut els: Option<Node> = None;
            if self.attempt_reserved("else").is_some() {
                els = Some(self.stmt());
            }
            return Node::new_if_node(Some(t), cond, then, els);
        }
        if let Some(t) = self.attempt_reserved("do") {
            self.loop_stack.push(t.pos);
            let body = self.stmt();
            self.loop_stack.pop();
            self.expect_reserved("while");
            self.expect_reserved("(");
            let cond = self.expr();
            self.expect_reserved(")");
            self.expect_reserved(";");
            let mut node = Node::new_while_node(Some(t), cond, body);
            node.nt = NodeType::DoWhile;
            return node;
        }
        if let Some(t) = self.attempt_reserved("while") {
            self.expect_reserved("(");
            let cond = self.expr();
            self.expect_reserved(")");
            self.loop_stack.push(t.pos);
            let body = self.stmt();
            self.loop_stack.pop();
            return Node::new_while_node(Some(t), cond, body);
        }
        if let Some(t) = self.attempt_reserved("for") {
            self.scope_stack.push(HashMap::new());
            self.expect_reserved("(");
            let mut ini: Option<Node> = None;
            let mut cond: Option<Node> = None;
            let mut upd: Option<Node> = None;
            if self.attempt_reserved(";").is_none() {
                ini = Some(if let Some(ty) = self.attempt_type() {
                    self.local_variable_definition(ty)
                } else {
                    self.expr()
                });
                self.expect_reserved(";");
            }
            if self.attempt_reserved(";").is_none() {
                cond = Some(self.expr());
                self.expect_reserved(";");
            }
            if self.attempt_reserved(")").is_none() {
                upd = Some(self.expr());
                self.expect_reserved(")");
            }
            self.loop_stack.push(t.pos);
            let body = self.stmt();
            self.loop_stack.pop();
            self.scope_stack.pop();
            return Node::new_for_node(Some(t), ini, cond, upd, body);
        }
        if let Some(node) = self.consume_block() {
            return node;
        }
        let storage = self
            .attempt_reserved("static")
            .or_else(|| self.attempt_reserved("extern"));
        if let Some(storage) = storage {
            let ty = self.expect_type();
            let declaration = self.local_static_definition(ty, &storage);
            self.expect_reserved(";");
            return declaration;
        }
        let node = if let Some(ty) = self.attempt_type() {
            if self.tokens[self.cur].s_value == ";" {
                Node {
                    nt: NodeType::Block,
                    ..Node::default()
                }
            } else {
                self.local_variable_definition(ty)
            }
        } else if let Some(t) = self.attempt_reserved("continue") {
            let target = self.loop_stack.last().copied().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(t.pos, "continue requires an enclosing loop");
                unreachable!()
            });
            Node {
                token: Some(t),
                nt: NodeType::Continue,
                value: Some(target),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_reserved("break") {
            Node {
                token: Some(t),
                nt: NodeType::Break,
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_reserved("return") {
            let mut operand = if self.tokens[self.cur].s_value == ";" {
                None
            } else {
                Some(self.expr())
            };
            let expected_void = self.return_type == Some(Type::Void);
            let actual_void = operand
                .as_ref()
                .is_none_or(|operand| operand.resolve_type() == Some(Type::Void));
            if expected_void != actual_void {
                self.error_logger.print_error_position(
                    t.pos,
                    "return value does not match function return type",
                );
            }
            if let Some(value) = &operand {
                if !expected_void {
                    self.expect_conversion(self.return_type.as_ref().unwrap(), value);
                }
            }
            if !expected_void {
                if let Some(value) = &mut operand {
                    Self::cast_integer(value, self.return_type.as_ref().unwrap());
                }
            }
            Node {
                token: Some(t),
                nt: NodeType::Return,
                lhs: operand.map(Box::new),
                ..Node::default()
            }
        } else {
            self.expr()
        };
        self.expect_reserved(";");
        node
    }

    fn local_static_definition(&mut self, base: Type, storage: &Token) -> Node {
        loop {
            let (token, mut ty) = self.expect_ident_with_type(base.clone());
            let external = storage.s_value == "extern";
            let name = if external {
                token.s_value.clone()
            } else {
                format!(".static.{}.{}", token.pos, token.s_value)
            };
            if ty.function_signature().is_some() {
                if !external {
                    self.error_logger.print_error_position(
                        token.pos,
                        "block static function declaration is invalid",
                    );
                }
                if let Some(existing) = self.functions.get(&name) {
                    if existing.cty != ty {
                        self.error_logger
                            .print_error_position(token.pos, "incompatible function redeclaration");
                    }
                } else {
                    self.functions.insert(
                        name.clone(),
                        Func {
                            cty: ty.clone(),
                            token: Some(token.clone()),
                            ..Func::default()
                        },
                    );
                }
                self.scope_stack
                    .last_mut()
                    .unwrap()
                    .insert(token.s_value, Identifier::StaticObject(ty, name));
                if self.attempt_reserved(",").is_none() {
                    break;
                }
                continue;
            }
            let data = if self.attempt_reserved("=").is_some() {
                if external {
                    self.error_logger
                        .print_error_position(token.pos, "block extern cannot have an initializer");
                }
                if matches!(&ty,Type::Arr(child,_) if **child==Type::I8)
                    && self.tokens[self.cur].tt == TokenType::Str
                {
                    let string = self.attempt(TokenType::Str).unwrap();
                    let mut bytes =
                        crate::tokenizer::decode_string_literal(&string.s_value).unwrap();
                    let Type::Arr(_, size) = &ty else {
                        unreachable!()
                    };
                    if *size != 0 && bytes.len() > *size {
                        self.error_logger
                            .print_error_position(string.pos, "string initializer is too long");
                    }
                    if *size == 0 || bytes.len() < *size {
                        bytes.push(0);
                    }
                    Some(GlobalVariableData::Arr(
                        bytes
                            .into_iter()
                            .map(|byte| GlobalVariableData::Elem(byte.to_string()))
                            .collect(),
                    ))
                } else {
                    Some(self.global_data())
                }
            } else {
                None
            };
            let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                elements.len()
            } else {
                0
            };
            ty = self.infer_array_size(ty, count, &token);
            self.expect_object_type(&ty, &token);
            if let Some(data) = &data {
                self.validate_global_initializer(&ty, data, &token);
            }
            if self
                .scope_stack
                .last()
                .unwrap()
                .contains_key(&token.s_value)
            {
                self.error_logger
                    .print_error_position(token.pos, "invalid redeclaration");
            }
            self.scope_stack.last_mut().unwrap().insert(
                token.s_value,
                Identifier::StaticObject(ty.clone(), name.clone()),
            );
            if let Some(existing) = self.global_variables.get(&name) {
                if existing.ty != ty {
                    self.error_logger
                        .print_error_position(token.pos, "incompatible object redeclaration");
                }
            } else {
                self.global_variables.insert(
                    name,
                    GlobalVariable {
                        ty,
                        data,
                        internal: !external,
                        declaration_only: external,
                    },
                );
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        Node {
            nt: NodeType::Block,
            ..Node::default()
        }
    }

    pub fn local_variable_definition(&mut self, ty: Type) -> Node {
        let mut vec = Vec::new();
        loop {
            let node = self.new_local_variable(ty.clone());
            if let Some(token) = self.attempt_reserved("=") {
                // if initializer element exists, push into AST
                vec.push(self.local_variable_initialization(&node, &token));
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        Node {
            nt: NodeType::DefVar,
            children: vec,
            ..Node::default()
        }
    }

    fn member_node(node: Node, member: &StructMember, token: &Token) -> Node {
        let address = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, node);
        let bytes = Node {
            nt: NodeType::Cast,
            token: Some(token.clone()),
            cty: Some(Type::Ptr(Box::new(Type::I8))),
            lhs: Some(Box::new(address)),
            ..Node::default()
        };
        let offset = Node::new_with_op(
            Some(token.clone()),
            NodeType::Add,
            bytes,
            Node::new_with_num(Some(token.clone()), member.offset),
        );
        let pointer = Node {
            nt: NodeType::Cast,
            token: Some(token.clone()),
            cty: Some(Type::Ptr(Box::new(member.ty.clone()))),
            lhs: Some(Box::new(offset)),
            ..Node::default()
        };
        Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, pointer)
    }

    fn aggregate_elements(node: &Node, token: &Token) -> Option<Vec<Node>> {
        match node.resolve_type()? {
            Type::Arr(_, size) => Some(
                (0..size)
                    .map(|index| Self::array_element(node, index, token))
                    .collect(),
            ),
            Type::Struct(ty) => Some(
                ty.members()
                    .iter()
                    .take(if ty.definition().borrow().is_union {
                        1
                    } else {
                        usize::MAX
                    })
                    .map(|member| Self::member_node(node.clone(), member, token))
                    .collect(),
            ),
            _ => None,
        }
    }

    fn array_element(node: &Node, index: usize, token: &Token) -> Node {
        Node::new_with_op_and_lhs(
            Some(token.clone()),
            NodeType::Deref,
            Node::new_with_op(
                Some(token.clone()),
                NodeType::Add,
                node.clone(),
                Node::new_with_num(Some(token.clone()), index),
            ),
        )
    }

    fn zero_initializer(node: &Node, token: &Token) -> Node {
        if let Some(elements) = Self::aggregate_elements(node, token) {
            Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: elements
                    .iter()
                    .map(|element| Self::zero_initializer(element, token))
                    .collect(),
                ..Node::default()
            }
        } else {
            Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                node.clone(),
                Node::new_with_num(Some(token.clone()), 0),
            )
        }
    }

    fn local_variable_initialization(&mut self, node: &Node, assign_token: &Token) -> Node {
        if matches!(node.resolve_type(), Some(Type::Arr(child, _)) if *child == Type::I8)
            && self.tokens[self.cur].tt == TokenType::Str
        {
            let token = self.attempt(TokenType::Str).unwrap();
            let bytes = crate::tokenizer::decode_string_literal(&token.s_value).unwrap();
            let Type::Arr(_, size) = node.resolve_type().unwrap() else {
                unreachable!()
            };
            if bytes.len() > size {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: (0..size)
                    .map(|index| {
                        Node::new_with_op(
                            Some(token.clone()),
                            NodeType::Assign,
                            Self::array_element(node, index, &token),
                            Node::new_with_num(
                                Some(token.clone()),
                                bytes.get(index).copied().unwrap_or(0) as usize,
                            ),
                        )
                    })
                    .collect(),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("{") {
            let elements = Self::aggregate_elements(node, &token);
            if elements.is_none() {
                let initializer = self.local_variable_initialization(node, assign_token);
                self.attempt_reserved(",");
                self.expect_reserved("}");
                return initializer;
            }
            let mut children = vec![Self::zero_initializer(node, assign_token)];
            let mut cursor = 0;
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(&node.resolve_type().unwrap());
                    if path.is_empty() {
                        path.push(cursor);
                    }
                    cursor = path[0] + 1;
                    let mut target = node.clone();
                    for index in path {
                        let targets = Self::aggregate_elements(&target, &token).unwrap();
                        target = targets.get(index).cloned().unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                token.pos,
                                "too many aggregate initializer elements",
                            );
                            unreachable!()
                        });
                    }
                    children.push(self.local_variable_initialization(&target, assign_token));
                    if self.attempt_reserved(",").is_none() {
                        self.expect_reserved("}");
                        break;
                    }
                    if self.attempt_reserved("}").is_some() {
                        break;
                    }
                }
            }
            Node {
                token: Some(assign_token.clone()),
                nt: NodeType::Block,
                children,
                ..Node::default()
            }
        } else {
            let rhs = self.assign();
            if matches!(node.resolve_type(), Some(Type::Struct(_))) {
                self.struct_assignment(node.clone(), rhs, assign_token.clone())
            } else {
                Node::new_with_op(
                    Some(assign_token.clone()),
                    NodeType::Assign,
                    node.clone(),
                    rhs,
                )
            }
        }
    }

    fn struct_assignment(&mut self, lhs: Node, rhs: Node, token: Token) -> Node {
        let ty = lhs.resolve_type().unwrap();
        if rhs.resolve_type() != Some(ty.clone()) {
            self.error_logger
                .print_error_position(token.pos, "incompatible struct assignment");
        }
        let mut children = vec![];
        let mut addresses = vec![];
        for operand in [lhs, rhs] {
            self.offset_size = (self.offset_size + 8).div_ceil(8) * 8;
            let temporary = Node {
                nt: NodeType::LocalVar,
                token: Some(token.clone()),
                cty: Some(Type::Ptr(Box::new(ty.clone()))),
                offset: Some(self.offset_size),
                ..Node::default()
            };
            children.push(Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                temporary.clone(),
                Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, operand),
            ));
            addresses.push(temporary);
        }
        for index in 0..ty.size_of() {
            let bytes = addresses
                .iter()
                .map(|address| {
                    let pointer = Node {
                        nt: NodeType::Cast,
                        token: Some(token.clone()),
                        cty: Some(Type::Ptr(Box::new(Type::I8))),
                        lhs: Some(Box::new(address.clone())),
                        ..Node::default()
                    };
                    Self::array_element(&pointer, index, &token)
                })
                .collect::<Vec<_>>();
            children.push(Node::new_with_op(
                Some(token.clone()),
                NodeType::Assign,
                bytes[0].clone(),
                bytes[1].clone(),
            ));
        }
        children.push(Node::new_with_op_and_lhs(
            Some(token.clone()),
            NodeType::Deref,
            addresses.remove(0),
        ));
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }

    pub fn consume_block(&mut self) -> Option<Node> {
        if let Some(t) = self.attempt_reserved("{") {
            self.scope_stack.push(HashMap::new());
            let mut children: Vec<Node> = Vec::new();
            while self.attempt_reserved("}").is_none() {
                children.push(self.stmt());
            }
            self.scope_stack.pop();
            Some(Node {
                token: Some(t),
                nt: NodeType::Block,
                children,
                ..Node::default()
            })
        } else {
            None
        }
    }
    pub fn expr(&mut self) -> Node {
        let first = self.assign();
        let Some(token) = self.attempt_reserved(",") else {
            return first;
        };
        let mut children = vec![first, self.assign()];
        while self.attempt_reserved(",").is_some() {
            children.push(self.assign());
        }
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }
    fn expect_assignable(&self, node: &Node) {
        if !(node.compound_literal
            || matches!(
                node.nt,
                NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref
            ))
            || matches!(
                node.resolve_type(),
                Some(Type::Arr(..) | Type::Func(..) | Type::VariadicFunc(..)) | None
            )
        {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "expression is not assignable",
            );
        }
    }

    fn compound_assignment(
        &mut self,
        token: Token,
        lhs: Node,
        rhs: Node,
        operation: NodeType,
    ) -> Node {
        self.expect_assignable(&lhs);
        // Save the lvalue address so side effects in it occur exactly once.
        let ty = Type::Ptr(Box::new(lhs.resolve_type().unwrap()));
        self.offset_size =
            (self.offset_size + ty.size_of()).div_ceil(ty.align_of()) * ty.align_of();
        let temporary = Node {
            nt: NodeType::LocalVar,
            token: Some(token.clone()),
            cty: Some(ty),
            offset: Some(self.offset_size),
            ..Node::default()
        };
        let address = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, lhs);
        let save_address = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            temporary.clone(),
            address,
        );
        let target = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, temporary);
        let value = Node::new_with_op(Some(token.clone()), operation, target.clone(), rhs);
        let update = Node::new_with_op(Some(token.clone()), NodeType::Assign, target, value);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![save_address, update],
            ..Node::default()
        }
    }

    fn assign(&mut self) -> Node {
        let node = self.ternary();
        if let Some(token) = self.attempt_reserved("=") {
            self.expect_assignable(&node);
            let rhs = self.assign();
            if matches!(node.resolve_type(), Some(Type::Struct(_))) {
                return self.struct_assignment(node, rhs, token);
            }
            return Node::new_with_op(Some(token), NodeType::Assign, node, rhs);
        }
        for (symbol, operation) in [
            ("+=", NodeType::Add),
            ("-=", NodeType::Sub),
            ("*=", NodeType::Mul),
            ("/=", NodeType::Div),
            ("%=", NodeType::Mod),
            ("<<=", NodeType::BitLeft),
            (">>=", NodeType::BitRight),
            ("&=", NodeType::BitAnd),
            ("^=", NodeType::BitXor),
            ("|=", NodeType::BitOr),
        ] {
            if let Some(token) = self.attempt_reserved(symbol) {
                let rhs = self.assign();
                return self.compound_assignment(token, node, rhs, operation);
            }
        }
        node
    }
    fn ternary(&mut self) -> Node {
        let node = self.logical_or();
        if let Some(t) = self.attempt_reserved("?") {
            let then = self.expr();
            self.expect_reserved(":");
            let els = self.ternary();
            let mut conditional = Node::new_if_node(Some(t), node, then, Some(els));
            conditional.nt = NodeType::Conditional;
            return conditional;
        }
        node
    }
    fn logical_or(&mut self) -> Node {
        let mut node = self.logical_and();
        loop {
            if let Some(t) = self.attempt_reserved("||") {
                node = Node::new_with_op(Some(t), NodeType::LogicalOr, node, self.logical_and());
            } else {
                return node;
            }
        }
    }
    fn logical_and(&mut self) -> Node {
        let mut node = self.bitwise_or();
        loop {
            if let Some(t) = self.attempt_reserved("&&") {
                node = Node::new_with_op(Some(t), NodeType::LogicalAnd, node, self.bitwise_or());
            } else {
                return node;
            }
        }
    }
    fn bitwise_or(&mut self) -> Node {
        let mut node = self.bitwise_xor();
        loop {
            if let Some(t) = self.attempt_reserved("|") {
                node = Node::new_with_op(Some(t), NodeType::BitOr, node, self.bitwise_xor())
            } else {
                return node;
            }
        }
    }
    fn bitwise_xor(&mut self) -> Node {
        let mut node = self.bitwise_and();
        loop {
            if let Some(t) = self.attempt_reserved("^") {
                node = Node::new_with_op(Some(t), NodeType::BitXor, node, self.bitwise_and())
            } else {
                return node;
            }
        }
    }
    fn bitwise_and(&mut self) -> Node {
        let mut node = self.equality();
        loop {
            if let Some(t) = self.attempt_reserved("&") {
                node = Node::new_with_op(Some(t), NodeType::BitAnd, node, self.equality())
            } else {
                return node;
            }
        }
    }
    fn equality(&mut self) -> Node {
        let mut node = self.relational();
        loop {
            if let Some(t) = self.attempt_reserved("==") {
                node = Node::new_with_op(Some(t), NodeType::Eq, node, self.relational())
            } else if let Some(t) = self.attempt_reserved("!=") {
                node = Node::new_with_op(Some(t), NodeType::Ne, node, self.relational())
            } else {
                return node;
            }
        }
    }
    fn relational(&mut self) -> Node {
        let mut node = self.bit_shift();
        loop {
            if let Some(t) = self.attempt_reserved("<") {
                node = Node::new_with_op(Some(t), NodeType::Lt, node, self.bit_shift())
            } else if let Some(t) = self.attempt_reserved("<=") {
                node = Node::new_with_op(Some(t), NodeType::Le, node, self.bit_shift())
            } else if let Some(t) = self.attempt_reserved(">") {
                node = Node::new_with_op(Some(t), NodeType::Lt, self.bit_shift(), node)
            } else if let Some(t) = self.attempt_reserved(">=") {
                node = Node::new_with_op(Some(t), NodeType::Le, self.bit_shift(), node)
            } else {
                return node;
            }
        }
    }
    fn bit_shift(&mut self) -> Node {
        let mut node = self.add();
        loop {
            if let Some(t) = self.attempt_reserved("<<") {
                node = Node::new_with_op(Some(t), NodeType::BitLeft, node, self.add())
            } else if let Some(t) = self.attempt_reserved(">>") {
                node = Node::new_with_op(Some(t), NodeType::BitRight, node, self.add())
            } else {
                return node;
            }
        }
    }
    fn add(&mut self) -> Node {
        let mut node = self.mul();
        loop {
            if let Some(t) = self.attempt_reserved("+") {
                node = Node::new_with_op(Some(t), NodeType::Add, node, self.mul())
            } else if let Some(t) = self.attempt_reserved("-") {
                node = Node::new_with_op(Some(t), NodeType::Sub, node, self.mul())
            } else {
                return node;
            }
        }
    }
    fn mul(&mut self) -> Node {
        let mut node = self.unary();
        loop {
            if let Some(t) = self.attempt_reserved("*") {
                node = Node::new_with_op(Some(t), NodeType::Mul, node, self.unary())
            } else if let Some(t) = self.attempt_reserved("/") {
                node = Node::new_with_op(Some(t), NodeType::Div, node, self.unary())
            } else if let Some(t) = self.attempt_reserved("%") {
                node = Node::new_with_op(Some(t), NodeType::Mod, node, self.unary())
            } else {
                return node;
            }
        }
    }
    fn initializer_expressions(node: Node, expressions: &mut Vec<Node>) {
        if matches!(node.nt, NodeType::Block | NodeType::DefVar) {
            for child in node.children {
                Self::initializer_expressions(child, expressions);
            }
        } else {
            expressions.push(node);
        }
    }

    fn unary(&mut self) -> Node {
        let saved = self.cur;
        if let Some(token) = self.attempt_reserved("(") {
            if let Some(base) = self.attempt_type() {
                let (name, mut ty, _) = self.declarator().resolve(base);
                self.validate_type_layout(&ty, &self.tokens[self.cur]);
                if name.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "cast requires a type name");
                }
                self.expect_reserved(")");
                if self.tokens[self.cur].s_value == "{" {
                    if matches!(ty, Type::Arr(_, 0)) {
                        let saved = self.cur;
                        let count = self.initializer_count();
                        self.cur = saved;
                        ty = self.infer_array_size(ty, count, &token);
                    }
                    let name = Token {
                        s_value: format!(".compound.{}", token.pos),
                        ..token.clone()
                    };
                    let literal = if self.return_type.is_some() {
                        let object = self.register_local_variable(name, ty);
                        let initializer = self.local_variable_initialization(&object, &token);
                        let mut children = vec![];
                        Self::initializer_expressions(initializer, &mut children);
                        children.push(object);
                        Node {
                            nt: NodeType::Sequence,
                            token: Some(token.clone()),
                            compound_literal: true,
                            children,
                            ..Node::default()
                        }
                    } else {
                        self.expect_object_type(&ty, &token);
                        let data = self.global_initializer(&ty);
                        self.validate_global_initializer(&ty, &data, &token);
                        self.global_variables.insert(
                            name.s_value.clone(),
                            GlobalVariable {
                                ty: ty.clone(),
                                data: Some(data),
                                internal: true,
                                declaration_only: false,
                            },
                        );
                        Node {
                            nt: NodeType::GlobalVar,
                            cty: Some(ty),
                            token: Some(token.clone()),
                            global_name: name.s_value,
                            ..Node::default()
                        }
                    };
                    return self.postfix(literal);
                }
                let operand = self.unary();
                if !(ty.integer_info().is_some() || matches!(ty, Type::Void | Type::Ptr(_)))
                    || (!matches!(
                        operand.resolve_type(),
                        Some(
                            Type::I8
                                | Type::I32
                                | Type::Integer { .. }
                                | Type::Bool
                                | Type::Ptr(_)
                                | Type::Arr(..)
                                | Type::Func(..)
                                | Type::VariadicFunc(..)
                        )
                    ) && ty != Type::Void)
                {
                    self.error_logger
                        .print_error_position(token.pos, "invalid cast");
                }
                return Node {
                    nt: NodeType::Cast,
                    token: Some(token),
                    cty: Some(ty),
                    lhs: Some(Box::new(operand)),
                    ..Node::default()
                };
            }
            self.cur = saved;
        }

        if let Some(t) = self.attempt_reserved("sizeof") {
            let saved = self.cur;
            let size = if self.attempt_reserved("(").is_some() {
                if let Some(base) = self.attempt_type() {
                    let (name, ty, _) = self.declarator().resolve(base);
                    self.validate_type_layout(&ty, &self.tokens[self.cur]);
                    if name.is_some() {
                        self.error_logger
                            .print_error_position(t.pos, "sizeof requires a type name");
                    }
                    self.expect_reserved(")");
                    if matches!(ty, Type::Void | Type::Func(..) | Type::VariadicFunc(..))
                        || ty.size_of() == 0
                    {
                        self.error_logger
                            .print_error_position(t.pos, "sizeof requires a complete object type");
                    }
                    ty.size_of()
                } else {
                    self.cur = saved;
                    self.unary().resolve_type().unwrap().size_of()
                }
            } else {
                self.unary().resolve_type().unwrap().size_of()
            };
            return Node {
                token: Some(t),
                value: Some(size),
                cty: Some(Type::I32),
                ..Node::default()
            };
        }
        if self.attempt_reserved("+").is_some() {
            return self.unary();
        } else if let Some(t) = self.attempt_reserved("-") {
            return Node::new_with_op(
                Some(t.clone()),
                NodeType::Sub,
                Node::new_with_num(Some(t.clone()), 0),
                self.unary(),
            );
        }
        if let Some(token) = self.attempt_reserved("&") {
            let operand = self.unary();
            if !(operand.compound_literal
                || matches!(
                    operand.nt,
                    NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref
                ))
            {
                self.error_logger
                    .print_error_position(token.pos, "expression is not addressable");
            }
            return Node::new_with_op_and_lhs(Some(token), NodeType::Addr, operand);
        }
        if let Some(token) = self.attempt_reserved("*") {
            let operand = self.unary();
            if operand.dest_type().is_none() || operand.dest_type() == Some(Type::Void) {
                self.error_logger
                    .print_error_position(token.pos, "cannot dereference a non-pointer");
            }
            return Node::new_with_op_and_lhs(Some(token), NodeType::Deref, operand);
        }
        if let Some(t) = self.attempt_reserved("~") {
            return Node {
                token: Some(t),
                nt: NodeType::BitNot,
                lhs: Some(Box::new(self.unary())),
                ..Node::default()
            };
        }
        if let Some(t) = self.attempt_reserved("!") {
            return Node {
                token: Some(t),
                nt: NodeType::Eq,
                lhs: Some(Box::new(self.unary())),
                rhs: Some(Box::new(Node::new_with_num(None, 0))),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("++") {
            let operand = self.unary();
            return self.compound_assignment(
                token,
                operand,
                Node::new_with_num(None, 1),
                NodeType::Add,
            );
        }
        if let Some(token) = self.attempt_reserved("--") {
            let operand = self.unary();
            return self.compound_assignment(
                token,
                operand,
                Node::new_with_num(None, 1),
                NodeType::Sub,
            );
        }
        self.prim()
    }
    fn prim(&mut self) -> Node {
        let node = if self.attempt_reserved("(").is_some() {
            let node = self.expr();
            self.expect_reserved(")");
            node
        } else if let Some(t) = self.attempt(TokenType::Str) {
            // String literal
            let node = Node {
                token: Some(t.clone()),
                nt: NodeType::GlobalVar,
                dest: self.new_string_literal(&t.s_value),
                ..Node::default()
            };
            Node {
                token: Some(t.clone()),
                nt: NodeType::Addr,
                cty: Some(Type::Ptr(Box::new(Type::I8))),
                lhs: Some(Box::new(node)),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt(TokenType::Num) {
            // Number literal
            Node {
                token: Some(t.clone()),
                nt: NodeType::Num,
                value: Some(t.i_value),
                cty: Some(Type::integer(
                    if t.s_value.contains('l') { 64 } else { 32 },
                    !t.s_value.contains('u'),
                )),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_ident() {
            if let Some(ident) = self.resolve_name(&t.s_value) {
                match ident {
                    Identifier::Local(ty, offset) => Node {
                        token: Some(t.clone()),
                        nt: NodeType::LocalVar,
                        cty: Some(ty.clone()),
                        offset: Some(*offset),
                        ..Node::default()
                    },
                    Identifier::StaticObject(ty, name) => Node {
                        token: Some(t.clone()),
                        nt: NodeType::GlobalVar,
                        cty: Some(ty.clone()),
                        global_name: name.clone(),
                        ..Node::default()
                    },
                    Identifier::Static(ty) => Node {
                        token: Some(t.clone()),
                        nt: NodeType::GlobalVar,
                        cty: Some(ty.clone()),
                        global_name: t.s_value.clone(),
                        ..Node::default()
                    },
                    Identifier::Constant(value) => {
                        Node::new_with_num(Some(t.clone()), *value as usize)
                    }
                    Identifier::TypeDef(..) => {
                        self.error_logger
                            .print_error_position(t.pos, "type name is not an expression");
                        unreachable!()
                    }
                }
            } else {
                self.error_logger
                    .print_error_position(t.pos, "undefined variable");
                unreachable!();
            }
        } else {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "unexpected token");
            unreachable!();
        };
        self.postfix(node)
    }
    fn postfix(&mut self, mut node: Node) -> Node {
        loop {
            if self.attempt_reserved("(").is_some() {
                // Call function
                let t = node.token.clone().unwrap();
                let (parameter_types, return_type, variadic) = node
                    .resolve_type()
                    .and_then(|ty| match ty {
                        Type::Ptr(child) => child.function_signature(),
                        _ => ty.function_signature(),
                    })
                    .unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(t.pos, "called expression is not a function");
                        unreachable!()
                    });
                if matches!(return_type, Type::Struct(_))
                    || parameter_types
                        .iter()
                        .any(|ty| matches!(ty, Type::Struct(_)))
                {
                    self.error_logger.print_error_position(
                        t.pos,
                        "struct function arguments and results must use pointers",
                    );
                }
                let mut args: Vec<Node> = Vec::new();
                if self.attempt_reserved(")").is_none() {
                    args.push(self.assign());
                    while self.attempt_reserved(")").is_none() {
                        self.expect_reserved(",");
                        args.push(self.assign());
                    }

                    // conforming to cdecl
                    // arguments are pushed onto the stack, from right to left
                    args.reverse();
                }
                if args.len() < parameter_types.len()
                    || (!variadic && args.len() != parameter_types.len())
                {
                    self.error_logger.print_error_position(
                        t.pos,
                        "argument count does not match function declaration",
                    );
                }
                let direct = matches!(node.nt, NodeType::GlobalVar)
                    && node
                        .resolve_type()
                        .is_some_and(|ty| ty.function_signature().is_some());
                let s_value = if direct {
                    node.global_name.clone()
                } else {
                    String::new()
                };
                let callee = if direct { None } else { Some(Box::new(node)) };
                node = Node {
                    token: Some(t),
                    nt: NodeType::CallFunc,
                    global_name: s_value,
                    cty: Some(return_type),
                    args,
                    parameter_types,
                    variadic,
                    lhs: callee,
                    ..Node::default()
                }
            } else if let Some(token) = self
                .attempt_reserved(".")
                .or_else(|| self.attempt_reserved("->"))
            {
                if token.s_value == "->" {
                    node = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, node);
                }
                let Some(Type::Struct(ty)) = node.resolve_type() else {
                    self.error_logger
                        .print_error_position(token.pos, "member access requires a struct");
                    unreachable!()
                };
                let name = self.attempt_ident().unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(self.tokens[self.cur].pos, "member name expected");
                    unreachable!()
                });
                let member = ty
                    .members()
                    .into_iter()
                    .find(|member| member.name == name.s_value)
                    .unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(name.pos, "unknown struct member");
                        unreachable!()
                    });
                node = Self::member_node(node, &member, &token);
            } else if let Some(b_token) = self.attempt_reserved("[") {
                // Subscript array
                node = Node::new_with_op(Some(b_token.clone()), NodeType::Add, node, self.expr());
                self.expect_reserved("]");
                node = Node {
                    token: Some(b_token.clone()),
                    nt: NodeType::Deref,
                    lhs: Some(Box::new(node)),
                    ..Node::default()
                }
            } else if let Some(token) = self.attempt_reserved("++") {
                // Suffix increment
                self.expect_assignable(&node);
                node = Node::new_with_op_and_lhs(Some(token), NodeType::SuffixIncr, node);
            } else if let Some(token) = self.attempt_reserved("--") {
                // Suffix decrement
                self.expect_assignable(&node);
                node = Node::new_with_op_and_lhs(Some(token), NodeType::SuffixDecr, node);
            } else {
                return node;
            }
        }
    }

    fn new_string_literal(&mut self, s: &str) -> String {
        self.string_literals.push(s.to_string());
        format!("@.str.{}", self.string_literals.len() - 1)
    }

    pub fn print_functions(&self) {
        self.functions
            .iter()
            .filter(|(_, f)| f.body.is_some())
            .for_each(|(s, f)| {
                eprintln!("[FUNC: {}]\n{}", s, f.to_debug_string());
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::error::NopLogger;

    use super::*;

    #[test]
    fn malformed_programs_have_source_diagnostics() {
        for (code, message) in [
            ("int main() { return", "unexpected eof"),
            ("int main() { int x; return x(); }", "not a function"),
            ("int a[2] = {1, 2, 3};", "too many array initializer"),
            ("int main() { return puts(); }", "argument count"),
            ("int main() { 3 = 4; }", "not assignable"),
            ("int main() { return *3; }", "non-pointer"),
            ("int x = 1 / 0;", "invalid arithmetic"),
            ("int a[2](int);", "invalid array element"),
            ("int x; char x;", "incompatible object redeclaration"),
            ("int x=1; int x=2;", "already initialized"),
            ("int main(){int *p;p=3;return 0;}", "incompatible value"),
        ] {
            let tokens = crate::tokenizer::Tokenizer::tokenize(code, false).unwrap();
            let logger = crate::error::ErrorPrinter::new(code);
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                AstBuilderImpl::new(&logger, &tokens).build(false);
            }))
            .unwrap_err();
            let text = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(text.contains(message), "{}", text);
            assert!(text.contains('^'), "{}", text);
        }
    }

    #[test]
    fn invalid_void_values_have_source_diagnostics() {
        for code in [
            "void f() {} int main() { return f(); }",
            "void f() {} int main() { int n = f(); return n; }",
            "void f() {} int main() { if (f()) return 1; return 0; }",
            "void f() {} int main() { return f() + 1; }",
            "int main() { void *p = 0; return *p; }",
        ] {
            let tokens = crate::tokenizer::Tokenizer::tokenize(code, false).unwrap();
            let logger = crate::error::ErrorPrinter::new(code);
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                AstBuilderImpl::new(&logger, &tokens).build(false);
            }))
            .unwrap_err();
            let text = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(text.contains('^'), "{text}");
        }
    }

    #[test]
    fn invalid_structs_have_source_diagnostics() {
        for (code, message) in [
            ("struct S { int x; int x; };", "duplicate struct member"),
            (
                "struct S; struct S x;",
                "object requires a complete non-void type",
            ),
            (
                "struct S { struct S x; };",
                "object requires a complete non-void type",
            ),
            (
                "struct S { int x; }; int main() { struct S s; return s.missing; }",
                "unknown struct member",
            ),
            (
                "struct S { int x; }; struct S s = {1,2};",
                "too many struct initializer",
            ),
            (
                "struct S { int x; }; int f(struct S s) { return 0; }",
                "struct function arguments",
            ),
        ] {
            let tokens = crate::tokenizer::Tokenizer::tokenize(code, false).unwrap();
            let logger = crate::error::ErrorPrinter::new(code);
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                AstBuilderImpl::new(&logger, &tokens).build(false);
            }))
            .unwrap_err();
            let text = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(text.contains(message) && text.contains('^'), "{text}");
        }
    }

    #[test]
    fn test_ast_builder() {
        let error_logger = NopLogger::default();
        let tokens = &vec![
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                s_value: "int".to_string(),
                i_value: 0,
            },
            Token {
                tt: TokenType::Ident,
                pos: 0,
                i_value: 0,
                s_value: "x".to_string(),
            },
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                i_value: 0,
                s_value: "=".to_string(),
            },
            Token {
                tt: TokenType::Num,
                pos: 0,
                i_value: 42,
                s_value: String::new(),
            },
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                i_value: 0,
                s_value: ";".to_string(),
            },
        ];
        let mut ast_builder = AstBuilderImpl::new(&error_logger, tokens);
        let program_ast = ast_builder.build(false);
        assert_eq!(program_ast.functions.len(), 0);
        assert_eq!(*program_ast.global_variables, {
            let mut map = HashMap::new();
            map.insert(
                "x".to_string(),
                GlobalVariable {
                    internal: false,
                    declaration_only: false,
                    ty: Type::I32,
                    data: Some(GlobalVariableData::Elem("42".to_string())),
                },
            );
            map
        });
        assert_eq!(program_ast.string_literals.len(), 0);
    }
}

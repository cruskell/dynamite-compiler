use crate::ctype::Type;
use crate::error;
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use crate::token::{Token, TokenType};
use std::collections::HashMap;

pub struct AstBuilderImpl<'a> {
    error_logger: &'a dyn error::ErrorLogger,
    tokens: &'a Vec<Token>,
    cur: usize,
    offset_size: usize,
    loop_stack: Vec<usize>,
    switch_stack: Vec<Vec<Node>>,
    return_type: Option<Type>,
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
            Identifier::Static(Type::Func(
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
            switch_stack: vec![],
            return_type: None,
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

    fn expect_ident_with_type(&mut self, ty: Type) -> (Token, Type) {
        let mut ty = ty.clone();
        while self.attempt_reserved("*").is_some() {
            ty = Type::Ptr(Box::new(ty));
        }
        if let Some(t) = self.attempt_ident() {
            let mut dimensions = vec![];
            while self.attempt_reserved("[").is_some() {
                if self.attempt_reserved("]").is_some() {
                    if !dimensions.is_empty() {
                        self.error_logger.print_error_position(
                            t.pos,
                            "only the outer array dimension may be omitted",
                        );
                    }
                    dimensions.push(0);
                } else {
                    let size = self.constant_array_size();
                    dimensions.push(size.i_value);
                    self.expect_reserved("]");
                }
            }
            for size in dimensions.into_iter().rev() {
                if ty.size_of().checked_mul(size).is_none() {
                    self.error_logger
                        .print_error_position(t.pos, "array size is too large");
                }
                ty = Type::Arr(Box::new(ty), size);
            }
            (t, ty)
        } else {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "ident expected");
            unreachable!();
        }
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
        self.expect_reserved("{");
        let mut depth = 0;
        let mut count = 0;
        let mut element = false;
        loop {
            let token = &self.tokens[self.cur];
            if token.tt == TokenType::EOF {
                self.error_logger
                    .print_error_position(token.pos, "unterminated initializer");
            }
            match token.s_value.as_str() {
                "}" if depth == 0 => {
                    return count + usize::from(element);
                }
                "," if depth == 0 => {
                    count += usize::from(element);
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
        for (name, ty) in [("int", Type::I32), ("char", Type::I8), ("void", Type::Void)] {
            if self.attempt_reserved(name).is_some() {
                return Some(ty);
            }
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
    fn expect_object_type(&self, ty: &Type, token: &Token) {
        if matches!(ty, Type::Void) || matches!(ty, Type::Arr(child, _) if **child == Type::Void) {
            self.error_logger
                .print_error_position(token.pos, "object cannot have void type");
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
        if self.attempt_reserved("typedef").is_some() {
            self.typedef_definition();
            self.expect_reserved(";");
            return;
        }
        let ty = self.expect_type();
        let cur_to_back = self.cur;
        let (t, return_type) = self.expect_ident_with_type(ty.clone());
        if self.attempt_reserved("(").is_some() {
            // function
            self.offset_size = 0;
            self.scope_stack.push(HashMap::new());
            let mut args: Vec<Node> = Vec::new();
            let saved = self.cur;
            let void_parameters =
                self.attempt_reserved("void").is_some() && self.attempt_reserved(")").is_some();
            if !void_parameters {
                self.cur = saved;
            }
            if !void_parameters && self.attempt_reserved(")").is_none() {
                loop {
                    let ty = self.expect_type();
                    let (token, ty) = self.expect_ident_with_type(ty);
                    let ty = if let Type::Arr(child, _) = ty {
                        Type::Ptr(child)
                    } else {
                        ty
                    };
                    args.push(self.register_local_variable(token, ty));
                    if self.attempt_reserved(",").is_none() {
                        break;
                    }
                }
                self.expect_reserved(")");
            }
            if args.len() >= 7 {
                self.error_logger
                    .print_error_position(t.pos, "count of args must be less than 7")
            }
            let arg_types: Vec<Type> = args
                .iter()
                .map(|arg| arg.resolve_type().clone().unwrap())
                .collect();
            self.scope_stack.first_mut().unwrap().insert(
                t.s_value.clone(),
                Identifier::Static(Type::Func(arg_types.clone(), Box::new(return_type.clone()))),
            );
            self.functions.insert(
                t.s_value.clone(),
                Func {
                    cty: Type::Func(arg_types.clone(), Box::new(return_type.clone())),
                    token: Some(t.clone()),
                    ..Func::default()
                },
            );
            self.return_type = Some(return_type.clone());
            let body = self.consume_block();
            self.return_type = None;
            if body.is_none() {
                self.expect_reserved(";");
            }
            self.scope_stack.pop();
            self.functions.insert(
                t.s_value.clone(),
                Func {
                    body,
                    cty: Type::Func(arg_types.clone(), Box::new(return_type.clone())),
                    offset_size: self.offset_size,
                    token: Some(t.clone()),
                    args,
                },
            );
        } else {
            // global variable
            self.cur = cur_to_back; // back the cursor
            loop {
                let (t, ty) = self.expect_ident_with_type(ty.clone());
                self.expect_object_type(&ty, &t);
                let data = self.attempt_reserved("=").map(|_| self.global_data());
                let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                    elements.len()
                } else {
                    0
                };
                let ty = self.infer_array_size(ty, count, &t);
                if let Some(data) = &data {
                    self.validate_global_initializer(&ty, data, &t);
                }
                self.scope_stack
                    .last_mut()
                    .unwrap()
                    .insert(t.s_value.clone(), Identifier::Static(ty.clone()));
                self.global_variables.insert(
                    t.s_value.clone(),
                    GlobalVariable {
                        ty: ty.clone(),
                        data,
                    },
                );
                if self.attempt_reserved(",").is_none() {
                    break;
                }
            }
            self.expect_reserved(";");
        }
    }
    fn resolve_name(&mut self, s: &str) -> Option<&Identifier> {
        self.scope_stack.iter().rev().find_map(|map| map.get(s))
    }
    fn validate_global_initializer(&self, ty: &Type, data: &GlobalVariableData, token: &Token) {
        match (ty, data) {
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
            (Type::I8 | Type::I32, GlobalVariableData::Elem(value)) if value.starts_with('@') => {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be a string");
            }
            _ => {}
        }
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
        } else if let Some(t) = self.attempt(TokenType::Str) {
            GlobalVariableData::Elem(self.new_string_literal(&t.s_value))
        } else {
            let expression = self.ternary();
            GlobalVariableData::Elem(format!("{}", self.eval(&expression)))
        }
    }
    fn eval(&mut self, node: &Node) -> i64 {
        match node.nt {
            NodeType::Num => return node.value.unwrap() as i64,
            NodeType::Cast => {
                let value = self.eval(node.lhs.as_ref().unwrap());
                return match node.cty.as_ref().unwrap() {
                    Type::I8 => value as i8 as i64,
                    Type::I32 => value as i32 as i64,
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
        let lhs = self.eval(node.lhs.as_ref().unwrap());
        let rhs = self.eval(node.rhs.as_ref().unwrap());
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
            NodeType::BitLeft if (0..32).contains(&rhs) => lhs.checked_shl(rhs as u32),
            NodeType::BitRight if (0..32).contains(&rhs) => lhs.checked_shr(rhs as u32),
            _ => None,
        };
        result.unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "invalid arithmetic in constant initializer",
            );
            unreachable!()
        })
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
        if matches!(node.nt, NodeType::CallFunc) {
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
            if !matches!(cond.resolve_type(), Some(Type::I8 | Type::I32)) {
                self.error_logger
                    .print_error_position(t.pos, "switch requires an integer expression");
            }
            self.switch_stack.push(vec![]);
            let body = self.stmt();
            let cases = self.switch_stack.pop().unwrap();
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
                Some(self.eval(&expr) as i32 as usize)
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
        let node = if let Some(ty) = self.attempt_type() {
            self.local_variable_definition(ty)
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
            let operand = if self.tokens[self.cur].s_value == ";" {
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
        if let Some(Type::Arr(_, size)) = node.resolve_type() {
            Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: (0..size)
                    .map(|index| {
                        Self::zero_initializer(&Self::array_element(node, index, token), token)
                    })
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
        if let Some(token) = self.attempt_reserved("{") {
            let Some(Type::Arr(_, size)) = node.resolve_type() else {
                let initializer = self.local_variable_initialization(node, assign_token);
                self.attempt_reserved(",");
                self.expect_reserved("}");
                return initializer;
            };
            let mut children = vec![];
            if self.attempt_reserved("}").is_none() {
                loop {
                    if children.len() >= size {
                        self.error_logger
                            .print_error_position(token.pos, "too many array initializer elements");
                    }
                    let element = Self::array_element(node, children.len(), &token);
                    children.push(self.local_variable_initialization(&element, assign_token));
                    if self.attempt_reserved(",").is_none() {
                        self.expect_reserved("}");
                        break;
                    }
                    if self.attempt_reserved("}").is_some() {
                        break;
                    }
                }
            }
            for index in children.len()..size {
                children.push(Self::zero_initializer(
                    &Self::array_element(node, index, &token),
                    assign_token,
                ));
            }
            Node {
                token: Some(assign_token.clone()),
                nt: NodeType::Block,
                children,
                ..Node::default()
            }
        } else {
            Node::new_with_op(
                Some(assign_token.clone()),
                NodeType::Assign,
                node.clone(),
                self.assign(),
            )
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
        if !matches!(
            node.nt,
            NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref
        ) || matches!(
            node.resolve_type(),
            Some(Type::Arr(..) | Type::Func(..)) | None
        ) {
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
            return Node::new_with_op(Some(token), NodeType::Assign, node, self.assign());
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
    fn unary(&mut self) -> Node {
        let saved = self.cur;
        if let Some(token) = self.attempt_reserved("(") {
            if let Some(mut ty) = self.attempt_type() {
                while self.attempt_reserved("*").is_some() {
                    ty = Type::Ptr(Box::new(ty));
                }
                self.expect_reserved(")");
                let operand = self.unary();
                if !matches!(ty, Type::Void | Type::I8 | Type::I32 | Type::Ptr(_))
                    || (!matches!(
                        operand.resolve_type(),
                        Some(Type::I8 | Type::I32 | Type::Ptr(_) | Type::Arr(..))
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
                if let Some(mut ty) = self.attempt_type() {
                    while self.attempt_reserved("*").is_some() {
                        ty = Type::Ptr(Box::new(ty));
                    }
                    let mut dimensions = vec![];
                    while self.attempt_reserved("[").is_some() {
                        let size = self.constant_array_size();
                        if size.i_value == 0 {
                            self.error_logger
                                .print_error_position(size.pos, "array size must be positive");
                        }
                        dimensions.push(size.i_value);
                        self.expect_reserved("]");
                    }
                    for size in dimensions.into_iter().rev() {
                        ty = Type::Arr(Box::new(ty), size);
                    }
                    self.expect_reserved(")");
                    if ty == Type::Void {
                        self.error_logger
                            .print_error_position(t.pos, "sizeof requires an object type");
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
            if !matches!(
                operand.nt,
                NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref
            ) {
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
        let mut node = if self.attempt_reserved("(").is_some() {
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
                cty: Some(Type::I32),
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
                    Identifier::Static(ty) => Node {
                        token: Some(t.clone()),
                        nt: NodeType::GlobalVar,
                        cty: Some(ty.clone()),
                        global_name: t.s_value.clone(),
                        ..Node::default()
                    },
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
        loop {
            if self.attempt_reserved("(").is_some() {
                // Call function
                let t = node.token.clone().unwrap();
                let (parameter_types, return_type) =
                    if let Some(Type::Func(parameters, return_type)) = node.resolve_type() {
                        (parameters, *return_type)
                    } else {
                        self.error_logger
                            .print_error_position(t.pos, "called expression is not a function");
                        unreachable!()
                    };
                let mut args: Vec<Node> = Vec::new();
                if self.attempt_reserved(")").is_none() {
                    args.push(self.assign());
                    while self.attempt_reserved(")").is_none() {
                        self.expect_reserved(",");
                        args.push(self.assign());
                    }
                    if args.len() >= 7 {
                        self.error_logger
                            .print_error_position(t.pos, "count of args must be less than 7")
                    }
                    // conforming to cdecl
                    // arguments are pushed onto the stack, from right to left
                    args.reverse();
                }
                if args.len() < parameter_types.len()
                    || (t.s_value != "printf" && args.len() != parameter_types.len())
                {
                    self.error_logger.print_error_position(
                        t.pos,
                        "argument count does not match function declaration",
                    );
                }
                let s_value = t.s_value.clone();
                node = Node {
                    token: Some(t),
                    nt: NodeType::CallFunc,
                    global_name: s_value,
                    cty: Some(return_type),
                    args,
                    ..Node::default()
                }
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
                    ty: Type::I32,
                    data: Some(GlobalVariableData::Elem("42".to_string())),
                },
            );
            map
        });
        assert_eq!(program_ast.string_literals.len(), 0);
    }
}

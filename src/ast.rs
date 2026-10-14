use crate::ctype::{BitField, Qualifiers, StructDefinition, StructMember, StructType, Type};
use crate::error;
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{Node, NodeType};
use crate::token::{Token, TokenType};
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

#[path = "varargs.rs"]
mod varargs;
#[path = "vla.rs"]
mod vla;

#[derive(Clone, Copy, PartialEq)]
enum AutomaticStorage {
    Default,
    Auto,
    Register,
}

type Parameters = Vec<(Option<Token>, Type, bool)>;
type ParameterScope = HashMap<String, Identifier>;
type ResolvedDeclarator = (Option<Token>, Type, Parameters, ParameterScope, Vec<Token>);

enum Declarator {
    Name(Option<Token>),
    Pointer(Box<Declarator>, Qualifiers),
    Array(Box<Declarator>, usize),
    VariableArray(Box<Declarator>, usize),
    ArrayQualifiers(Box<Declarator>, Qualifiers),
    UnspecifiedArray(Box<Declarator>, Token),
    Function(
        Box<Declarator>,
        Parameters,
        bool,
        ParameterScope,
        Vec<Token>,
    ),
}

impl Declarator {
    fn resolve(self, base: Type) -> ResolvedDeclarator {
        match self {
            Self::Name(token) => (token, base, vec![], HashMap::new(), vec![]),
            Self::Pointer(inner, q) => {
                let ty = Type::Ptr(Box::new(base));
                inner.resolve(if q == Qualifiers::default() {
                    ty
                } else {
                    Type::Qualified(Box::new(ty), q)
                })
            }
            Self::Array(inner, size) => inner.resolve(Type::Arr(Box::new(base), size)),
            Self::VariableArray(inner, offset) => inner.resolve(Type::Vla(Box::new(base), offset)),
            Self::ArrayQualifiers(inner, q) => inner.resolve(Type::Qualified(Box::new(base), q)),
            Self::UnspecifiedArray(inner, token) => {
                let (name, ty, parameters, scope, mut stars) =
                    inner.resolve(Type::Vla(Box::new(base), usize::MAX));
                stars.push(token);
                (name, ty, parameters, scope, stars)
            }
            Self::Function(inner, parameters, variadic, scope, stars) => {
                let types = parameters.iter().map(|(_, ty, _)| ty.clone()).collect();
                let ty = if variadic {
                    Type::VariadicFunc(types, Box::new(base))
                } else {
                    Type::Func(types, Box::new(base))
                };
                let function_type = ty.clone();
                let (name, ty, nested, nested_scope, nested_stars) = inner.resolve(ty);
                let (parameters, scope, stars) = if ty == function_type {
                    (parameters, scope, stars)
                } else {
                    (nested, nested_scope, nested_stars)
                };
                (name, ty, parameters, scope, stars)
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
    mutable_static: Option<Token>,
    return_pointer: Option<usize>,
    variadic_parameters: Option<Vec<Type>>,
    variadic_last: Option<usize>,
    pending_bounds: Vec<Node>,
    parameter_depth: usize,
    dynamic_objects: Vec<Node>,
    vla_active: Vec<usize>,
    jump_depths: Vec<usize>,
    continue_depths: HashMap<usize, usize>,
    switch_vla_depths: Vec<usize>,
    struct_definitions: Vec<Rc<RefCell<StructDefinition>>>,
    scope_stack: Vec<HashMap<String, Identifier>>,
    pub functions: HashMap<String, Func>,
    pub global_variables: HashMap<String, GlobalVariable>,
    pub string_literals: Vec<Token>,
}

pub struct ProgramAst<'a> {
    pub functions: &'a HashMap<String, Func>,
    pub global_variables: &'a HashMap<String, GlobalVariable>,
    pub string_literals: &'a Vec<Token>,
}

pub enum Identifier {
    TypeDef(Type),
    Constant(i64),
    StaticObject(Type, String),
    Local(Type, usize, bool),
    Dynamic(Type, usize, bool),
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
                if function.inline_only() {
                    if let Some(token) = &function.mutable_static {
                        self.error_logger.print_error_position(
                            token.pos,
                            "external inline definition cannot define a mutable static object",
                        );
                    }
                    self.validate_inline_body(body);
                }
            }
        }
        for function in self.functions.values_mut() {
            function.cty = function.cty.without_qualifiers();
            for arg in &mut function.args {
                Self::erase_qualifiers(arg);
            }
            if let Some(body) = &mut function.body {
                Self::erase_qualifiers(body);
                Self::lower_variable_arithmetic(body);
                Self::insert_integer_conversions(body);
                crate::bitfield::lower(body, &mut function.offset_size);
            }
        }
        for global in self.global_variables.values_mut() {
            global.ty = global.ty.without_qualifiers();
        }
        for definition in &self.struct_definitions {
            for member in &mut definition.borrow_mut().members {
                member.ty = member.ty.without_qualifiers();
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
            String::from("malloc"),
            Identifier::Static(Type::Func(
                vec![Type::integer(64, false)],
                Box::new(Type::Ptr(Box::new(Type::Void))),
            )),
        ),
        (
            String::from("free"),
            Identifier::Static(Type::Func(
                vec![Type::Ptr(Box::new(Type::Void))],
                Box::new(Type::Void),
            )),
        ),
        (
            String::from("printf"),
            Identifier::Static(Type::VariadicFunc(
                vec![Type::Ptr(Box::new(Type::Qualified(
                    Box::new(Type::I8),
                    Qualifiers {
                        constant: true,
                        volatile: false,
                        restricted: false,
                    },
                )))],
                Box::new(Type::I32),
            )),
        ),
        (
            String::from("puts"),
            Identifier::Static(Type::Func(
                vec![Type::Ptr(Box::new(Type::Qualified(
                    Box::new(Type::I8),
                    Qualifiers {
                        constant: true,
                        volatile: false,
                        restricted: false,
                    },
                )))],
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
            mutable_static: None,
            return_pointer: None,
            variadic_parameters: None,
            variadic_last: None,
            pending_bounds: vec![],
            parameter_depth: 0,
            dynamic_objects: vec![],
            vla_active: vec![],
            jump_depths: vec![],
            continue_depths: HashMap::new(),
            switch_vla_depths: vec![],
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
    fn validate_type_layout(&self, ty: &Type, token: &Token) {
        match ty {
            Type::Qualified(child, q) => {
                if q.restricted
                    && !matches!(child.unqualified(), Type::Ptr(pointee) if pointee.function_signature().is_none())
                {
                    self.error_logger.print_error_position(
                        token.pos,
                        "restrict requires a pointer to an object or void",
                    );
                }
                self.validate_type_layout(child, token);
            }
            Type::Ptr(child) => self.validate_type_layout(child, token),
            Type::Arr(child, _) | Type::Vla(child, _) => {
                if child.has_flexible_array() {
                    self.error_logger.print_error_position(
                        token.pos,
                        "array element cannot contain a flexible array member",
                    );
                }
                if matches!(
                    **child,
                    Type::Void | Type::Func(..) | Type::VariadicFunc(..)
                ) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid array element type");
                }
                if child.checked_size_of() == Some(0) {
                    self.error_logger.print_error_position(
                        token.pos,
                        "array element requires a complete object type",
                    );
                }
                self.validate_type_layout(child, token);
            }
            Type::Func(args, result) | Type::VariadicFunc(args, result) => {
                if matches!(
                    **result,
                    Type::Arr(..) | Type::Vla(..) | Type::Func(..) | Type::VariadicFunc(..)
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
        let mut pointers = vec![];
        while self.attempt_reserved("*").is_some() {
            pointers.push(self.attempt_qualifiers());
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
                let mut minimum = self.attempt_reserved("static");
                let qualifiers = self.attempt_qualifiers();
                if minimum.is_none() {
                    minimum = self.attempt_reserved("static");
                }
                if minimum.is_some() || qualifiers != Qualifiers::default() {
                    if self.parameter_depth == 0
                        || !matches!(direct, Declarator::Name(_))
                        || !pointers.is_empty()
                    {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur].pos,
                            "array parameter modifiers require an outer parameter array",
                        );
                    }
                    direct = Declarator::ArrayQualifiers(Box::new(direct), qualifiers);
                }
                if let Some(star) = self.attempt_reserved("*") {
                    if self.parameter_depth == 0 || minimum.is_some() {
                        self.error_logger.print_error_position(
                            star.pos,
                            "unspecified array bound requires a function prototype parameter",
                        );
                    }
                    self.expect_reserved("]");
                    direct = Declarator::UnspecifiedArray(Box::new(direct), star);
                    continue;
                }
                if minimum.is_some() && self.tokens[self.cur].s_value == "]" {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "static array parameter requires a bound",
                    );
                }
                if self.attempt_reserved("]").is_some() {
                    if matches!(
                        direct,
                        Declarator::Array(..) | Declarator::VariableArray(..)
                    ) {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur - 1].pos,
                            "only the outer array dimension may be omitted",
                        );
                    }
                    direct = Declarator::Array(Box::new(direct), 0);
                } else {
                    let token = self.tokens[self.cur].clone();
                    let expression = self.assign();
                    self.expect_reserved("]");
                    if Self::integer_constant(&expression) {
                        let size = self.eval(&expression);
                        if size <= 0 {
                            self.error_logger
                                .print_error_position(token.pos, "array size must be positive");
                        }
                        direct = Declarator::Array(Box::new(direct), size as usize);
                    } else {
                        if expression
                            .resolve_type()
                            .is_none_or(|ty| ty.integer_info().is_none())
                        {
                            self.error_logger.print_error_position(
                                token.pos,
                                "array bound requires an integer expression",
                            );
                        }
                        let bound = self.vla_temporary(&token, Type::integer(64, false));
                        self.pending_bounds.push(Node::new_with_op(
                            Some(token),
                            NodeType::Assign,
                            bound.clone(),
                            expression,
                        ));
                        direct = Declarator::VariableArray(Box::new(direct), bound.offset.unwrap());
                    }
                }
            } else if self.attempt_reserved("(").is_some() {
                self.scope_stack.push(HashMap::new());
                let mut parameters = vec![];
                let mut stars = vec![];
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
                        let (base, storage, _alignment) =
                            self.attempt_automatic_type(true).unwrap_or_else(|| {
                                self.error_logger.print_error_position(
                                    self.tokens[self.cur].pos,
                                    "parameter type expected",
                                );
                                unreachable!()
                            });
                        let registered = storage == AutomaticStorage::Register;
                        if base == Type::Void
                            && parameters.is_empty()
                            && self.attempt_reserved(")").is_some()
                        {
                            break;
                        }
                        self.parameter_depth += 1;
                        let (name, ty, _, _, mut parameter_stars) = self.declarator().resolve(base);
                        self.parameter_depth -= 1;
                        stars.append(&mut parameter_stars);
                        if matches!(ty.unqualified(), Type::Arr(..) | Type::Vla(..)) {
                            self.validate_type_layout(ty.unqualified(), &self.tokens[self.cur]);
                        }
                        let qualifiers = ty.qualifiers();
                        let ty = match ty.unqualified() {
                            Type::Arr(child, _) | Type::Vla(child, _) => {
                                let pointer = Type::Ptr(child.clone());
                                if qualifiers == Qualifiers::default() {
                                    pointer
                                } else {
                                    Type::Qualified(Box::new(pointer), qualifiers)
                                }
                            }
                            Type::Func(..) | Type::VariadicFunc(..) => Type::Ptr(Box::new(ty)),
                            _ => ty,
                        };
                        self.validate_type_layout(&ty, &self.tokens[self.cur]);
                        if let Some(name) = &name {
                            self.scope_stack.last_mut().unwrap().insert(
                                name.s_value.clone(),
                                Identifier::Local(
                                    ty.clone(),
                                    usize::MAX - parameters.len(),
                                    registered,
                                ),
                            );
                        }
                        parameters.push((name, ty, registered));
                        if self.attempt_reserved(")").is_some() {
                            break;
                        }
                        self.expect_reserved(",");
                    }
                }
                let scope = self
                    .scope_stack
                    .pop()
                    .unwrap()
                    .into_iter()
                    .filter(|(_, identifier)| {
                        matches!(identifier, Identifier::TypeDef(_) | Identifier::Constant(_))
                    })
                    .collect();
                direct = Declarator::Function(Box::new(direct), parameters, variadic, scope, stars);
            } else {
                break;
            }
        }
        for q in pointers.into_iter().rev() {
            direct = Declarator::Pointer(Box::new(direct), q);
        }
        direct
    }

    fn expect_ident_with_type(&mut self, ty: Type) -> (Token, Type) {
        let (name, ty, _, _, _) = self.declarator().resolve(ty);
        let token = name.unwrap_or_else(|| {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "ident expected");
            unreachable!()
        });
        self.validate_type_layout(&ty, &token);
        (token, ty)
    }
    fn new_local_variable(&mut self, ty: Type, alignment: usize) -> Node {
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
        self.register_local_variable_aligned(t, ty, alignment)
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
            return token.string_bytes.len() / token.string_encoding.unit_size() + 1;
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
        self.register_local_variable_aligned(t, ty, 0)
    }

    fn register_local_variable_aligned(
        &mut self,
        t: Token,
        ty: Type,
        requested_alignment: usize,
    ) -> Node {
        let alignment = ty.align_of().max(requested_alignment);
        if alignment > 16 {
            self.error_logger.print_error_position(
                t.pos,
                "requested local alignment exceeds the supported stack alignment",
            );
        }
        if ty.is_variable_array() {
            self.expect_object_type(&ty, &t);
            self.offset_size = self.offset_size.div_ceil(8) * 8 + 8;
            if self.scope_stack.last().unwrap().contains_key(&t.s_value) {
                self.error_logger
                    .print_error_position(t.pos, "invalid redeclaration");
            }
            self.scope_stack.last_mut().unwrap().insert(
                t.s_value.clone(),
                Identifier::Dynamic(ty.clone(), self.offset_size, false),
            );
            return Self::dynamic_node(ty, self.offset_size, t);
        }
        self.expect_object_type(&ty, &t);
        self.offset_size += ty.size_of();
        self.offset_size = self.offset_size.div_ceil(alignment) * alignment;
        if self.scope_stack.last().unwrap().contains_key(&t.s_value) {
            self.error_logger
                .print_error_position(t.pos, "invalid redeclaration");
        }
        self.scope_stack.last_mut().unwrap().insert(
            t.s_value.clone(),
            Identifier::Local(ty.clone(), self.offset_size, false),
        );
        Node {
            token: Some(t),
            nt: NodeType::LocalVar,
            cty: Some(ty),
            offset: Some(self.offset_size),
            ..Node::default()
        }
    }
    fn mark_registered(&mut self, node: &mut Node) {
        let name = &node.token.as_ref().unwrap().s_value;
        match self.scope_stack.last_mut().unwrap().get_mut(name).unwrap() {
            Identifier::Local(_, _, registered) | Identifier::Dynamic(_, _, registered) => {
                *registered = true
            }
            _ => unreachable!(),
        }
        node.registered = true;
    }

    fn attempt_automatic_type(
        &mut self,
        parameter: bool,
    ) -> Option<(Type, AutomaticStorage, usize)> {
        let saved = self.cur;
        let mut storage: Option<Token> = None;
        let mut qualifiers = Qualifiers::default();
        let mut alignment = 0;
        loop {
            if let Some(value) = self.attempt_alignment_specifier() {
                alignment = alignment.max(value);
            } else if let Some(token) = self
                .attempt_reserved("auto")
                .or_else(|| self.attempt_reserved("register"))
            {
                if storage.is_some() || (parameter && token.s_value == "auto") {
                    self.error_logger
                        .print_error_position(token.pos, "invalid automatic storage class");
                }
                storage = Some(token);
            } else {
                let q = self.attempt_qualifiers();
                if q == Qualifiers::default() {
                    break;
                }
                qualifiers.constant |= q.constant;
                qualifiers.volatile |= q.volatile;
                qualifiers.restricted |= q.restricted;
            }
        }
        let Some(mut ty) = self.attempt_type() else {
            if storage.is_some() {
                self.error_logger.print_error_position(
                    self.tokens[self.cur].pos,
                    "type expected after storage class",
                );
            }
            self.cur = saved;
            return None;
        };
        while let Some(value) = self.attempt_alignment_specifier() {
            alignment = alignment.max(value);
        }
        if let Some(token) = self
            .attempt_reserved("auto")
            .or_else(|| self.attempt_reserved("register"))
        {
            if storage.is_some() || (parameter && token.s_value == "auto") {
                self.error_logger
                    .print_error_position(token.pos, "invalid automatic storage class");
            }
            storage = Some(token);
        }
        if qualifiers != Qualifiers::default() {
            ty = Type::Qualified(Box::new(ty), qualifiers);
        }
        if parameter && *ty.unqualified() == Type::Void {
            if let Some(token) = &storage {
                self.error_logger
                    .print_error_position(token.pos, "storage class requires an object parameter");
            }
        }
        let storage = match storage.as_ref().map(|token| token.s_value.as_str()) {
            Some("register") => AutomaticStorage::Register,
            Some("auto") => AutomaticStorage::Auto,
            _ => AutomaticStorage::Default,
        };
        if storage == AutomaticStorage::Register && alignment != 0 {
            self.error_logger.print_error_position(
                self.tokens[saved].pos,
                "_Alignas cannot be applied to a register object",
            );
        }
        if parameter && alignment != 0 {
            self.error_logger.print_error_position(
                self.tokens[saved].pos,
                "_Alignas cannot be applied to a parameter",
            );
        }
        Some((ty, storage, alignment))
    }

    fn attempt_qualifiers(&mut self) -> Qualifiers {
        let mut q = Qualifiers::default();
        loop {
            if self.attempt_reserved("const").is_some() {
                q.constant = true;
            } else if self.attempt_reserved("volatile").is_some() {
                q.volatile = true;
            } else if self.attempt_reserved("restrict").is_some() {
                q.restricted = true;
            } else {
                break;
            }
        }
        q
    }

    fn attempt_alignment_specifier(&mut self) -> Option<usize> {
        let token = self.attempt_reserved("_Alignas")?;
        self.expect_reserved("(");
        let alignment = if let Some(base) = self.attempt_type() {
            let (name, ty, _, _, _) = self.declarator().resolve(base);
            if name.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "_Alignas requires a type name");
            }
            if matches!(
                ty.unqualified(),
                Type::Void | Type::Func(..) | Type::VariadicFunc(..)
            ) || ty.size_of() == 0
            {
                self.error_logger
                    .print_error_position(token.pos, "_Alignas requires a complete object type");
            }
            ty.align_of()
        } else {
            let expression = self.ternary();
            if !Self::integer_constant(&expression) {
                self.error_logger.print_error_position(
                    token.pos,
                    "_Alignas requires an integer constant expression",
                );
            }
            let value = self.eval(&expression);
            if value < 0 {
                self.error_logger
                    .print_error_position(token.pos, "_Alignas value cannot be negative");
            }
            value as usize
        };
        self.expect_reserved(")");
        if alignment != 0 && !alignment.is_power_of_two() {
            self.error_logger
                .print_error_position(token.pos, "_Alignas value must be zero or a power of two");
        }
        Some(alignment)
    }

    fn attempt_type(&mut self) -> Option<Type> {
        let saved = self.cur;
        let mut q = self.attempt_qualifiers();
        let Some(ty) = self.attempt_unqualified_type() else {
            self.cur = saved;
            return None;
        };
        let suffix = self.attempt_qualifiers();
        q.constant |= suffix.constant;
        q.volatile |= suffix.volatile;
        q.restricted |= suffix.restricted;
        if q.restricted
            && !matches!(ty.unqualified(), Type::Ptr(pointee) if pointee.function_signature().is_none())
        {
            self.error_logger.print_error_position(
                self.tokens[saved].pos,
                "restrict requires a pointer to an object or void",
            );
        }
        Some(if q == Qualifiers::default() {
            ty
        } else {
            Type::Qualified(Box::new(ty), q)
        })
    }
    fn attempt_unqualified_type(&mut self) -> Option<Type> {
        if let Some(token) = self.attempt_reserved("enum") {
            return Some(self.enum_type(token));
        }
        if let Some(token) = self
            .attempt_reserved("struct")
            .or_else(|| self.attempt_reserved("union"))
        {
            return Some(self.struct_type(token));
        }
        for (name, bits) in [("float", 32), ("double", 64)] {
            if self.attempt_reserved(name).is_some() {
                return Some(Type::Float(bits));
            }
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
            let mut bit_address = 0usize;
            let mut has_bit_fields = false;
            let mut has_flexible_array = false;
            let mut align = 1;
            while self.attempt_reserved("}").is_none() {
                if self.attempt_static_assert() {
                    continue;
                }
                if has_flexible_array && !is_union {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "flexible array member must be last",
                    );
                }
                let mut requested_alignment = 0;
                while let Some(value) = self.attempt_alignment_specifier() {
                    requested_alignment = requested_alignment.max(value);
                }
                let base = self.expect_type();
                while let Some(value) = self.attempt_alignment_specifier() {
                    requested_alignment = requested_alignment.max(value);
                }
                loop {
                    let (name, member_type, _, _, _) = self.declarator().resolve(base.clone());
                    let position = name.as_ref().unwrap_or(&token);
                    self.validate_type_layout(&member_type, position);
                    if member_type.is_variably_modified() {
                        self.error_logger.print_error_position(
                            position.pos,
                            "struct member cannot have a variably modified type",
                        );
                    }
                    let flexible = matches!(member_type.unqualified(), Type::Arr(_, 0));
                    let member_alignment = member_type.align_of().max(requested_alignment);
                    if flexible {
                        if is_union || members.is_empty() || name.is_none() {
                            self.error_logger.print_error_position(position.pos,
                                "flexible array requires a named final member after another struct member");
                        }
                        self.validate_type_layout(&member_type, position);
                        has_flexible_array = true;
                    } else {
                        self.expect_object_type(&member_type, position);
                        if member_type.has_flexible_array() {
                            if !is_union {
                                self.error_logger.print_error_position(
                                    position.pos,
                                    "struct member cannot contain a flexible array member",
                                );
                            }
                            has_flexible_array = true;
                        }
                    }
                    let (field, offset) = if self.attempt_reserved(":").is_some() {
                        has_bit_fields = true;
                        let expression = self.ternary();
                        let width = self.eval(&expression);
                        let Some((bits, _)) = member_type.integer_info() else {
                            self.error_logger.print_error_position(
                                position.pos,
                                "bit-field requires an integer type",
                            );
                            unreachable!()
                        };
                        if width < 0
                            || width > i64::from(bits)
                            || (width == 0 && name.is_some())
                            || (*member_type.unqualified() == Type::Bool && width > 1)
                        {
                            self.error_logger
                                .print_error_position(position.pos, "invalid bit-field width");
                        }
                        let width = width as u8;
                        let container = usize::from(bits);
                        if is_union {
                            bit_address = 0;
                        }
                        if requested_alignment > member_type.align_of() {
                            bit_address =
                                bit_address.div_ceil(member_alignment * 8) * member_alignment * 8;
                            align = align.max(member_alignment);
                        }
                        if width == 0 || usize::from(width) > container - bit_address % container {
                            bit_address = bit_address.div_ceil(container) * container;
                        }
                        // Anonymous fields affect AAPCS64 alignment but not SysV alignment.
                        if name.is_some() || cfg!(target_arch = "aarch64") {
                            align = align.max(member_type.align_of());
                        }
                        let field = BitField {
                            width,
                            shift: (bit_address % container) as u8,
                        };
                        let offset = bit_address / container * member_type.size_of();
                        bit_address += usize::from(width);
                        size = size.max(bit_address.div_ceil(8));
                        (field, offset)
                    } else {
                        if name.is_none() && !matches!(member_type.unqualified(), Type::Struct(_)) {
                            self.error_logger
                                .print_error_position(token.pos, "member name expected");
                        }
                        size = size.div_ceil(member_alignment) * member_alignment;
                        align = align.max(member_alignment);
                        let offset = if is_union { 0 } else { size };
                        size = if is_union {
                            size.max(member_type.size_of())
                        } else {
                            size + member_type.size_of()
                        };
                        bit_address = size * 8;
                        (BitField { width: 0, shift: 0 }, offset)
                    };
                    if name.is_some() || matches!(member_type.unqualified(), Type::Struct(_)) {
                        let anonymous = name.is_none();
                        let visible = if anonymous {
                            let Type::Struct(child) = member_type.unqualified() else {
                                unreachable!()
                            };
                            child
                                .visible_members()
                                .into_iter()
                                .map(|(name, _)| name)
                                .collect::<Vec<_>>()
                        } else {
                            vec![name.as_ref().unwrap().s_value.clone()]
                        };
                        for existing in &members {
                            let existing_names = if existing.anonymous {
                                let Type::Struct(child) = existing.ty.unqualified() else {
                                    unreachable!()
                                };
                                child
                                    .visible_members()
                                    .into_iter()
                                    .map(|(name, _)| name)
                                    .collect::<Vec<_>>()
                            } else {
                                vec![existing.name.clone()]
                            };
                            if visible.iter().any(|name| existing_names.contains(name)) {
                                self.error_logger
                                    .print_error_position(position.pos, "duplicate struct member");
                            }
                        }
                        members.push(StructMember {
                            name: name.map_or_else(
                                || format!(".anonymous.{}", members.len()),
                                |name| name.s_value,
                            ),
                            anonymous,
                            ty: member_type,
                            offset,
                            bit_field: (field.width != 0).then_some(field),
                        });
                    }
                    if self.attempt_reserved(",").is_none() {
                        break;
                    }
                    if flexible {
                        self.error_logger.print_error_position(
                            self.tokens[self.cur].pos,
                            "flexible array member must be last",
                        );
                    }
                }
                self.expect_reserved(";");
            }
            if members.is_empty() && size == 0 {
                self.error_logger
                    .print_error_position(token.pos, "struct requires at least one member");
            }
            *ty.definition().borrow_mut() = StructDefinition {
                members,
                is_union,
                has_bit_fields,
                has_flexible_array,
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
            if self.return_type.is_none() && ty.is_variably_modified() {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified typedef requires block scope",
                );
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

    fn attempt_static_assert(&mut self) -> bool {
        let Some(token) = self.attempt_reserved("_Static_assert") else {
            return false;
        };
        self.expect_reserved("(");
        let expression = self.ternary();
        if !Self::integer_constant(&expression)
            || expression
                .resolve_type()
                .is_none_or(|ty| ty.integer_info().is_none())
        {
            self.error_logger.print_error_position(
                token.pos,
                "static assertion requires an integer constant expression",
            );
        }
        self.expect_reserved(",");
        let message = self.attempt(TokenType::Str).unwrap_or_else(|| {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "static assertion requires a string literal message",
            );
            unreachable!()
        });
        self.expect_reserved(")");
        self.expect_reserved(";");
        if self.eval(&expression) == 0 {
            let bytes = message.string_bytes.clone();
            self.error_logger.print_error_position(
                token.pos,
                &format!(
                    "static assertion failed: {}",
                    String::from_utf8_lossy(&bytes)
                ),
            );
        }
        true
    }

    fn global_definition(&mut self) {
        if self.attempt_static_assert() {
            return;
        }
        self.offset_size = 0;
        self.pending_bounds.clear();
        self.vla_active.clear();
        if let Some(token) = self
            .attempt_reserved("auto")
            .or_else(|| self.attempt_reserved("register"))
        {
            self.error_logger.print_error_position(
                token.pos,
                "automatic storage class requires block or parameter scope",
            );
        }
        let mut storage: Option<Token> = None;
        let mut inline = false;
        let mut qualifiers = Qualifiers::default();
        let mut alignment = 0;
        let mut thread_local = false;
        loop {
            if self.attempt_reserved("_Thread_local").is_some() {
                thread_local = true;
            } else if self.attempt_reserved("inline").is_some() {
                inline = true;
            } else if let Some(value) = self.attempt_alignment_specifier() {
                alignment = alignment.max(value);
            } else if let Some(token) = self
                .attempt_reserved("static")
                .or_else(|| self.attempt_reserved("extern"))
            {
                if storage.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting storage classes");
                } else {
                    storage = Some(token);
                }
            } else {
                let q = self.attempt_qualifiers();
                if q == Qualifiers::default() {
                    break;
                }
                qualifiers.constant |= q.constant;
                qualifiers.volatile |= q.volatile;
                qualifiers.restricted |= q.restricted;
            }
        }
        let internal = storage.as_ref().is_some_and(|s| s.s_value == "static");
        let external = storage.as_ref().is_some_and(|s| s.s_value == "extern");
        if self.attempt_reserved("typedef").is_some() {
            if thread_local {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "_Thread_local cannot be applied to a typedef",
                );
            }
            if alignment != 0 {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "_Alignas cannot be applied to a typedef",
                );
            }
            self.typedef_definition();
            self.expect_reserved(";");
            return;
        }
        if thread_local && storage.is_none() {
            self.error_logger.print_error_position(
                self.tokens[self.cur - 1].pos,
                "_Thread_local requires static or extern storage duration",
            );
        }
        if storage
            .as_ref()
            .is_some_and(|storage| storage.s_value == "static")
            && external
        {
            self.error_logger
                .print_error_position(self.tokens[self.cur - 1].pos, "conflicting storage classes");
        }
        let mut base = self.expect_type();
        while let Some(value) = self.attempt_alignment_specifier() {
            alignment = alignment.max(value);
        }
        if qualifiers != Qualifiers::default() {
            base = Type::Qualified(Box::new(base), qualifiers);
        }
        inline |= self.attempt_reserved("inline").is_some();
        if self.attempt_reserved(";").is_some() {
            if thread_local {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "_Thread_local requires an object declaration",
                );
            }
            if inline {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "inline requires a function declaration",
                );
            }
            return;
        }
        loop {
            let (name, ty, parameters, parameter_scope, stars) =
                self.declarator().resolve(base.clone());
            let token = name.unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "ident expected");
                unreachable!()
            });
            self.validate_type_layout(&ty, &token);
            if ty.is_variably_modified() {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified declaration requires block scope",
                );
            }
            if inline && ty.function_signature().is_none() {
                self.error_logger
                    .print_error_position(token.pos, "inline requires a function declaration");
            }
            if ty.function_signature().is_some() {
                if alignment != 0 {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Alignas cannot be applied to a function",
                    );
                }
                if thread_local {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Thread_local cannot be applied to a function",
                    );
                }
                if self.tokens[self.cur].s_value == "{" && !stars.is_empty() {
                    self.error_logger.print_error_position(
                        stars[0].pos,
                        "unspecified array bound is only allowed in a function prototype",
                    );
                }
                if self.global_variables.contains_key(&token.s_value) {
                    self.error_logger
                        .print_error_position(token.pos, "function conflicts with object");
                }
                self.scope_stack.push(parameter_scope);
                let mut args = vec![];
                for (i, (name, ty, registered)) in parameters.into_iter().enumerate() {
                    let name = name.unwrap_or_else(|| Token {
                        s_value: format!(".parameter.{i}"),
                        ..token.clone()
                    });
                    let mut argument = self.register_local_variable(name, ty);
                    if registered {
                        self.mark_registered(&mut argument);
                    }
                    args.push(argument);
                }
                self.scope_stack
                    .first_mut()
                    .unwrap()
                    .insert(token.s_value.clone(), Identifier::Static(ty.clone()));
                if let Some(existing) = self.functions.get(&token.s_value) {
                    if !existing.cty.compatible(&ty) {
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
                let external_definition = internal
                    || external
                    || !inline
                    || self.functions[&token.s_value].external_definition;
                self.functions
                    .get_mut(&token.s_value)
                    .unwrap()
                    .external_definition = external_definition;
                let (_, result, _) = ty.function_signature().unwrap();
                self.return_pointer = if matches!(result, Type::Struct(_)) {
                    self.offset_size = self.offset_size.div_ceil(8) * 8 + 8;
                    Some(self.offset_size)
                } else {
                    None
                };
                self.variadic_last = args.last().and_then(|arg| arg.offset);
                self.variadic_parameters = ty
                    .function_signature()
                    .unwrap()
                    .2
                    .then(|| ty.function_signature().unwrap().0);
                self.return_type = Some(result);
                self.labels.clear();
                self.defined_labels.clear();
                let mut bounds = self.take_bounds();
                Self::resolve_parameter_bounds(&mut bounds, &args);
                self.mutable_static = None;
                let mut body = self.consume_block();
                if let Some(body) = &mut body {
                    bounds.append(&mut body.children);
                    body.children = bounds;
                    self.resolve_vla_gotos(body);
                }
                self.vla_active.clear();
                for (name, position) in &self.labels {
                    if !self.defined_labels.contains(name) {
                        self.error_logger
                            .print_error_position(*position, "goto target is not defined");
                    }
                }
                self.return_type = None;
                self.variadic_last = args.last().and_then(|arg| arg.offset);
                self.variadic_parameters = None;
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
                        inline_definition: inline && body.is_some(),
                        external_definition,
                        mutable_static: self.mutable_static.take(),
                        body,
                        cty: ty,
                        token: Some(token),
                        args,
                        offset_size: self.offset_size,
                        internal,
                        return_pointer: self.return_pointer,
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
                existing.alignment = existing.alignment.max(alignment);
                if existing.thread_local != thread_local {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting thread storage duration");
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
                        alignment,
                        thread_local,
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
        let ty = ty.unqualified();
        match (ty, data) {
            (_, GlobalVariableData::Zero) => {}
            (Type::Struct(ty), GlobalVariableData::Union { index, value }) => {
                self.validate_global_initializer(&ty.members()[*index].ty, value, token);
            }
            (Type::Struct(ty), GlobalVariableData::Arr(elements)) => {
                let members = ty.members();
                let capacity = if ty.definition().borrow().is_union {
                    1
                } else {
                    members
                        .iter()
                        .take_while(|member| !matches!(member.ty.unqualified(), Type::Arr(_, 0)))
                        .count()
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
            (ty, GlobalVariableData::Address { .. }) if ty.is_arithmetic() => {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be an address");
            }
            (ty, GlobalVariableData::Elem(value))
                if ty.is_arithmetic() && value.starts_with('@') =>
            {
                self.error_logger
                    .print_error_position(token.pos, "integer initializer cannot be a string");
            }
            _ => {}
        }
    }

    fn aggregate_type(ty: &Type, index: usize) -> Option<Type> {
        match ty.unqualified() {
            Type::Arr(child, size) if *size == 0 || index < *size => Some(*child.clone()),
            Type::Struct(st) => st
                .members()
                .get(index)
                .filter(|member| !matches!(member.ty.unqualified(), Type::Arr(_, 0)))
                .map(|member| member.ty.clone()),
            _ => None,
        }
    }
    fn designation(&mut self, ty: &Type) -> Vec<usize> {
        let mut current = ty.clone();
        let mut path = vec![];
        loop {
            let indices = if let Some(token) = self.attempt_reserved("[") {
                let expression = self.ternary();
                let index = self.eval(&expression);
                self.expect_reserved("]");
                if index < 0 || !matches!(current.unqualified(), Type::Arr(..)) {
                    self.error_logger
                        .print_error_position(token.pos, "invalid array designator");
                }
                vec![index as usize]
            } else if let Some(token) = self.attempt_reserved(".") {
                let name = self.attempt_ident().unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(token.pos, "member designator expected");
                    unreachable!()
                });
                let Type::Struct(st) = current.unqualified() else {
                    self.error_logger
                        .print_error_position(token.pos, "member designator requires an aggregate");
                    unreachable!()
                };
                st.member_path(&name.s_value).unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(name.pos, "unknown designated member");
                    unreachable!()
                })
            } else {
                break;
            };
            for index in indices {
                current = Self::aggregate_type(&current, index).unwrap_or_else(|| {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "designator is out of bounds",
                    );
                    unreachable!()
                });
                path.push(index);
            }
        }
        if !path.is_empty() {
            self.expect_reserved("=");
        }
        path
    }
    fn set_global_element(
        ty: &Type,
        data: &mut GlobalVariableData,
        path: &[usize],
        value: GlobalVariableData,
    ) {
        if path.is_empty() {
            *data = value;
            return;
        }
        let child = Self::aggregate_type(ty, path[0]).unwrap();
        if matches!(ty.unqualified(), Type::Struct(st) if st.definition().borrow().is_union) {
            if !matches!(data, GlobalVariableData::Union { index, .. } if *index == path[0]) {
                *data = GlobalVariableData::Union {
                    index: path[0],
                    value: Box::new(GlobalVariableData::Zero),
                };
            }
            let GlobalVariableData::Union {
                value: selected, ..
            } = data
            else {
                unreachable!()
            };
            Self::set_global_element(&child, selected, &path[1..], value);
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
        Self::set_global_element(&child, &mut elements[path[0]], &path[1..], value);
    }
    fn global_initializer(&mut self, ty: &Type) -> GlobalVariableData {
        if matches!(ty.unqualified(), Type::Arr(child, _) if child.unqualified().size_of() == self.tokens[self.cur].string_encoding.unit_size())
            && self.tokens[self.cur].tt == TokenType::Str
        {
            let token = self.attempt(TokenType::Str).unwrap();
            let mut bytes = token.string_bytes.clone();
            let unit_size = token.string_encoding.unit_size();
            let Type::Arr(_, size) = ty.unqualified() else {
                unreachable!()
            };
            if *size != 0 && bytes.len() / unit_size > *size {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            if *size == 0 || bytes.len() / unit_size < *size {
                bytes.extend(vec![0; unit_size]);
            }
            return GlobalVariableData::Arr(
                bytes
                    .chunks(unit_size)
                    .map(|unit| {
                        GlobalVariableData::Elem(
                            unit.iter()
                                .enumerate()
                                .map(|(index, byte)| usize::from(*byte) << (index * 8))
                                .sum::<usize>()
                                .to_string(),
                        )
                    })
                    .collect(),
            );
        }
        if matches!(ty.unqualified(), Type::Arr(..) | Type::Struct(_))
            && self.attempt_reserved("{").is_some()
        {
            let mut data = GlobalVariableData::Arr(vec![]);
            let mut cursor = 0;
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(ty);
                    if path.is_empty() {
                        if cursor > 0
                            && matches!(ty.unqualified(), Type::Struct(st) if st.definition().borrow().is_union)
                        {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                "too many union initializer elements",
                            );
                        }
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
                    Self::set_global_element(ty, &mut data, &path, value);
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
        if self.attempt_reserved("{").is_some() {
            let value = self.global_initializer(ty);
            self.attempt_reserved(",");
            self.expect_reserved("}");
            return GlobalVariableData::Arr(vec![value]);
        }
        let expression = self.ternary();
        if let Some((symbol, offset)) = self.constant_address(&expression) {
            return GlobalVariableData::Address { symbol, offset };
        }
        let value = if ty.float_bits().is_some() {
            ty.float_constant(self.eval_float(&expression)) as i64
        } else if expression
            .resolve_type()
            .is_some_and(|ty| ty.float_bits().is_some())
        {
            self.eval_float(&expression) as i64
        } else {
            self.eval(&expression)
        };
        GlobalVariableData::Elem(value.to_string())
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

    fn eval_float_as(&mut self, node: &Node, ty: &Type) -> f64 {
        let value = self.eval_float(node);
        ty.float_value(ty.float_constant(value))
    }
    fn eval_float(&mut self, node: &Node) -> f64 {
        let ty = node.resolve_type().unwrap();
        if ty.integer_info().is_some() {
            return if ty.integer_info().unwrap().1 {
                self.eval(node) as f64
            } else {
                self.eval(node) as u64 as f64
            };
        }
        let value = match node.nt {
            NodeType::Num => ty.float_value(node.value.unwrap()),
            NodeType::Cast => self.eval_float(node.lhs.as_ref().unwrap()),
            NodeType::Add => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    + self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Sub => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    - self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Mul => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    * self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Div => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    / self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Conditional => {
                if self.eval_float(node.cond.as_ref().unwrap()) != 0.0 {
                    self.eval_float(node.then.as_ref().unwrap())
                } else {
                    self.eval_float(node.els.as_ref().unwrap())
                }
            }
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "floating initializer is not constant",
                );
                unreachable!()
            }
        };
        ty.float_value(ty.float_constant(value))
    }
    fn eval(&mut self, node: &Node) -> i64 {
        if node
            .resolve_type()
            .is_some_and(|ty| ty.float_bits().is_some())
        {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "integer constant expression required",
            );
        }
        if matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        ) && node
            .lhs
            .as_ref()
            .is_some_and(|lhs| lhs.resolve_type().unwrap().float_bits().is_some())
        {
            let lhs = self.eval_float(node.lhs.as_ref().unwrap());
            let rhs = self.eval_float(node.rhs.as_ref().unwrap());
            return i64::from(match node.nt {
                NodeType::Eq => lhs == rhs,
                NodeType::Ne => lhs != rhs,
                NodeType::Lt => lhs < rhs,
                NodeType::Le => lhs <= rhs,
                _ => unreachable!(),
            });
        }
        match node.nt {
            NodeType::Num => return node.value.unwrap() as i64,
            NodeType::Cast => {
                let child = node.lhs.as_ref().unwrap();
                let value = if child
                    .resolve_type()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    self.eval_float(child) as i64
                } else {
                    self.eval(child)
                };
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
    fn erase_qualifiers(node: &mut Node) {
        node.volatile_access = node
            .object_type()
            .is_some_and(|ty| ty.qualifiers().volatile);
        if let Some(ty) = &mut node.cty {
            *ty = ty.without_qualifiers();
        }
        node.parameter_types = node
            .parameter_types
            .iter()
            .map(Type::without_qualifiers)
            .collect();
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
            Self::erase_qualifiers(child);
        }
        for child in node.children.iter_mut().chain(&mut node.args) {
            Self::erase_qualifiers(child);
        }
    }

    fn cast_integer(node: &mut Node, target: &Type) {
        if target.is_arithmetic() && node.resolve_type().as_ref() != Some(target) {
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
        if matches!(
            node.nt,
            NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le
        ) {
            let lhs = node.lhs.as_ref().unwrap().resolve_type().unwrap();
            let rhs = node.rhs.as_ref().unwrap().resolve_type().unwrap();
            if lhs.float_bits().is_some() || rhs.float_bits().is_some() {
                let common = Type::arithmetic_type(&lhs, &rhs);
                Self::cast_integer(node.lhs.as_mut().unwrap(), &common);
                Self::cast_integer(node.rhs.as_mut().unwrap(), &common);
            }
        }
        if matches!(
            node.nt,
            NodeType::If
                | NodeType::Conditional
                | NodeType::While
                | NodeType::DoWhile
                | NodeType::For
        ) {
            if let Some(condition) = &mut node.cond {
                if condition
                    .resolve_type()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    Self::cast_integer(condition, &Type::Bool);
                }
            }
        }
        if matches!(node.nt, NodeType::LogicalAnd | NodeType::LogicalOr) {
            for operand in [&mut node.lhs, &mut node.rhs].into_iter().flatten() {
                if operand
                    .resolve_type()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    Self::cast_integer(operand, &Type::Bool);
                }
            }
        }
        if node.nt == NodeType::Conditional
            && node
                .resolve_type()
                .is_some_and(|ty| ty.float_bits().is_some())
        {
            let common = node.resolve_type().unwrap();
            Self::cast_integer(node.then.as_mut().unwrap(), &common);
            Self::cast_integer(node.els.as_mut().unwrap(), &common);
        }
        match node.nt {
            NodeType::Assign => {
                let target = node.lhs.as_ref().unwrap().resolve_type().unwrap();
                Self::cast_integer(node.rhs.as_mut().unwrap(), &target);
            }
            NodeType::CallFunc => {
                for (index, arg) in node.args.iter_mut().rev().enumerate() {
                    let target = node.parameter_types.get(index).cloned().unwrap_or_else(|| {
                        let ty = arg.resolve_type().unwrap().promoted();
                        if ty == Type::Float(32) {
                            Type::Float(64)
                        } else {
                            ty
                        }
                    });
                    Self::cast_integer(arg, &target);
                }
            }
            _ => {}
        }
    }

    fn expect_conversion(&self, expected: &Type, operand: &Node) {
        let actual = operand.resolve_type();
        let compatible = match (expected.unqualified(), actual.as_ref()) {
            (target, Some(source)) if target.is_arithmetic() && source.is_arithmetic() => true,
            (Type::Bool, Some(Type::Ptr(_) | Type::Arr(..) | Type::Vla(..))) => true,
            (
                Type::Ptr(target),
                Some(Type::Ptr(source) | Type::Arr(source, _) | Type::Vla(source, _)),
            ) => {
                (!source.qualifiers().constant || target.qualifiers().constant)
                    && (!source.qualifiers().volatile || target.qualifiers().volatile)
                    && (target.compatible(source)
                        || *target.unqualified() == Type::Void
                        || *source.unqualified() == Type::Void)
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

    fn validate_inline_body(&self, node: &Node) {
        let internal_object = self
            .global_variables
            .get(&node.global_name)
            .is_some_and(|object| {
                object.internal
                    && !(node.global_name.starts_with(".static.")
                        && object.ty.qualifiers().constant)
            });
        let internal_function = self
            .functions
            .get(&node.global_name)
            .is_some_and(|function| function.internal);
        if internal_object || internal_function {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "external inline definition cannot refer to an internal identifier",
            );
        }
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            self.validate_inline_body(child);
        }
        for child in node.children.iter().chain(&node.args) {
            self.validate_inline_body(child);
        }
    }

    fn validate_void_expressions(&self, node: &Node) {
        if matches!(
            node.nt,
            NodeType::Mod
                | NodeType::BitAnd
                | NodeType::BitOr
                | NodeType::BitXor
                | NodeType::BitLeft
                | NodeType::BitRight
                | NodeType::BitNot
        ) {
            for operand in [&node.lhs, &node.rhs].into_iter().flatten() {
                if operand
                    .resolve_type()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    self.error_logger.print_error_position(
                        node.token.as_ref().unwrap().pos,
                        "operation requires integer operands",
                    );
                }
            }
        }

        if matches!(
            node.nt,
            NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le
        ) {
            let lhs = node.lhs.as_ref().unwrap().resolve_type().unwrap();
            let rhs = node.rhs.as_ref().unwrap().resolve_type().unwrap();
            if (lhs.float_bits().is_some() || rhs.float_bits().is_some())
                && !(lhs.is_arithmetic() && rhs.is_arithmetic())
            {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "floating operation requires arithmetic operands",
                );
            }
        }
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
        if self.attempt_static_assert() {
            return Node {
                nt: NodeType::Block,
                ..Node::default()
            };
        }
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
                vla_scope: self.vla_active.clone(),
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
                args: self.dynamic_objects.clone(),
                vla_scope: self.vla_active.clone(),
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
                children: self.take_bounds(),
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
            self.switch_vla_depths.push(self.vla_active.len());
            self.jump_depths.push(self.dynamic_objects.len());
            self.switch_stack.push(vec![]);
            self.switch_types
                .push(cond.resolve_type().unwrap().promoted());
            let body = self.stmt();
            let cases = self.switch_stack.pop().unwrap();
            self.switch_types.pop();
            self.switch_vla_depths.pop();
            self.jump_depths.pop();
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
            if self.vla_active.len() > *self.switch_vla_depths.last().unwrap() {
                self.error_logger.print_error_position(
                    t.pos,
                    "switch label enters the scope of a variably modified declaration",
                );
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
            self.continue_depths
                .insert(t.pos, self.dynamic_objects.len());
            self.jump_depths.push(self.dynamic_objects.len());
            let body = self.stmt();
            self.loop_stack.pop();
            self.jump_depths.pop();
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
            self.continue_depths
                .insert(t.pos, self.dynamic_objects.len());
            self.jump_depths.push(self.dynamic_objects.len());
            let body = self.stmt();
            self.loop_stack.pop();
            self.jump_depths.pop();
            return Node::new_while_node(Some(t), cond, body);
        }
        if let Some(t) = self.attempt_reserved("for") {
            let object_depth = self.dynamic_objects.len();
            let bound_depth = self.vla_active.len();
            self.scope_stack.push(HashMap::new());
            self.expect_reserved("(");
            let mut ini: Option<Node> = None;
            let mut cond: Option<Node> = None;
            let mut upd: Option<Node> = None;
            if self.attempt_reserved(";").is_none() {
                ini = Some(
                    if let Some((ty, storage, alignment)) = self.attempt_automatic_type(false) {
                        self.local_variable_definition_with_storage(ty, storage, alignment)
                    } else {
                        self.expr()
                    },
                );
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
            self.continue_depths
                .insert(t.pos, self.dynamic_objects.len());
            self.jump_depths.push(self.dynamic_objects.len());
            let body = self.stmt();
            self.loop_stack.pop();
            self.jump_depths.pop();
            self.scope_stack.pop();
            let loop_node = Node::new_for_node(Some(t), ini, cond, upd, body);
            let mut children = vec![loop_node];
            children.extend(self.cleanup_arrays(object_depth));
            self.dynamic_objects.truncate(object_depth);
            self.vla_active.truncate(bound_depth);
            return Node {
                nt: NodeType::Block,
                children,
                ..Node::default()
            };
        }
        if let Some(node) = self.consume_block() {
            return node;
        }
        let mut requested_alignment = 0;
        let mut thread_local = false;
        while let Some(value) = self.attempt_alignment_specifier() {
            requested_alignment = requested_alignment.max(value);
        }
        let inline = self.attempt_reserved("inline");
        let storage = self
            .attempt_reserved("static")
            .or_else(|| self.attempt_reserved("extern"));
        let inline = inline.or_else(|| self.attempt_reserved("inline"));
        let mut storage = storage;
        loop {
            if self.attempt_reserved("_Thread_local").is_some() {
                thread_local = true;
            } else if let Some(token) = self
                .attempt_reserved("static")
                .or_else(|| self.attempt_reserved("extern"))
            {
                if storage.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting storage classes");
                } else {
                    storage = Some(token);
                }
            } else {
                break;
            }
        }
        if storage.is_some() || inline.is_some() || thread_local {
            let mut storage = storage.unwrap_or_else(|| {
                inline.clone().unwrap_or_else(|| Token {
                    s_value: "static".to_owned(),
                    ..self.tokens[self.cur - 1].clone()
                })
            });
            if storage.s_value == "inline" {
                storage.s_value = "extern".to_owned();
            }
            while let Some(value) = self.attempt_alignment_specifier() {
                requested_alignment = requested_alignment.max(value);
            }
            let ty = self.expect_type();
            while let Some(value) = self.attempt_alignment_specifier() {
                requested_alignment = requested_alignment.max(value);
            }
            let declaration = self.local_static_definition(
                ty,
                &storage,
                inline.is_some(),
                requested_alignment,
                thread_local,
            );
            self.expect_reserved(";");
            return declaration;
        }
        let node = if let Some((ty, storage, alignment)) = self.attempt_automatic_type(false) {
            if self.tokens[self.cur].s_value == ";" {
                Node {
                    nt: NodeType::Block,
                    ..Node::default()
                }
            } else {
                self.local_variable_definition_with_storage(ty, storage, alignment)
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
                offset: self.return_pointer,
                lhs: operand.map(Box::new),
                ..Node::default()
            }
        } else {
            self.expr()
        };
        self.expect_reserved(";");
        self.cleanup_jump(node)
    }

    fn local_static_definition(
        &mut self,
        base: Type,
        storage: &Token,
        inline: bool,
        requested_alignment: usize,
        thread_local: bool,
    ) -> Node {
        let mut bounds = vec![];
        loop {
            let (token, mut ty) = self.expect_ident_with_type(base.clone());
            if ty.is_variable_array() || (storage.s_value == "extern" && ty.is_variably_modified())
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified static or extern declaration is invalid",
                );
            }
            bounds.extend(self.take_bounds());
            let external = storage.s_value == "extern";
            let name = if external {
                token.s_value.clone()
            } else {
                format!(".static.{}.{}", token.pos, token.s_value)
            };
            if ty.function_signature().is_some() {
                if thread_local {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Thread_local cannot be applied to a function",
                    );
                }
                if requested_alignment != 0 {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Alignas cannot be applied to a function",
                    );
                }
                if !external {
                    self.error_logger.print_error_position(
                        token.pos,
                        "block static function declaration is invalid",
                    );
                }
                if let Some(existing) = self.functions.get(&name) {
                    if !existing.cty.compatible(&ty) {
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
            if inline {
                self.error_logger
                    .print_error_position(token.pos, "inline requires a function declaration");
            }
            if !external && !ty.qualifiers().constant && self.mutable_static.is_none() {
                self.mutable_static = Some(token.clone());
            }
            let data = if self.attempt_reserved("=").is_some() {
                if external {
                    self.error_logger
                        .print_error_position(token.pos, "block extern cannot have an initializer");
                }
                if matches!(&ty, Type::Arr(child, _) if child.unqualified().size_of() == self.tokens[self.cur].string_encoding.unit_size())
                    && self.tokens[self.cur].tt == TokenType::Str
                {
                    let string = self.attempt(TokenType::Str).unwrap();
                    let mut bytes = string.string_bytes.clone();
                    let Type::Arr(_, size) = &ty else {
                        unreachable!()
                    };
                    let unit_size = string.string_encoding.unit_size();
                    if *size != 0 && bytes.len() / unit_size > *size {
                        self.error_logger
                            .print_error_position(string.pos, "string initializer is too long");
                    }
                    if *size == 0 || bytes.len() / unit_size < *size {
                        bytes.extend(vec![0; unit_size]);
                    }
                    Some(GlobalVariableData::Arr(
                        bytes
                            .chunks(unit_size)
                            .map(|unit| {
                                GlobalVariableData::Elem(
                                    unit.iter()
                                        .enumerate()
                                        .map(|(index, byte)| usize::from(*byte) << (index * 8))
                                        .sum::<usize>()
                                        .to_string(),
                                )
                            })
                            .collect(),
                    ))
                } else {
                    Some(self.global_initializer(&ty))
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
                if existing.thread_local != thread_local {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting thread storage duration");
                }
            } else {
                self.global_variables.insert(
                    name,
                    GlobalVariable {
                        ty,
                        alignment: requested_alignment,
                        thread_local,
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
            children: bounds,
            ..Node::default()
        }
    }

    pub fn local_variable_definition(&mut self, ty: Type) -> Node {
        self.local_variable_definition_with_storage(ty, AutomaticStorage::Default, 0)
    }

    fn local_variable_definition_with_storage(
        &mut self,
        ty: Type,
        storage: AutomaticStorage,
        alignment: usize,
    ) -> Node {
        let mut vec = Vec::new();
        loop {
            let mut node = self.new_local_variable(ty.clone(), alignment);
            if storage != AutomaticStorage::Default
                && node.resolve_type().unwrap().function_signature().is_some()
            {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "automatic storage class requires an object declaration",
                );
            }
            if storage == AutomaticStorage::Register {
                self.mark_registered(&mut node);
            }
            vec.extend(self.take_bounds());
            if node.resolve_type().unwrap().is_variable_array() {
                if self.tokens[self.cur].s_value == "=" {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "variable length array cannot have an initializer",
                    );
                }
                vec.push(self.allocate_variable_array(&node));
            }
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
        let registered = node.registered;
        let qualifiers = node
            .object_type()
            .map_or(Qualifiers::default(), |ty| ty.qualifiers());
        let mut member = member.clone();
        if qualifiers != Qualifiers::default() {
            member.ty = Type::Qualified(Box::new(member.ty), qualifiers);
        }
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
        let mut result = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, pointer);
        result.bit_field = member.bit_field;
        result.registered = registered;
        result
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
                    .filter(|member| !matches!(member.ty.unqualified(), Type::Arr(_, 0)))
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
        if matches!(node.resolve_type(), Some(Type::Struct(st)) if st.definition().borrow().is_union)
        {
            let address =
                Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, node.clone());
            let bytes = Node {
                nt: NodeType::Cast,
                token: Some(token.clone()),
                cty: Some(Type::Ptr(Box::new(Type::I8))),
                lhs: Some(Box::new(address)),
                ..Node::default()
            };
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: (0..node.resolve_type().unwrap().size_of())
                    .map(|index| {
                        Node::new_with_op(
                            Some(token.clone()),
                            NodeType::Assign,
                            Self::array_element(&bytes, index, token),
                            Node::new_with_num(Some(token.clone()), 0),
                        )
                    })
                    .collect(),
                ..Node::default()
            };
        }
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
        if matches!(node.resolve_type(), Some(Type::Arr(child, _)) if child.unqualified().size_of() == self.tokens[self.cur].string_encoding.unit_size())
            && self.tokens[self.cur].tt == TokenType::Str
        {
            let token = self.attempt(TokenType::Str).unwrap();
            let bytes = token.string_bytes.clone();
            let unit_size = token.string_encoding.unit_size();
            let Type::Arr(_, size) = node.resolve_type().unwrap() else {
                unreachable!()
            };
            if bytes.len() / unit_size > size {
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
                                (0..unit_size)
                                    .map(|byte| {
                                        usize::from(
                                            bytes
                                                .get(index * unit_size + byte)
                                                .copied()
                                                .unwrap_or(0),
                                        ) << (byte * 8)
                                    })
                                    .sum(),
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
            let mut selected_unions = HashMap::new();
            let mut cursor = 0;
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(&node.resolve_type().unwrap());
                    if path.is_empty() {
                        if cursor > 0
                            && matches!(node.resolve_type(), Some(Type::Struct(st)) if st.definition().borrow().is_union)
                        {
                            self.error_logger.print_error_position(
                                token.pos,
                                "too many union initializer elements",
                            );
                        }
                        path.push(cursor);
                    }
                    cursor = path[0] + 1;
                    let mut target = node.clone();
                    let mut prefix = vec![];
                    for index in path {
                        let union = matches!(target.resolve_type(), Some(Type::Struct(st)) if st.definition().borrow().is_union);
                        let targets = Self::aggregate_elements(&target, &token).unwrap();
                        target = targets.get(index).cloned().unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                token.pos,
                                "too many aggregate initializer elements",
                            );
                            unreachable!()
                        });
                        if union && selected_unions.insert(prefix.clone(), index) != Some(index) {
                            children.push(Self::zero_initializer(&target, assign_token));
                        }
                        prefix.push(index);
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
                cty: Some(Type::Ptr(Box::new(operand.object_type().unwrap()))),
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
                    let qualifiers = address.dest_type().unwrap().qualifiers();
                    let byte_type = if qualifiers == Qualifiers::default() {
                        Type::I8
                    } else {
                        Type::Qualified(Box::new(Type::I8), qualifiers)
                    };
                    let pointer = Node {
                        nt: NodeType::Cast,
                        token: Some(token.clone()),
                        cty: Some(Type::Ptr(Box::new(byte_type))),
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
            let object_depth = self.dynamic_objects.len();
            let bound_depth = self.vla_active.len();
            let mut children: Vec<Node> = Vec::new();
            while self.attempt_reserved("}").is_none() {
                children.push(self.stmt());
            }
            children.extend(self.cleanup_arrays(object_depth));
            self.dynamic_objects.truncate(object_depth);
            self.vla_active.truncate(bound_depth);
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
        if node
            .object_type()
            .is_some_and(|ty| ty.qualifiers().constant)
        {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "cannot modify a const object",
            );
        }
        if !(node.compound_literal
            || matches!(
                node.nt,
                NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref
            ))
            || matches!(
                node.resolve_type(),
                Some(Type::Arr(..) | Type::Vla(..) | Type::Func(..) | Type::VariadicFunc(..))
                    | None
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
        let field = lhs.bit_field;
        let ty = Type::Ptr(Box::new(lhs.object_type().unwrap()));
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
        let mut target = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, temporary);
        target.bit_field = field;
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

    fn offsetof_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let base = self.expect_type();
        let (name, mut ty, _, _, _) = self.declarator().resolve(base);
        if name.is_some() || ty.is_variably_modified() {
            self.error_logger
                .print_error_position(token.pos, "offsetof requires a fixed type name");
        }
        self.expect_reserved(",");
        let mut offset = 0;
        loop {
            let member_name = self.attempt_ident().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "offsetof member expected");
                unreachable!()
            });
            let Type::Struct(st) = ty.unqualified() else {
                self.error_logger
                    .print_error_position(member_name.pos, "offsetof member requires an aggregate");
                unreachable!()
            };
            let path = st.member_path(&member_name.s_value).unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(member_name.pos, "unknown offsetof member");
                unreachable!()
            });
            for index in path {
                let Type::Struct(st) = ty.unqualified() else {
                    unreachable!()
                };
                let member = st.members()[index].clone();
                if member.bit_field.is_some() {
                    self.error_logger.print_error_position(
                        member_name.pos,
                        "offsetof cannot be applied to a bit-field",
                    );
                }
                offset += member.offset;
                ty = member.ty;
            }
            while let Some(bracket) = self.attempt_reserved("[") {
                let index = self.ternary();
                if !Self::integer_constant(&index) {
                    self.error_logger.print_error_position(
                        bracket.pos,
                        "offsetof index requires an integer constant",
                    );
                }
                let index = self.eval(&index);
                self.expect_reserved("]");
                let Type::Arr(child, _) = ty.unqualified() else {
                    self.error_logger
                        .print_error_position(bracket.pos, "offsetof subscript requires an array");
                    unreachable!()
                };
                if index < 0 {
                    self.error_logger
                        .print_error_position(bracket.pos, "offsetof index cannot be negative");
                }
                offset = (index as usize)
                    .checked_mul(child.size_of())
                    .and_then(|stride| offset.checked_add(stride))
                    .unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(bracket.pos, "offsetof result is too large");
                        unreachable!()
                    });
                ty = *child.clone();
            }
            if self.attempt_reserved(".").is_none() {
                break;
            }
        }
        self.expect_reserved(")");
        Node {
            token: Some(token),
            value: Some(offset),
            cty: Some(Type::integer(64, false)),
            ..Node::default()
        }
    }

    fn sizeof_expression(&mut self, token: &Token) -> Node {
        let pending_start = self.pending_bounds.len();
        let expression = self.unary();
        if expression.bit_field.is_some() {
            self.error_logger
                .print_error_position(token.pos, "sizeof cannot be applied to a bit-field");
        }
        let ty = expression.resolve_type().unwrap();
        if ty.size_of() == 0
            || matches!(
                ty.unqualified(),
                Type::Void | Type::Func(..) | Type::VariadicFunc(..)
            )
        {
            self.error_logger
                .print_error_position(token.pos, "sizeof requires a complete object type");
        }
        if ty.is_variable_array() {
            let mut children = self.pending_bounds.split_off(pending_start);
            children.push(expression);
            children.push(Self::runtime_size(&ty));
            Node {
                nt: NodeType::Sequence,
                children,
                token: Some(token.clone()),
                ..Node::default()
            }
        } else {
            Node {
                token: Some(token.clone()),
                value: Some(ty.size_of()),
                cty: Some(Type::integer(64, false)),
                ..Node::default()
            }
        }
    }
    fn unary(&mut self) -> Node {
        let saved = self.cur;
        if let Some(token) = self.attempt_reserved("(") {
            if let Some(base) = self.attempt_type() {
                let pending_start = self.pending_bounds.len();
                let (name, mut ty, _, _, _) = self.declarator().resolve(base);
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
                                alignment: 0,
                                thread_local: false,
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
                let source = operand.resolve_type().unwrap();
                if (ty.float_bits().is_some() && source.dest_type().is_some())
                    || (source.float_bits().is_some() && matches!(ty.unqualified(), Type::Ptr(_)))
                {
                    self.error_logger
                        .print_error_position(token.pos, "invalid floating pointer cast");
                }
                if !(ty.is_arithmetic() || matches!(ty, Type::Void | Type::Ptr(_)))
                    || (!matches!(
                        operand.resolve_type(),
                        Some(
                            Type::I8
                                | Type::I32
                                | Type::Integer { .. }
                                | Type::Bool
                                | Type::Float(_)
                                | Type::Ptr(_)
                                | Type::Arr(..)
                                | Type::Vla(..)
                                | Type::Func(..)
                                | Type::VariadicFunc(..)
                        )
                    ) && ty != Type::Void)
                {
                    self.error_logger
                        .print_error_position(token.pos, "invalid cast");
                }
                let result = Node {
                    nt: NodeType::Cast,
                    token: Some(token.clone()),
                    cty: Some(ty),
                    lhs: Some(Box::new(operand)),
                    ..Node::default()
                };
                let mut bounds = self.pending_bounds.split_off(pending_start);
                if bounds.is_empty() {
                    return result;
                }
                bounds.push(result);
                return Node {
                    nt: NodeType::Sequence,
                    token: Some(token),
                    children: bounds,
                    ..Node::default()
                };
            }
            self.cur = saved;
        }

        if let Some(t) = self.attempt_reserved("sizeof") {
            let saved = self.cur;
            if self.attempt_reserved("(").is_some() {
                if let Some(base) = self.attempt_type() {
                    let pending_start = self.pending_bounds.len();
                    let (name, ty, _, _, _) = self.declarator().resolve(base);
                    self.validate_type_layout(&ty, &t);
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
                    let mut children = self.pending_bounds.split_off(pending_start);
                    children.push(Self::runtime_size(&ty));
                    return if children.len() == 1 {
                        children.pop().unwrap()
                    } else {
                        Node {
                            nt: NodeType::Sequence,
                            token: Some(t),
                            children,
                            ..Node::default()
                        }
                    };
                }
                self.cur = saved;
            }
            return self.sizeof_expression(&t);
        }
        if let Some(token) = self.attempt_reserved("_Alignof") {
            let pending_start = self.pending_bounds.len();
            self.expect_reserved("(");
            let base = self.expect_type();
            let (name, ty, _, _, _) = self.declarator().resolve(base);
            self.pending_bounds.truncate(pending_start);
            self.validate_type_layout(&ty, &token);
            if name.is_some()
                || matches!(
                    ty.unqualified(),
                    Type::Void | Type::Func(..) | Type::VariadicFunc(..)
                )
                || ty.size_of() == 0
            {
                self.error_logger
                    .print_error_position(token.pos, "_Alignof requires a complete object type");
            }
            self.expect_reserved(")");
            return Node {
                token: Some(token),
                value: Some(ty.align_of()),
                cty: Some(Type::integer(64, false)),
                ..Node::default()
            };
        }
        if self.attempt_reserved("+").is_some() {
            return self.unary();
        } else if let Some(t) = self.attempt_reserved("-") {
            let operand = self.unary();
            if let Some(ty) = operand
                .resolve_type()
                .filter(|ty| ty.float_bits().is_some())
            {
                let minus_one = Node {
                    cty: Some(ty.clone()),
                    value: Some(ty.float_constant(-1.0)),
                    token: Some(t.clone()),
                    ..Node::default()
                };
                return Node::new_with_op(Some(t), NodeType::Mul, operand, minus_one);
            }
            return Node::new_with_op(
                Some(t.clone()),
                NodeType::Sub,
                Node::new_with_num(Some(t), 0),
                operand,
            );
        }
        if let Some(token) = self.attempt_reserved("&") {
            let operand = self.unary();
            if operand.registered {
                self.error_logger.print_error_position(
                    token.pos,
                    "cannot take the address of a register object",
                );
            }
            if operand.bit_field.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "cannot take the address of a bit-field");
            }
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
    fn generic_selection(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let bounds_start = self.pending_bounds.len();
        let control = self.assign();
        self.validate_void_expressions(&control);
        self.pending_bounds.truncate(bounds_start);
        let control_type = match control.resolve_type().unwrap() {
            Type::Arr(child, _) | Type::Vla(child, _) => Type::Ptr(child),
            ty @ (Type::Func(..) | Type::VariadicFunc(..)) => Type::Ptr(Box::new(ty)),
            ty => ty,
        };
        self.expect_reserved(",");
        let mut types: Vec<Type> = vec![];
        let mut selected = None;
        let mut fallback = None;
        loop {
            let association = self.tokens[self.cur].clone();
            let type_bounds_start = self.pending_bounds.len();
            let ty = if self.attempt_reserved("default").is_some() {
                if fallback.is_some() {
                    self.error_logger.print_error_position(
                        association.pos,
                        "duplicate generic default association",
                    );
                }
                None
            } else {
                let base = self.expect_type();
                let (name, ty, _, _, _) = self.declarator().resolve(base);
                self.validate_type_layout(&ty, &association);
                if name.is_some()
                    || ty.is_variably_modified()
                    || ty.size_of() == 0
                    || matches!(
                        ty.unqualified(),
                        Type::Void | Type::Func(..) | Type::VariadicFunc(..)
                    )
                {
                    self.error_logger.print_error_position(
                        association.pos,
                        "generic association requires a complete non-variably-modified object type",
                    );
                }
                if types.iter().any(|existing| {
                    existing.qualifiers() == ty.qualifiers() && existing.compatible(&ty)
                }) {
                    self.error_logger.print_error_position(
                        association.pos,
                        "duplicate compatible generic association type",
                    );
                }
                types.push(ty.clone());
                Some(ty)
            };
            // Prototype parameter bounds in association types are unevaluated.
            self.pending_bounds.truncate(type_bounds_start);
            self.expect_reserved(":");
            let bounds_start = self.pending_bounds.len();
            let expression = self.assign();
            self.validate_void_expressions(&expression);
            let bounds = self.pending_bounds.split_off(bounds_start);
            if let Some(ty) = ty {
                if control_type.qualifiers() == ty.qualifiers() && control_type.compatible(&ty) {
                    selected = Some((expression, bounds));
                }
            } else {
                fallback = Some((expression, bounds));
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        self.expect_reserved(")");
        let (expression, bounds) = selected.or(fallback).unwrap_or_else(|| {
            self.error_logger
                .print_error_position(token.pos, "generic selection has no compatible association");
            unreachable!()
        });
        self.pending_bounds.extend(bounds);
        expression
    }

    fn prim(&mut self) -> Node {
        let node = if let Some(token) = self.attempt_reserved("_Generic") {
            self.generic_selection(token)
        } else if self.attempt_reserved("(").is_some() {
            let node = self.expr();
            self.expect_reserved(")");
            node
        } else if let Some(t) = self.attempt(TokenType::Str) {
            // String literal
            let element_type = match t.string_encoding {
                crate::token::StringEncoding::Narrow | crate::token::StringEncoding::Utf8 => {
                    Type::I8
                }
                crate::token::StringEncoding::Utf16 => Type::integer(16, false),
                crate::token::StringEncoding::Utf32 | crate::token::StringEncoding::Wide => {
                    Type::integer(32, false)
                }
            };
            let node = Node {
                token: Some(t.clone()),
                nt: NodeType::GlobalVar,
                dest: self.new_string_literal(&t),
                ..Node::default()
            };
            Node {
                token: Some(t.clone()),
                nt: NodeType::Addr,
                cty: Some(Type::Ptr(Box::new(element_type))),
                lhs: Some(Box::new(node)),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt(TokenType::Float) {
            Node {
                nt: NodeType::Num,
                value: Some(t.i_value),
                cty: Some(Type::Float(if t.s_value == "f" { 32 } else { 64 })),
                token: Some(t),
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
            if t.s_value == "__builtin_offsetof" {
                return self.offsetof_builtin(t);
            }
            if t.s_value.starts_with("__builtin_va_") {
                return self.varargs_builtin(t);
            }
            let thread_local = self
                .global_variables
                .get(&t.s_value)
                .is_some_and(|object| object.thread_local);
            if let Some(ident) = self.resolve_name(&t.s_value) {
                match ident {
                    Identifier::Dynamic(ty, offset, registered) => {
                        let mut node = Self::dynamic_node(ty.clone(), *offset, t.clone());
                        node.registered = *registered;
                        node
                    }
                    Identifier::Local(ty, offset, registered) => Node {
                        registered: *registered,
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
                        thread_local,
                        ..Node::default()
                    },
                    Identifier::Static(ty) => Node {
                        token: Some(t.clone()),
                        nt: NodeType::GlobalVar,
                        cty: Some(ty.clone()),
                        global_name: t.s_value.clone(),
                        thread_local,
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
    fn suffix_update(&mut self, token: Token, operand: Node, increment: bool) -> Node {
        self.expect_assignable(&operand);
        let ty = operand.resolve_type().unwrap();
        if ty.float_bits().is_none()
            && !ty
                .dest_type()
                .is_some_and(|child| child.is_variable_array())
        {
            return Node::new_with_op_and_lhs(
                Some(token),
                if increment {
                    NodeType::SuffixIncr
                } else {
                    NodeType::SuffixDecr
                },
                operand,
            );
        }
        let pointer = self.register_local_variable(
            Token {
                s_value: format!(".suffix.pointer.{}", token.pos),
                ..token.clone()
            },
            Type::Ptr(Box::new(ty.clone())),
        );
        let previous = self.register_local_variable(
            Token {
                s_value: format!(".suffix.value.{}", token.pos),
                ..token.clone()
            },
            ty.clone(),
        );
        let address = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, operand);
        let save = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            pointer.clone(),
            address,
        );
        let target = Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, pointer);
        let old = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            previous.clone(),
            target.clone(),
        );
        let one = Node {
            cty: Some(if ty.float_bits().is_some() {
                ty.clone()
            } else {
                Type::I32
            }),
            value: Some(if ty.float_bits().is_some() {
                ty.float_constant(1.0)
            } else {
                1
            }),
            token: Some(token.clone()),
            ..Node::default()
        };
        let value = Node::new_with_op(
            Some(token.clone()),
            if increment {
                NodeType::Add
            } else {
                NodeType::Sub
            },
            previous.clone(),
            one,
        );
        let store = Node::new_with_op(Some(token.clone()), NodeType::Assign, target, value);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![save, old, store, previous],
            ..Node::default()
        }
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
                for argument in &mut args {
                    if matches!(argument.resolve_type(), Some(Type::Struct(_))) {
                        let ty = argument.resolve_type().unwrap();
                        let temporary = self.register_local_variable(
                            Token {
                                s_value: format!(".argument.{}.{}", t.pos, self.offset_size),
                                ..t.clone()
                            },
                            ty,
                        );
                        *argument =
                            self.struct_assignment(temporary, std::mem::take(argument), t.clone());
                    }
                }
                let result_offset = if matches!(return_type, Type::Struct(_)) {
                    self.expect_object_type(&return_type, &t);
                    self.offset_size =
                        self.offset_size.div_ceil(8) * 8 + return_type.size_of().div_ceil(8) * 8;
                    Some(self.offset_size)
                } else {
                    None
                };
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
                    offset: result_offset,
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
                let path = ty.member_path(&name.s_value).unwrap_or_else(|| {
                    self.error_logger
                        .print_error_position(name.pos, "unknown struct member");
                    unreachable!()
                });
                for index in path {
                    let Some(Type::Struct(st)) = node.resolve_type() else {
                        unreachable!()
                    };
                    node = Self::member_node(node, &st.members()[index], &token);
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
                node = self.suffix_update(token, node, true);
            } else if let Some(token) = self.attempt_reserved("--") {
                // Suffix decrement
                node = self.suffix_update(token, node, false);
            } else {
                return node;
            }
        }
    }

    fn new_string_literal(&mut self, token: &Token) -> String {
        self.string_literals.push(token.clone());
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
            ("int main(){const int x=1;x=2;}", "const object"),
            (
                "typedef int A[2];int main(){const A a={1};a[0]=2;}",
                "const object",
            ),
            ("int main(){int x;int *const p=&x;p=0;}", "const object"),
            ("int main(){const int x=1;int *p=&x;}", "incompatible value"),
            ("struct S{int x:33;};", "invalid bit-field width"),
            ("struct S{int x:0;};", "invalid bit-field width"),
            ("struct S{int *x:3;};", "integer type"),
            (
                "struct S{int x:3;};int main(){struct S s;return sizeof(s.x);}",
                "sizeof cannot",
            ),
            (
                "struct S{int x:3;};int main(){struct S s;int *p=&s.x;}",
                "address of a bit-field",
            ),
            ("int main(){return 1.5%2;}", "integer operands"),
            ("int main(){return 1.0<<2;}", "integer operands"),
            ("int main(){int *p;return p+1.5;}", "incompatible value"),
            ("int a[1.5];", "integer expression"),
            (
                "struct S;int main(){int n=2;struct S a[n];}",
                "complete object type",
            ),
            (
                "typedef int A[];int main(){int n=2;A a[n];}",
                "complete object type",
            ),
            ("int main(){return (int*)1.5;}", "floating pointer cast"),
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
                ..Token::default()
            },
            Token {
                tt: TokenType::Ident,
                pos: 0,
                i_value: 0,
                s_value: "x".to_string(),
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                i_value: 0,
                s_value: "=".to_string(),
                ..Token::default()
            },
            Token {
                tt: TokenType::Num,
                pos: 0,
                i_value: 42,
                s_value: String::new(),
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                i_value: 0,
                s_value: ";".to_string(),
                ..Token::default()
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
                    alignment: 0,
                    thread_local: false,
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

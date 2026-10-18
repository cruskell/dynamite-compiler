use crate::ctype::{Qualifiers, StructDefinition, StructMember, StructType, Type};
use crate::error;
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{AtomicOperation, Node, NodeType};
use crate::token::{Token, TokenType};
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

mod atomic;
mod attributes;
mod builtins;
mod const_eval;
mod declarations;
mod declarators;
mod initializers;
mod inline_asm;
mod inline_calls;
mod lowering;
mod object_size;
mod overflow;
mod prefetch;
mod statement_expression;
mod transparent_union;
mod type_parser;
mod type_query;
mod validation;
mod varargs;
mod vla;

pub struct AstBuilderImpl<'a> {
    // Input and diagnostics.
    error_logger: &'a dyn error::ErrorLogger,
    tokens: &'a Vec<Token>,
    cur: usize,

    // Scopes, types and declaration context.
    scope_stack: Vec<HashMap<String, Identifier>>,
    struct_definitions: Vec<Rc<RefCell<StructDefinition>>>,
    next_enum_id: usize,
    declaration_attributes_state: Option<DeclarationAttributes>,
    parameter_depth: usize,

    // Current function.
    current_function: Option<Token>,
    return_type: Option<Type>,
    return_pointer: Option<usize>,
    mutable_static: Option<Token>,
    variadic_parameters: Option<Vec<Type>>,
    variadic_last: Option<usize>,

    // Local storage and variable-length arrays.
    offset_size: usize,
    next_deferred_local: usize,
    pending_bounds: Vec<Node>,
    dynamic_objects: Vec<Node>,
    vla_active: Vec<usize>,

    // Control flow and jump lifetimes.
    loop_stack: Vec<usize>,
    continue_depths: HashMap<usize, usize>,
    switch_stack: Vec<Vec<Node>>,
    switch_types: Vec<Type>,
    switch_vla_depths: Vec<usize>,
    labels: HashMap<String, usize>,
    defined_labels: HashSet<String>,
    jump_depths: Vec<usize>,

    // Translation-unit output.
    pub functions: HashMap<String, Func>,
    pub global_variables: HashMap<String, GlobalVariable>,
    pub string_literals: Vec<Token>,
    pub symbol_names: HashMap<String, String>,
}

pub struct ProgramAst<'a> {
    pub functions: &'a HashMap<String, Func>,
    pub global_variables: &'a HashMap<String, GlobalVariable>,
    pub string_literals: &'a Vec<Token>,
    pub symbol_names: &'a HashMap<String, String>,
}

pub enum Identifier {
    TypeDef(Type),
    EnumTag {
        ty: Type,
        fixed: bool,
        defined: bool,
    },
    Constant(i64, Type),

    Local(Type, usize, bool, usize),
    Dynamic(Type, usize, bool),
    Static(Type),
    StaticObject(Type, String),
}

#[derive(Clone, Copy, PartialEq)]
enum Nullability {
    Nullable,
    Nonnull,
    Unspecified,
    NullableResult,
}

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
    Array(Box<Declarator>, Option<usize>),
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
                let ty = Type::Ptr(Rc::new(base));
                inner.resolve(if q == Qualifiers::default() {
                    ty
                } else {
                    Type::Qualified(Rc::new(ty), q)
                })
            }
            Self::Array(inner, size) => inner.resolve(Type::Arr(Rc::new(base), size)),
            Self::VariableArray(inner, offset) => inner.resolve(Type::Vla(Rc::new(base), offset)),
            Self::ArrayQualifiers(inner, q) => inner.resolve(Type::Qualified(Rc::new(base), q)),
            Self::UnspecifiedArray(inner, token) => {
                let (name, ty, parameters, scope, mut stars) =
                    inner.resolve(Type::Vla(Rc::new(base), usize::MAX));
                stars.push(token);
                (name, ty, parameters, scope, stars)
            }
            Self::Function(inner, parameters, variadic, scope, stars) => {
                let types = parameters
                    .iter()
                    .map(|(_, ty, _)| ty.clone())
                    .collect::<Vec<_>>();
                let ty = if variadic {
                    Type::VariadicFunc(Rc::new(types), Rc::new(base))
                } else {
                    Type::Func(Rc::new(types), Rc::new(base))
                };
                // A direct name keeps this signature; nested declarators supply their own parameters.
                let direct = matches!(inner.as_ref(), Self::Name(_));
                let (name, ty, nested, nested_scope, nested_stars) = inner.resolve(ty);
                let (parameters, scope, stars) = if direct {
                    (parameters, scope, stars)
                } else {
                    (nested, nested_scope, nested_stars)
                };
                (name, ty, parameters, scope, stars)
            }
        }
    }
}

#[derive(Clone, Default)]
struct DeclarationAttributes {
    typedef_declarator: bool,
    word_mode: bool,
    vector_size: Option<usize>,
    transparent_union: bool,

    gnu_inline: bool,
    noinline: bool,

    common: bool,
    weak_import: bool,
    hidden: bool,
}

pub(crate) const METADATA_ATTRIBUTES: &[&str] = &[
    "availability",
    "deprecated",
    "unavailable",
    "unused",
    "may_alias",
    "warn_unused_result",
    "format",
    "format_arg",
    "nonnull",
    "returns_nonnull",
    "malloc",
    "alloc_size",
    "alloc_align",
    "noreturn",
    "nothrow",
    "noescape",
    "always_inline",
    "const",
    "pure",
    "cold",
    "hot",
    "not_tail_called",
    "disable_tail_calls",
    "swift_attr",
    "swift_name",
    "swift_private",
    "swift_wrapper",
    "objc_bridge",
    "objc_bridge_mutable",
    "objc_boxable",
    "cf_consumed",
    "cf_returns_retained",
    "cf_returns_not_retained",
    "enum_extensibility",
    "flag_enum",
    "sentinel",
    "diagnose_if",
];

pub trait AstBuilder {
    fn build(&mut self, is_debug: bool) -> ProgramAst;
}

impl AstBuilder for AstBuilderImpl<'_> {
    fn build(&mut self, is_debug: bool) -> ProgramAst {
        let timer = crate::timing::Timer::new("parse.declarations");
        while !self.at_eof() {
            if self.attempt_reserved(";").is_none() {
                self.global_definition();
            }
        }
        drop(timer);
        let timer = crate::timing::Timer::new("parse.validate");
        for function in self.functions.values() {
            if let Some(token) = &function.token {
                self.validate_type_layout(&function.cty, token);
            }
            if let Some(body) = &function.body {
                self.validate_void_expressions(body);
                self.validate_storage_only_expressions(body, false);
                if function.inline_only() && !function.gnu_inline {
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
        drop(timer);
        let timer = crate::timing::Timer::new("parse.lower");
        // Source-type lowering updates annotations; value conversions preserve them.
        for function in self.functions.values_mut() {
            function.cty.lower_for_codegen();
            for arg in &mut function.args {
                Self::lower_source_types(arg);
                if let Some(ty) = &mut arg.cty {
                    ty.lower_parameter_for_codegen();
                }
            }
            if let Some(body) = &mut function.body {
                Self::lower_source_types(body);
                Self::lower_variable_arithmetic(body);
                overflow::lower(body, &mut function.offset_size);
                Self::insert_integer_conversions(body);
                crate::bitfield::lower(body, &mut function.offset_size);
            }
        }
        for global in self.global_variables.values_mut() {
            global.ty.lower_for_codegen();
        }
        for definition in &self.struct_definitions {
            // Parameter normalization may inspect this record through a callback type.
            let mut members = definition.borrow().members.clone();
            for member in &mut members {
                member.ty.lower_for_codegen();
            }
            definition.borrow_mut().members = members;
        }
        drop(timer);
        let _timer = crate::timing::Timer::new("parse.inline");
        self.lower_gnu_inline_calls();
        if is_debug {
            self.print_functions();
        }
        ProgramAst {
            functions: &self.functions,
            global_variables: &self.global_variables,
            string_literals: &self.string_literals,
            symbol_names: &self.symbol_names,
        }
    }
}

pub fn reserved_functions() -> HashMap<String, Identifier> {
    [
        (
            String::from("malloc"),
            Identifier::Static(Type::Func(
                Rc::new(vec![Type::integer(64, false)]),
                Rc::new(Type::Ptr(Rc::new(Type::Void))),
            )),
        ),
        (
            String::from("free"),
            Identifier::Static(Type::Func(
                Rc::new(vec![Type::Ptr(Rc::new(Type::Void))]),
                Rc::new(Type::Void),
            )),
        ),
        (
            String::from("printf"),
            Identifier::Static(Type::VariadicFunc(
                Rc::new(vec![Type::Ptr(Rc::new(Type::Qualified(
                    Rc::new(Type::I8),
                    Qualifiers {
                        constant: true,
                        volatile: false,
                        restricted: false,
                    },
                )))]),
                Rc::new(Type::I32),
            )),
        ),
        (
            String::from("puts"),
            Identifier::Static(Type::Func(
                Rc::new(vec![Type::Ptr(Rc::new(Type::Qualified(
                    Rc::new(Type::I8),
                    Qualifiers {
                        constant: true,
                        volatile: false,
                        restricted: false,
                    },
                )))]),
                Rc::new(Type::I32),
            )),
        ),
        (
            String::from("putchar"),
            Identifier::Static(Type::Func(Rc::new(vec![Type::I32]), Rc::new(Type::I32))),
        ),
        (
            String::from("exit"),
            Identifier::Static(Type::Func(Rc::new(vec![Type::I32]), Rc::new(Type::I32))),
        ),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>()
}

impl<'a> AstBuilderImpl<'a> {
    pub fn new(error_logger: &'a dyn error::ErrorLogger, tokens: &'a Vec<Token>) -> Self {
        let mut builder = Self {
            error_logger,
            tokens,
            cur: 0,
            scope_stack: vec![reserved_functions()],
            struct_definitions: vec![],
            next_enum_id: 0,
            declaration_attributes_state: None,
            parameter_depth: 0,
            current_function: None,
            return_type: None,
            return_pointer: None,
            mutable_static: None,
            variadic_parameters: None,
            variadic_last: None,
            offset_size: 0,
            next_deferred_local: usize::MAX / 2,
            pending_bounds: vec![],
            dynamic_objects: vec![],
            vla_active: vec![],
            loop_stack: vec![],
            continue_depths: HashMap::new(),
            switch_stack: vec![],
            switch_types: vec![],
            switch_vla_depths: vec![],
            labels: HashMap::new(),
            defined_labels: HashSet::new(),
            jump_depths: vec![],
            functions: HashMap::new(),
            global_variables: HashMap::new(),
            string_literals: Vec::new(),
            symbol_names: HashMap::new(),
        };
        let va_list = builder.builtin_va_list_type();
        builder.scope_stack[0].insert("__builtin_va_list".into(), Identifier::TypeDef(va_list));
        builder
    }
    // Borrow input tokens; AST construction clones only the tokens it retains.
    fn attempt_reserved(&mut self, s_value: &str) -> Option<&'a Token> {
        if let TokenType::Reserved = self.tokens[self.cur].tt {
            if self.tokens[self.cur].s_value == s_value {
                self.cur += 1;
                return Some(&self.tokens[self.cur - 1]);
            }
        }
        None
    }
    fn attempt_inline(&mut self) -> Option<&'a Token> {
        self.attempt_reserved("inline")
            .or_else(|| self.attempt_reserved("__inline"))
            .or_else(|| self.attempt_reserved("__inline__"))
    }
    fn attempt_ident(&mut self) -> Option<Token> {
        if let TokenType::Ident = self.tokens[self.cur].tt {
            self.cur += 1;
            Some(self.tokens[self.cur - 1].clone())
        } else {
            None
        }
    }
    fn expect_reserved(&mut self, s_value: &str) -> &'a Token {
        if let TokenType::Reserved = self.tokens[self.cur].tt {
            if self.tokens[self.cur].s_value == s_value {
                self.cur += 1;
                return &self.tokens[self.cur - 1];
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

    fn stmt(&mut self) -> Node {
        while self.attempt_reserved("__extension__").is_some() {}
        self.declaration_attributes(false, None, true);
        if matches!(
            self.tokens[self.cur].tt,
            TokenType::Reserved | TokenType::Ident
        ) && matches!(
            self.tokens[self.cur].s_value.as_str(),
            "asm" | "__asm" | "__asm__"
        ) {
            let token = self.tokens[self.cur].clone();
            self.cur += 1;
            return self.inline_assembly(token);
        }
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
            if !self.defined_labels.insert(token.s_value.to_string()) {
                self.error_logger
                    .print_error_position(token.pos, "duplicate label");
            }
            let id = *self
                .labels
                .entry(token.s_value.to_string())
                .or_insert(token.pos);
            let vla_scope = self.vla_active.clone();
            let lhs = Some(Box::new(self.stmt()));
            return Node {
                nt: NodeType::Label,
                token: Some(token),
                lhs,
                value: Some(id),
                vla_scope,
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("goto") {
            let name = self.attempt_ident().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "goto requires a label");
                unreachable!()
            });
            let id = *self
                .labels
                .entry(name.s_value.to_string())
                .or_insert(name.pos);
            self.expect_reserved(";");
            return Node {
                nt: NodeType::Goto,
                token: Some(token.clone()),
                value: Some(id),
                args: self.dynamic_objects.clone(),
                vla_scope: self.vla_active.clone(),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("typedef") {
            self.typedef_definition();
            self.expect_reserved(";");
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: self.take_bounds(),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved(";") {
            return Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
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
                nt: NodeType::Switch,
                token: Some(t.clone()),
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
                token: Some(t.clone()),
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
            return Node::new_if_node(Some(t.clone()), cond, then, els);
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
            let mut node = Node::new_while_node(Some(t.clone()), cond, body);
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
            return Node::new_while_node(Some(t.clone()), cond, body);
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
            let loop_node = Node::new_for_node(Some(t.clone()), ini, cond, upd, body);
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
        let mut requested_alignment = self.object_alignment_annotations();
        let mut thread_local = false;
        while let Some(value) = self.attempt_alignment_specifier() {
            requested_alignment = requested_alignment.max(value);
        }
        let inline = self.attempt_inline();
        let storage = self
            .attempt_reserved("static")
            .or_else(|| self.attempt_reserved("extern"));
        let inline = inline.or_else(|| self.attempt_inline());
        let mut storage = storage;
        loop {
            requested_alignment = requested_alignment.max(self.object_alignment_annotations());
            if self.attempt_thread_local() {
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
        if thread_local && storage.is_none() {
            self.error_logger.print_error_position(
                self.tokens[self.cur - 1].pos,
                "block thread-local declaration requires static or extern storage",
            );
        }
        if storage.is_some() || inline.is_some() || thread_local {
            let mut storage = storage.cloned().unwrap_or_else(|| {
                inline.cloned().unwrap_or_else(|| Token {
                    s_value: "static".to_owned().into(),
                    ..self.tokens[self.cur - 1].clone()
                })
            });
            if matches!(
                storage.s_value.as_str(),
                "inline" | "__inline" | "__inline__"
            ) {
                storage.s_value = "extern".to_owned().into();
            }
            requested_alignment = requested_alignment.max(self.object_alignment_annotations());
            let ty = self.expect_type();
            requested_alignment = requested_alignment.max(self.object_alignment_annotations());
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
            if storage == AutomaticStorage::Register && requested_alignment != 0 {
                self.error_logger.print_error_position(
                    self.tokens[self.cur].pos,
                    "alignment cannot be applied to a register object",
                );
            }
            if self.tokens[self.cur].s_value == ";" {
                if alignment.max(requested_alignment) != 0 {
                    self.error_logger.print_error_position(
                        self.tokens[self.cur].pos,
                        "alignment requires an object declaration",
                    );
                }
                Node {
                    nt: NodeType::Block,
                    ..Node::default()
                }
            } else {
                self.local_variable_definition_with_storage(
                    ty,
                    storage,
                    alignment.max(requested_alignment),
                )
            }
        } else if requested_alignment != 0 {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "alignment requires an object declaration",
            );
            unreachable!()
        } else if let Some(t) = self.attempt_reserved("continue") {
            let target = self.loop_stack.last().copied().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(t.pos, "continue requires an enclosing loop");
                unreachable!()
            });
            Node {
                nt: NodeType::Continue,
                token: Some(t.clone()),
                value: Some(target),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_reserved("break") {
            Node {
                nt: NodeType::Break,
                token: Some(t.clone()),
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
                nt: NodeType::Return,
                token: Some(t.clone()),
                lhs: operand.map(Box::new),
                offset: self.return_pointer,
                ..Node::default()
            }
        } else {
            self.expr()
        };
        self.expect_reserved(";");
        self.cleanup_jump(node)
    }

    pub fn consume_block(&mut self) -> Option<Node> {
        let (token, mut children, cleanup) = self.consume_block_parts()?;
        children.extend(cleanup);
        Some(Node {
            nt: NodeType::Block,
            token: Some(token),
            children,
            ..Node::default()
        })
    }

    fn consume_block_parts(&mut self) -> Option<(Token, Vec<Node>, Vec<Node>)> {
        if let Some(t) = self.attempt_reserved("{") {
            self.scope_stack.push(HashMap::new());
            let object_depth = self.dynamic_objects.len();
            let bound_depth = self.vla_active.len();
            let mut children: Vec<Node> = Vec::new();
            while self.attempt_reserved("}").is_none() {
                children.push(self.stmt());
            }
            let cleanup = self.cleanup_arrays(object_depth);
            self.dynamic_objects.truncate(object_depth);
            self.vla_active.truncate(bound_depth);
            self.scope_stack.pop();
            Some((t.clone(), children, cleanup))
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
            token: Some(token.clone()),
            children,
            ..Node::default()
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
        let ty = Type::Ptr(Rc::new(lhs.object_type().unwrap()));
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
                return self.struct_assignment(node, rhs, token.clone());
            }
            return Node::new_with_op(Some(token.clone()), NodeType::Assign, node, rhs);
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
                return self.compound_assignment(token.clone(), node, rhs, operation);
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
            let mut conditional = Node::new_if_node(Some(t.clone()), node, then, Some(els));
            conditional.nt = NodeType::Conditional;
            return conditional;
        }
        node
    }
    fn logical_or(&mut self) -> Node {
        let mut node = self.logical_and();
        loop {
            if let Some(t) = self.attempt_reserved("||") {
                node = Node::new_with_op(
                    Some(t.clone()),
                    NodeType::LogicalOr,
                    node,
                    self.logical_and(),
                );
            } else {
                return node;
            }
        }
    }
    fn logical_and(&mut self) -> Node {
        let mut node = self.bitwise_or();
        loop {
            if let Some(t) = self.attempt_reserved("&&") {
                node = Node::new_with_op(
                    Some(t.clone()),
                    NodeType::LogicalAnd,
                    node,
                    self.bitwise_or(),
                );
            } else {
                return node;
            }
        }
    }
    fn bitwise_or(&mut self) -> Node {
        let mut node = self.bitwise_xor();
        loop {
            if let Some(t) = self.attempt_reserved("|") {
                node = Node::new_with_op(Some(t.clone()), NodeType::BitOr, node, self.bitwise_xor())
            } else {
                return node;
            }
        }
    }
    fn bitwise_xor(&mut self) -> Node {
        let mut node = self.bitwise_and();
        loop {
            if let Some(t) = self.attempt_reserved("^") {
                node =
                    Node::new_with_op(Some(t.clone()), NodeType::BitXor, node, self.bitwise_and())
            } else {
                return node;
            }
        }
    }
    fn bitwise_and(&mut self) -> Node {
        let mut node = self.equality();
        loop {
            if let Some(t) = self.attempt_reserved("&") {
                node = Node::new_with_op(Some(t.clone()), NodeType::BitAnd, node, self.equality())
            } else {
                return node;
            }
        }
    }
    fn equality(&mut self) -> Node {
        let mut node = self.relational();
        loop {
            if let Some(t) = self.attempt_reserved("==") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Eq, node, self.relational())
            } else if let Some(t) = self.attempt_reserved("!=") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Ne, node, self.relational())
            } else {
                return node;
            }
        }
    }
    fn relational(&mut self) -> Node {
        let mut node = self.bit_shift();
        loop {
            if let Some(t) = self.attempt_reserved("<") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Lt, node, self.bit_shift())
            } else if let Some(t) = self.attempt_reserved("<=") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Le, node, self.bit_shift())
            } else if let Some(t) = self.attempt_reserved(">") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Lt, self.bit_shift(), node)
            } else if let Some(t) = self.attempt_reserved(">=") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Le, self.bit_shift(), node)
            } else {
                return node;
            }
        }
    }
    fn bit_shift(&mut self) -> Node {
        let mut node = self.add();
        loop {
            if let Some(t) = self.attempt_reserved("<<") {
                node = Node::new_with_op(Some(t.clone()), NodeType::BitLeft, node, self.add())
            } else if let Some(t) = self.attempt_reserved(">>") {
                node = Node::new_with_op(Some(t.clone()), NodeType::BitRight, node, self.add())
            } else {
                return node;
            }
        }
    }
    fn add(&mut self) -> Node {
        let mut node = self.mul();
        loop {
            if let Some(t) = self.attempt_reserved("+") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Add, node, self.mul())
            } else if let Some(t) = self.attempt_reserved("-") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Sub, node, self.mul())
            } else {
                return node;
            }
        }
    }
    fn mul(&mut self) -> Node {
        let mut node = self.unary();
        loop {
            if let Some(t) = self.attempt_reserved("*") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Mul, node, self.unary())
            } else if let Some(t) = self.attempt_reserved("/") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Div, node, self.unary())
            } else if let Some(t) = self.attempt_reserved("%") {
                node = Node::new_with_op(Some(t.clone()), NodeType::Mod, node, self.unary())
            } else {
                return node;
            }
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
        if !ty.is_complete_object()
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
                token: Some(token.clone()),
                children,
                ..Node::default()
            }
        } else {
            Node {
                token: Some(token.clone()),
                cty: Some(Type::integer(64, false)),
                value: Some(ty.size_of()),
                ..Node::default()
            }
        }
    }
    fn unary(&mut self) -> Node {
        while self.attempt_reserved("__extension__").is_some() {}
        let saved = self.cur;
        if let Some(token) = self.attempt_reserved("(") {
            let pending_start = self.pending_bounds.len();
            if let Some(base) = self.attempt_type() {
                let (name, mut ty, _, _, _) = self.declarator().resolve(base);
                self.validate_type_layout(&ty, &self.tokens[self.cur]);
                if name.is_some() {
                    self.error_logger
                        .print_error_position(token.pos, "cast requires a type name");
                }
                self.expect_reserved(")");
                if self.tokens[self.cur].s_value == "{" {
                    let inferred = matches!(ty, Type::Arr(_, None));
                    let name = Token {
                        s_value: format!(".compound.{}", token.pos).into(),
                        ..token.clone()
                    };
                    let literal = if self.return_type.is_some() {
                        let (object, initializer) = if inferred {
                            let object = self.register_incomplete_local_array(name, ty, 0);
                            self.inferred_local_array_initializer(&object, token)
                        } else {
                            let object = self.register_local_variable(name, ty);
                            let initializer = self.local_variable_initialization(&object, token);
                            (object, initializer)
                        };
                        let mut children = vec![];
                        Self::initializer_expressions(initializer, &mut children);
                        children.push(object);
                        Node {
                            nt: NodeType::Sequence,
                            token: Some(token.clone()),
                            children,
                            compound_literal: true,
                            ..Node::default()
                        }
                    } else {
                        let data = self.global_initializer(&ty);
                        let count = if let GlobalVariableData::Arr(elements) = &data {
                            elements.len()
                        } else {
                            0
                        };
                        ty = self.infer_array_size(ty, count, token);
                        self.expect_object_type(&ty, token);
                        self.validate_global_initializer(&ty, &data, token);
                        self.global_variables.insert(
                            name.s_value.to_string(),
                            GlobalVariable {
                                ty: ty.clone(),
                                alignment: 0,
                                data: Some(data),
                                internal: true,
                                declaration_only: false,
                                common: false,
                                weak_import: false,
                                hidden: false,
                                thread_local: false,
                            },
                        );
                        Node {
                            nt: NodeType::GlobalVar,
                            token: Some(token.clone()),
                            cty: Some(ty),
                            global_name: name.s_value.to_string(),
                            ..Node::default()
                        }
                    };
                    return self.postfix(literal);
                }
                let ty = ty.unqualified().clone();
                let operand = self.unary();
                let source = operand.resolve_type().unwrap();
                if (ty.float_bits().is_some() && source.is_pointer_value())
                    || (source.float_bits().is_some() && matches!(ty.unqualified(), Type::Ptr(_)))
                {
                    self.error_logger
                        .print_error_position(token.pos, "invalid floating pointer cast");
                }
                if !(ty.is_arithmetic() || matches!(ty, Type::Void | Type::Ptr(_)))
                    || (!matches!(
                        source.unqualified(),
                        Type::Bool
                            | Type::I8
                            | Type::SignedChar
                            | Type::I32
                            | Type::Integer { .. }
                            | Type::LongLong { .. }
                            | Type::Enum(_)
                            | Type::Float(_)
                            | Type::LongDouble(_)
                            | Type::Ptr(_)
                            | Type::Arr(..)
                            | Type::Vla(..)
                            | Type::Func(..)
                            | Type::VariadicFunc(..)
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
                    token: Some(token.clone()),
                    children: bounds,
                    ..Node::default()
                };
            }
            self.cur = saved;
        }

        if let Some(t) = self.attempt_reserved("sizeof") {
            let saved = self.cur;
            if self.attempt_reserved("(").is_some() {
                let pending_start = self.pending_bounds.len();
                if let Some(base) = self.attempt_type() {
                    let (name, ty, _, _, _) = self.declarator().resolve(base);
                    self.validate_type_layout(&ty, t);
                    if name.is_some() {
                        self.error_logger
                            .print_error_position(t.pos, "sizeof requires a type name");
                    }
                    self.expect_reserved(")");
                    if matches!(ty, Type::Void | Type::Func(..) | Type::VariadicFunc(..))
                        || !ty.is_complete_object()
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
                            token: Some(t.clone()),
                            children,
                            ..Node::default()
                        }
                    };
                }
                self.cur = saved;
            }
            return self.sizeof_expression(t);
        }
        if let Some(token) = self.attempt_reserved("_Alignof") {
            let pending_start = self.pending_bounds.len();
            self.expect_reserved("(");
            let base = self.expect_type();
            let (name, ty, _, _, _) = self.declarator().resolve(base);
            self.pending_bounds.truncate(pending_start);
            self.validate_type_layout(&ty, token);
            if name.is_some()
                || matches!(
                    ty.unqualified(),
                    Type::Void | Type::Func(..) | Type::VariadicFunc(..)
                )
                || !ty.is_complete_object()
            {
                self.error_logger
                    .print_error_position(token.pos, "_Alignof requires a complete object type");
            }
            self.expect_reserved(")");
            return Node {
                token: Some(token.clone()),
                cty: Some(Type::integer(64, false)),
                value: Some(ty.align_of()),
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
                    token: Some(t.clone()),
                    cty: Some(ty.clone()),
                    value: Some(ty.float_constant(-1.0)),
                    ..Node::default()
                };
                return Node::new_with_op(Some(t.clone()), NodeType::Mul, operand, minus_one);
            }
            return Node::new_with_op(
                Some(t.clone()),
                NodeType::Sub,
                Node::new_with_num(Some(t.clone()), 0),
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
            return Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, operand);
        }
        if let Some(token) = self.attempt_reserved("*") {
            let operand = self.unary();
            if operand.dest_type().is_none() || operand.dest_type() == Some(Type::Void) {
                self.error_logger
                    .print_error_position(token.pos, "cannot dereference a non-pointer");
            }
            return Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Deref, operand);
        }
        if let Some(t) = self.attempt_reserved("~") {
            return Node::new_with_op_and_lhs(Some(t.clone()), NodeType::BitNot, self.unary());
        }
        if let Some(t) = self.attempt_reserved("!") {
            let lhs = Some(Box::new(self.unary()));
            return Node {
                nt: NodeType::Eq,
                token: Some(t.clone()),
                lhs,
                rhs: Some(Box::new(Node::new_with_num(None, 0))),
                ..Node::default()
            };
        }
        if let Some(token) = self.attempt_reserved("++") {
            let operand = self.unary();
            return self.compound_assignment(
                token.clone(),
                operand,
                Node::new_with_num(None, 1),
                NodeType::Add,
            );
        }
        if let Some(token) = self.attempt_reserved("--") {
            let operand = self.unary();
            return self.compound_assignment(
                token.clone(),
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
            ty @ (Type::Func(..) | Type::VariadicFunc(..)) => Type::Ptr(Rc::new(ty)),
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
                    || !ty.is_complete_object()
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
                    if selected.is_some() {
                        self.error_logger.print_error_position(
                            association.pos,
                            "generic selection matches multiple association types",
                        );
                    }
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
            self.generic_selection(token.clone())
        } else if let Some(token) = self.attempt_reserved("(") {
            if self.tokens[self.cur].tt == TokenType::Reserved
                && self.tokens[self.cur].s_value == "{"
            {
                self.statement_expression(token.clone())
            } else {
                let node = self.expr();
                self.expect_reserved(")");
                node
            }
        } else if let Some(t) = self.attempt(TokenType::Str) {
            // String literal
            let element_type = match t.string_encoding {
                crate::token::StringEncoding::Narrow | crate::token::StringEncoding::Utf8 => {
                    Type::I8
                }
                crate::token::StringEncoding::Utf16 => Type::integer(16, false),
                crate::token::StringEncoding::Utf32 => Type::integer(32, false),
                crate::token::StringEncoding::Wide => Type::wide_character(),
            };
            let count = t.encoded_string().len() / t.string_encoding.unit_size();
            let dest = self.new_string_literal(&t);
            Node {
                nt: NodeType::GlobalVar,
                token: Some(t.clone()),
                cty: Some(Type::Arr(Rc::new(element_type), Some(count))),
                dest,
                ..Node::default()
            }
        } else if let Some(t) = self.attempt(TokenType::Float) {
            let value = Some(t.i_value);
            let extended_float = t.extended_float.clone();
            let cty = Some(if t.s_value == "l" {
                self.long_double_type(&t)
            } else {
                Type::Float(if t.s_value == "f" { 32 } else { 64 })
            });
            Node {
                nt: NodeType::Num,
                token: Some(t),
                cty,
                value,
                extended_float,
                ..Node::default()
            }
        } else if let Some(t) = self.attempt(TokenType::Num) {
            let ty = match t.string_encoding {
                crate::token::StringEncoding::Utf16 => Type::integer(16, false),
                crate::token::StringEncoding::Utf32 => Type::integer(32, false),
                crate::token::StringEncoding::Wide => Type::wide_character(),
                _ if t.s_value.contains("ll") => Type::LongLong {
                    signed: !t.s_value.contains('u'),
                },
                _ => Type::integer(
                    if t.s_value.contains('l') { 64 } else { 32 },
                    !t.s_value.contains('u'),
                ),
            };
            let value = if t.string_encoding == crate::token::StringEncoding::Wide {
                ty.convert_constant(t.i_value as i64) as usize
            } else {
                t.i_value
            };
            Node {
                nt: NodeType::Num,
                token: Some(t),
                cty: Some(ty),
                value: Some(value),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt_ident() {
            if t.s_value == "__builtin_types_compatible_p" {
                return self.types_compatible_builtin(t);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin_nan" | "__builtin_nanf" | "__builtin_nanl"
            ) {
                return self.floating_nan_builtin(t);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin_inff"
                    | "__builtin_inf"
                    | "__builtin_infl"
                    | "__builtin_huge_valf"
                    | "__builtin_huge_val"
                    | "__builtin_huge_vall"
            ) {
                return self.floating_infinity_builtin(t);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin_fabsf" | "__builtin_fabs" | "__builtin_fabsl"
            ) {
                return self.floating_abs_builtin(t);
            }
            if let Some(operation) = Self::atomic_operation(&t.s_value) {
                return self.atomic_builtin(t, operation);
            }
            if t.s_value == "__builtin_prefetch" {
                return self.prefetch_builtin(t);
            }
            if t.s_value == "__builtin_bzero" {
                return self.runtime_builtin(
                    t,
                    "bzero",
                    Type::Func(
                        Rc::new(vec![
                            Type::Ptr(Rc::new(Type::Void)),
                            Type::integer(64, false),
                        ]),
                        Rc::new(Type::Void),
                    ),
                );
            }
            if t.s_value == "__builtin_constant_p" {
                return self.constant_query_builtin(t);
            }
            if let Some((bits, leading)) = match t.s_value.as_str() {
                "__builtin_clz" => Some((32, true)),
                "__builtin_clzl" | "__builtin_clzll" => Some((64, true)),
                "__builtin_ctz" => Some((32, false)),
                "__builtin_ctzl" | "__builtin_ctzll" => Some((64, false)),
                _ => None,
            } {
                return self.bit_scan_builtin(t, bits, leading);
            }
            if let Some(bits) = match t.s_value.as_str() {
                "__builtin_popcount" => Some(32),
                "__builtin_popcountl" | "__builtin_popcountll" => Some(64),
                _ => None,
            } {
                return self.population_count_builtin(t, bits);
            }
            if let Some(bits) = match t.s_value.as_str() {
                "__builtin_bswap16" => Some(16),
                "__builtin_bswap32" => Some(32),
                "__builtin_bswap64" => Some(64),
                _ => None,
            } {
                return self.byte_swap_builtin(t, bits);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin___sprintf_chk" | "__builtin___snprintf_chk"
            ) {
                return self.checked_format_builtin(t);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin___strlcpy_chk" | "__builtin___strlcat_chk"
            ) {
                return self.checked_bsd_string_builtin(t);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin_add_overflow" | "__builtin_sub_overflow"
            ) {
                let operation = if t.s_value == "__builtin_sub_overflow" {
                    NodeType::SubOverflow
                } else {
                    NodeType::AddOverflow
                };
                return self.overflow_builtin(t, operation);
            }
            if matches!(
                t.s_value.as_str(),
                "__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__"
            ) {
                let name = self.predefined_function_name(t);
                return self.postfix(name);
            }
            if t.s_value == "__builtin_expect" {
                return self.branch_expectation_builtin(t);
            }
            if t.s_value == "__builtin_alloca" {
                if self.current_function.is_none() {
                    self.error_logger
                        .print_error_position(t.pos, "alloca requires function scope");
                }
                self.expect_reserved("(");
                let mut size = self.assign();
                if size
                    .resolve_type_view()
                    .is_none_or(|ty| ty.integer_info().is_none())
                {
                    self.error_logger
                        .print_error_position(t.pos, "alloca size must be an integer");
                }
                Self::cast_integer(&mut size, &Type::integer(64, false));
                self.expect_reserved(")");
                return Node {
                    nt: NodeType::StackAlloc,
                    token: Some(t),
                    cty: Some(Type::Ptr(Rc::new(Type::Void))),
                    lhs: Some(Box::new(size)),
                    ..Node::default()
                };
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin___memcpy_chk"
                    | "__builtin___memmove_chk"
                    | "__builtin___memset_chk"
                    | "__builtin_memcpy"
                    | "__builtin_memmove"
                    | "__builtin_memset"
            ) {
                return self.memory_builtin(t);
            }
            if t.s_value == "__builtin_object_size" {
                return self.object_size_builtin(t);
            }
            if t.s_value == "__builtin_offsetof" {
                return self.offsetof_builtin(t);
            }
            if t.s_value.starts_with("__builtin_va_") {
                return self.varargs_builtin(t);
            }
            let thread_local = self
                .global_variables
                .get(t.s_value.as_str())
                .is_some_and(|object| object.thread_local);
            if let Some(ident) = self.resolve_name(&t.s_value) {
                match ident {
                    Identifier::TypeDef(..) | Identifier::EnumTag { .. } => {
                        self.error_logger
                            .print_error_position(t.pos, "type name is not an expression");
                        unreachable!()
                    }
                    Identifier::Constant(value, ty) => Node {
                        cty: Some(ty.clone()),
                        ..Node::new_with_num(Some(t.clone()), *value as usize)
                    },
                    Identifier::Local(ty, offset, registered, alignment) => Node {
                        nt: NodeType::LocalVar,
                        token: Some(t.clone()),
                        cty: Some(ty.clone()),
                        offset: Some(*offset),
                        alignment: *alignment,
                        registered: *registered,
                        ..Node::default()
                    },
                    Identifier::Dynamic(ty, offset, registered) => {
                        let mut node = Self::dynamic_node(ty.clone(), *offset, t.clone());
                        node.registered = *registered;
                        node
                    }
                    Identifier::Static(ty) => Node {
                        nt: NodeType::GlobalVar,
                        token: Some(t.clone()),
                        cty: Some(ty.clone()),
                        global_name: t.s_value.to_string(),
                        thread_local,
                        ..Node::default()
                    },
                    Identifier::StaticObject(ty, name) => Node {
                        nt: NodeType::GlobalVar,
                        token: Some(t.clone()),
                        cty: Some(ty.clone()),
                        global_name: name.clone(),
                        thread_local: self
                            .global_variables
                            .get(name)
                            .is_some_and(|object| object.thread_local),
                        ..Node::default()
                    },
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
                s_value: format!(".suffix.pointer.{}", token.pos).into(),
                ..token.clone()
            },
            Type::Ptr(Rc::new(ty.clone())),
        );
        let previous = self.register_local_variable(
            Token {
                s_value: format!(".suffix.value.{}", token.pos).into(),
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
            token: Some(token.clone()),
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
                        Type::Ptr(child) => Rc::unwrap_or_clone(child).into_function_signature(),
                        _ => ty.into_function_signature(),
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
                for (argument, parameter) in args.iter_mut().rev().zip(&parameter_types) {
                    self.convert_transparent_argument(parameter, argument, &t);
                }
                for argument in &mut args {
                    if matches!(argument.resolve_type(), Some(Type::Struct(_))) {
                        let ty = argument.resolve_type().unwrap();
                        let temporary = self.register_local_variable(
                            Token {
                                s_value: format!(".argument.{}.{}", t.pos, self.offset_size).into(),
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
                    let alignment = return_type.align_of().max(8);
                    self.offset_size = self.offset_size.div_ceil(alignment) * alignment
                        + return_type.size_of().max(1).div_ceil(alignment) * alignment;
                    Some(self.offset_size)
                } else {
                    None
                };
                let direct = matches!(node.nt, NodeType::GlobalVar)
                    && node
                        .resolve_type()
                        .is_some_and(|ty| ty.function_signature_ref().is_some());
                let s_value = if direct {
                    node.global_name.clone()
                } else {
                    String::new()
                };
                let callee = if direct { None } else { Some(Box::new(node)) };
                node = Node {
                    nt: NodeType::CallFunc,
                    token: Some(t),
                    cty: Some(return_type),
                    lhs: callee,
                    global_name: s_value,
                    args,
                    parameter_types,
                    variadic,
                    offset: result_offset,
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
                    node = Self::member_node(node, &st.member(index), token);
                }
            } else if let Some(b_token) = self.attempt_reserved("[") {
                if matches!(
                    node.resolve_type().as_ref().map(Type::unqualified),
                    Some(Type::Vector(..))
                ) {
                    self.error_logger.print_error_position(
                        b_token.pos,
                        "vector element access is not supported",
                    );
                }
                // Subscript array
                node = Node::new_with_op(Some(b_token.clone()), NodeType::Add, node, self.expr());
                self.expect_reserved("]");
                node = Node {
                    nt: NodeType::Deref,
                    token: Some(b_token.clone()),
                    lhs: Some(Box::new(node)),
                    ..Node::default()
                }
            } else if let Some(token) = self.attempt_reserved("++") {
                // Suffix increment
                node = self.suffix_update(token.clone(), node, true);
            } else if let Some(token) = self.attempt_reserved("--") {
                // Suffix decrement
                node = self.suffix_update(token.clone(), node, false);
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
                s_value: "int".to_string().into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Ident,
                pos: 0,
                s_value: "x".to_string().into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                s_value: "=".to_string().into(),
                i_value: 0,
                ..Token::default()
            },
            Token {
                tt: TokenType::Num,
                pos: 0,
                s_value: String::new().into(),
                i_value: 42,
                ..Token::default()
            },
            Token {
                tt: TokenType::Reserved,
                pos: 0,
                s_value: ";".to_string().into(),
                i_value: 0,
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
                    weak_import: false,
                    hidden: false,
                    common: false,
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

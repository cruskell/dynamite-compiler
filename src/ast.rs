use crate::ctype::{
    BitField, EnumType, Qualifiers, StructDefinition, StructMember, StructType, Type,
};
use crate::error;
use crate::func::Func;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::node::{AtomicOperation, Node, NodeType};
use crate::token::{Token, TokenType};
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

#[path = "atomic.rs"]
mod atomic;
#[path = "inline_asm.rs"]
mod inline_asm;
#[path = "inline_calls.rs"]
mod inline_calls;
#[path = "object_size.rs"]
mod object_size;
#[path = "overflow.rs"]
mod overflow;
#[path = "prefetch.rs"]
mod prefetch;
#[path = "statement_expression.rs"]
mod statement_expression;
#[path = "transparent_union.rs"]
mod transparent_union;
#[path = "type_query.rs"]
mod type_query;
#[path = "varargs.rs"]
mod varargs;
#[path = "vla.rs"]
mod vla;

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

#[derive(Clone, Default)]
struct DeclarationAttributes {
    gnu_inline: bool,
    noinline: bool,
    weak_import: bool,
    common: bool,
    typedef_declarator: bool,
    transparent_union: bool,
    vector_size: Option<usize>,
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
    current_function: Option<Token>,
    mutable_static: Option<Token>,
    return_pointer: Option<usize>,
    variadic_parameters: Option<Vec<Type>>,
    variadic_last: Option<usize>,
    pending_bounds: Vec<Node>,
    parameter_depth: usize,
    declaration_attributes_state: Option<DeclarationAttributes>,
    dynamic_objects: Vec<Node>,
    vla_active: Vec<usize>,
    jump_depths: Vec<usize>,
    continue_depths: HashMap<usize, usize>,
    switch_vla_depths: Vec<usize>,
    next_enum_id: usize,
    next_deferred_local: usize,
    struct_definitions: Vec<Rc<RefCell<StructDefinition>>>,
    scope_stack: Vec<HashMap<String, Identifier>>,
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
    Constant(i64, Type),
    EnumTag {
        ty: Type,
        fixed: bool,
        defined: bool,
    },
    StaticObject(Type, String),
    Local(Type, usize, bool, usize),
    Dynamic(Type, usize, bool),
    Static(Type),
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
        while !self.at_eof() {
            if self.attempt_reserved(";").is_none() {
                self.global_definition();
            }
        }
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
        let mut builder = Self {
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
            current_function: None,
            mutable_static: None,
            return_pointer: None,
            variadic_parameters: None,
            variadic_last: None,
            pending_bounds: vec![],
            parameter_depth: 0,
            declaration_attributes_state: None,
            dynamic_objects: vec![],
            vla_active: vec![],
            jump_depths: vec![],
            continue_depths: HashMap::new(),
            switch_vla_depths: vec![],
            next_enum_id: 0,
            next_deferred_local: usize::MAX / 2,
            struct_definitions: vec![],
            scope_stack: vec![reserved_functions()],
            functions: HashMap::new(),
            global_variables: HashMap::new(),
            string_literals: Vec::new(),
            symbol_names: HashMap::new(),
        };
        let va_list = builder.builtin_va_list_type();
        builder.scope_stack[0].insert("__builtin_va_list".into(), Identifier::TypeDef(va_list));
        builder
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
    fn attempt_inline(&mut self) -> Option<Token> {
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
                    && !matches!(child.unqualified(), Type::Ptr(pointee) if pointee.function_signature_ref().is_none())
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
                if !child.is_complete_object() {
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
                for ty in args.iter().chain(std::iter::once(result.as_ref())) {
                    if matches!(ty.unqualified(), Type::Struct(_))
                        && ty.contains_reduced_alignment()
                    {
                        self.error_logger.print_error_position(token.pos, "records containing reduced-alignment typedefs cannot be passed or returned by value");
                    }
                    if ty.contains_vector() {
                        self.error_logger.print_error_position(token.pos, "vectors and records containing vectors cannot be passed or returned by value");
                    }
                    if matches!(ty.unqualified(), Type::Integer128 { .. }) {
                        self.error_logger.print_error_position(
                            token.pos,
                            "128-bit integer parameters and results are not supported",
                        );
                    }
                    if matches!(ty.unqualified(), Type::Struct(_)) && ty.align_of() > 16 {
                        self.error_logger.print_error_position(
                            token.pos,
                            "records aligned above 16 bytes cannot be passed or returned by value",
                        );
                    }
                    if matches!(ty.unqualified(), Type::Struct(_)) && ty.contains_float16() {
                        self.error_logger.print_error_position(
                            token.pos,
                            "records containing _Float16 cannot be passed or returned by value",
                        );
                    }
                    self.validate_type_layout(ty, token);
                }
            }
            _ => {}
        }
        if ty.checked_size_of().is_none() {
            self.error_logger
                .print_error_position(token.pos, "array size is too large");
        }
    }

    fn assembler_label(&mut self, name: &Token) {
        if !matches!(
            self.tokens[self.cur].s_value.as_str(),
            "asm" | "__asm" | "__asm__"
        ) {
            return;
        }
        self.cur += 1;
        self.expect_reserved("(");
        let label = self.attempt(TokenType::Str).unwrap_or_else(|| {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "assembler label requires a string literal",
            );
            unreachable!()
        });
        let value = std::str::from_utf8(&label.string_bytes).unwrap_or("");
        if label.string_encoding != crate::token::StringEncoding::Narrow
            || value.is_empty()
            || value == "."
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.$".contains(&byte))
        {
            self.error_logger.print_error_position(
                label.pos,
                "assembler label requires an identifier-like symbol",
            );
        }
        self.expect_reserved(")");
        if let Some(previous) = self.symbol_names.get(&name.s_value) {
            if previous != value {
                self.error_logger
                    .print_error_position(label.pos, "conflicting assembler labels");
            }
        }
        self.symbol_names
            .insert(name.s_value.clone(), value.to_owned());
        self.declaration_annotations();
    }

    fn declaration_annotations(&mut self) {
        self.declaration_attributes(false, None, false);
    }

    fn declaration_attributes(
        &mut self,
        allow_alignment: bool,
        mut packed: Option<&mut bool>,
        stop_before_alignment: bool,
    ) -> Vec<usize> {
        let mut alignments = vec![];
        while matches!(self.tokens[self.cur].tt, TokenType::Ident)
            && matches!(
                self.tokens[self.cur].s_value.as_str(),
                "__attribute__" | "__attribute"
            )
        {
            // Qualifier parsing leaves semantic alignment attributes to the declaration owner.
            if stop_before_alignment {
                let mut depth = 0usize;
                let mut aligned = false;
                for token in &self.tokens[self.cur + 1..] {
                    match token.s_value.as_str() {
                        "(" => depth += 1,
                        ")" if depth > 0 => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ if depth == 2 && token.s_value.trim_matches('_') == "aligned" => {
                            aligned = true
                        }
                        _ => {}
                    }
                }
                if aligned {
                    break;
                }
            }
            let token = self.tokens[self.cur].clone();
            self.cur += 1;
            self.expect_reserved("(");
            self.expect_reserved("(");
            while self.attempt_reserved(")").is_none() {
                if self.attempt_reserved(",").is_some() {
                    continue;
                }
                let name = self.tokens[self.cur].clone();
                if !matches!(name.tt, TokenType::Ident | TokenType::Reserved) {
                    self.error_logger
                        .print_error_position(name.pos, "attribute name expected");
                }
                if name.s_value.trim_matches('_') == "noinline" {
                    if self.parameter_depth != 0
                        || self.declaration_attributes_state.is_none()
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|a| a.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "noinline requires a file-scope function declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "noinline does not take arguments");
                    }
                    self.declaration_attributes_state.as_mut().unwrap().noinline = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "gnu_inline" {
                    if self.parameter_depth != 0
                        || self.declaration_attributes_state.is_none()
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "gnu_inline requires a file-scope inline function declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "gnu_inline does not take arguments");
                    }
                    self.declaration_attributes_state
                        .as_mut()
                        .unwrap()
                        .gnu_inline = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "weak_import" {
                    if self.parameter_depth != 0
                        || self.declaration_attributes_state.is_none()
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "weak_import requires a file-scope declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "weak_import does not take arguments");
                    }
                    self.declaration_attributes_state
                        .as_mut()
                        .unwrap()
                        .weak_import = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "visibility" {
                    self.cur += 1;
                    self.expect_reserved("(");
                    let visibility = self.attempt(TokenType::Str).unwrap_or_else(|| {
                        self.error_logger
                            .print_error_position(name.pos, "visibility requires a string literal");
                        unreachable!()
                    });
                    if visibility.string_encoding.unit_size() != 1
                        || visibility.string_bytes != b"default"
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "only default symbol visibility is supported",
                        );
                    }
                    self.expect_reserved(")");
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "transparent_union" {
                    if packed.is_none()
                        && !self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "transparent_union requires a union type or typedef",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger.print_error_position(
                            name.pos,
                            "transparent_union does not take arguments",
                        );
                    }
                    self.declaration_attributes_state
                        .get_or_insert_with(DeclarationAttributes::default)
                        .transparent_union = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "common" {
                    if self.parameter_depth != 0
                        || self
                            .declaration_attributes_state
                            .as_ref()
                            .is_none_or(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "common requires a file-scope object declaration",
                        );
                    }
                    self.cur += 1;
                    if self.tokens[self.cur].s_value == "(" {
                        self.error_logger
                            .print_error_position(name.pos, "common does not take arguments");
                    }
                    self.declaration_attributes_state.as_mut().unwrap().common = true;
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "vector_size" {
                    if self.parameter_depth != 0
                        || !self
                            .declaration_attributes_state
                            .as_ref()
                            .is_some_and(|attributes| attributes.typedef_declarator)
                    {
                        self.error_logger.print_error_position(
                            name.pos,
                            "vector_size is supported only on scalar typedef declarators",
                        );
                    }
                    self.cur += 1;
                    self.expect_reserved("(");
                    let mut attributes = self.declaration_attributes_state.take().unwrap();
                    let size = self.ternary();
                    if !Self::integer_constant(&size) {
                        self.error_logger.print_error_position(
                            name.pos,
                            "vector size requires an integer constant expression",
                        );
                    }
                    let size = self.eval(&size);
                    if size <= 0 || size > 16 || !(size as usize).is_power_of_two() {
                        self.error_logger.print_error_position(
                            name.pos,
                            "supported vector sizes are powers of two through 16 bytes",
                        );
                    }
                    self.expect_reserved(")");
                    let previous = attributes.vector_size.replace(size as usize);
                    self.declaration_attributes_state = Some(attributes);
                    if previous.is_some_and(|previous| previous != size as usize) {
                        self.error_logger
                            .print_error_position(name.pos, "conflicting vector sizes");
                    }
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if name.s_value.trim_matches('_') == "may_alias"
                    && self.tokens[self.cur + 1].s_value == "("
                {
                    self.error_logger
                        .print_error_position(name.pos, "may_alias does not take arguments");
                }
                let supported = METADATA_ATTRIBUTES.contains(&name.s_value.trim_matches('_'));
                if name.s_value.trim_matches('_') == "packed" {
                    if let Some(packed) = packed.as_deref_mut() {
                        self.cur += 1;
                        if self.tokens[self.cur].s_value == "(" {
                            self.error_logger.print_error_position(
                                name.pos,
                                "packed attribute does not take arguments",
                            );
                        }
                        *packed = true;
                        if self.tokens[self.cur].s_value != ")" {
                            self.expect_reserved(",");
                        }
                        continue;
                    }
                }
                if allow_alignment && name.s_value.trim_matches('_') == "aligned" {
                    self.cur += 1;
                    let value = if self.attempt_reserved("(").is_some() {
                        let expression = self.ternary();
                        if !Self::integer_constant(&expression) {
                            self.error_logger.print_error_position(
                                name.pos,
                                "alignment requires an integer constant expression",
                            );
                        }
                        let value = self.eval(&expression);
                        self.expect_reserved(")");
                        value
                    } else {
                        16
                    };
                    if value <= 0 || !(value as usize).is_power_of_two() || value > 4096 {
                        self.error_logger.print_error_position(
                            name.pos,
                            "alignment must be a power of two no greater than 4096",
                        );
                    }
                    alignments.push(value as usize);
                    if self.tokens[self.cur].s_value != ")" {
                        self.expect_reserved(",");
                    }
                    continue;
                }
                if !supported {
                    self.error_logger
                        .print_error_position(name.pos, "unsupported declaration attribute");
                }
                self.cur += 1;
                if self.attempt_reserved("(").is_some() {
                    let mut depth = 1;
                    while depth != 0 {
                        if self.tokens[self.cur].tt == TokenType::EOF {
                            self.error_logger.print_error_position(
                                token.pos,
                                "unterminated attribute arguments",
                            );
                        }
                        if self.attempt_reserved("(").is_some() {
                            depth += 1;
                        } else if self.attempt_reserved(")").is_some() {
                            depth -= 1;
                        } else {
                            self.cur += 1;
                        }
                    }
                }
                if self.tokens[self.cur].s_value != ")" {
                    self.expect_reserved(",");
                }
            }
            self.expect_reserved(")");
        }
        alignments
    }

    fn object_alignment_annotations(&mut self) -> usize {
        let mut alignment = 0;
        loop {
            alignment = alignment.max(
                self.declaration_attributes(true, None, false)
                    .into_iter()
                    .max()
                    .unwrap_or(0),
            );
            let Some(value) = self.attempt_alignment_specifier() else {
                break;
            };
            alignment = alignment.max(value);
        }
        alignment
    }

    fn record_attributes(&mut self) -> (usize, bool, bool) {
        let mut packed = false;
        let alignment = self
            .declaration_attributes(true, Some(&mut packed), false)
            .into_iter()
            .max()
            .unwrap_or(0);
        let transparent = self
            .declaration_attributes_state
            .as_mut()
            .is_some_and(|attributes| std::mem::take(&mut attributes.transparent_union));
        (alignment, packed, transparent)
    }

    fn declarator(&mut self) -> Declarator {
        self.declarator_with_alignment(None)
    }

    fn declarator_with_alignment(&mut self, mut alignments: Option<&mut Vec<usize>>) -> Declarator {
        if let Some(alignments) = alignments.as_deref_mut() {
            alignments.extend(self.declaration_attributes(true, None, false));
        } else {
            self.declaration_annotations();
        }
        let mut pointers = vec![];
        while self.attempt_reserved("*").is_some() {
            pointers.push(self.pointer_qualifiers());
        }
        let mut direct = if self.attempt_reserved("(").is_some() {
            let inner = self.declarator_with_alignment(alignments.as_deref_mut());
            self.expect_reserved(")");
            inner
        } else {
            Declarator::Name(self.attempt_ident())
        };
        loop {
            if let Some(alignments) = alignments.as_deref_mut() {
                alignments.extend(self.declaration_attributes(true, None, false));
            } else {
                self.declaration_annotations();
            }
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
                    direct = Declarator::Array(Box::new(direct), None);
                } else {
                    let token = self.tokens[self.cur].clone();
                    let expression = self.assign();
                    self.expect_reserved("]");
                    if Self::integer_constant(&expression) {
                        let size = self.eval(&expression);
                        if size < 0 || (size == 0 && minimum.is_some()) {
                            self.error_logger
                                .print_error_position(token.pos, "array size must be positive");
                        }
                        direct = Declarator::Array(Box::new(direct), Some(size as usize));
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
                                    ty.align_of(),
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
                        matches!(
                            identifier,
                            Identifier::TypeDef(_)
                                | Identifier::Constant(..)
                                | Identifier::EnumTag { .. }
                        )
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

    fn expect_ident_with_type(
        &mut self,
        ty: Type,
        alignments: Option<&mut Vec<usize>>,
    ) -> (Token, Type) {
        let (name, ty, _, _, _) = self.declarator_with_alignment(alignments).resolve(ty);
        let token = name.unwrap_or_else(|| {
            self.error_logger
                .print_error_position(self.tokens[self.cur].pos, "ident expected");
            unreachable!()
        });
        self.validate_type_layout(&ty, &token);
        (token, ty)
    }
    fn new_local_variable(&mut self, ty: Type, alignment: usize) -> (Node, bool) {
        let mut alignments = vec![alignment];
        let (token, ty) = self.expect_ident_with_type(ty, Some(&mut alignments));
        let alignment = alignments.into_iter().max().unwrap_or(0);
        if matches!(ty, Type::Arr(_, None)) {
            if self.tokens[self.cur].s_value != "=" {
                self.error_logger.print_error_position(
                    token.pos,
                    "incomplete automatic array requires an initializer",
                );
            }
            (
                self.register_incomplete_local_array(token, ty, alignment),
                true,
            )
        } else {
            (
                self.register_local_variable_aligned(token, ty, alignment),
                false,
            )
        }
    }

    fn register_incomplete_local_array(
        &mut self,
        token: Token,
        ty: Type,
        requested_alignment: usize,
    ) -> Node {
        let alignment = ty.align_of().max(requested_alignment);
        if alignment > 16 {
            self.error_logger.print_error_position(
                token.pos,
                "requested local alignment exceeds the supported stack alignment",
            );
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
        // Reserve an identity while expressions are parsed; allocate the completed object later.
        let offset = self.next_deferred_local;
        self.next_deferred_local -= 1;
        self.scope_stack.last_mut().unwrap().insert(
            token.s_value.clone(),
            Identifier::Local(ty.clone(), offset, false, alignment),
        );
        Node {
            nt: NodeType::LocalVar,
            token: Some(token),
            cty: Some(ty),
            offset: Some(offset),
            alignment,
            ..Node::default()
        }
    }

    fn resolve_deferred_local(node: &mut Node, offset: usize, object: &Node) {
        if node.nt == NodeType::LocalVar && node.offset == Some(offset) {
            node.offset = object.offset;
            node.cty = object.cty.clone();
        }
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
            Self::resolve_deferred_local(child, offset, object);
        }
        for child in node.children.iter_mut().chain(&mut node.args) {
            Self::resolve_deferred_local(child, offset, object);
        }
    }

    fn inferred_local_array_initializer(&mut self, object: &Node, token: &Token) -> (Node, Node) {
        if self.tokens[self.cur].s_value != "{"
            && self
                .string_initializer_index(&object.resolve_type().unwrap())
                .is_none()
        {
            self.error_logger.print_error_position(
                token.pos,
                "array initializer requires braces or a string literal",
            );
        }
        let mut count = 0;
        let mut initializer =
            self.local_variable_initialization_with_count(object, token, Some(&mut count));
        let ty = self.infer_array_size(object.cty.clone().unwrap(), count, token);
        let name = object.token.clone().unwrap();
        self.scope_stack.last_mut().unwrap().remove(&name.s_value);
        let mut completed = self.register_local_variable_aligned(name, ty, object.alignment);
        if object.registered {
            self.mark_registered(&mut completed);
        }
        Self::resolve_deferred_local(&mut initializer, object.offset.unwrap(), &completed);
        // Zero omitted elements after the final bound is known, before evaluating any initializer.
        initializer
            .children
            .insert(0, Self::zero_initializer(&completed, token));
        (completed, initializer)
    }

    fn infer_array_size(&self, ty: Type, count: usize, token: &Token) -> Type {
        if let Type::Arr(child, None) = ty {
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
            Type::Arr(child, Some(count))
        } else {
            ty
        }
    }

    fn string_initializer_index(&self, ty: &Type) -> Option<usize> {
        let index = self.cur + usize::from(self.tokens[self.cur].s_value == "{");
        let token = self.tokens.get(index)?;
        if token.tt == TokenType::Str
            && matches!(ty.unqualified(), Type::Arr(child, _)
                if child.integer_info().is_some()
                    && child.size_of() == token.string_encoding.unit_size())
        {
            Some(index)
        } else {
            None
        }
    }

    fn attempt_string_initializer(&mut self, ty: &Type) -> Option<Token> {
        let index = self.string_initializer_index(ty)?;
        let braced = index != self.cur;
        self.cur = index;
        let token = self.attempt(TokenType::Str).unwrap();
        if braced {
            self.attempt_reserved(",");
            self.expect_reserved("}");
        }
        Some(token)
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
        // Distinct internal slots preserve each zero-sized object's type and alignment.
        self.offset_size += ty.size_of().max(1);
        self.offset_size = self.offset_size.div_ceil(alignment) * alignment;
        if self.scope_stack.last().unwrap().contains_key(&t.s_value) {
            self.error_logger
                .print_error_position(t.pos, "invalid redeclaration");
        }
        self.scope_stack.last_mut().unwrap().insert(
            t.s_value.clone(),
            Identifier::Local(ty.clone(), self.offset_size, false, alignment),
        );
        Node {
            token: Some(t),
            nt: NodeType::LocalVar,
            alignment,
            cty: Some(ty),
            offset: Some(self.offset_size),
            ..Node::default()
        }
    }
    fn mark_registered(&mut self, node: &mut Node) {
        let name = &node.token.as_ref().unwrap().s_value;
        match self.scope_stack.last_mut().unwrap().get_mut(name).unwrap() {
            Identifier::Local(_, _, registered, _) | Identifier::Dynamic(_, _, registered) => {
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
            if !parameter {
                alignment = alignment.max(self.object_alignment_annotations());
            }
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
        if !parameter {
            alignment = alignment.max(self.object_alignment_annotations());
        }
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

    fn nullability(&self) -> Option<Nullability> {
        match self.tokens[self.cur].s_value.as_str() {
            "_Nullable" => Some(Nullability::Nullable),
            "_Nonnull" => Some(Nullability::Nonnull),
            "_Null_unspecified" => Some(Nullability::Unspecified),
            "_Nullable_result" => Some(Nullability::NullableResult),
            _ => None,
        }
    }

    fn pointer_qualifiers(&mut self) -> Qualifiers {
        self.qualifiers_with_nullability(&mut None)
    }

    fn qualifiers_with_nullability(&mut self, nullability: &mut Option<Nullability>) -> Qualifiers {
        let mut qualifiers = Qualifiers::default();
        loop {
            let current = self.attempt_qualifiers();
            qualifiers.constant |= current.constant;
            qualifiers.volatile |= current.volatile;
            qualifiers.restricted |= current.restricted;
            let Some(annotation) = self.nullability() else {
                break;
            };
            if nullability.is_some_and(|previous| previous != annotation) {
                self.error_logger.print_error_position(
                    self.tokens[self.cur].pos,
                    "conflicting pointer nullability annotations",
                );
            }
            *nullability = Some(annotation);
            self.cur += 1;
        }
        qualifiers
    }

    fn attempt_qualifiers(&mut self) -> Qualifiers {
        let mut q = Qualifiers::default();
        loop {
            self.declaration_attributes(false, None, true);
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
            ) || !ty.is_complete_object()
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
        let mut nullability = None;
        let mut q = self.qualifiers_with_nullability(&mut nullability);
        let Some(ty) = self.attempt_unqualified_type(&mut q) else {
            self.cur = saved;
            return None;
        };
        let suffix = if matches!(ty.unqualified(), Type::Ptr(_)) {
            self.qualifiers_with_nullability(&mut nullability)
        } else {
            let qualifiers = self.attempt_qualifiers();
            if nullability.is_some() || self.nullability().is_some() {
                let position = if nullability.is_some() {
                    saved
                } else {
                    self.cur
                };
                self.error_logger.print_error_position(
                    self.tokens[position].pos,
                    "nullability requires a pointer type",
                );
            }
            qualifiers
        };
        q.constant |= suffix.constant;
        q.volatile |= suffix.volatile;
        q.restricted |= suffix.restricted;
        if q.restricted
            && !matches!(ty.unqualified(), Type::Ptr(pointee) if pointee.function_signature_ref().is_none())
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
    fn long_double_type(&self, token: &Token) -> Type {
        if !cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            self.error_logger.print_error_position(
                token.pos,
                "long double is only supported on targets with a binary64 long double ABI",
            );
        }
        Type::LongDouble(64)
    }

    fn typeof_type(&mut self, token: Token) -> Type {
        self.expect_reserved("(");
        let pending_start = self.pending_bounds.len();
        let (ty, expression) = if let Some(base) = self.attempt_type() {
            let (name, ty, _, _, _) = self.declarator().resolve(base);
            if name.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "typeof requires a type name or expression");
            }
            (ty, None)
        } else {
            let expression = self.expr();
            self.validate_void_expressions(&expression);
            if expression.bit_field.is_some() {
                self.error_logger
                    .print_error_position(token.pos, "typeof cannot be applied to a bit-field");
            }
            (
                expression.resolve_qualified_type().unwrap(),
                Some(expression),
            )
        };
        self.expect_reserved(")");
        if ty.is_variably_modified() {
            if let Some(expression) = expression {
                self.pending_bounds.push(expression);
            }
        } else {
            self.pending_bounds.truncate(pending_start);
        }
        ty
    }

    fn attempt_unqualified_type(&mut self, qualifiers: &mut Qualifiers) -> Option<Type> {
        if self.tokens[self.cur].tt != TokenType::Reserved {
            if self.tokens[self.cur].tt == TokenType::Ident {
                let name = self.tokens[self.cur].s_value.clone();
                if let Some(Identifier::TypeDef(ty)) = self.resolve_name(&name) {
                    let ty = ty.clone();
                    self.cur += 1;
                    return Some(ty);
                }
            }
            return None;
        }
        if let Some(token) = self
            .attempt_reserved("typeof")
            .or_else(|| self.attempt_reserved("__typeof"))
            .or_else(|| self.attempt_reserved("__typeof__"))
        {
            return Some(self.typeof_type(token));
        }
        if let Some(token) = self.attempt_reserved("enum") {
            return Some(self.enum_type(token));
        }
        if let Some(token) = self
            .attempt_reserved("struct")
            .or_else(|| self.attempt_reserved("union"))
        {
            return Some(self.struct_type(token));
        }
        let mut specifiers = vec![];
        loop {
            let q = self.attempt_qualifiers();
            qualifiers.constant |= q.constant;
            qualifiers.volatile |= q.volatile;
            qualifiers.restricted |= q.restricted;
            let token = &self.tokens[self.cur];
            if token.tt != TokenType::Reserved
                || !crate::tokenizer::TYPES.contains(&token.s_value.as_str())
                || matches!(token.s_value.as_str(), "typeof" | "__typeof" | "__typeof__")
            {
                break;
            }
            specifiers.push(token.clone());
            self.cur += 1;
        }
        if specifiers.is_empty() {
            return None;
        }
        let count = |name: &str| {
            specifiers
                .iter()
                .filter(|token| token.s_value == name)
                .count()
        };
        let signs = count("signed") + count("unsigned");
        if signs > 1 {
            self.error_logger
                .print_error_position(specifiers[0].pos, "duplicate signedness");
        }
        let signed = count("unsigned") == 0;
        let ints = count("int");
        let longs = count("long");
        let others = specifiers.len() - signs - ints - longs;
        let ty = if others == 0 && ints <= 1 && longs <= 2 {
            Some(if longs == 2 {
                Type::LongLong { signed }
            } else {
                Type::integer(if longs == 1 { 64 } else { 32 }, signed)
            })
        } else if others == 1 && count("char") == 1 && ints == 0 && longs == 0 {
            Some(if signed && signs != 0 {
                Type::SignedChar
            } else {
                Type::integer(8, signed)
            })
        } else if others == 1 && count("short") == 1 && ints <= 1 && longs == 0 {
            Some(Type::integer(16, signed))
        } else if others == 1 && count("__int128") == 1 && ints == 0 && longs == 0 {
            Some(Type::Integer128 { signed })
        } else if others == 1 && count("double") == 1 && ints == 0 && signs == 0 && longs <= 1 {
            Some(if longs == 1 {
                self.long_double_type(&specifiers[0])
            } else {
                Type::Float(64)
            })
        } else if specifiers.len() == 1 {
            match specifiers[0].s_value.as_str() {
                "void" => Some(Type::Void),
                "bool" | "_Bool" => Some(Type::Bool),
                "float" => Some(Type::Float(32)),
                "_Float16" => Some(Type::Float16),
                "__int128_t" => Some(Type::Integer128 { signed: true }),
                "__uint128_t" => Some(Type::Integer128 { signed: false }),
                _ => None,
            }
        } else {
            None
        };
        if ty.is_none() {
            self.error_logger
                .print_error_position(specifiers[0].pos, "invalid combination of type specifiers");
        }
        ty
    }
    fn enum_integer_type(&self, minimum: i128, maximum: i128, token: &Token) -> Type {
        if minimum >= i128::from(i32::MIN) && maximum <= i128::from(i32::MAX) {
            Type::I32
        } else if minimum >= 0 && maximum <= i128::from(u32::MAX) {
            Type::integer(32, false)
        } else if minimum >= i128::from(i64::MIN) && maximum <= i128::from(i64::MAX) {
            Type::integer(64, minimum < 0)
        } else if minimum >= 0 && maximum <= i128::from(u64::MAX) {
            Type::integer(64, false)
        } else {
            self.error_logger.print_error_position(
                token.pos,
                "enumerator range exceeds supported integer types",
            );
            unreachable!()
        }
    }

    fn enum_type(&mut self, token: Token) -> Type {
        let attributes = self.declaration_attributes_state.take();
        let result = self.enum_type_body(token);
        self.declaration_attributes_state = attributes;
        result
    }

    fn enum_type_body(&mut self, token: Token) -> Type {
        self.declaration_annotations();
        let name = self.attempt_ident();
        self.declaration_annotations();
        let key = name.as_ref().map(|name| format!(".enum.{}", name.s_value));
        // A generic association also places ':' immediately after an enum type name.
        let underlying_follows = self.tokens.get(self.cur + 1).is_some_and(|token| {
            crate::tokenizer::TYPES.contains(&token.s_value.as_str())
                || matches!(
                    token.s_value.as_str(),
                    "struct"
                        | "union"
                        | "enum"
                        | "const"
                        | "volatile"
                        | "restrict"
                        | "__restrict"
                        | "__restrict__"
                        | "__attribute__"
                )
                || matches!(
                    self.resolve_name(&token.s_value),
                    Some(Identifier::TypeDef(_))
                )
        });
        let underlying = if self.tokens[self.cur].s_value == ":" && underlying_follows {
            self.cur += 1;
            let position = self.tokens[self.cur].pos;
            let ty = self.expect_type().unqualified().clone();
            if ty.integer_info().is_none() || matches!(ty, Type::Enum(_)) {
                self.error_logger.print_error_position(
                    position,
                    "enum underlying type must be a supported integer type",
                );
            }
            Some(ty)
        } else {
            None
        };
        let defining = self.tokens[self.cur].s_value == "{";
        let previous = key.as_ref().and_then(|key| {
            let binding = if defining || underlying.is_some() {
                self.scope_stack.last().unwrap().get(key)
            } else {
                self.resolve_name(key)
            };
            if let Some(Identifier::EnumTag { ty, fixed, defined }) = binding {
                Some((ty.clone(), *fixed, *defined))
            } else {
                None
            }
        });
        if !defining && underlying.is_none() {
            return previous.map(|(ty, _, _)| ty).unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(token.pos, "enum requires a known tag or definition");
                unreachable!()
            });
        }
        if !defining && name.is_none() {
            self.error_logger
                .print_error_position(token.pos, "enum declaration requires a tag or definition");
        }
        let fixed = underlying.is_some();
        let previously_defined = previous.as_ref().is_some_and(|(_, _, defined)| *defined);
        let ty = underlying.unwrap_or(Type::I32);
        let enum_type = if let Some((previous, previous_fixed, defined)) = previous {
            if defining && defined {
                self.error_logger
                    .print_error_position(token.pos, "enum tag is already defined");
            }
            if previous_fixed != fixed {
                self.error_logger.print_error_position(
                    token.pos,
                    "enum redeclaration changes whether its underlying type is fixed",
                );
            }
            let Type::Enum(value) = &previous else {
                unreachable!()
            };
            if *value.underlying.borrow() != ty {
                self.error_logger.print_error_position(
                    token.pos,
                    "enum redeclaration has a different underlying type",
                );
            }
            previous
        } else {
            let value = Type::Enum(EnumType {
                id: self.next_enum_id,
                underlying: Rc::new(RefCell::new(ty.clone())),
            });
            self.next_enum_id += 1;
            value
        };
        if let Some(key) = &key {
            self.scope_stack.last_mut().unwrap().insert(
                key.clone(),
                Identifier::EnumTag {
                    ty: enum_type.clone(),
                    fixed,
                    defined: defining || previously_defined,
                },
            );
        }
        if !defining {
            return enum_type;
        }
        self.expect_reserved("{");
        let (bits, signed) = ty.integer_info().unwrap();
        let (minimum, maximum) = if !fixed {
            (i128::from(i64::MIN), i128::from(u64::MAX))
        } else if ty == Type::Bool {
            (0, 1)
        } else if signed {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        } else {
            (0, (1i128 << bits) - 1)
        };
        let mut value = 0i128;
        let mut smallest = i128::MAX;
        let mut largest = i128::MIN;
        let mut enumerators = vec![];
        loop {
            let name = self.attempt_ident().unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "enumerator expected");
                unreachable!()
            });
            self.declaration_annotations();
            if self.attempt_reserved("=").is_some() {
                let expression = self.ternary();
                if !Self::integer_constant(&expression) {
                    self.error_logger.print_error_position(
                        name.pos,
                        "enumerator requires an integer constant expression",
                    );
                }
                let result = self.eval(&expression);
                value = if expression
                    .resolve_type()
                    .unwrap()
                    .integer_info()
                    .is_some_and(|(_, signed)| !signed)
                {
                    i128::from(result as u64)
                } else {
                    i128::from(result)
                };
            }
            if value < minimum || value > maximum {
                self.error_logger.print_error_position(
                    name.pos,
                    "enumerator is outside the underlying type range",
                );
            }
            if self.scope_stack.last().unwrap().contains_key(&name.s_value) {
                self.error_logger
                    .print_error_position(name.pos, "duplicate enumerator");
            }
            smallest = smallest.min(value);
            largest = largest.max(value);
            let constant_type = if fixed {
                enum_type.clone()
            } else {
                self.enum_integer_type(value, value, &name)
            };
            enumerators.push((name.s_value.clone(), value as i64));
            self.scope_stack.last_mut().unwrap().insert(
                name.s_value,
                Identifier::Constant(value as i64, constant_type),
            );
            value += 1;
            if self.attempt_reserved("}").is_some() {
                break;
            }
            self.expect_reserved(",");
            if self.attempt_reserved("}").is_some() {
                break;
            }
        }
        if fixed {
            return enum_type;
        }
        let ty = self.enum_integer_type(smallest, largest, &token);
        let scope = self.scope_stack.last_mut().unwrap();
        for (name, value) in enumerators {
            scope.insert(name, Identifier::Constant(value, ty.clone()));
        }
        if let Some(key) = key {
            scope.insert(
                key,
                Identifier::EnumTag {
                    ty: enum_type.clone(),
                    fixed: false,
                    defined: true,
                },
            );
        }
        let Type::Enum(value) = &enum_type else {
            unreachable!()
        };
        // The supported LP64 ABIs use unsigned int for nonnegative 32-bit enums.
        // Enumerator constants keep their separately selected source-language type.
        *value.underlying.borrow_mut() = if smallest >= 0 && largest <= i128::from(u32::MAX) {
            Type::integer(32, false)
        } else {
            ty
        };
        enum_type
    }

    fn struct_type(&mut self, token: Token) -> Type {
        let attributes = self.declaration_attributes_state.take();
        let result = self.struct_type_body(token);
        self.declaration_attributes_state = attributes;
        result
    }

    fn struct_type_body(&mut self, token: Token) -> Type {
        let is_union = token.s_value == "union";
        let packing = token.packing;
        let (mut requested_record_alignment, mut packed, mut transparent) =
            self.record_attributes();
        let name = self.attempt_ident();
        self.declaration_attributes(false, None, true);
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
        if ty.definition().borrow().complete && requested_record_alignment != 0 {
            self.error_logger.print_error_position(
                token.pos,
                "alignment on an existing record reference is not supported",
            );
        }
        if ty.definition().borrow().is_union != is_union {
            self.error_logger
                .print_error_position(token.pos, "aggregate tag has a different kind");
        }
        let inherited_record_alignment = ty.definition().borrow().align;
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
            let mut align = ty.definition().borrow().align.max(1);
            let mut natural_align = 1;
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
                    let mut member_alignments = vec![];
                    let (name, member_type, _, _, _) = self
                        .declarator_with_alignment(Some(&mut member_alignments))
                        .resolve(base.clone());
                    let requested_alignment = requested_alignment
                        .max(member_alignments.iter().copied().max().unwrap_or(0));
                    let position = name.as_ref().unwrap_or(&token);
                    self.validate_type_layout(&member_type, position);
                    if member_type.is_variably_modified() {
                        self.error_logger.print_error_position(
                            position.pos,
                            "struct member cannot have a variably modified type",
                        );
                    }
                    let flexible = matches!(member_type.unqualified(), Type::Arr(_, None));
                    let member_alignment = member_type
                        .align_of()
                        .max(requested_alignment)
                        .min(packing.unwrap_or(usize::MAX));
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
                        if !member_alignments.is_empty() {
                            self.error_logger.print_error_position(
                                position.pos,
                                "alignment attributes on bit-fields are not supported",
                            );
                        }
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
                            natural_align = natural_align.max(member_alignment);
                        }
                        if width == 0
                            || (packing.is_none()
                                && usize::from(width) > container - bit_address % container)
                        {
                            bit_address = bit_address.div_ceil(container) * container;
                        }
                        // AAPCS64 zero-width fields retain natural alignment on Linux.
                        if name.is_some() || cfg!(target_arch = "aarch64") {
                            let field_alignment = if width == 0
                                && cfg!(all(target_arch = "aarch64", target_os = "linux"))
                            {
                                member_type.align_of()
                            } else {
                                member_alignment
                            };
                            align = align.max(field_alignment);
                            natural_align = natural_align.max(member_type.align_of());
                        }
                        let bytewise = packing.is_some();
                        let field = BitField {
                            width,
                            shift: (bit_address % if bytewise { 8 } else { container }) as u8,
                            bytewise,
                        };
                        let offset = if bytewise {
                            bit_address / 8
                        } else {
                            bit_address / container * member_type.size_of()
                        };
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
                        natural_align = natural_align.max(member_alignment);
                        let offset = if is_union { 0 } else { size };
                        size = if is_union {
                            size.max(member_type.size_of())
                        } else {
                            size + member_type.size_of()
                        };
                        bit_address = size * 8;
                        (
                            BitField {
                                width: 0,
                                shift: 0,
                                bytewise: false,
                            },
                            offset,
                        )
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
                            requested_alignment,
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
                transparent_union: false,
                has_bit_fields,
                has_flexible_array,
                size: size.div_ceil(align) * align,
                align,
                natural_align,
                packing,
                complete: true,
            };
        }
        if defining {
            let (alignment, trailing_packed, trailing_transparent) = self.record_attributes();
            transparent |= trailing_transparent;
            requested_record_alignment = requested_record_alignment.max(alignment);
            packed |= trailing_packed;
        } else {
            self.declaration_attributes(false, None, true);
        }
        if packed {
            if !defining {
                self.error_logger.print_error_position(
                    token.pos,
                    "packed attribute requires a record definition",
                );
            }
            let definition = ty.definition();
            let mut definition = definition.borrow_mut();
            if definition.has_bit_fields {
                self.error_logger.print_error_position(
                    token.pos,
                    "GNU packed records with bit-fields are not supported",
                );
            }
            let mut size = 0usize;
            let mut align = 1;
            for member in &mut definition.members {
                // GNU packing lowers natural alignment; explicit member alignment may raise it.
                let alignment = member
                    .requested_alignment
                    .max(1)
                    .min(packing.unwrap_or(usize::MAX));
                align = align.max(alignment);
                member.offset = if is_union {
                    0
                } else {
                    size.div_ceil(alignment) * alignment
                };
                size = size.max(member.offset + member.ty.size_of());
            }
            definition.natural_align = align;
            definition.align = align.max(inherited_record_alignment);
            definition.size = size.div_ceil(definition.align) * definition.align;
            definition.packing = Some(1);
        }
        if requested_record_alignment != 0 {
            let definition = ty.definition();
            let mut definition = definition.borrow_mut();
            definition.align = definition.align.max(requested_record_alignment);
            definition.size = definition.size.div_ceil(definition.align) * definition.align;
        }
        let ty = Type::Struct(ty);
        if transparent {
            self.apply_transparent_union(&ty, &token);
        }
        ty
    }

    fn expect_object_type(&self, ty: &Type, token: &Token) {
        if !ty.is_complete_object()
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
        let saved_attributes = self.declaration_attributes_state.take();
        self.declaration_attributes_state = Some(DeclarationAttributes {
            typedef_declarator: true,
            ..DeclarationAttributes::default()
        });
        let prefix_alignments = self.declaration_attributes(true, None, false);
        let prefix_attributes = self.declaration_attributes_state.take().unwrap();
        let base = self.expect_type();
        loop {
            self.declaration_attributes_state = Some(prefix_attributes.clone());
            let mut alignments = prefix_alignments.clone();
            let (name, mut ty, _, _, _) = self
                .declarator_with_alignment(Some(&mut alignments))
                .resolve(base.clone());
            let token = name.unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "ident expected");
                unreachable!()
            });
            let attributes = self.declaration_attributes_state.take().unwrap();
            if attributes.transparent_union {
                self.apply_transparent_union(&ty, &token);
            }
            if let Some(size) = attributes.vector_size {
                let element = ty.unqualified();
                if !(element.integer_info().is_some() && *element != Type::Bool
                    || matches!(element, Type::Float(32 | 64)))
                    || size % element.size_of().max(1) != 0
                {
                    self.error_logger.print_error_position(token.pos, "vector_size requires an ordinary integer or float scalar typedef with a matching element size");
                }
                let qualifiers = ty.qualifiers();
                let vector = Type::Vector(Box::new(element.clone()), size / element.size_of());
                ty = if qualifiers == Qualifiers::default() {
                    vector
                } else {
                    Type::Qualified(Box::new(vector), qualifiers)
                };
            }
            self.validate_type_layout(&ty, &token);
            if let Some(alignment) = alignments.into_iter().max() {
                if !ty.is_complete_object() || ty.function_signature_ref().is_some() {
                    self.error_logger.print_error_position(
                        token.pos,
                        "typedef alignment requires a complete object type",
                    );
                }
                if alignment != ty.align_of() {
                    if alignment > ty.natural_align_of()
                        || !(ty.integer_info().is_some()
                            || matches!(ty.unqualified(), Type::Float(32 | 64) | Type::Ptr(_)))
                    {
                        self.error_logger.print_error_position(token.pos, "non-natural typedef alignment is supported only for scalar alignment reductions");
                    }
                    ty = Type::Aligned(Box::new(ty), alignment);
                }
            }
            if let Some(previous) = self.scope_stack.last().unwrap().get(&token.s_value) {
                let valid = matches!(previous, Identifier::TypeDef(previous)
                    if !previous.is_variably_modified()
                        && !ty.is_variably_modified()
                        && previous.qualifiers() == ty.qualifiers()
                        && previous.align_of() == ty.align_of()
                        && previous.identical(&ty));
                if !valid {
                    self.error_logger
                        .print_error_position(token.pos, "incompatible typedef redeclaration");
                }
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
        self.declaration_attributes_state = saved_attributes;
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

    fn attempt_thread_local(&mut self) -> bool {
        self.attempt_reserved("_Thread_local")
            .or_else(|| self.attempt_reserved("__thread"))
            .is_some()
    }

    fn global_definition(&mut self) {
        while self.attempt_reserved("__extension__").is_some() {}
        if self.attempt_static_assert() {
            return;
        }
        // Attribute collection belongs to this declaration header, not its body or parameters.
        self.declaration_attributes_state = Some(DeclarationAttributes::default());
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
            alignment = alignment.max(self.object_alignment_annotations());
            if self.attempt_thread_local() {
                thread_local = true;
            } else if self.attempt_inline().is_some() {
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
            if self
                .declaration_attributes_state
                .take()
                .is_some_and(|a| a.gnu_inline || a.noinline || a.weak_import || a.common)
            {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "declaration linkage attributes cannot be applied to a typedef",
                );
            }
            if thread_local {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "thread-local storage cannot be applied to a typedef",
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
        if storage
            .as_ref()
            .is_some_and(|storage| storage.s_value == "static")
            && external
        {
            self.error_logger
                .print_error_position(self.tokens[self.cur - 1].pos, "conflicting storage classes");
        }
        let mut base = self.expect_type();
        alignment = alignment.max(self.object_alignment_annotations());
        if qualifiers != Qualifiers::default() {
            base = Type::Qualified(Box::new(base), qualifiers);
        }
        inline |= self.attempt_inline().is_some();
        if self.attempt_reserved(";").is_some() {
            if alignment != 0 {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "alignment requires an object declaration",
                );
            }
            if self
                .declaration_attributes_state
                .take()
                .is_some_and(|a| a.gnu_inline || a.noinline || a.weak_import || a.common)
            {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "declaration attributes require a declarator",
                );
            }
            if thread_local {
                self.error_logger.print_error_position(
                    self.tokens[self.cur - 1].pos,
                    "thread-local storage requires an object declaration",
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
        let inherited_attributes = self.declaration_attributes_state.take().unwrap_or_default();
        loop {
            self.declaration_attributes_state = Some(inherited_attributes.clone());
            let mut alignments = vec![alignment];
            let (name, mut ty, parameters, parameter_scope, stars) = self
                .declarator_with_alignment(Some(&mut alignments))
                .resolve(base.clone());
            let alignment = alignments.into_iter().max().unwrap_or(0);
            let token = name.unwrap_or_else(|| {
                self.error_logger
                    .print_error_position(self.tokens[self.cur].pos, "ident expected");
                unreachable!()
            });
            self.assembler_label(&token);
            let attributes = self.declaration_attributes_state.take().unwrap_or_default();
            if attributes.noinline && ty.function_signature_ref().is_none() {
                self.error_logger
                    .print_error_position(token.pos, "noinline requires a function declaration");
            }
            if attributes.gnu_inline && (!inline || ty.function_signature_ref().is_none()) {
                self.error_logger.print_error_position(
                    token.pos,
                    "gnu_inline requires an inline function declaration",
                );
            }
            if attributes.common
                && (ty.function_signature_ref().is_some() || attributes.weak_import)
            {
                self.error_logger.print_error_position(
                    token.pos,
                    "common requires an object declaration without weak_import",
                );
            }
            self.validate_type_layout(&ty, &token);
            if ty.is_variably_modified() {
                self.error_logger.print_error_position(
                    token.pos,
                    "variably modified declaration requires block scope",
                );
            }
            if inline && ty.function_signature_ref().is_none() {
                self.error_logger
                    .print_error_position(token.pos, "inline requires a function declaration");
            }
            if ty.function_signature_ref().is_some() {
                if alignment != 0 {
                    self.error_logger.print_error_position(
                        token.pos,
                        "_Alignas cannot be applied to a function",
                    );
                }
                if thread_local {
                    self.error_logger.print_error_position(
                        token.pos,
                        "thread-local storage cannot be applied to a function",
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
                let definition = self.tokens[self.cur].s_value == "{";
                self.scope_stack.push(parameter_scope);
                let mut args = vec![];
                if definition {
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
                let previous = &self.functions[&token.s_value];
                if attributes.gnu_inline && previous.body.is_some() && !previous.gnu_inline {
                    self.error_logger.print_error_position(
                        token.pos,
                        "gnu_inline must be declared before the function definition",
                    );
                }
                let gnu_inline = attributes.gnu_inline || previous.gnu_inline;
                let noinline = attributes.noinline || previous.noinline;
                let replaceable_inline_body = previous.gnu_inline && previous.inline_only();
                let weak_import = attributes.weak_import || previous.weak_import;
                if weak_import && (internal || definition || previous.body.is_some()) {
                    self.error_logger.print_error_position(token.pos, "weak_import requires an external function declaration without a definition");
                }
                let external_definition = if gnu_inline {
                    if !definition && previous.body.is_some() {
                        previous.external_definition
                    } else {
                        internal || !inline || !external
                    }
                } else {
                    internal || external || !inline || previous.external_definition
                };
                let function = self.functions.get_mut(&token.s_value).unwrap();
                function.external_definition = external_definition;
                function.noinline = noinline;
                let (_, result, _) = ty.function_signature_ref().unwrap();
                if definition && result.unqualified() != &Type::Void {
                    self.expect_object_type(result, &token);
                }
                self.return_pointer = if definition && matches!(result, Type::Struct(_)) {
                    self.offset_size = self.offset_size.div_ceil(8) * 8 + 8;
                    Some(self.offset_size)
                } else {
                    None
                };
                self.variadic_last = args.last().and_then(|arg| arg.offset);
                let (parameters, _, variadic) = ty.function_signature_ref().unwrap();
                self.variadic_parameters = variadic.then(|| parameters.to_vec());
                self.return_type = Some(result.clone());
                self.current_function = Some(token.clone());
                self.labels.clear();
                self.defined_labels.clear();
                let mut bounds = self.take_bounds();
                Self::resolve_parameter_bounds(&mut bounds, &args);
                self.mutable_static = None;
                let mut body = self.consume_block();
                if body.is_some() {
                    let (parameters, result, _) = ty.function_signature_ref().unwrap();
                    if parameters
                        .iter()
                        .chain(std::iter::once(result))
                        .any(|ty| matches!(ty.unqualified(), Type::Float16))
                    {
                        self.error_logger.print_error_position(token.pos,
                            "_Float16 parameters and results in function definitions are not supported");
                    }
                }
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
                self.current_function = None;
                self.variadic_last = args.last().and_then(|arg| arg.offset);
                self.variadic_parameters = None;
                self.scope_stack.pop();
                if body.is_none() {
                    self.expect_reserved(";");
                }
                if self.functions[&token.s_value].body.is_some() {
                    if body.is_some() {
                        if !replaceable_inline_body || !external_definition {
                            self.error_logger
                                .print_error_position(token.pos, "function is already defined");
                        }
                    } else {
                        return;
                    }
                }
                self.functions.insert(
                    token.s_value.clone(),
                    Func {
                        inline_definition: inline && body.is_some(),
                        gnu_inline,
                        noinline,
                        weak_import,
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
            if let Some(previous) = self.global_variables.get(&token.s_value) {
                ty = self.composite_object_declaration(&previous.ty, ty, &token);
            }
            if self.functions.contains_key(&token.s_value) {
                self.error_logger
                    .print_error_position(token.pos, "object conflicts with function");
            }
            // The object's scope begins after its declarator, before its initializer.
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value.clone(), Identifier::Static(ty.clone()));
            self.global_variables
                .entry(token.s_value.clone())
                .or_insert_with(|| GlobalVariable {
                    ty: ty.clone(),
                    alignment,
                    thread_local,
                    weak_import: attributes.weak_import,
                    common: attributes.common,
                    declaration_only: external,
                    internal,
                    data: None,
                });
            let data = if self.attempt_reserved("=").is_some() {
                Some(self.global_initializer(&ty))
            } else {
                None
            };
            let common = attributes.common
                || self
                    .global_variables
                    .get(&token.s_value)
                    .is_some_and(|object| object.common);
            let weak_import = attributes.weak_import
                || self
                    .global_variables
                    .get(&token.s_value)
                    .is_some_and(|object| object.weak_import);
            if weak_import
                && (common
                    || !external
                    || internal
                    || thread_local
                    || data.is_some()
                    || self
                        .global_variables
                        .get(&token.s_value)
                        .is_some_and(|object| !object.declaration_only))
            {
                self.error_logger.print_error_position(token.pos, "weak_import requires an external non-TLS object declaration without a definition");
            }
            let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                elements.len()
            } else {
                0
            };
            let ty = if external && data.is_none() && Self::incomplete_external_object(&ty) {
                ty
            } else {
                let ty = self.infer_array_size(ty, count, &token);
                self.expect_object_type(&ty, &token);
                ty
            };
            if let Some(data) = &data {
                self.validate_global_initializer(&ty, data, &token);
            }
            self.scope_stack
                .last_mut()
                .unwrap()
                .insert(token.s_value.clone(), Identifier::Static(ty.clone()));
            {
                let existing = self.global_variables.get_mut(&token.s_value).unwrap();
                if existing.internal != internal && !external {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting object linkage");
                }
                if !external || data.is_some() {
                    existing.declaration_only = false;
                }
                if existing.ty != ty {
                    existing.ty = ty.clone();
                }
                existing.alignment = existing.alignment.max(alignment);
                existing.weak_import = weak_import;
                existing.common = common;
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
            }
            if self.attempt_reserved(",").is_none() {
                break;
            }
        }
        self.expect_reserved(";");
    }
    fn incomplete_external_object(ty: &Type) -> bool {
        match ty.unqualified() {
            Type::Arr(child, None) => child.is_complete_object(),
            Type::Struct(record) => !record.definition().borrow().complete,
            _ => false,
        }
    }

    fn composite_object_declaration(&self, previous: &Type, declared: Type, token: &Token) -> Type {
        if previous == &declared {
            return declared;
        }
        if previous.qualifiers() == declared.qualifiers() {
            if let (Type::Arr(left, left_count), Type::Arr(right, right_count)) =
                (previous.unqualified(), declared.unqualified())
            {
                if left.qualifiers() == right.qualifiers()
                    && left.compatible(right)
                    && (left_count.is_none() || right_count.is_none())
                {
                    return if right_count.is_none() {
                        previous.clone()
                    } else {
                        declared
                    };
                }
            }
        }
        if previous.qualifiers() == declared.qualifiers() && previous.compatible(&declared) {
            return declared;
        }
        self.error_logger
            .print_error_position(token.pos, "incompatible object redeclaration");
        unreachable!()
    }

    fn resolve_name(&self, s: &str) -> Option<&Identifier> {
        self.scope_stack.iter().rev().find_map(|map| map.get(s))
    }
    fn validate_global_initializer(&self, ty: &Type, data: &GlobalVariableData, token: &Token) {
        let ty = ty.unqualified();
        match (ty, data) {
            (_, GlobalVariableData::Zero) => {}
            (Type::Float16, _) => {
                self.error_logger.print_error_position(
                    token.pos,
                    "_Float16 value initialization is not supported",
                );
            }
            (Type::Integer128 { .. }, _) => {
                self.error_logger.print_error_position(
                    token.pos,
                    "128-bit integer value initialization is not supported",
                );
            }
            (Type::Struct(ty), GlobalVariableData::Union { index, value }) => {
                self.validate_global_initializer(&ty.member(*index).ty, value, token);
            }
            (Type::Struct(ty), GlobalVariableData::Arr(elements)) => {
                let members = ty.members();
                let capacity = if ty.definition().borrow().is_union {
                    1
                } else {
                    members
                        .iter()
                        .take_while(|member| !matches!(member.ty.unqualified(), Type::Arr(_, None)))
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
                if size.is_some_and(|size| elements.len() > size) {
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
            (ty, GlobalVariableData::Address { .. })
                if ty.is_arithmetic() && ty.integer_info().is_none_or(|(bits, _)| bits != 64) =>
            {
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
            Type::Arr(child, size) if size.is_none_or(|size| index < size) => Some(*child.clone()),
            Type::Struct(st) => st
                .members()
                .get(index)
                .filter(|member| !matches!(member.ty.unqualified(), Type::Arr(_, None)))
                .map(|member| member.ty.clone()),
            _ => None,
        }
    }
    fn initializer_subobject(ty: &Type, path: &[usize]) -> Option<Type> {
        path.iter()
            .try_fold(ty.clone(), |ty, index| Self::aggregate_type(&ty, *index))
    }

    fn next_initializer_path(ty: &Type, mut path: Vec<usize>) -> Option<Vec<usize>> {
        while let Some(index) = path.pop() {
            let parent = Self::initializer_subobject(ty, &path).unwrap();
            if matches!(parent.unqualified(), Type::Struct(st) if st.definition().borrow().is_union)
            {
                continue;
            }
            if Self::aggregate_type(&parent, index + 1).is_some() {
                path.push(index + 1);
                return Some(path);
            }
        }
        None
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
        if ty.contains_vector() {
            self.error_logger.print_error_position(
                self.tokens[self.cur].pos,
                "explicit vector and vector-containing aggregate initializers are not supported",
            );
        }
        if let Some(token) = self.attempt_string_initializer(ty) {
            let mut bytes = token.string_bytes.clone();
            let unit_size = token.string_encoding.unit_size();
            let Type::Arr(_, size) = ty.unqualified() else {
                unreachable!()
            };
            if size.is_some_and(|size| bytes.len() / unit_size > size) {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            if size.is_none_or(|size| bytes.len() / unit_size < size) {
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
            let mut cursor = Some(vec![0]);
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(ty);
                    if path.is_empty() {
                        path = cursor.clone().unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                self.tokens[self.cur].pos,
                                match ty.unqualified() {
                                    Type::Struct(st) if st.definition().borrow().is_union => {
                                        "too many union initializer elements"
                                    }
                                    Type::Struct(_) => "too many struct initializer elements",
                                    _ => "too many array initializer elements",
                                },
                            );
                            unreachable!()
                        });
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
                    if self.tokens[self.cur].s_value != "{" {
                        while self.string_initializer_index(&child).is_none() {
                            let Some(first) = Self::aggregate_type(&child, 0) else {
                                break;
                            };
                            path.push(0);
                            child = first;
                        }
                    }
                    let value = self.global_initializer(&child);
                    cursor = Self::next_initializer_path(ty, path.clone());
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
        self.validate_storage_only_expressions(&expression, false);
        if let Some((symbol, offset)) = self.constant_address(&expression) {
            self.expect_conversion(ty, &expression);
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
            NodeType::GlobalVar if node.thread_local => None,
            NodeType::GlobalVar
                if node.resolve_type().is_some_and(|ty| {
                    matches!(ty, Type::Arr(..) | Type::Func(..) | Type::VariadicFunc(..))
                }) =>
            {
                Some((
                    if node.dest.is_empty() {
                        node.global_name.clone()
                    } else {
                        node.dest.trim_start_matches('@').to_string()
                    },
                    0,
                ))
            }
            NodeType::Addr => {
                let child = node.lhs.as_ref().unwrap();
                if matches!(child.nt, NodeType::GlobalVar) && !child.thread_local {
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
            NodeType::Cast
                if node.resolve_type().is_some_and(|ty| {
                    matches!(ty.unqualified(), Type::Ptr(_))
                        || ty.integer_info().is_some_and(|(bits, _)| bits == 64)
                }) =>
            {
                self.constant_address(node.lhs.as_ref().unwrap())
            }
            NodeType::Add => {
                let lhs = node.lhs.as_ref().unwrap();
                let rhs = node.rhs.as_ref().unwrap();
                self.constant_address_offset(lhs, rhs, false)
                    .or_else(|| self.constant_address_offset(rhs, lhs, false))
            }
            NodeType::Sub => self.constant_address_offset(
                node.lhs.as_ref().unwrap(),
                node.rhs.as_ref().unwrap(),
                true,
            ),
            _ => None,
        }
    }

    fn constant_address_offset(
        &mut self,
        base: &Node,
        amount: &Node,
        subtract: bool,
    ) -> Option<(String, i64)> {
        let (symbol, offset) = self.constant_address(base)?;
        if !Self::integer_constant(amount) {
            return None;
        }
        let ty = base.resolve_type()?;
        let stride = if ty.is_pointer_value() {
            base.dest_type()?.size_of() as i64
        } else if ty.integer_info().is_some_and(|(bits, _)| bits == 64) {
            1
        } else {
            return None;
        };
        let delta = self.eval(amount).checked_mul(stride)?;
        Some((
            symbol,
            if subtract {
                offset.checked_sub(delta)?
            } else {
                offset.checked_add(delta)?
            },
        ))
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
            NodeType::FloatAbs => self.eval_float_as(node.lhs.as_ref().unwrap(), &ty).abs(),
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
            NodeType::BitNot => {
                let value = !self.eval(node.lhs.as_ref().unwrap());
                return node.resolve_type().unwrap().convert_constant(value);
            }
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
    fn lower_source_types(node: &mut Node) {
        Self::lower_transparent_call(node);
        node.volatile_access |= node.is_volatile_object();
        if let Some(ty) = &mut node.cty {
            ty.lower_for_codegen();
        }
        for ty in &mut node.parameter_types {
            ty.lower_parameter_for_codegen();
        }
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
            Self::lower_source_types(child);
        }
        for child in node.children.iter_mut().chain(&mut node.args) {
            Self::lower_source_types(child);
        }
    }

    fn cast_integer(node: &mut Node, target: &Type) {
        if target.is_arithmetic() && node.resolve_type_view().as_deref() != Some(target) {
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
            let lhs = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
            let rhs = node.rhs.as_ref().unwrap().resolve_type_view().unwrap();
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
                    .resolve_type_view()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    Self::cast_integer(condition, &Type::Bool);
                }
            }
        }
        if matches!(node.nt, NodeType::LogicalAnd | NodeType::LogicalOr) {
            for operand in [&mut node.lhs, &mut node.rhs].into_iter().flatten() {
                if operand
                    .resolve_type_view()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    Self::cast_integer(operand, &Type::Bool);
                }
            }
        }
        if node.nt == NodeType::Conditional
            && node
                .resolve_type_view()
                .is_some_and(|ty| ty.float_bits().is_some())
        {
            let common = node.resolve_type_view().unwrap().into_owned();
            Self::cast_integer(node.then.as_mut().unwrap(), &common);
            Self::cast_integer(node.els.as_mut().unwrap(), &common);
        }
        match node.nt {
            NodeType::Assign => {
                let target = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
                Self::cast_integer(node.rhs.as_mut().unwrap(), &target);
            }
            NodeType::CallFunc => {
                for (index, arg) in node.args.iter_mut().rev().enumerate() {
                    let target = node.parameter_types.get(index).cloned().unwrap_or_else(|| {
                        let ty = arg.resolve_type_view().unwrap().promoted();
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

    fn can_convert(expected: &Type, operand: &Node) -> bool {
        let actual = operand.resolve_type_view();
        match (
            expected.unqualified(),
            actual.as_deref().map(Type::unqualified),
        ) {
            (target, Some(source)) if target.is_arithmetic() && source.is_arithmetic() => true,
            (Type::Bool, Some(source)) if source.is_pointer_value() => true,
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
                target.compatible(source)
            }
            (Type::Ptr(_), Some(Type::I32)) => {
                matches!(operand.nt, NodeType::Num) && operand.value == Some(0)
            }
            (target, Some(source)) => target == source,
            _ => false,
        }
    }

    fn expect_conversion(&self, expected: &Type, operand: &Node) {
        if !Self::can_convert(expected, operand) {
            self.error_logger.print_error_position(
                operand.token.as_ref().map_or(0, |token| token.pos),
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
                    && !(node.global_name.starts_with(".static.") && object.ty.is_const_object())
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

    fn validate_storage_only_expressions(&self, node: &Node, address_only: bool) {
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| {
                    matches!(ty.unqualified(), Type::Struct(_)) && ty.contains_reduced_alignment()
                })
        {
            self.error_logger.print_error_position(node.token.as_ref().map_or(0, |token| token.pos), "records containing reduced-alignment typedefs cannot be passed or returned by value");
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| matches!(ty.unqualified(), Type::Struct(_)) && ty.contains_float16())
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "records containing _Float16 cannot be passed or returned by value",
            );
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| matches!(ty.unqualified(), Type::Struct(_)) && ty.align_of() > 16)
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "records aligned above 16 bytes cannot be passed or returned by value",
            );
        }
        if node.nt == NodeType::CallFunc
            && node
                .args
                .iter()
                .filter_map(Node::resolve_type_view)
                .chain(node.resolve_type_view())
                .any(|ty| Type::decay_view(ty).contains_vector())
        {
            self.error_logger.print_error_position(
                node.token.as_ref().map_or(0, |token| token.pos),
                "vectors and records containing vectors cannot be passed or returned by value",
            );
        }
        if !address_only {
            let message = match node.resolve_type_view().as_deref().map(Type::unqualified) {
                Some(Type::Integer128 { .. }) => {
                    Some("128-bit integer value operations are not supported")
                }
                Some(Type::Float16) => Some("_Float16 value operations are not supported"),
                Some(Type::Vector(..)) => Some("vector value operations are not supported"),
                _ => None,
            };
            if let Some(message) = message {
                self.error_logger.print_error_position(
                    node.token.as_ref().map_or(0, |token| token.pos),
                    message,
                );
            }
        }
        if let Some(lhs) = &node.lhs {
            self.validate_storage_only_expressions(lhs, node.nt == NodeType::Addr);
        }
        for child in [
            &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        .map(Box::as_ref)
        .chain(node.children.iter())
        .chain(node.args.iter())
        {
            self.validate_storage_only_expressions(child, false);
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
                    .resolve_type_view()
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
            let lhs = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
            let rhs = node.rhs.as_ref().unwrap().resolve_type_view().unwrap();
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
            if operand.resolve_type_view().as_deref() == Some(&Type::Void) {
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
                &node.lhs.as_ref().unwrap().resolve_type_view().unwrap(),
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
            let then_void =
                node.then.as_ref().unwrap().resolve_type_view().as_deref() == Some(&Type::Void);
            let else_void =
                node.els.as_ref().unwrap().resolve_type_view().as_deref() == Some(&Type::Void);
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
        while self.attempt_reserved("__extension__").is_some() {}
        self.declaration_attributes(false, None, true);
        if matches!(
            self.tokens[self.cur].tt,
            TokenType::Ident | TokenType::Reserved
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
            let mut storage = storage.unwrap_or_else(|| {
                inline.clone().unwrap_or_else(|| Token {
                    s_value: "static".to_owned(),
                    ..self.tokens[self.cur - 1].clone()
                })
            });
            if matches!(
                storage.s_value.as_str(),
                "inline" | "__inline" | "__inline__"
            ) {
                storage.s_value = "extern".to_owned();
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
            let mut alignments = vec![requested_alignment];
            let (token, mut ty) = self.expect_ident_with_type(base.clone(), Some(&mut alignments));
            let requested_alignment = alignments.into_iter().max().unwrap_or(0);
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
            if ty.function_signature_ref().is_some() {
                if thread_local {
                    self.error_logger.print_error_position(
                        token.pos,
                        "thread-local storage cannot be applied to a function",
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
            if !external && !ty.is_const_object() && self.mutable_static.is_none() {
                self.mutable_static = Some(token.clone());
            }
            if external {
                if let Some(previous) = self.global_variables.get(&name) {
                    ty = self.composite_object_declaration(&previous.ty, ty, &token);
                }
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
                token.s_value.clone(),
                Identifier::StaticObject(ty.clone(), name.clone()),
            );
            let new_object = !self.global_variables.contains_key(&name);
            self.global_variables
                .entry(name.clone())
                .or_insert_with(|| GlobalVariable {
                    ty: ty.clone(),
                    alignment: requested_alignment,
                    thread_local,
                    internal: !external,
                    weak_import: false,
                    common: false,
                    declaration_only: external,
                    data: None,
                });
            let data = if self.attempt_reserved("=").is_some() {
                if external {
                    self.error_logger
                        .print_error_position(token.pos, "block extern cannot have an initializer");
                }
                Some(self.global_initializer(&ty))
            } else {
                None
            };
            let count = if let Some(GlobalVariableData::Arr(elements)) = &data {
                elements.len()
            } else {
                0
            };
            if !(external && data.is_none() && Self::incomplete_external_object(&ty)) {
                ty = self.infer_array_size(ty, count, &token);
                self.expect_object_type(&ty, &token);
            }
            if let Some(data) = &data {
                self.validate_global_initializer(&ty, data, &token);
            }
            self.scope_stack.last_mut().unwrap().insert(
                token.s_value,
                Identifier::StaticObject(ty.clone(), name.clone()),
            );
            {
                let existing = self.global_variables.get_mut(&name).unwrap();
                existing.alignment = existing.alignment.max(requested_alignment);
                if existing.ty != ty {
                    if external || new_object {
                        existing.ty = ty.clone();
                    } else {
                        self.error_logger
                            .print_error_position(token.pos, "incompatible object redeclaration");
                    }
                }
                if existing.thread_local != thread_local {
                    self.error_logger
                        .print_error_position(token.pos, "conflicting thread storage duration");
                }
                if new_object {
                    existing.data = data;
                }
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
            let (mut node, inferred) = self.new_local_variable(ty.clone(), alignment);
            if storage != AutomaticStorage::Default
                && node
                    .resolve_type()
                    .unwrap()
                    .function_signature_ref()
                    .is_some()
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
                vec.push(if inferred {
                    self.inferred_local_array_initializer(&node, &token).1
                } else {
                    self.local_variable_initialization(&node, &token)
                });
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
        result.member_access = true;
        result.bit_field = member.bit_field;
        result.registered = registered;
        result
    }

    fn aggregate_elements(node: &Node, token: &Token) -> Option<Vec<Node>> {
        match node.resolve_type()? {
            Type::Arr(_, size) => Some(
                (0..size.unwrap_or(0))
                    .map(|index| Self::array_element(node, index, token))
                    .collect(),
            ),
            Type::Struct(ty) => Some(
                ty.members()
                    .iter()
                    .filter(|member| !matches!(member.ty.unqualified(), Type::Arr(_, None)))
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
        let ty = node.object_type().unwrap();
        if matches!(ty.unqualified(), Type::Arr(..) | Type::Struct(_))
            && !ty.is_variably_modified()
            && !ty.contains_volatile_subobject()
            && ty.size_of() != 0
        {
            return Node {
                nt: NodeType::ZeroFill {
                    bytes: ty.size_of(),
                },
                token: Some(token.clone()),
                cty: Some(Type::Void),
                lhs: Some(Box::new(node.clone())),
                ..Node::default()
            };
        }
        let mut padding = vec![];
        if let Type::Struct(record) = ty.unqualified() {
            let is_union = record.definition().borrow().is_union;
            let address =
                Node::new_with_op_and_lhs(Some(token.clone()), NodeType::Addr, node.clone());
            let bytes = Node {
                nt: NodeType::Cast,
                token: Some(token.clone()),
                cty: Some(Type::Ptr(Box::new(Type::I8))),
                lhs: Some(Box::new(address)),
                ..Node::default()
            };
            // Clear gaps and bit-field storage before member-wise initialization.
            // Typed member stores retain their normal volatile access widths.
            let mut ranges = if is_union {
                vec![]
            } else {
                record
                    .members()
                    .iter()
                    .filter(|member| member.bit_field.is_none())
                    .map(|member| (member.offset, member.offset + member.ty.size_of()))
                    .collect::<Vec<_>>()
            };
            ranges.sort_unstable();
            let mut cursor = 0;
            for (start, end) in ranges
                .into_iter()
                .chain(std::iter::once((ty.size_of(), ty.size_of())))
            {
                for index in cursor..start {
                    padding.push(Node::new_with_op(
                        Some(token.clone()),
                        NodeType::Assign,
                        Self::array_element(&bytes, index, token),
                        Node::new_with_num(Some(token.clone()), 0),
                    ));
                }
                cursor = cursor.max(end);
            }
            if is_union {
                return Node {
                    nt: NodeType::Block,
                    token: Some(token.clone()),
                    children: padding,
                    ..Node::default()
                };
            }
        }
        if let Some(elements) = Self::aggregate_elements(node, token) {
            padding.extend(
                elements
                    .iter()
                    .map(|element| Self::zero_initializer(element, token)),
            );
            Node {
                nt: NodeType::Block,
                token: Some(token.clone()),
                children: padding,
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
        self.local_variable_initialization_with_count(node, assign_token, None)
    }

    fn local_expression_initializer(&mut self, node: &Node, rhs: Node, token: &Token) -> Node {
        if matches!(node.resolve_type(), Some(Type::Struct(_))) {
            self.struct_assignment(node.clone(), rhs, token.clone())
        } else {
            Node::new_with_op(Some(token.clone()), NodeType::Assign, node.clone(), rhs)
        }
    }

    fn local_variable_initialization_with_count(
        &mut self,
        node: &Node,
        assign_token: &Token,
        mut outer_count: Option<&mut usize>,
    ) -> Node {
        if node.resolve_type().is_some_and(|ty| ty.contains_vector()) {
            self.error_logger.print_error_position(
                assign_token.pos,
                "explicit vector and vector-containing aggregate initializers are not supported",
            );
        }
        if let Some(token) = self.attempt_string_initializer(&node.resolve_type().unwrap()) {
            let bytes = token.string_bytes.clone();
            let unit_size = token.string_encoding.unit_size();
            let Type::Arr(_, size) = node.resolve_type().unwrap() else {
                unreachable!()
            };
            let size = size.unwrap_or(bytes.len() / unit_size + 1);
            if let Some(count) = outer_count.as_deref_mut() {
                *count = size;
            }
            if bytes.len() / unit_size > size {
                self.error_logger
                    .print_error_position(token.pos, "string initializer is too long");
            }
            if size != 0 && !node.object_type().unwrap().contains_volatile_subobject() {
                let Type::Arr(element, _) = node.resolve_type().unwrap() else {
                    unreachable!()
                };
                let literal_size = bytes.len() / unit_size + 1;
                let literal = Node {
                    nt: NodeType::GlobalVar,
                    token: Some(token.clone()),
                    cty: Some(Type::Arr(element, Some(literal_size))),
                    dest: self.new_string_literal(&token),
                    ..Node::default()
                };
                let mut children = vec![];
                if size > literal_size {
                    children.push(Node {
                        nt: NodeType::ZeroFill {
                            bytes: size * unit_size,
                        },
                        cty: Some(Type::Void),
                        token: Some(token.clone()),
                        lhs: Some(Box::new(node.clone())),
                        ..Node::default()
                    });
                }
                children.push(Node {
                    nt: NodeType::CopyBytes {
                        bytes: size.min(literal_size) * unit_size,
                    },
                    cty: Some(Type::Void),
                    token: Some(token.clone()),
                    lhs: Some(Box::new(node.clone())),
                    rhs: Some(Box::new(literal)),
                    ..Node::default()
                });
                return Node {
                    nt: NodeType::Block,
                    token: Some(token),
                    children,
                    ..Node::default()
                };
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
            if !matches!(node.resolve_type(), Some(Type::Arr(..) | Type::Struct(_))) {
                let initializer = self.local_variable_initialization(node, assign_token);
                self.attempt_reserved(",");
                self.expect_reserved("}");
                return initializer;
            }
            let mut children = vec![Self::zero_initializer(node, assign_token)];
            let mut selected_unions = HashMap::new();
            let ty = node.resolve_type().unwrap();
            let mut cursor = Some(vec![0]);
            if self.attempt_reserved("}").is_none() {
                loop {
                    let mut path = self.designation(&node.resolve_type().unwrap());
                    if path.is_empty() {
                        path = cursor.clone().unwrap_or_else(|| {
                            self.error_logger.print_error_position(
                                token.pos,
                                "too many aggregate initializer elements",
                            );
                            unreachable!()
                        });
                    }
                    let mut child = Self::initializer_subobject(&ty, &path).unwrap_or_else(|| {
                        self.error_logger.print_error_position(
                            token.pos,
                            "too many aggregate initializer elements",
                        );
                        unreachable!()
                    });
                    let rhs = if self.tokens[self.cur].s_value == "{"
                        || self.string_initializer_index(&child).is_some()
                    {
                        None
                    } else {
                        let rhs = self.assign();
                        while !(matches!(child.unqualified(), Type::Struct(_))
                            && rhs
                                .resolve_type()
                                .is_some_and(|source| child.compatible(&source)))
                        {
                            let Some(first) = Self::aggregate_type(&child, 0) else {
                                break;
                            };
                            path.push(0);
                            child = first;
                        }
                        Some(rhs)
                    };
                    if let Some(count) = outer_count.as_deref_mut() {
                        *count = (*count).max(path[0] + 1);
                    }
                    cursor = Self::next_initializer_path(&ty, path.clone());
                    let mut target = node.clone();
                    let mut prefix = vec![];
                    for index in path {
                        let union = matches!(target.resolve_type(), Some(Type::Struct(st)) if st.definition().borrow().is_union);
                        let selected = match target.resolve_type().unwrap() {
                            Type::Arr(_, size) if size.is_none_or(|size| index < size) => {
                                Some(Self::array_element(&target, index, &token))
                            }
                            _ => Self::aggregate_elements(&target, &token)
                                .and_then(|targets| targets.get(index).cloned()),
                        };
                        target = selected.unwrap_or_else(|| {
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
                    children.push(if let Some(rhs) = rhs {
                        self.local_expression_initializer(&target, rhs, assign_token)
                    } else {
                        self.local_variable_initialization(&target, assign_token)
                    });
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
            self.local_expression_initializer(node, rhs, assign_token)
        }
    }

    fn struct_assignment(&mut self, lhs: Node, rhs: Node, token: Token) -> Node {
        let ty = lhs.resolve_type().unwrap();
        self.expect_object_type(&ty, &token);
        if rhs.resolve_type() != Some(ty.clone()) {
            self.error_logger
                .print_error_position(token.pos, "incompatible struct assignment");
        }
        if ty.size_of() != 0
            && !lhs.object_type().unwrap().contains_volatile_subobject()
            && !rhs.object_type().unwrap().contains_volatile_subobject()
        {
            return Node {
                nt: NodeType::CopyBytes {
                    bytes: ty.size_of(),
                },
                cty: Some(ty),
                token: Some(token),
                lhs: Some(Box::new(lhs)),
                rhs: Some(Box::new(rhs)),
                ..Node::default()
            };
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
        let (token, mut children, cleanup) = self.consume_block_parts()?;
        children.extend(cleanup);
        Some(Node {
            token: Some(token),
            nt: NodeType::Block,
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
            Some((t, children, cleanup))
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
                let member = st.member(index);
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
                        s_value: format!(".compound.{}", token.pos),
                        ..token.clone()
                    };
                    let literal = if self.return_type.is_some() {
                        let (object, initializer) = if inferred {
                            let object = self.register_incomplete_local_array(name, ty, 0);
                            self.inferred_local_array_initializer(&object, &token)
                        } else {
                            let object = self.register_local_variable(name, ty);
                            let initializer = self.local_variable_initialization(&object, &token);
                            (object, initializer)
                        };
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
                        let data = self.global_initializer(&ty);
                        let count = if let GlobalVariableData::Arr(elements) = &data {
                            elements.len()
                        } else {
                            0
                        };
                        ty = self.infer_array_size(ty, count, &token);
                        self.expect_object_type(&ty, &token);
                        self.validate_global_initializer(&ty, &data, &token);
                        self.global_variables.insert(
                            name.s_value.clone(),
                            GlobalVariable {
                                ty: ty.clone(),
                                alignment: 0,
                                thread_local: false,
                                data: Some(data),
                                internal: true,
                                weak_import: false,
                                common: false,
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
                        Type::I8
                            | Type::Enum(_)
                            | Type::SignedChar
                            | Type::LongLong { .. }
                            | Type::I32
                            | Type::Integer { .. }
                            | Type::Bool
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
                let pending_start = self.pending_bounds.len();
                if let Some(base) = self.attempt_type() {
                    let (name, ty, _, _, _) = self.declarator().resolve(base);
                    self.validate_type_layout(&ty, &t);
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
                || !ty.is_complete_object()
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

    fn predefined_function_name(&mut self, token: Token) -> Node {
        let Some(function) = self.current_function.clone() else {
            self.error_logger
                .print_error_position(token.pos, "__func__ is only available inside a function");
            return Node::default();
        };
        let name = format!(".static.{}.__func__", function.pos);
        let element = Type::Qualified(
            Box::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        );
        let ty = Type::Arr(Box::new(element), Some(function.s_value.len() + 1));
        self.global_variables
            .entry(name.clone())
            .or_insert_with(|| GlobalVariable {
                ty: ty.clone(),
                alignment: 1,
                thread_local: false,
                internal: true,
                weak_import: false,
                common: false,
                declaration_only: false,
                data: Some(GlobalVariableData::Arr(
                    function
                        .s_value
                        .bytes()
                        .chain([0])
                        .map(|byte| GlobalVariableData::Elem(byte.to_string()))
                        .collect(),
                )),
            });
        Node {
            nt: NodeType::GlobalVar,
            global_name: name,
            token: Some(token),
            cty: Some(ty),
            ..Node::default()
        }
    }

    fn branch_expectation_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let value = self.assign();
        self.expect_reserved(",");
        let expected = self.assign();
        self.expect_reserved(")");
        let ty = Type::integer(64, true);
        self.expect_conversion(&ty, &value);
        self.expect_conversion(&ty, &expected);
        let cast = |operand| Node {
            nt: NodeType::Cast,
            cty: Some(ty.clone()),
            lhs: Some(Box::new(operand)),
            token: Some(token.clone()),
            ..Node::default()
        };
        let value = cast(value);
        if !expected.has_side_effects() {
            return value;
        }
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "branch expectation is not a constant expression");
        }
        let expected = cast(expected);
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".expect.{}.{}", token.pos, self.offset_size),
                ..token.clone()
            },
            ty,
        );
        let save = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            temporary.clone(),
            value,
        );
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![save, expected, temporary],
            ..Node::default()
        }
    }

    fn floating_infinity_builtin(&mut self, token: Token) -> Node {
        let ty = match token.s_value.as_str() {
            "__builtin_inff" | "__builtin_huge_valf" => Type::Float(32),
            "__builtin_inf" | "__builtin_huge_val" => Type::Float(64),
            "__builtin_infl" | "__builtin_huge_vall" => self.long_double_type(&token),
            _ => unreachable!(),
        };
        self.expect_reserved("(");
        self.expect_reserved(")");
        Node {
            token: Some(token),
            value: Some(ty.float_constant(f64::INFINITY)),
            cty: Some(ty),
            ..Node::default()
        }
    }

    fn floating_nan_builtin(&mut self, token: Token) -> Node {
        let (name, ty) = match token.s_value.as_str() {
            "__builtin_nanf" => ("nanf", Type::Float(32)),
            "__builtin_nan" => ("nan", Type::Float(64)),
            "__builtin_nanl" => ("nanl", self.long_double_type(&token)),
            _ => unreachable!(),
        };
        let pointer = Type::Ptr(Box::new(Type::Qualified(
            Box::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let call = self.runtime_builtin(
            token.clone(),
            name,
            Type::Func(vec![pointer], Box::new(ty.clone())),
        );
        let literal = call
            .args
            .first()
            .filter(|argument| call.nt == NodeType::CallFunc && argument.nt == NodeType::GlobalVar)
            .and_then(|argument| argument.token.as_ref())
            .filter(|literal| {
                literal.tt == TokenType::Str && literal.string_encoding.unit_size() == 1
            });
        if let Some(literal) = literal {
            let payload = Self::nan_payload(&literal.string_bytes);
            let bits = if ty.float_bits() == Some(32) {
                0x7fc0_0000 | (payload & 0x003f_ffff)
            } else {
                0x7ff8_0000_0000_0000 | (payload & 0x0007_ffff_ffff_ffff)
            };
            return Node {
                token: Some(token),
                value: Some(bits as usize),
                cty: Some(ty),
                ..Node::default()
            };
        }
        call
    }

    fn nan_payload(bytes: &[u8]) -> u64 {
        let bytes = bytes.split(|byte| *byte == 0).next().unwrap();
        let (radix, digits) = if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
            (16, &bytes[2..])
        } else if bytes.starts_with(b"0") {
            (8, &bytes[1..])
        } else {
            (10, bytes)
        };
        let mut value = 0u64;
        for byte in digits {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return 0,
            };
            if u64::from(digit) >= radix {
                return 0;
            }
            // Literal payloads retain their low bits even when their spelling exceeds u64.
            value = value.wrapping_mul(radix).wrapping_add(u64::from(digit));
        }
        value
    }

    fn floating_abs_builtin(&mut self, token: Token) -> Node {
        let ty = match token.s_value.as_str() {
            "__builtin_fabsf" => Type::Float(32),
            "__builtin_fabs" => Type::Float(64),
            "__builtin_fabsl" => self.long_double_type(&token),
            _ => unreachable!(),
        };
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        Node {
            nt: NodeType::FloatAbs,
            token: Some(token),
            cty: Some(ty),
            lhs: Some(Box::new(argument)),
            ..Node::default()
        }
    }

    fn checked_memory_builtin(&mut self, token: Token) -> Node {
        let pointer = Type::Ptr(Box::new(Type::Void));
        let const_pointer = Type::Ptr(Box::new(Type::Qualified(
            Box::new(Type::Void),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let (name, second) = match token.s_value.as_str() {
            "__builtin___memcpy_chk" => ("__memcpy_chk", const_pointer),
            "__builtin___memmove_chk" => ("__memmove_chk", const_pointer),
            "__builtin___memset_chk" => ("__memset_chk", Type::I32),
            _ => unreachable!(),
        };
        let size = Type::integer(64, false);
        let ty = Type::Func(
            vec![pointer.clone(), second, size.clone(), size],
            Box::new(pointer),
        );
        self.runtime_builtin(token, name, ty)
    }

    fn checked_format_builtin(&mut self, token: Token) -> Node {
        let pointer = Type::Ptr(Box::new(Type::I8));
        let format = Type::Ptr(Box::new(Type::Qualified(
            Box::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let size = Type::integer(64, false);
        let (name, parameters) = match token.s_value.as_str() {
            "__builtin___sprintf_chk" => ("__sprintf_chk", vec![pointer, Type::I32, size, format]),
            "__builtin___snprintf_chk" => (
                "__snprintf_chk",
                vec![pointer, size.clone(), Type::I32, size, format],
            ),
            _ => unreachable!(),
        };
        self.runtime_builtin(
            token,
            name,
            Type::VariadicFunc(parameters, Box::new(Type::I32)),
        )
    }

    fn checked_bsd_string_builtin(&mut self, token: Token) -> Node {
        let pointer = Type::Ptr(Box::new(Type::I8));
        let source = Type::Ptr(Box::new(Type::Qualified(
            Box::new(Type::I8),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        )));
        let size = Type::integer(64, false);
        let name = match token.s_value.as_str() {
            "__builtin___strlcpy_chk" => "__strlcpy_chk",
            "__builtin___strlcat_chk" => "__strlcat_chk",
            _ => unreachable!(),
        };
        self.runtime_builtin(
            token,
            name,
            Type::Func(
                vec![pointer, source, size.clone(), size.clone()],
                Box::new(size),
            ),
        )
    }

    fn runtime_builtin(&mut self, token: Token, name: &str, ty: Type) -> Node {
        if self.tokens[self.cur].s_value != "(" {
            self.error_logger
                .print_error_position(token.pos, "runtime builtin must be called directly");
        }
        if self.global_variables.contains_key(name)
            || self.scope_stack[0]
                .get(name)
                .is_some_and(|identifier| !matches!(identifier, Identifier::Static(_)))
        {
            self.error_logger
                .print_error_position(token.pos, "runtime symbol conflicts with an identifier");
        }
        if let Some(function) = self.functions.get(name) {
            if !function.cty.compatible(&ty) {
                self.error_logger
                    .print_error_position(token.pos, "incompatible runtime declaration");
            }
        }
        self.functions.entry(name.into()).or_insert_with(|| Func {
            cty: ty.clone(),
            token: Some(token.clone()),
            ..Func::default()
        });
        self.scope_stack[0]
            .entry(name.into())
            .or_insert_with(|| Identifier::Static(ty.clone()));
        self.postfix(Node {
            nt: NodeType::GlobalVar,
            cty: Some(ty),
            token: Some(token),
            global_name: name.into(),
            ..Node::default()
        })
    }

    fn foldable_integer_constant(&mut self, node: &Node) -> bool {
        if !Self::integer_constant(node) {
            return false;
        }
        if node.nt == NodeType::Cast
            && node
                .lhs
                .as_ref()
                .is_some_and(|child| child.nt == NodeType::Num)
        {
            return true;
        }
        if [&node.lhs, &node.rhs, &node.cond, &node.then, &node.els]
            .into_iter()
            .flatten()
            .any(|child| !self.foldable_integer_constant(child))
        {
            return false;
        }
        if matches!(node.nt, NodeType::Div | NodeType::Mod) {
            let right = self.eval(node.rhs.as_ref().unwrap());
            if right == 0 {
                return false;
            }
            let ty = node.resolve_type().unwrap();
            let (bits, signed) = ty.integer_info().unwrap();
            if signed
                && right == -1
                && i128::from(self.eval(node.lhs.as_ref().unwrap())) == -(1i128 << (bits - 1))
            {
                return false;
            }
        }
        if matches!(node.nt, NodeType::BitLeft | NodeType::BitRight) {
            let shift = self.eval(node.rhs.as_ref().unwrap());
            let bits = node.resolve_type().unwrap().integer_info().unwrap().0;
            if shift < 0 || shift >= i64::from(bits) {
                return false;
            }
        }
        true
    }

    fn constant_query_builtin(&mut self, token: Token) -> Node {
        self.expect_reserved("(");
        let bounds_start = self.pending_bounds.len();
        let operand = self.assign();
        self.expect_reserved(")");
        let has_bounds = self.pending_bounds.len() != bounds_start;
        self.pending_bounds.truncate(bounds_start);
        let literal = operand.nt == NodeType::Num
            || (operand.nt == NodeType::GlobalVar
                && operand
                    .token
                    .as_ref()
                    .is_some_and(|token| token.tt == TokenType::Str));
        let known = !has_bounds
            && !operand.has_side_effects()
            && (literal || self.foldable_integer_constant(&operand));
        Node::new_with_num(Some(token), usize::from(known))
    }

    fn bit_scan_builtin(&mut self, token: Token, bits: u8, leading: bool) -> Node {
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        let ty = Type::integer(bits, false);
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        if Self::integer_constant(&argument) {
            let value = self.eval(&argument) as u64 & (u64::MAX >> (64 - bits));
            let count = if value == 0 {
                u32::from(bits)
            } else if leading {
                value.leading_zeros() - u32::from(64 - bits)
            } else {
                value.trailing_zeros()
            };
            return Node::new_with_num(Some(token), count as usize);
        }
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "bit-scan operand is not a constant expression");
        }
        Node {
            cty: Some(Type::I32),
            ..Node::new_with_op_and_lhs(Some(token), NodeType::BitScan { bits, leading }, argument)
        }
    }

    fn population_count_builtin(&mut self, token: Token, bits: u8) -> Node {
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        let ty = Type::integer(bits, false);
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        if Self::integer_constant(&argument) {
            let value = self.eval(&argument) as u64 & (u64::MAX >> (64 - bits));
            return Node::new_with_num(Some(token), value.count_ones() as usize);
        }
        if self.return_type.is_none() {
            self.error_logger.print_error_position(
                token.pos,
                "population-count operand is not a constant expression",
            );
        }
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".popcount.{}.{}", token.pos, self.offset_size),
                ..token.clone()
            },
            ty.clone(),
        );
        let binary = |nt, lhs, rhs| Node::new_with_op(Some(token.clone()), nt, lhs, rhs);
        let number = |value| Node {
            cty: Some(ty.clone()),
            ..Node::new_with_num(Some(token.clone()), value & (usize::MAX >> (64 - bits)))
        };
        let assign = |value| binary(NodeType::Assign, temporary.clone(), value);
        let shift = |amount| binary(NodeType::BitRight, temporary.clone(), number(amount));
        let mut children = vec![assign(argument)];
        // Sum adjacent groups of bits, then accumulate the byte counts.
        children.push(assign(binary(
            NodeType::Sub,
            temporary.clone(),
            binary(NodeType::BitAnd, shift(1), number(0x5555_5555_5555_5555)),
        )));
        children.push(assign(binary(
            NodeType::Add,
            binary(
                NodeType::BitAnd,
                temporary.clone(),
                number(0x3333_3333_3333_3333),
            ),
            binary(NodeType::BitAnd, shift(2), number(0x3333_3333_3333_3333)),
        )));
        children.push(assign(binary(
            NodeType::BitAnd,
            binary(NodeType::Add, temporary.clone(), shift(4)),
            number(0x0f0f_0f0f_0f0f_0f0f),
        )));
        for amount in [8, 16, 32]
            .into_iter()
            .filter(|amount| *amount < usize::from(bits))
        {
            children.push(assign(binary(
                NodeType::Add,
                temporary.clone(),
                shift(amount),
            )));
        }
        let mut result = binary(NodeType::BitAnd, temporary.clone(), number(127));
        Self::cast_integer(&mut result, &Type::I32);
        children.push(result);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children,
            ..Node::default()
        }
    }

    fn byte_swap_builtin(&mut self, token: Token, bits: u8) -> Node {
        self.expect_reserved("(");
        let mut argument = self.assign();
        self.expect_reserved(")");
        let ty = if bits == 64 {
            Type::LongLong { signed: false }
        } else {
            Type::integer(bits, false)
        };
        self.expect_conversion(&ty, &argument);
        Self::cast_integer(&mut argument, &ty);
        if Self::integer_constant(&argument) {
            let value = self.eval(&argument) as u64;
            let value = match bits {
                16 => u64::from((value as u16).swap_bytes()),
                32 => u64::from((value as u32).swap_bytes()),
                64 => value.swap_bytes(),
                _ => unreachable!(),
            };
            return Node {
                cty: Some(ty),
                ..Node::new_with_num(Some(token), value as usize)
            };
        }
        if self.return_type.is_none() {
            self.error_logger
                .print_error_position(token.pos, "byte-swap operand is not a constant expression");
        }
        let temporary = self.register_local_variable(
            Token {
                s_value: format!(".bswap.{}.{}", token.pos, self.offset_size),
                ..token.clone()
            },
            ty.clone(),
        );
        let assignment = Node::new_with_op(
            Some(token.clone()),
            NodeType::Assign,
            temporary.clone(),
            argument,
        );
        let mut result = Node::new_with_num(Some(token.clone()), 0);
        for index in 0..usize::from(bits / 8) {
            let shifted = Node::new_with_op(
                Some(token.clone()),
                NodeType::BitRight,
                temporary.clone(),
                Node::new_with_num(Some(token.clone()), index * 8),
            );
            let byte = Node::new_with_op(
                Some(token.clone()),
                NodeType::BitAnd,
                shifted,
                Node::new_with_num(Some(token.clone()), 255),
            );
            let byte = Node::new_with_op(
                Some(token.clone()),
                NodeType::BitLeft,
                byte,
                Node::new_with_num(Some(token.clone()), usize::from(bits) - 8 - index * 8),
            );
            result = Node::new_with_op(Some(token.clone()), NodeType::BitOr, result, byte);
        }
        Self::cast_integer(&mut result, &ty);
        Node {
            nt: NodeType::Sequence,
            token: Some(token),
            children: vec![assignment, result],
            ..Node::default()
        }
    }

    fn prim(&mut self) -> Node {
        let node = if let Some(token) = self.attempt_reserved("_Generic") {
            self.generic_selection(token)
        } else if let Some(token) = self.attempt_reserved("(") {
            if self.tokens[self.cur].tt == TokenType::Reserved
                && self.tokens[self.cur].s_value == "{"
            {
                self.statement_expression(token)
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
            Node {
                token: Some(t.clone()),
                nt: NodeType::GlobalVar,
                cty: Some(Type::Arr(Box::new(element_type), Some(count))),
                dest: self.new_string_literal(&t),
                ..Node::default()
            }
        } else if let Some(t) = self.attempt(TokenType::Float) {
            Node {
                nt: NodeType::Num,
                value: Some(t.i_value),
                cty: Some(if t.s_value == "l" {
                    self.long_double_type(&t)
                } else {
                    Type::Float(if t.s_value == "f" { 32 } else { 64 })
                }),
                token: Some(t),
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
                token: Some(t),
                nt: NodeType::Num,
                value: Some(value),
                cty: Some(ty),
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
                        vec![Type::Ptr(Box::new(Type::Void)), Type::integer(64, false)],
                        Box::new(Type::Void),
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
            if t.s_value == "__func__" {
                let name = self.predefined_function_name(t);
                return self.postfix(name);
            }
            if t.s_value == "__builtin_expect" {
                return self.branch_expectation_builtin(t);
            }
            if matches!(
                t.s_value.as_str(),
                "__builtin___memcpy_chk" | "__builtin___memmove_chk" | "__builtin___memset_chk"
            ) {
                return self.checked_memory_builtin(t);
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
                .get(&t.s_value)
                .is_some_and(|object| object.thread_local);
            if let Some(ident) = self.resolve_name(&t.s_value) {
                match ident {
                    Identifier::Dynamic(ty, offset, registered) => {
                        let mut node = Self::dynamic_node(ty.clone(), *offset, t.clone());
                        node.registered = *registered;
                        node
                    }
                    Identifier::Local(ty, offset, registered, alignment) => Node {
                        registered: *registered,
                        alignment: *alignment,
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
                        thread_local: self
                            .global_variables
                            .get(name)
                            .is_some_and(|object| object.thread_local),
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
                    Identifier::Constant(value, ty) => Node {
                        cty: Some(ty.clone()),
                        ..Node::new_with_num(Some(t.clone()), *value as usize)
                    },
                    Identifier::TypeDef(..) | Identifier::EnumTag { .. } => {
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
                        Type::Ptr(child) => child.into_function_signature(),
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
                    node = Self::member_node(node, &st.member(index), &token);
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
                    weak_import: false,
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

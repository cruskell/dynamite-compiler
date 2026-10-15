use crate::ctype::{BitField, Type};
use crate::token::Token;
use std::mem::swap;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum AtomicRmwOperation {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Nand,
    Exchange,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum AtomicOperation {
    Load,
    Store,
    Update(AtomicRmwOperation, bool),
    CompareExchange(bool),
}

/// Decode GNU numbered register operands, leaving target-specific modifiers to the backend.
pub(crate) fn render_inline_assembly(
    template: &str,
    mut operand: impl FnMut(usize, Option<char>) -> Option<String>,
) -> Option<String> {
    let mut rendered = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            rendered.push(ch);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            rendered.push('%');
            continue;
        }
        let modifier = chars
            .peek()
            .copied()
            .filter(char::is_ascii_alphabetic)
            .map(|ch| {
                chars.next();
                ch
            });
        let mut index = None;
        while let Some(ch) = chars.peek().copied().filter(char::is_ascii_digit) {
            chars.next();
            index = Some(
                index
                    .unwrap_or(0usize)
                    .checked_mul(10)?
                    .checked_add(ch.to_digit(10)? as usize)?,
            );
        }
        rendered.push_str(&operand(index?, modifier)?);
    }
    Some(rendered)
}

#[derive(Debug, PartialEq, Clone)]
pub enum NodeType {
    InlineAsm(bool, bool),
    Prefetch(bool, u8),
    Atomic(AtomicOperation),
    VaStart,
    VaEnd,
    Assign,
    Cast,
    FloatAbs,
    LocalVar,
    GlobalVar,
    Add,
    AddOverflow,
    SubOverflow,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    BitLeft,
    BitRight,
    BitAnd,
    BitXor,
    BitOr,
    BitNot,
    LogicalAnd,
    LogicalOr,
    Num,
    Return,
    If,
    Conditional,
    While,
    DoWhile,
    For,
    Switch,
    Case,
    Break,
    Continue,
    Goto,
    Label,
    Block,
    Sequence,
    CallFunc,
    Addr,
    Deref,
    DefVar,
    SuffixIncr,
    SuffixDecr,
}

impl NodeType {
    /// Evaluation policy for ordinary binary value operations handled by the backends.
    pub(crate) fn left_operand_first(&self) -> bool {
        matches!(self, Self::BitAnd | Self::BitXor | Self::BitOr)
    }
}

impl Default for NodeType {
    fn default() -> Self {
        Self::Num
    }
}

#[derive(Default, Clone)]
pub struct Node {
    pub nt: NodeType,
    pub bit_field: Option<BitField>,
    /// Preserve lowered member identity for closest-subobject size queries.
    pub member_access: bool,
    pub compound_literal: bool,
    pub volatile_access: bool,
    pub registered: bool,
    pub thread_local: bool,
    pub cty: Option<Type>,
    pub token: Option<Token>,
    pub lhs: Option<Box<Node>>,
    pub rhs: Option<Box<Node>>,
    // for number
    pub value: Option<usize>,
    // for "if", "for", "while" statement
    pub cond: Option<Box<Node>>,
    pub then: Option<Box<Node>>,
    // for "if" statement
    pub els: Option<Box<Node>>,
    // for "for" statement
    pub ini: Option<Box<Node>>,
    pub upd: Option<Box<Node>>,
    // For blocks, local definitions, or inline assembly output destinations.
    pub children: Vec<Node>,
    // flag of global variables
    pub global_name: String,
    /// String symbol name, or the validated source template for inline assembly.
    pub dest: String,
    // Function arguments or inline assembly inputs.
    pub args: Vec<Node>,
    pub parameter_types: Vec<Type>,
    pub variadic: bool,
    pub vla_scope: Vec<usize>,
    // for declaration of local variables
    pub offset: Option<usize>,
    /// Storage alignment requested by the local object declaration.
    pub alignment: usize,
}

impl Node {
    /// Whether evaluating this expression can call, write, or read a volatile value.
    pub(crate) fn has_side_effects(&self) -> bool {
        Self::has_effects(self, true)
    }

    fn has_effects(node: &Self, value: bool) -> bool {
        if matches!(
            node.nt,
            NodeType::InlineAsm(_, _)
                | NodeType::Prefetch(..)
                | NodeType::Atomic(_)
                | NodeType::Assign
                | NodeType::CallFunc
                | NodeType::AddOverflow
                | NodeType::SubOverflow
                | NodeType::SuffixIncr
                | NodeType::SuffixDecr
                | NodeType::VaStart
                | NodeType::VaEnd
        ) || (value
            && (node.volatile_access
                || node.object_type().is_some_and(|ty| {
                    ty.qualifiers().volatile
                        && !matches!(ty.unqualified(), Type::Arr(..) | Type::Vla(..))
                })))
        {
            return true;
        }
        if node.nt == NodeType::Addr {
            return node
                .lhs
                .as_deref()
                .is_some_and(|child| Self::has_effects(child, false));
        }
        [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        .any(|child| Self::has_effects(child, true))
            || node
                .children
                .iter()
                .chain(&node.args)
                .any(|child| Self::has_effects(child, true))
    }

    pub fn new_with_op(token: Option<Token>, nt: NodeType, lhs: Node, rhs: Node) -> Self {
        let (mut lhs, mut rhs) = (lhs, rhs);
        if let NodeType::Add = nt {
            if lhs.dest_type().is_none() && rhs.dest_type().is_some() {
                swap(&mut lhs, &mut rhs);
            }
        }
        Self {
            token,
            nt,
            lhs: Some(Box::new(lhs)),
            rhs: Some(Box::new(rhs)),
            ..Self::default()
        }
    }
    pub fn new_with_op_and_lhs(token: Option<Token>, nt: NodeType, lhs: Node) -> Self {
        Self {
            token,
            nt,
            lhs: Some(Box::new(lhs)),
            ..Self::default()
        }
    }
    pub fn new_with_num(token: Option<Token>, value: usize) -> Self {
        Self {
            token,
            nt: NodeType::Num,
            value: Some(value),
            ..Self::default()
        }
    }
    pub fn new_if_node(token: Option<Token>, cond: Node, then: Node, els: Option<Node>) -> Self {
        Self {
            token,
            nt: NodeType::If,
            cond: Some(Box::new(cond)),
            then: Some(Box::new(then)),
            els: els.map(Box::new),
            ..Self::default()
        }
    }
    pub fn new_while_node(token: Option<Token>, cond: Node, then: Node) -> Self {
        Self {
            token,
            nt: NodeType::While,
            cond: Some(Box::new(cond)),
            then: Some(Box::new(then)),
            ..Self::default()
        }
    }
    pub fn new_for_node(
        token: Option<Token>,
        ini: Option<Node>,
        cond: Option<Node>,
        upd: Option<Node>,
        then: Node,
    ) -> Self {
        Self {
            token,
            nt: NodeType::For,
            ini: ini.map(Box::new),
            cond: cond.map(Box::new),
            upd: upd.map(Box::new),
            then: Some(Box::new(then)),
            ..Self::default()
        }
    }
    pub fn contains_case(&self) -> bool {
        if matches!(self.nt, NodeType::Switch) {
            return false;
        }
        if matches!(self.nt, NodeType::Case | NodeType::Label) {
            return true;
        }
        self.children.iter().any(Self::contains_case)
            || [
                &self.lhs, &self.rhs, &self.cond, &self.then, &self.els, &self.ini, &self.upd,
            ]
            .into_iter()
            .flatten()
            .any(|node| node.contains_case())
    }

    pub fn object_type(&self) -> Option<Type> {
        match self.nt {
            NodeType::LocalVar | NodeType::GlobalVar => self.cty.clone(),
            NodeType::Deref => self.lhs.as_ref()?.resolve_type()?.dest_type(),
            NodeType::SuffixIncr | NodeType::SuffixDecr => self.lhs.as_ref()?.object_type(),
            _ => self.resolve_type(),
        }
    }
    pub fn resolve_type(&self) -> Option<Type> {
        if self.bit_field.is_some_and(|field| field.width < 32) {
            return Some(Type::I32);
        }
        self.resolve_qualified_type().map(|ty| {
            if let Type::Qualified(child, _) = &ty {
                if let Type::Arr(element, size) = child.unqualified() {
                    return Type::Arr(
                        Box::new(Type::Qualified(element.clone(), ty.qualifiers())),
                        *size,
                    );
                }
                if let Type::Vla(element, offset) = child.unqualified() {
                    return Type::Vla(
                        Box::new(Type::Qualified(element.clone(), ty.qualifiers())),
                        *offset,
                    );
                }
            }
            ty.unqualified().clone()
        })
    }
    pub(crate) fn resolve_qualified_type(&self) -> Option<Type> {
        match self.nt {
            NodeType::Eq
            | NodeType::Ne
            | NodeType::Lt
            | NodeType::Le
            | NodeType::LogicalAnd
            | NodeType::LogicalOr => Some(Type::I32),
            NodeType::Conditional => {
                let then = self
                    .then
                    .as_ref()
                    .unwrap()
                    .resolve_type()
                    .map(|ty| ty.decayed());
                let otherwise = self
                    .els
                    .as_ref()
                    .unwrap()
                    .resolve_type()
                    .map(|ty| ty.decayed());
                if then.as_ref().is_some_and(|ty| ty.is_arithmetic())
                    && otherwise.as_ref().is_some_and(|ty| ty.is_arithmetic())
                {
                    Some(Type::arithmetic_type(
                        then.as_ref().unwrap(),
                        otherwise.as_ref().unwrap(),
                    ))
                } else {
                    match (then, otherwise) {
                        (Some(Type::Ptr(child) | Type::Arr(child, _) | Type::Vla(child, _)), _)
                        | (_, Some(Type::Ptr(child) | Type::Arr(child, _) | Type::Vla(child, _))) => {
                            Some(Type::Ptr(child))
                        }
                        (then, _) => then,
                    }
                }
            }
            NodeType::Cast | NodeType::FloatAbs | NodeType::InlineAsm(_, _) => self.cty.clone(),
            NodeType::Num => Some(self.cty.clone().unwrap_or(Type::I32)),
            NodeType::Sequence => self.children.last().and_then(Node::resolve_type),
            NodeType::Add | NodeType::Sub => {
                let ty = self.lhs.as_ref().unwrap().resolve_type();
                if matches!(self.nt, NodeType::Sub)
                    && self.rhs.as_ref().unwrap().dest_type().is_some()
                {
                    return Some(Type::I32);
                }
                if ty.as_ref().is_some_and(|ty| ty.dest_type().is_some()) {
                    ty
                } else {
                    Some(Type::arithmetic_type(
                        &ty.unwrap(),
                        &self.rhs.as_ref().unwrap().resolve_type().unwrap(),
                    ))
                }
            }
            NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitAnd
            | NodeType::BitXor
            | NodeType::BitOr => Some(Type::arithmetic_type(
                &self.lhs.as_ref().unwrap().resolve_type().unwrap(),
                &self.rhs.as_ref().unwrap().resolve_type().unwrap(),
            )),
            NodeType::BitNot | NodeType::BitLeft | NodeType::BitRight => Some(
                self.lhs
                    .as_ref()
                    .unwrap()
                    .resolve_type()
                    .unwrap()
                    .promoted(),
            ),
            NodeType::Prefetch(..)
            | NodeType::Atomic(_)
            | NodeType::LocalVar
            | NodeType::CallFunc
            | NodeType::AddOverflow
            | NodeType::SubOverflow
            | NodeType::GlobalVar
            | NodeType::VaStart
            | NodeType::VaEnd => self.cty.clone(),
            NodeType::Addr => {
                if let Some(ty) = self.lhs.as_ref().unwrap().object_type() {
                    Some(Type::Ptr(Box::new(ty)))
                } else {
                    self.cty.clone()
                }
            }
            NodeType::Deref => self.lhs.as_ref().unwrap().dest_type(),
            _ => {
                if let Some(node) = self.lhs.as_ref() {
                    if let Some(ty) = node.resolve_type() {
                        return Some(ty);
                    }
                }
                if let Some(node) = self.rhs.as_ref() {
                    if let Some(ty) = node.resolve_type() {
                        return Some(ty);
                    }
                }
                if let Some(node) = self.then.as_ref() {
                    if let Some(ty) = node.resolve_type() {
                        return Some(ty);
                    }
                }
                None
            }
        }
    }
    pub fn dest_type(&self) -> Option<Type> {
        if let Some(t) = self.resolve_type() {
            t.dest_type()
        } else {
            None
        }
    }
    pub fn to_debug_string(&self, indent: usize) -> String {
        match self.nt {
            NodeType::LocalVar => {
                format!(
                    "LocalVar: {{ type: {:?}, offset: {} }}",
                    self.cty.as_ref().unwrap(),
                    self.offset.unwrap()
                )
            }
            NodeType::GlobalVar => {
                format!("GlobalVar: {{ name: {}{} }}", &self.global_name, &self.dest)
            }
            NodeType::Num => {
                format!("Num: {}", self.value.unwrap())
            }
            _ => [
                vec![format!("{:?}:", self.nt)],
                self.ini
                    .iter()
                    .map(|c| {
                        format!(
                            "{}ini->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.cond
                    .iter()
                    .map(|c| {
                        format!(
                            "{}cond->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.upd
                    .iter()
                    .map(|c| {
                        format!(
                            "{}upd->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.then
                    .iter()
                    .map(|c| {
                        format!(
                            "{}then->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.els
                    .iter()
                    .map(|c| {
                        format!(
                            "{}els->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.lhs
                    .iter()
                    .map(|c| {
                        format!(
                            "{}lhs->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.rhs
                    .iter()
                    .map(|c| {
                        format!(
                            "{}rhs->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.children
                    .iter()
                    .map(|c| {
                        format!(
                            "{}children->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.args
                    .iter()
                    .map(|c| {
                        format!(
                            "{}args->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
            ]
            .concat()
            .join("\n"),
        }
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_debug_string(2))
    }
}

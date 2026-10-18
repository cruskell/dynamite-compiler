use crate::ctype::{BitField, Type};
use crate::token::Token;
use std::borrow::Cow;
use std::mem::swap;
use std::rc::Rc;

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

#[derive(Debug, PartialEq, Clone)]
pub enum NodeType {
    // Values and conversions.
    Num,
    LocalVar,
    GlobalVar,
    Addr,
    Deref,
    Cast,

    // Arithmetic, bitwise and logical expressions.
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitLeft,
    BitRight,
    BitAnd,
    BitXor,
    BitOr,
    BitNot,
    Eq,
    Ne,
    Lt,
    Le,
    LogicalAnd,
    LogicalOr,
    Conditional,

    // Assignments, calls and expression sequencing.
    Assign,
    SuffixIncr,
    SuffixDecr,
    CallFunc,
    Sequence,

    // Declarations and control flow.
    DefVar,
    Block,
    Return,
    If,
    While,
    DoWhile,
    For,
    Switch,
    Case,
    Break,
    Continue,
    Goto,
    Label,

    // Builtins and lowered operations.
    AddOverflow,
    SubOverflow,
    FloatAbs,
    BitScan {
        bits: u8,
        leading: bool,
    },
    VaStart,
    VaEnd,
    StackAlloc,
    Prefetch(bool, u8),
    Atomic(AtomicOperation),
    InlineAsm(bool, bool),
    ZeroFill {
        bytes: usize,
    },
    /// Copy storage once, retaining the destination address as the result.
    CopyBytes {
        bytes: usize,
    },
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
    pub token: Option<Token>,
    /// Stored object/call/cast type, or an inferred scalar arithmetic result.
    pub cty: Option<Type>,

    pub lhs: Option<Box<Node>>,
    pub rhs: Option<Box<Node>>,
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

    // for number
    pub value: Option<usize>,
    pub extended_float: Option<Box<u128>>,

    // Global symbol name.
    pub global_name: String,
    /// String symbol name, or the validated source template for inline assembly.
    pub dest: String,

    // Function arguments or inline assembly inputs.
    pub args: Vec<Node>,
    pub parameter_types: Vec<Type>,
    pub variadic: bool,

    // for declaration of local variables
    pub offset: Option<usize>,
    /// Storage alignment requested by the local object declaration.
    pub alignment: usize,
    pub bit_field: Option<BitField>,
    pub vla_scope: Vec<usize>,

    /// Preserve lowered member identity for closest-subobject size queries.
    pub member_access: bool,
    pub compound_literal: bool,
    pub volatile_access: bool,
    pub registered: bool,
    pub thread_local: bool,
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
                | NodeType::ZeroFill { .. }
                | NodeType::CopyBytes { .. }
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
        let mut node = Self {
            token,
            nt,
            lhs: Some(Box::new(lhs)),
            rhs: Some(Box::new(rhs)),
            ..Self::default()
        };
        node.refresh_arithmetic_type();
        node
    }
    pub fn new_with_op_and_lhs(token: Option<Token>, nt: NodeType, lhs: Node) -> Self {
        let mut node = Self {
            token,
            nt,
            lhs: Some(Box::new(lhs)),
            ..Self::default()
        };
        node.refresh_arithmetic_type();
        node
    }

    /// Recompute an arithmetic result after changing its operands or their types.
    /// Refresh children first when changing an existing expression tree.
    pub fn refresh_arithmetic_type(&mut self) {
        // Reuse cty so queries need neither a subtree walk nor a separate cache.
        if matches!(
            self.nt,
            NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Mod
                | NodeType::BitAnd
                | NodeType::BitXor
                | NodeType::BitOr
                | NodeType::BitNot
                | NodeType::BitLeft
                | NodeType::BitRight
        ) {
            self.cty = self
                .infer_arithmetic_type_view()
                .filter(|ty| ty.is_arithmetic())
                .map(Cow::into_owned);
        }
    }

    fn infer_arithmetic_type_view(&self) -> Option<Cow<'_, Type>> {
        let lhs = self.lhs.as_ref()?.resolve_type_view()?;
        match self.nt {
            NodeType::Add | NodeType::Sub => {
                let rhs = self.rhs.as_ref()?;
                if self.nt == NodeType::Sub && rhs.dest_type_view().is_some() {
                    Some(Cow::Owned(Type::I32))
                } else if lhs.dest_type_ref().is_some() {
                    Some(lhs)
                } else {
                    Some(Cow::Owned(Type::arithmetic_type(
                        &lhs,
                        rhs.resolve_type_view()?.as_ref(),
                    )))
                }
            }
            NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitAnd
            | NodeType::BitXor
            | NodeType::BitOr => Some(Cow::Owned(Type::arithmetic_type(
                &lhs,
                self.rhs.as_ref()?.resolve_type_view()?.as_ref(),
            ))),
            NodeType::BitNot | NodeType::BitLeft | NodeType::BitRight => {
                Some(Cow::Owned(lhs.promoted()))
            }
            _ => None,
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
        self.object_type_view().map(Cow::into_owned)
    }
    pub(crate) fn object_type_view(&self) -> Option<Cow<'_, Type>> {
        match self.nt {
            NodeType::LocalVar | NodeType::GlobalVar => self.cty.as_ref().map(Cow::Borrowed),
            NodeType::Deref => self.lhs.as_ref()?.dest_type_view(),
            NodeType::SuffixIncr | NodeType::SuffixDecr => self.lhs.as_ref()?.object_type_view(),
            _ => self.resolve_type_view(),
        }
    }
    pub(crate) fn is_volatile_object(&self) -> bool {
        match self.nt {
            NodeType::LocalVar | NodeType::GlobalVar => {
                self.cty.as_ref().is_some_and(|ty| ty.qualifiers().volatile)
            }
            NodeType::Deref => self
                .object_type_view()
                .is_some_and(|ty| ty.qualifiers().volatile),
            NodeType::SuffixIncr | NodeType::SuffixDecr => self
                .lhs
                .as_ref()
                .is_some_and(|node| node.is_volatile_object()),
            // Value expressions discard outer object qualifiers in resolve_type.
            _ => false,
        }
    }
    pub fn resolve_type(&self) -> Option<Type> {
        self.resolve_type_view().map(Cow::into_owned)
    }
    /// Borrow stored expression types; own types constructed by value conversion.
    pub(crate) fn resolve_type_view(&self) -> Option<Cow<'_, Type>> {
        if self.bit_field.is_some_and(|field| field.width < 32) {
            return Some(Cow::Owned(Type::I32));
        }
        self.resolve_qualified_type_view().map(|ty| {
            if let Type::Qualified(child, _) = ty.as_ref() {
                if let Type::Arr(element, size) = child.unqualified() {
                    return Cow::Owned(Type::Arr(
                        Rc::new(Type::Qualified(Rc::clone(element), ty.qualifiers())),
                        *size,
                    ));
                }
                if let Type::Vla(element, offset) = child.unqualified() {
                    return Cow::Owned(Type::Vla(
                        Rc::new(Type::Qualified(Rc::clone(element), ty.qualifiers())),
                        *offset,
                    ));
                }
            }
            Type::unqualified_view(ty)
        })
    }
    pub(crate) fn resolve_qualified_type(&self) -> Option<Type> {
        self.resolve_qualified_type_view().map(Cow::into_owned)
    }
    pub(crate) fn resolve_qualified_type_view(&self) -> Option<Cow<'_, Type>> {
        match self.nt {
            NodeType::Eq
            | NodeType::Ne
            | NodeType::Lt
            | NodeType::Le
            | NodeType::LogicalAnd
            | NodeType::LogicalOr => Some(Cow::Owned(Type::I32)),
            NodeType::Conditional => {
                let then = self
                    .then
                    .as_ref()
                    .unwrap()
                    .resolve_type_view()
                    .map(Type::decay_view);
                let otherwise = self
                    .els
                    .as_ref()
                    .unwrap()
                    .resolve_type_view()
                    .map(Type::decay_view);
                if then.as_ref().is_some_and(|ty| ty.is_arithmetic())
                    && otherwise.as_ref().is_some_and(|ty| ty.is_arithmetic())
                {
                    Some(Cow::Owned(Type::arithmetic_type(
                        then.as_ref().unwrap(),
                        otherwise.as_ref().unwrap(),
                    )))
                } else if then.as_ref().is_some_and(|ty| matches!(**ty, Type::Ptr(_))) {
                    then
                } else if otherwise
                    .as_ref()
                    .is_some_and(|ty| matches!(**ty, Type::Ptr(_)))
                {
                    otherwise
                } else {
                    then
                }
            }
            NodeType::Cast | NodeType::FloatAbs | NodeType::InlineAsm(_, _) => {
                self.cty.as_ref().map(Cow::Borrowed)
            }
            NodeType::Num => Some(
                self.cty
                    .as_ref()
                    .map_or(Cow::Owned(Type::I32), Cow::Borrowed),
            ),
            NodeType::Sequence => self.children.last().and_then(Node::resolve_type_view),
            NodeType::Add
            | NodeType::Sub
            | NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitAnd
            | NodeType::BitXor
            | NodeType::BitOr
            | NodeType::BitNot
            | NodeType::BitLeft
            | NodeType::BitRight => self
                .cty
                .as_ref()
                .map(Cow::Borrowed)
                .or_else(|| self.infer_arithmetic_type_view()),
            NodeType::Prefetch(..)
            | NodeType::StackAlloc
            | NodeType::BitScan { .. }
            | NodeType::ZeroFill { .. }
            | NodeType::CopyBytes { .. }
            | NodeType::Atomic(_)
            | NodeType::LocalVar
            | NodeType::CallFunc
            | NodeType::AddOverflow
            | NodeType::SubOverflow
            | NodeType::GlobalVar
            | NodeType::VaStart
            | NodeType::VaEnd => self.cty.as_ref().map(Cow::Borrowed),
            NodeType::Addr => {
                if let Some(ty) = self.lhs.as_ref().unwrap().object_type_view() {
                    Some(Cow::Owned(Type::Ptr(Rc::new(ty.into_owned()))))
                } else {
                    self.cty.as_ref().map(Cow::Borrowed)
                }
            }
            NodeType::Deref => self.lhs.as_ref().unwrap().dest_type_view(),
            _ => {
                if let Some(node) = self.lhs.as_ref() {
                    if let Some(ty) = node.resolve_type_view() {
                        return Some(ty);
                    }
                }
                if let Some(node) = self.rhs.as_ref() {
                    if let Some(ty) = node.resolve_type_view() {
                        return Some(ty);
                    }
                }
                if let Some(node) = self.then.as_ref() {
                    if let Some(ty) = node.resolve_type_view() {
                        return Some(ty);
                    }
                }
                None
            }
        }
    }
    pub fn dest_type(&self) -> Option<Type> {
        self.dest_type_view().map(Cow::into_owned)
    }
    pub(crate) fn dest_type_view(&self) -> Option<Cow<'_, Type>> {
        match Type::unqualified_view(self.resolve_type_view()?) {
            Cow::Borrowed(ty) => ty.dest_type_ref().map(Cow::Borrowed),
            Cow::Owned(Type::Ptr(child) | Type::Arr(child, _) | Type::Vla(child, _)) => {
                Some(Cow::Owned(Rc::unwrap_or_clone(child)))
            }
            Cow::Owned(_) => None,
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
            .inspect(|_| {
                chars.next();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctype::Qualifiers;

    fn object(ty: Type) -> Node {
        Node {
            nt: NodeType::LocalVar,
            cty: Some(ty),
            ..Node::default()
        }
    }

    #[test]
    fn arithmetic_types_are_stored_and_refreshed_after_operand_changes() {
        let mut sum = Node::new_with_op(
            None,
            NodeType::Add,
            object(Type::I32),
            object(Type::integer(64, false)),
        );
        assert_eq!(sum.resolve_type(), Some(Type::integer(64, false)));
        assert!(matches!(sum.resolve_type_view(), Some(Cow::Borrowed(_))));
        sum.rhs.as_mut().unwrap().cty = Some(Type::Float(64));
        sum.refresh_arithmetic_type();
        assert_eq!(sum.resolve_type(), Some(Type::Float(64)));

        let shift = Node::new_with_op(
            None,
            NodeType::BitLeft,
            object(Type::I8),
            object(Type::integer(64, false)),
        );
        assert_eq!(shift.resolve_type(), Some(Type::I32));
        let inverse = Node::new_with_op_and_lhs(None, NodeType::BitNot, shift);
        assert_eq!(inverse.resolve_type(), Some(Type::I32));

        let direct = Node {
            nt: NodeType::Mul,
            lhs: Some(Box::new(sum)),
            rhs: Some(Box::new(inverse)),
            ..Node::default()
        };
        assert_eq!(direct.resolve_type(), Some(Type::Float(64)));
    }

    #[test]
    fn type_views_borrow_stored_types_and_preserve_object_qualification() {
        let node = object(Type::Qualified(
            Rc::new(Type::Aligned(Rc::new(Type::Ptr(Rc::new(Type::I32))), 16)),
            Qualifiers {
                volatile: true,
                ..Qualifiers::default()
            },
        ));
        assert!(
            matches!(node.resolve_qualified_type_view(), Some(Cow::Borrowed(ty)) if std::ptr::eq(ty, node.cty.as_ref().unwrap()))
        );
        let Some(Cow::Borrowed(ty)) = node.resolve_type_view() else {
            panic!("stored value type should be borrowed")
        };
        assert!(std::ptr::eq(ty, node.cty.as_ref().unwrap().unqualified()));
        assert_eq!(node.resolve_type().as_ref(), Some(ty));
        assert!(node.is_volatile_object());

        let deref = Node::new_with_op_and_lhs(
            None,
            NodeType::Deref,
            object(Type::Ptr(Rc::new(node.cty.clone().unwrap()))),
        );
        assert!(deref.is_volatile_object());
        assert!(matches!(
            deref.resolve_type_view(),
            Some(Cow::Borrowed(Type::Ptr(_)))
        ));
        let sequence = Node {
            nt: NodeType::Sequence,
            children: vec![deref],
            ..Node::default()
        };
        assert!(matches!(
            sequence.resolve_type_view(),
            Some(Cow::Borrowed(Type::Ptr(_)))
        ));
    }

    #[test]
    fn type_views_own_constructed_types_and_preserve_array_qualification() {
        let qualifiers = Qualifiers {
            constant: true,
            ..Qualifiers::default()
        };
        for array in [
            Type::Arr(Rc::new(Type::I32), Some(3)),
            Type::Vla(Rc::new(Type::I32), 8),
        ] {
            let node = object(Type::Qualified(Rc::new(array), qualifiers));
            let Some(Cow::Owned(ty)) = node.resolve_type_view() else {
                panic!("qualified array value type must be constructed")
            };
            assert_eq!(ty.dest_type_ref().unwrap().qualifiers(), qualifiers);
            assert_eq!(node.resolve_type(), Some(ty));
            let addr = Node::new_with_op_and_lhs(None, NodeType::Addr, node);
            assert!(
                matches!(addr.resolve_type_view(), Some(Cow::Owned(Type::Ptr(child))) if child.qualifiers() == qualifiers)
            );
        }
        let mut field = object(Type::integer(32, false));
        field.bit_field = Some(BitField {
            width: 3,
            shift: 0,
            bytewise: false,
        });
        assert!(matches!(
            field.resolve_type_view(),
            Some(Cow::Owned(Type::I32))
        ));
    }

    #[test]
    fn conditional_type_views_borrow_pointers_and_decay_functions() {
        let function = Type::Func(
            Rc::new(vec![Type::Ptr(Rc::new(Type::I8))]),
            Rc::new(Type::I32),
        );
        for ty in [
            Type::Ptr(Rc::new(function.clone())),
            function,
            Type::Arr(Rc::new(Type::I32), Some(2)),
        ] {
            let borrowed = matches!(ty, Type::Ptr(_));
            let node = Node::new_if_node(
                None,
                Node::new_with_num(None, 1),
                object(ty.clone()),
                Some(Node::new_with_num(None, 0)),
            );
            let node = Node {
                nt: NodeType::Conditional,
                ..node
            };
            let view = node.resolve_type_view().unwrap();
            assert_eq!(matches!(view, Cow::Borrowed(_)), borrowed);
            assert_eq!(*view, ty.decayed());
        }
    }
}

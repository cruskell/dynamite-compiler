mod debug;
mod effects;
mod types;
mod walk;

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
    pub fn new_with_op(token: Option<Token>, nt: NodeType, lhs: Node, rhs: Node) -> Self {
        let (mut lhs, mut rhs) = (lhs, rhs);
        if let NodeType::Add = nt {
            if lhs.dest_type().is_none() && rhs.dest_type().is_some() {
                swap(&mut lhs, &mut rhs);
            }
        }
        let mut node = Self {
            nt,
            token,
            lhs: Some(Box::new(lhs)),
            rhs: Some(Box::new(rhs)),
            ..Self::default()
        };
        node.refresh_arithmetic_type();
        node
    }
    pub fn new_with_op_and_lhs(token: Option<Token>, nt: NodeType, lhs: Node) -> Self {
        let mut node = Self {
            nt,
            token,
            lhs: Some(Box::new(lhs)),
            ..Self::default()
        };
        node.refresh_arithmetic_type();
        node
    }

    pub fn new_with_num(token: Option<Token>, value: usize) -> Self {
        Self {
            nt: NodeType::Num,
            token,
            value: Some(value),
            ..Self::default()
        }
    }
    pub fn new_if_node(token: Option<Token>, cond: Node, then: Node, els: Option<Node>) -> Self {
        Self {
            nt: NodeType::If,
            token,
            cond: Some(Box::new(cond)),
            then: Some(Box::new(then)),
            els: els.map(Box::new),
            ..Self::default()
        }
    }
    pub fn new_while_node(token: Option<Token>, cond: Node, then: Node) -> Self {
        Self {
            nt: NodeType::While,
            token,
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
            nt: NodeType::For,
            token,
            cond: cond.map(Box::new),
            then: Some(Box::new(then)),
            ini: ini.map(Box::new),
            upd: upd.map(Box::new),
            ..Self::default()
        }
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
    use std::{borrow::Cow, rc::Rc};

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

use std::{
    cell::RefCell,
    fmt,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Qualifiers {
    pub constant: bool,
    pub volatile: bool,
}

#[derive(Clone)]
pub struct StructType {
    pub id: usize,
    definition: Weak<RefCell<StructDefinition>>,
}

#[derive(Default)]
pub struct StructDefinition {
    pub members: Vec<StructMember>,
    pub is_union: bool,
    pub has_bit_fields: bool,
    pub size: usize,
    pub align: usize,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BitField {
    pub width: u8,
    pub shift: u8,
}

#[derive(Clone, Debug)]
pub struct StructMember {
    pub name: String,
    pub ty: Type,
    pub offset: usize,
    pub bit_field: Option<BitField>,
}

impl StructType {
    pub fn new(id: usize) -> (Self, Rc<RefCell<StructDefinition>>) {
        let definition = Rc::new(RefCell::new(StructDefinition::default()));
        (
            Self {
                id,
                definition: Rc::downgrade(&definition),
            },
            definition,
        )
    }
    pub fn definition(&self) -> Rc<RefCell<StructDefinition>> {
        self.definition
            .upgrade()
            .expect("struct definitions outlive the compiler pipeline")
    }
    pub fn has_bit_fields(&self) -> bool {
        self.definition().borrow().has_bit_fields
    }
    pub fn members(&self) -> Vec<StructMember> {
        self.definition().borrow().members.clone()
    }
}
impl PartialEq for StructType {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl fmt::Debug for StructType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Struct({})", self.id)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    Void,
    Qualified(Box<Type>, Qualifiers),
    Struct(StructType),
    I8,
    I32,
    Integer { bits: u8, signed: bool },
    Bool,
    Float(u8),
    Ptr(Box<Type>),
    Arr(Box<Type>, usize),
    Func(Vec<Type>, Box<Type>),
    VariadicFunc(Vec<Type>, Box<Type>),
}

impl Type {
    pub fn float_bits(&self) -> Option<u8> {
        match self.unqualified() {
            Self::Float(bits) => Some(*bits),
            _ => None,
        }
    }
    pub fn is_arithmetic(&self) -> bool {
        self.integer_info().is_some() || self.float_bits().is_some()
    }
    pub fn float_value(&self, bits: usize) -> f64 {
        if self.float_bits() == Some(32) {
            f32::from_bits(bits as u32) as f64
        } else {
            f64::from_bits(bits as u64)
        }
    }
    pub fn float_constant(&self, value: f64) -> usize {
        if self.float_bits() == Some(32) {
            (value as f32).to_bits() as usize
        } else {
            value.to_bits() as usize
        }
    }

    pub fn integer(bits: u8, signed: bool) -> Self {
        match (bits, signed) {
            (8, true) => Self::I8,
            (32, true) => Self::I32,
            _ => Self::Integer { bits, signed },
        }
    }
    pub fn integer_info(&self) -> Option<(u8, bool)> {
        match self {
            Self::I8 => Some((8, true)),
            Self::I32 => Some((32, true)),
            Self::Integer { bits, signed } => Some((*bits, *signed)),
            Self::Bool => Some((8, false)),
            Self::Qualified(child, _) => child.integer_info(),
            _ => None,
        }
    }
    pub fn promoted(&self) -> Self {
        if self.integer_info().is_some_and(|(bits, _)| bits < 32) {
            Self::I32
        } else {
            self.clone()
        }
    }
    pub fn arithmetic_type(lhs: &Self, rhs: &Self) -> Self {
        if lhs.float_bits().is_some() || rhs.float_bits().is_some() {
            return Self::Float(
                lhs.float_bits()
                    .unwrap_or(0)
                    .max(rhs.float_bits().unwrap_or(0)),
            );
        }
        let lhs = lhs.promoted();
        let rhs = rhs.promoted();
        let (Some((lb, ls)), Some((rb, rs))) = (lhs.integer_info(), rhs.integer_info()) else {
            return lhs;
        };
        match lb.cmp(&rb) {
            std::cmp::Ordering::Equal => Self::integer(lb, ls && rs),
            std::cmp::Ordering::Greater => Self::integer(lb, ls),
            std::cmp::Ordering::Less => Self::integer(rb, rs),
        }
    }
    pub fn convert_constant(&self, value: i64) -> i64 {
        if *self == Self::Bool {
            return i64::from(value != 0);
        }
        match self.integer_info() {
            Some((bits, signed)) if bits < 64 => {
                let shift = 64 - bits;
                if signed {
                    (value << shift) >> shift
                } else {
                    (value as u64 & ((1u64 << bits) - 1)) as i64
                }
            }
            _ => value,
        }
    }
    pub fn unqualified(&self) -> &Self {
        match self {
            Self::Qualified(child, _) => child.unqualified(),
            _ => self,
        }
    }
    pub fn qualifiers(&self) -> Qualifiers {
        match self {
            Self::Qualified(child, q) => {
                let inherited = child.qualifiers();
                Qualifiers {
                    constant: q.constant || inherited.constant,
                    volatile: q.volatile || inherited.volatile,
                }
            }
            _ => Qualifiers::default(),
        }
    }
    pub fn without_qualifiers(&self) -> Self {
        match self {
            Self::Qualified(child, _) => child.without_qualifiers(),
            Self::Ptr(child) => Self::Ptr(Box::new(child.without_qualifiers())),
            Self::Arr(child, size) => Self::Arr(Box::new(child.without_qualifiers()), *size),
            Self::Func(args, result) => Self::Func(
                args.iter().map(Self::without_qualifiers).collect(),
                Box::new(result.without_qualifiers()),
            ),
            Self::VariadicFunc(args, result) => Self::VariadicFunc(
                args.iter().map(Self::without_qualifiers).collect(),
                Box::new(result.without_qualifiers()),
            ),
            _ => self.clone(),
        }
    }
    pub fn function_signature(&self) -> Option<(Vec<Type>, Type, bool)> {
        match self.unqualified() {
            Self::Func(args, result) => Some((args.clone(), *result.clone(), false)),
            Self::VariadicFunc(args, result) => Some((args.clone(), *result.clone(), true)),
            _ => None,
        }
    }

    pub fn checked_size_of(&self) -> Option<usize> {
        match self {
            Self::Arr(child, size) => child.checked_size_of()?.checked_mul(*size),
            _ => Some(self.size_of()),
        }
    }

    pub fn size_of(&self) -> usize {
        match self {
            Type::Qualified(child, _) => child.size_of(),
            Type::Void => 0,
            Type::Struct(ty) => ty.definition().borrow().size,
            Type::I8 => 1,
            Type::I32 => 4,
            Type::Integer { bits, .. } => (*bits as usize) / 8,
            Type::Bool => 1,
            Type::Float(bits) => usize::from(*bits) / 8,
            Type::Ptr(_) => 8,
            Type::Arr(t, s) => t.size_of() * s,
            Type::Func(..) | Type::VariadicFunc(..) => 1,
        }
    }
    pub fn align_of(&self) -> usize {
        match self {
            Self::Qualified(child, _) => child.align_of(),
            Self::Void => 1,
            Self::Struct(ty) => ty.definition().borrow().align.max(1),
            Self::Arr(child, _) => child.align_of(),
            _ => self.size_of(),
        }
    }
    pub fn dest_type(&self) -> Option<Type> {
        match self {
            Type::Qualified(child, _) => child.dest_type(),
            Type::Ptr(c) => Some(*c.clone()),
            Type::Arr(c, _) => Some(*c.clone()),
            _ => None,
        }
    }
}

impl Default for Type {
    fn default() -> Self {
        Self::I32
    }
}

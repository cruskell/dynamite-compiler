use std::{
    cell::RefCell,
    fmt,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Qualifiers {
    pub constant: bool,
    pub volatile: bool,
    pub restricted: bool,
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
    pub has_flexible_array: bool,
    pub size: usize,
    pub align: usize,
    /// Member-derived alignment before record-level alignment attributes.
    pub natural_align: usize,
    /// Packed storage marker: a pragma limit, or 1 for GNU packed records.
    pub packing: Option<usize>,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BitField {
    pub width: u8,
    pub shift: u8,
    /// Access only the bytes occupied by a pragma-packed field.
    pub bytewise: bool,
}

#[derive(Clone, Debug)]
pub struct StructMember {
    pub name: String,
    pub anonymous: bool,
    /// Explicit member alignment before record packing is applied.
    pub requested_alignment: usize,
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
    pub fn visible_members(&self) -> Vec<(String, Vec<usize>)> {
        self.members()
            .iter()
            .enumerate()
            .flat_map(|(index, member)| {
                if member.anonymous {
                    let Type::Struct(child) = member.ty.unqualified() else {
                        unreachable!()
                    };
                    child
                        .visible_members()
                        .into_iter()
                        .map(|(name, mut path)| {
                            path.insert(0, index);
                            (name, path)
                        })
                        .collect()
                } else {
                    vec![(member.name.clone(), vec![index])]
                }
            })
            .collect()
    }
    pub fn member_path(&self, name: &str) -> Option<Vec<usize>> {
        self.visible_members()
            .into_iter()
            .find_map(|(member, path)| (member == name).then_some(path))
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
    Integer128 {
        signed: bool,
    },
    Integer {
        bits: u8,
        signed: bool,
    },
    Bool,
    Float16,
    Float(u8),
    LongDouble(u8),
    Ptr(Box<Type>),
    /// None denotes an omitted bound; Some(0) is a complete GNU zero-length array.
    Arr(Box<Type>, Option<usize>),
    // The offset names the captured bound; runtime_size computes the language size.
    Vla(Box<Type>, usize),
    Func(Vec<Type>, Box<Type>),
    VariadicFunc(Vec<Type>, Box<Type>),
}

impl Type {
    pub fn compatible(&self, other: &Self) -> bool {
        match (self.unqualified(), other.unqualified()) {
            (Self::Vla(left, _), Self::Vla(right, _) | Self::Arr(right, _))
            | (Self::Arr(left, _), Self::Vla(right, _))
            | (Self::Ptr(left), Self::Ptr(right)) => {
                left.qualifiers() == right.qualifiers() && left.compatible(right)
            }
            (Self::Arr(left, count), Self::Arr(right, other_count)) => {
                count == other_count
                    && left.qualifiers() == right.qualifiers()
                    && left.compatible(right)
            }
            (Self::Func(args, result), Self::Func(other_args, other_result))
            | (Self::VariadicFunc(args, result), Self::VariadicFunc(other_args, other_result)) => {
                args.len() == other_args.len()
                    && args
                        .iter()
                        .zip(other_args)
                        .all(|(left, right)| left.compatible(right))
                    && result.compatible(other_result)
            }
            (left, right) => left == right,
        }
    }

    pub fn has_flexible_array(&self) -> bool {
        match self.unqualified() {
            Self::Struct(ty) => ty.definition().borrow().has_flexible_array,
            Self::Arr(child, _) => child.has_flexible_array(),
            _ => false,
        }
    }

    pub fn is_variable_array(&self) -> bool {
        match self.unqualified() {
            Self::Vla(..) => true,
            Self::Arr(child, _) => child.is_variable_array(),
            _ => false,
        }
    }
    pub fn is_variably_modified(&self) -> bool {
        match self.unqualified() {
            Self::Vla(..) => true,
            Self::Arr(child, _) | Self::Ptr(child) => child.is_variably_modified(),
            _ => false,
        }
    }

    pub fn float_bits(&self) -> Option<u8> {
        match self.unqualified() {
            Self::Float(bits) | Self::LongDouble(bits) => Some(*bits),
            _ => None,
        }
    }
    /// Apply array-to-pointer and function-to-pointer value conversions.
    pub fn decayed(&self) -> Self {
        match self.unqualified() {
            Self::Arr(child, _) | Self::Vla(child, _) => Self::Ptr(child.clone()),
            ty @ (Self::Func(..) | Self::VariadicFunc(..)) => Self::Ptr(Box::new(ty.clone())),
            ty => ty.clone(),
        }
    }

    /// Whether value conversion yields a pointer, including arrays and functions.
    pub fn is_pointer_value(&self) -> bool {
        matches!(
            self.unqualified(),
            Self::Ptr(_) | Self::Arr(..) | Self::Vla(..) | Self::Func(..) | Self::VariadicFunc(..)
        )
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
            let bits = lhs
                .float_bits()
                .unwrap_or(0)
                .max(rhs.float_bits().unwrap_or(0));
            return if matches!(lhs.unqualified(), Self::LongDouble(_))
                || matches!(rhs.unqualified(), Self::LongDouble(_))
            {
                Self::LongDouble(bits)
            } else {
                Self::Float(bits)
            };
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
                    restricted: q.restricted || inherited.restricted,
                }
            }
            _ => Qualifiers::default(),
        }
    }
    /// Const qualification of an object, including recursively qualified arrays.
    pub fn is_const_object(&self) -> bool {
        self.qualifiers().constant
            || match self.unqualified() {
                Self::Arr(child, _) | Self::Vla(child, _) => child.is_const_object(),
                _ => false,
            }
    }

    pub fn without_qualifiers(&self) -> Self {
        match self {
            Self::Qualified(child, _) => child.without_qualifiers(),
            Self::Ptr(child) => Self::Ptr(Box::new(child.without_qualifiers())),
            Self::Arr(child, size) => Self::Arr(Box::new(child.without_qualifiers()), *size),
            Self::Vla(child, offset) => Self::Vla(Box::new(child.without_qualifiers()), *offset),
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

    /// Whether the type describes an object with a known layout, including zero-sized GNU objects.
    pub fn is_complete_object(&self) -> bool {
        match self.unqualified() {
            Self::Void | Self::Func(..) | Self::VariadicFunc(..) => false,
            Self::Arr(child, count) => count.is_some() && child.is_complete_object(),
            Self::Vla(child, _) => child.is_complete_object(),
            Self::Struct(record) => record.definition().borrow().complete,
            _ => true,
        }
    }

    pub fn checked_size_of(&self) -> Option<usize> {
        match self {
            Self::Arr(child, size) => child.checked_size_of()?.checked_mul(size.unwrap_or(0)),
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
            Type::Float16 => 2,
            Type::Integer128 { .. } => 16,
            Type::Integer { bits, .. } => (*bits as usize) / 8,
            Type::Bool => 1,
            Type::Float(bits) | Type::LongDouble(bits) => usize::from(*bits) / 8,
            Type::Ptr(_) => 8,
            Type::Arr(t, s) => t.size_of() * s.unwrap_or(0),
            Type::Vla(..) => 8,
            Type::Func(..) | Type::VariadicFunc(..) => 1,
        }
    }
    pub fn contains_float16(&self) -> bool {
        match self.unqualified() {
            Self::Float16 => true,
            Self::Arr(child, _) | Self::Vla(child, _) => child.contains_float16(),
            Self::Struct(ty) => ty
                .members()
                .iter()
                .any(|member| member.ty.contains_float16()),
            _ => false,
        }
    }

    pub fn natural_align_of(&self) -> usize {
        if let Self::Struct(ty) = self.unqualified() {
            let definition = ty.definition();
            let definition = definition.borrow();
            if definition.natural_align != 0 {
                return definition.natural_align;
            }
        }
        self.align_of()
    }

    pub fn align_of(&self) -> usize {
        match self {
            Self::Qualified(child, _) => child.align_of(),
            Self::Void => 1,
            Self::Struct(ty) => ty.definition().borrow().align.max(1),
            Self::Arr(child, _) | Self::Vla(child, _) => child.align_of(),
            _ => self.size_of(),
        }
    }
    pub fn dest_type(&self) -> Option<Type> {
        match self {
            Type::Qualified(child, _) => child.dest_type(),
            Type::Ptr(c) => Some(*c.clone()),
            Type::Arr(c, _) | Type::Vla(c, _) => Some(*c.clone()),
            _ => None,
        }
    }
}

impl Default for Type {
    fn default() -> Self {
        Self::I32
    }
}

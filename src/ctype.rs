mod arithmetic;
mod compatibility;
mod layout;
mod lowering;
mod records;

use std::{
    borrow::Cow,
    cell::RefCell,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Qualifiers {
    pub constant: bool,
    pub volatile: bool,
    pub restricted: bool,
}

#[derive(Clone, Debug)]
pub struct EnumType {
    pub id: usize,
    pub underlying: Rc<RefCell<Type>>,
}

impl PartialEq for EnumType {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone)]
pub struct StructType {
    pub id: usize,
    definition: Weak<RefCell<StructDefinition>>,
}

#[derive(Default)]
pub struct StructDefinition {
    pub members: Vec<StructMember>,
    pub complete: bool,
    pub is_union: bool,
    /// Function parameters use the first scalar member's calling convention.
    pub transparent_union: bool,
    pub has_bit_fields: bool,
    pub has_flexible_array: bool,

    pub size: usize,
    pub align: usize,
    /// Member-derived alignment before record-level alignment attributes.
    pub natural_align: usize,
    /// Packed storage marker: a pragma limit, or 1 for GNU packed records.
    pub packing: Option<usize>,
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
    pub ty: Type,

    pub offset: usize,
    /// Explicit member alignment before record packing is applied.
    pub requested_alignment: usize,
    pub bit_field: Option<BitField>,

    pub anonymous: bool,
}

/// Recursive type components are shared; lowering uses copy-on-write mutations.
#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    // Scalar types.
    Void,
    Bool,
    I8,
    SignedChar,
    I32,
    Integer {
        bits: u8,
        signed: bool,
    },
    LongLong {
        signed: bool,
    },
    Integer128 {
        signed: bool,
    },
    Enum(EnumType),
    Float16,
    Float(u8),
    LongDouble(u8),

    // Derived and aggregate types.
    Ptr(Rc<Type>),
    /// None denotes an omitted bound; Some(0) is a complete GNU zero-length array.
    Arr(Rc<Type>, Option<usize>),
    // The offset names the captured bound; runtime_size computes the language size.
    Vla(Rc<Type>, usize),
    /// Vector element type and lane count; value operations are not yet supported.
    Vector(Rc<Type>, usize),
    Struct(StructType),
    Func(Rc<Vec<Type>>, Rc<Type>),
    VariadicFunc(Rc<Vec<Type>>, Rc<Type>),

    // Type wrappers.
    Qualified(Rc<Type>, Qualifiers),
    /// Object alignment supplied by a scalar typedef, independent of type identity.
    Aligned(Rc<Type>, usize),
}

impl Type {
    /// The supported target ABI's wchar_t scalar type.
    pub fn wide_character() -> Self {
        Self::integer(32, !cfg!(all(target_arch = "aarch64", target_os = "linux")))
    }

    /// GNU type queries ignore outer qualification, including qualification of array elements.
    pub fn unqualified_for_type_query(&self) -> Self {
        match self.unqualified() {
            Self::Arr(child, count) => {
                Self::Arr(Rc::new(child.unqualified_for_type_query()), *count)
            }
            Self::Vla(child, offset) => {
                Self::Vla(Rc::new(child.unqualified_for_type_query()), *offset)
            }
            ty => ty.clone(),
        }
    }

    pub fn has_flexible_array(&self) -> bool {
        match self.unqualified() {
            Self::Arr(child, _) => child.has_flexible_array(),
            Self::Struct(ty) => ty.definition().borrow().has_flexible_array,
            _ => false,
        }
    }

    pub fn is_variable_array(&self) -> bool {
        match self.unqualified() {
            Self::Arr(child, _) => child.is_variable_array(),
            Self::Vla(..) => true,
            _ => false,
        }
    }
    pub fn is_variably_modified(&self) -> bool {
        match self.unqualified() {
            Self::Ptr(child) | Self::Arr(child, _) => child.is_variably_modified(),
            Self::Vla(..) => true,
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
            Self::Arr(child, _) | Self::Vla(child, _) => Self::Ptr(Rc::clone(child)),
            ty @ (Self::Func(..) | Self::VariadicFunc(..)) => Self::Ptr(Rc::new(ty.clone())),
            ty => ty.clone(),
        }
    }

    /// Preserve a borrowed view, or move through wrappers in an owned result.
    pub(crate) fn unqualified_view(ty: Cow<'_, Self>) -> Cow<'_, Self> {
        match ty {
            Cow::Borrowed(ty) => Cow::Borrowed(ty.unqualified()),
            Cow::Owned(mut ty) => {
                while let Self::Qualified(child, _) | Self::Aligned(child, _) = ty {
                    ty = Rc::unwrap_or_clone(child);
                }
                Cow::Owned(ty)
            }
        }
    }
    /// Apply value conversions without copying types that remain unchanged.
    pub(crate) fn decay_view(ty: Cow<'_, Self>) -> Cow<'_, Self> {
        match Self::unqualified_view(ty) {
            Cow::Borrowed(Self::Arr(child, _) | Self::Vla(child, _)) => {
                Cow::Owned(Self::Ptr(Rc::clone(child)))
            }
            Cow::Borrowed(ty @ (Self::Func(..) | Self::VariadicFunc(..))) => {
                Cow::Owned(Self::Ptr(Rc::new(ty.clone())))
            }
            Cow::Owned(Self::Arr(child, _) | Self::Vla(child, _)) => Cow::Owned(Self::Ptr(child)),
            Cow::Owned(ty @ (Self::Func(..) | Self::VariadicFunc(..))) => {
                Cow::Owned(Self::Ptr(Rc::new(ty)))
            }
            ty => ty,
        }
    }

    /// Whether value conversion yields a pointer, including arrays and functions.
    pub fn is_pointer_value(&self) -> bool {
        matches!(
            self.unqualified(),
            Self::Ptr(_) | Self::Arr(..) | Self::Vla(..) | Self::Func(..) | Self::VariadicFunc(..)
        )
    }

    pub fn unqualified(&self) -> &Self {
        match self {
            Self::Qualified(child, _) | Self::Aligned(child, _) => child.unqualified(),
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
            Self::Aligned(child, _) => child.qualifiers(),
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

    pub fn dest_type(&self) -> Option<Type> {
        self.dest_type_ref().cloned()
    }
    pub(crate) fn dest_type_ref(&self) -> Option<&Type> {
        match self.unqualified() {
            Self::Ptr(child) | Self::Arr(child, _) | Self::Vla(child, _) => Some(child),
            _ => None,
        }
    }
}

impl Default for Type {
    fn default() -> Self {
        Self::I32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowering_shared_types_preserves_source_qualifiers() {
        let qualified = Type::Qualified(
            Rc::new(Type::I32),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        );
        let source = Type::Ptr(Rc::new(qualified));
        let mut lowered = source.clone();
        lowered.lower_for_codegen();
        assert!(source.dest_type_ref().unwrap().qualifiers().constant);
        assert_eq!(lowered, Type::Ptr(Rc::new(Type::I32)));
    }

    #[test]
    fn lowering_shared_signatures_preserves_other_declarations() {
        let parameter = Type::Qualified(
            Rc::new(Type::I32),
            Qualifiers {
                constant: true,
                ..Qualifiers::default()
            },
        );
        let source = Type::Func(Rc::new(vec![parameter]), Rc::new(Type::I32));
        let mut lowered = source.clone();
        lowered.lower_for_codegen();
        assert!(
            source.function_signature_ref().unwrap().0[0]
                .qualifiers()
                .constant
        );
        assert_eq!(lowered.function_signature_ref().unwrap().0, &[Type::I32]);
    }
}

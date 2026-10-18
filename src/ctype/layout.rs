use super::Type;

impl Type {
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
            Type::Void => 0,
            Type::Bool => 1,
            Type::I8 | Type::SignedChar => 1,
            Type::I32 => 4,
            Type::Integer { bits, .. } => (*bits as usize) / 8,
            Type::LongLong { .. } => 8,
            Type::Integer128 { .. } => 16,
            Type::Enum(value) => value.underlying.borrow().size_of(),
            Type::Float16 => 2,
            Type::LongDouble(80) => 16,
            Type::Float(bits) | Type::LongDouble(bits) => usize::from(*bits) / 8,
            Type::Ptr(_) => 8,
            Type::Arr(t, s) => t.size_of() * s.unwrap_or(0),
            Type::Vla(..) => 8,
            Type::Vector(element, lanes) => element.size_of() * lanes,
            Type::Struct(ty) => ty.definition().borrow().size,
            Type::Func(..) | Type::VariadicFunc(..) => 1,
            Type::Qualified(child, _) | Type::Aligned(child, _) => child.size_of(),
        }
    }

    pub(crate) fn contains_volatile_subobject(&self) -> bool {
        self.qualifiers().volatile
            || match self.unqualified() {
                Self::Arr(child, _) | Self::Vla(child, _) => child.contains_volatile_subobject(),
                Self::Struct(record) => record
                    .definition()
                    .borrow()
                    .members
                    .iter()
                    .any(|member| member.ty.contains_volatile_subobject()),
                // A volatile pointee does not make the pointer object volatile.
                _ => false,
            }
    }

    pub fn contains_vector(&self) -> bool {
        match self.unqualified() {
            Self::Arr(child, _) | Self::Vla(child, _) => child.contains_vector(),
            Self::Vector(..) => true,
            Self::Struct(record) => record
                .definition()
                .borrow()
                .members
                .iter()
                .any(|member| member.ty.contains_vector()),
            _ => false,
        }
    }

    pub fn contains_float16(&self) -> bool {
        match self.unqualified() {
            Self::Float16 => true,
            Self::Arr(child, _) | Self::Vla(child, _) => child.contains_float16(),
            Self::Struct(ty) => ty
                .definition()
                .borrow()
                .members
                .iter()
                .any(|member| member.ty.contains_float16()),
            _ => false,
        }
    }

    /// Whether an object contains a non-binary64 long double format.
    pub fn contains_extended_long_double(&self) -> bool {
        match self.unqualified() {
            Self::LongDouble(bits) => *bits != 64,
            Self::Arr(child, _) | Self::Vla(child, _) => child.contains_extended_long_double(),
            Self::Struct(ty) => ty
                .definition()
                .borrow()
                .members
                .iter()
                .any(|member| member.ty.contains_extended_long_double()),
            _ => false,
        }
    }

    pub fn contains_typedef_alignment(&self) -> bool {
        match self {
            Self::Arr(child, _) | Self::Vla(child, _) | Self::Qualified(child, _) => {
                child.contains_typedef_alignment()
            }
            Self::Struct(record) => record
                .definition()
                .borrow()
                .members
                .iter()
                .any(|member| member.ty.contains_typedef_alignment()),
            Self::Aligned(child, alignment) => {
                *alignment != child.natural_align_of() || child.contains_typedef_alignment()
            }
            _ => false,
        }
    }

    pub fn natural_align_of(&self) -> usize {
        if let Self::Qualified(child, _) | Self::Aligned(child, _) = self {
            return child.natural_align_of();
        }
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
            Self::Void => 1,
            Self::Arr(child, _) | Self::Vla(child, _) => child.align_of(),
            Self::Struct(ty) => ty.definition().borrow().align.max(1),
            Self::Qualified(child, _) => child.align_of(),
            Self::Aligned(_, alignment) => *alignment,
            _ => self.size_of(),
        }
    }
}

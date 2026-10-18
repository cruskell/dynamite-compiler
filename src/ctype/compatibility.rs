use super::{StructMember, Type};

impl Type {
    pub fn compatible(&self, other: &Self) -> bool {
        match (self.unqualified(), other.unqualified()) {
            (Self::Enum(left), Self::Enum(right)) => left == right,
            (Self::Enum(value), other) | (other, Self::Enum(value)) => {
                value.underlying.borrow().compatible(other)
            }
            (Self::Ptr(left), Self::Ptr(right))
            | (Self::Arr(left, _), Self::Vla(right, _))
            | (Self::Vla(left, _), Self::Arr(right, _) | Self::Vla(right, _)) => {
                left.qualifiers() == right.qualifiers() && left.compatible(right)
            }
            (Self::Arr(left, count), Self::Arr(right, other_count)) => {
                (count == other_count || count.is_none() || other_count.is_none())
                    && left.qualifiers() == right.qualifiers()
                    && left.compatible(right)
            }
            (Self::Func(args, result), Self::Func(other_args, other_result))
            | (Self::VariadicFunc(args, result), Self::VariadicFunc(other_args, other_result)) => {
                args.len() == other_args.len()
                    && args
                        .iter()
                        .zip(other_args.iter())
                        .all(|(left, right)| left.parameter_compatible(right))
                    && result.qualifiers() == other_result.qualifiers()
                    && result.compatible(other_result)
            }
            (left, right) => left == right,
        }
    }

    /// Return the scalar members of a validated GNU transparent union.
    pub fn transparent_union_members(&self) -> Option<Vec<StructMember>> {
        let Self::Struct(record) = self.unqualified() else {
            return None;
        };
        if record.definition().borrow().transparent_union {
            Some(record.members())
        } else {
            None
        }
    }

    fn parameter_compatible(&self, other: &Self) -> bool {
        match (
            self.transparent_union_members(),
            other.transparent_union_members(),
        ) {
            (Some(_), Some(_)) | (None, None) => self.compatible(other),
            (Some(members), None) => members.iter().any(|member| member.ty.compatible(other)),
            (None, Some(members)) => members.iter().any(|member| self.compatible(&member.ty)),
        }
    }

    /// Typedef redeclarations require identical types, not merely compatible array bounds or enums.
    pub fn identical(&self, other: &Self) -> bool {
        self.qualifiers() == other.qualifiers() && self.identical_unqualified(other)
    }

    fn identical_unqualified(&self, other: &Self) -> bool {
        match (self.unqualified(), other.unqualified()) {
            (Self::Ptr(left), Self::Ptr(right)) => left.identical(right),
            (Self::Arr(left, count), Self::Arr(right, other_count)) => {
                count == other_count && left.identical(right)
            }
            (Self::Func(args, result), Self::Func(other_args, other_result))
            | (Self::VariadicFunc(args, result), Self::VariadicFunc(other_args, other_result)) => {
                args.len() == other_args.len()
                    && args
                        .iter()
                        .zip(other_args.iter())
                        .all(|(left, right)| left.identical_unqualified(right))
                    && result.identical(other_result)
            }
            (left, right) => left == right,
        }
    }
}

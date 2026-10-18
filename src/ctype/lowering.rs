use super::Type;
use std::rc::Rc;

impl Type {
    /// Normalize a transparent-union parameter without changing ordinary union values or results.
    pub fn parameter_for_codegen(&self) -> Self {
        let mut ty = self.clone();
        ty.lower_parameter_for_codegen();
        ty
    }

    pub(crate) fn lower_parameter_for_codegen(&mut self) {
        let first = if let Self::Struct(record) = self.unqualified() {
            let definition = record.definition();
            let definition = definition.borrow();
            definition
                .transparent_union
                .then(|| definition.members[0].ty.clone())
        } else {
            None
        };
        if let Some(first) = first {
            *self = if matches!(first.unqualified(), Self::Ptr(_)) {
                // Opaque pointer ABI types also break recursive callback parameter types.
                Self::Ptr(Rc::new(Self::Void))
            } else {
                first
            };
        }
        self.lower_for_codegen();
    }

    /// Erase source qualifiers and enum identities after semantic validation.
    pub fn lowered_for_codegen(&self) -> Self {
        let mut ty = self.clone();
        ty.lower_for_codegen();
        ty
    }

    /// Normalize an owned type after validation, reusing its child storage.
    #[inline]
    pub(crate) fn lower_for_codegen(&mut self) {
        // Scalar annotations need no recursive normalization.
        match self {
            Self::Enum(..)
            | Self::Ptr(..)
            | Self::Arr(..)
            | Self::Vla(..)
            | Self::Vector(..)
            | Self::Func(..)
            | Self::VariadicFunc(..)
            | Self::Qualified(..)
            | Self::Aligned(..) => self.lower_compound_for_codegen(),
            _ => {}
        }
    }

    fn lower_compound_for_codegen(&mut self) {
        if matches!(self, Self::Qualified(..)) {
            let Self::Qualified(child, _) = std::mem::take(self) else {
                unreachable!()
            };
            *self = Rc::unwrap_or_clone(child);
            self.lower_for_codegen();
            return;
        }
        match self {
            Self::Enum(value) => {
                let underlying = value.underlying.borrow().clone();
                *self = underlying;
                self.lower_for_codegen();
            }
            Self::Ptr(child)
            | Self::Arr(child, _)
            | Self::Vla(child, _)
            | Self::Vector(child, _)
            | Self::Aligned(child, _) => Rc::make_mut(child).lower_for_codegen(),
            Self::Func(args, result) | Self::VariadicFunc(args, result) => {
                for arg in Rc::make_mut(args) {
                    arg.lower_parameter_for_codegen();
                }
                Rc::make_mut(result).lower_for_codegen();
            }
            _ => {}
        }
    }

    pub fn function_signature(&self) -> Option<(Vec<Type>, Type, bool)> {
        self.function_signature_ref()
            .map(|(args, result, variadic)| (args.to_vec(), result.clone(), variadic))
    }

    /// Inspect a signature without cloning its parameter and result types.
    pub(crate) fn function_signature_ref(&self) -> Option<(&[Type], &Type, bool)> {
        match self.unqualified() {
            Self::Func(args, result) => Some((args, result, false)),
            Self::VariadicFunc(args, result) => Some((args, result, true)),
            _ => None,
        }
    }

    /// Reuse an owned signature's storage when constructing a call.
    pub(crate) fn into_function_signature(self) -> Option<(Vec<Type>, Type, bool)> {
        match self {
            Self::Func(args, result) => Some((
                Rc::unwrap_or_clone(args),
                Rc::unwrap_or_clone(result),
                false,
            )),
            Self::VariadicFunc(args, result) => {
                Some((Rc::unwrap_or_clone(args), Rc::unwrap_or_clone(result), true))
            }
            Self::Qualified(child, _) | Self::Aligned(child, _) => {
                Rc::unwrap_or_clone(child).into_function_signature()
            }
            _ => None,
        }
    }
}

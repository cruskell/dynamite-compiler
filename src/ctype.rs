#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    Void,
    I8,
    I32,
    Ptr(Box<Type>),
    Arr(Box<Type>, usize),
    Func(Vec<Type>, Box<Type>),
    VariadicFunc(Vec<Type>, Box<Type>),
}

impl Type {
    pub fn function_signature(&self) -> Option<(Vec<Type>, Type, bool)> {
        match self {
            Self::Func(args, result) => Some((args.clone(), *result.clone(), false)),
            Self::VariadicFunc(args, result) => Some((args.clone(), *result.clone(), true)),
            _ => None,
        }
    }

    pub fn size_of(&self) -> usize {
        match self {
            Type::Void => 0,
            Type::I8 => 1,
            Type::I32 => 4,
            Type::Ptr(_) => 8,
            Type::Arr(t, s) => t.size_of() * s,
            Type::Func(..) | Type::VariadicFunc(..) => 1,
        }
    }
    pub fn align_of(&self) -> usize {
        match self {
            Self::Void => 1,
            Self::Arr(child, _) => child.align_of(),
            _ => self.size_of(),
        }
    }
    pub fn dest_type(&self) -> Option<Type> {
        match self {
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

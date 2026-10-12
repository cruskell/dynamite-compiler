use std::{
    cell::RefCell,
    fmt,
    rc::{Rc, Weak},
};

#[derive(Clone)]
pub struct StructType {
    pub id: usize,
    definition: Weak<RefCell<StructDefinition>>,
}

#[derive(Default)]
pub struct StructDefinition {
    pub members: Vec<StructMember>,
    pub size: usize,
    pub align: usize,
    pub complete: bool,
}

#[derive(Clone, Debug)]
pub struct StructMember {
    pub name: String,
    pub ty: Type,
    pub offset: usize,
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
    Struct(StructType),
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

    pub fn checked_size_of(&self) -> Option<usize> {
        match self {
            Self::Arr(child, size) => child.checked_size_of()?.checked_mul(*size),
            _ => Some(self.size_of()),
        }
    }

    pub fn size_of(&self) -> usize {
        match self {
            Type::Void => 0,
            Type::Struct(ty) => ty.definition().borrow().size,
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
            Self::Struct(ty) => ty.definition().borrow().align.max(1),
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

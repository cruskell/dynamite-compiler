use super::{StructDefinition, StructMember, StructType, Type};
use std::{cell::RefCell, fmt, rc::Rc};

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
    pub fn member(&self, index: usize) -> StructMember {
        self.definition().borrow().members[index].clone()
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
        let definition = self.definition();
        let definition = definition.borrow();
        for (index, member) in definition.members.iter().enumerate() {
            if member.anonymous {
                let Type::Struct(child) = member.ty.unqualified() else {
                    unreachable!()
                };
                if let Some(mut path) = child.member_path(name) {
                    path.insert(0, index);
                    return Some(path);
                }
            } else if member.name == name {
                return Some(vec![index]);
            }
        }
        None
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

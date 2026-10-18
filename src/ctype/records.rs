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
        let mut members = vec![];
        self.for_each_visible_member(&mut |name, path| {
            members.push((name.to_owned(), path.to_vec()));
        });
        members
    }

    pub(crate) fn for_each_visible_member(&self, operation: &mut impl FnMut(&str, &[usize])) {
        fn visit(
            record: &StructType,
            path: &mut Vec<usize>,
            operation: &mut impl FnMut(&str, &[usize]),
        ) {
            let definition = record.definition();
            let definition = definition.borrow();
            for (index, member) in definition.members.iter().enumerate() {
                path.push(index);
                if member.anonymous {
                    let Type::Struct(child) = member.ty.unqualified() else {
                        unreachable!()
                    };
                    visit(child, path, operation);
                } else {
                    operation(&member.name, path);
                }
                path.pop();
            }
        }
        visit(self, &mut vec![], operation);
    }

    pub fn member_path(&self, name: &str) -> Option<Vec<usize>> {
        fn find(record: &StructType, name: &str, path: &mut Vec<usize>) -> bool {
            let definition = record.definition();
            let definition = definition.borrow();
            for (index, member) in definition.members.iter().enumerate() {
                path.push(index);
                if member.anonymous {
                    let Type::Struct(child) = member.ty.unqualified() else {
                        unreachable!()
                    };
                    if find(child, name, path) {
                        return true;
                    }
                } else if member.name == name {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = vec![];
        find(self, name, &mut path).then_some(path)
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

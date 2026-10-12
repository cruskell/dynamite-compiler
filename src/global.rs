use crate::ctype::Type;
#[derive(Debug, PartialEq)]
pub struct GlobalVariable {
    pub ty: Type,
    pub data: Option<GlobalVariableData>,
}
#[derive(Debug, PartialEq)]
pub enum GlobalVariableData {
    Elem(String),
    Address { symbol: String, offset: i64 },
    Arr(Vec<GlobalVariableData>),
}

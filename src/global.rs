use crate::ctype::Type;
#[derive(Debug, PartialEq)]
pub struct GlobalVariable {
    pub ty: Type,
    pub internal: bool,
    pub declaration_only: bool,
    pub data: Option<GlobalVariableData>,
}
#[derive(Debug, PartialEq)]
pub enum GlobalVariableData {
    Zero,
    Elem(String),
    Address { symbol: String, offset: i64 },
    Arr(Vec<GlobalVariableData>),
}

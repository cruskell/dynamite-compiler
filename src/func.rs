use crate::ctype::Type;
use crate::node::Node;
use crate::token::Token;

#[derive(Default)]
pub struct Func {
    pub body: Option<Node>,
    pub internal: bool,
    pub cty: Type,
    pub offset_size: usize,
    pub token: Option<Token>,
    pub args: Vec<Node>,
}

impl Func {
    pub fn to_debug_string(&self) -> String {
        format!(
            "args:\n{:?}\nbody:\n{:?}",
            self.args
                .iter()
                .map(|arg| format!("  {}", arg.to_debug_string(4))),
            self.body
                .iter()
                .map(|n| format!("  {}", n.to_debug_string(4))),
        )
    }
}

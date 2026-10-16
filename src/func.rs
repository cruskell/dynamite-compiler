use crate::ctype::Type;
use crate::node::Node;
use crate::token::Token;

#[derive(Default)]
pub struct Func {
    pub body: Option<Node>,
    pub internal: bool,
    pub inline_definition: bool,
    pub gnu_inline: bool,
    pub noinline: bool,
    pub weak_import: bool,
    pub hidden: bool,
    pub external_definition: bool,
    pub mutable_static: Option<Token>,
    pub return_pointer: Option<usize>,
    pub cty: Type,
    pub offset_size: usize,
    pub token: Option<Token>,
    pub args: Vec<Node>,
}

impl Func {
    pub fn inline_only(&self) -> bool {
        self.inline_definition && !self.external_definition && !self.internal
    }

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

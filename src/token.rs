#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    Reserved,
    Ident,
    Num,
    Float,
    Str,
    EOF,
}

impl Default for TokenType {
    fn default() -> Self {
        Self::Num
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Token {
    pub tt: TokenType,
    pub pos: usize,
    pub i_value: usize,
    pub s_value: String,
}

impl Token {
    pub fn print(&self) {
        match self.tt {
            TokenType::Float => eprintln!("float: {}", self.s_value),
            TokenType::Num => {
                eprintln!("num: {}", self.i_value)
            }
            TokenType::Reserved => {
                eprintln!("rsv: {}", &self.s_value)
            }
            TokenType::Ident => {
                eprintln!("idt: {}", &self.s_value)
            }
            TokenType::Str => {
                eprintln!("str: {}", &self.s_value)
            }
            TokenType::EOF => {
                eprintln!("EOF")
            }
        }
    }
}

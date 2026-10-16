#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    Reserved,
    Ident,
    Num,
    Float,
    Version,
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
    /// Active pragma pack limit at this source position.
    pub packing: Option<usize>,
    pub i_value: usize,
    pub extended_float: Option<Box<u128>>,
    pub s_value: smol_str::SmolStr,
    pub string_encoding: StringEncoding,
    pub string_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum StringEncoding {
    #[default]
    Narrow,
    Utf8,
    Utf16,
    Utf32,
    Wide,
}

impl StringEncoding {
    pub fn unit_size(self) -> usize {
        match self {
            Self::Narrow | Self::Utf8 => 1,
            Self::Utf16 => 2,
            Self::Utf32 | Self::Wide => 4,
        }
    }

    pub fn element_type(self) -> &'static str {
        match self {
            Self::Narrow | Self::Utf8 => "i8",
            Self::Utf16 => "i16",
            Self::Utf32 | Self::Wide => "i32",
        }
    }
}

impl Token {
    pub fn decoded_string(&self) -> Vec<u8> {
        self.string_bytes.clone()
    }

    pub fn encoded_string(&self) -> Vec<u8> {
        let mut bytes = self.string_bytes.clone();
        bytes.resize(bytes.len() + self.string_encoding.unit_size(), 0);
        bytes
    }
}

impl Token {
    pub fn print(&self) {
        match self.tt {
            TokenType::Float => eprintln!("float: {}", self.s_value),
            TokenType::Version => eprintln!("version: {}", self.s_value),
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

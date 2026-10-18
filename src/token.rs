/// Shared literal storage keeps non-string tokens compact.
#[derive(Debug, Clone, Default)]
pub struct StringBytes(Option<std::rc::Rc<Vec<u8>>>);

impl From<Vec<u8>> for StringBytes {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            Self::default()
        } else {
            Self(Some(std::rc::Rc::new(bytes)))
        }
    }
}
impl std::ops::Deref for StringBytes {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        static EMPTY: Vec<u8> = Vec::new();
        self.0.as_deref().unwrap_or(&EMPTY)
    }
}
impl std::ops::DerefMut for StringBytes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        std::rc::Rc::make_mut(self.0.get_or_insert_with(|| std::rc::Rc::new(Vec::new())))
    }
}
impl PartialEq for StringBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl StringBytes {
    /// Returns owned bytes, reusing the allocation when it is not shared.
    pub fn into_vec(self) -> Vec<u8> {
        self.0.map(std::rc::Rc::unwrap_or_clone).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::StringBytes;

    #[test]
    fn literal_bytes_clones_can_be_mutated_and_consumed_independently() {
        let original = StringBytes::from(vec![0, 128, 255]);
        let mut changed = original.clone();
        changed.push(42);
        changed[0] = 1;
        assert_eq!(original.clone().into_vec(), vec![0, 128, 255]);
        assert_eq!(original.into_vec(), vec![0, 128, 255]);
        assert_eq!(changed.into_vec(), vec![1, 128, 255, 42]);

        let mut empty = StringBytes::default();
        assert_eq!(empty, Vec::new().into());
        let unchanged = empty.clone();
        empty.push(7);
        assert!(unchanged.into_vec().is_empty());
        assert_eq!(empty.into_vec(), vec![7]);
    }
}

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
    pub packing: Option<u8>,
    pub s_value: smol_str::SmolStr,

    pub i_value: usize,
    pub extended_float: Option<Box<u128>>,

    pub string_encoding: StringEncoding,
    pub string_bytes: StringBytes,
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
        self.string_bytes.as_slice().to_vec()
    }

    pub fn encoded_string(&self) -> Vec<u8> {
        let mut bytes = self.string_bytes.as_slice().to_vec();
        bytes.resize(bytes.len() + self.string_encoding.unit_size(), 0);
        bytes
    }
}

impl Token {
    pub fn print(&self) {
        match self.tt {
            TokenType::Reserved => {
                eprintln!("rsv: {}", &self.s_value)
            }
            TokenType::Ident => {
                eprintln!("idt: {}", &self.s_value)
            }
            TokenType::Num => {
                eprintln!("num: {}", self.i_value)
            }
            TokenType::Float => eprintln!("float: {}", self.s_value),
            TokenType::Version => eprintln!("version: {}", self.s_value),
            TokenType::Str => {
                eprintln!("str: {}", &self.s_value)
            }
            TokenType::EOF => {
                eprintln!("EOF")
            }
        }
    }
}

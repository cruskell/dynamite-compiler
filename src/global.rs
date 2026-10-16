use crate::ctype::Type;
use std::collections::HashMap;
#[derive(Debug, PartialEq)]
pub struct GlobalVariable {
    pub ty: Type,
    pub alignment: usize,
    pub thread_local: bool,
    pub weak_import: bool,
    pub hidden: bool,
    pub common: bool,
    pub internal: bool,
    pub declaration_only: bool,
    pub data: Option<GlobalVariableData>,
}
#[derive(Debug, PartialEq)]
pub enum GlobalVariableData {
    Zero,
    Elem(String),
    Address {
        symbol: String,
        offset: i64,
    },
    Arr(Vec<GlobalVariableData>),
    Union {
        index: usize,
        value: Box<GlobalVariableData>,
    },
}

pub fn initializer_bytes(
    ty: &Type,
    data: Option<&GlobalVariableData>,
    offset: usize,
    bytes: &mut [u8],
    addresses: &mut HashMap<usize, (String, i64)>,
) {
    match ty.unqualified() {
        Type::Struct(st) => {
            let elements = if let Some(GlobalVariableData::Arr(v)) = data {
                Some(v)
            } else {
                None
            };
            let union = st.definition().borrow().is_union;
            let selected = if let Some(GlobalVariableData::Union { index, .. }) = data {
                *index
            } else {
                0
            };
            for (index, member) in st.members().iter().enumerate() {
                if union && index != selected {
                    continue;
                }
                let data = if let Some(GlobalVariableData::Union { value, .. }) = data {
                    Some(value.as_ref())
                } else {
                    elements.and_then(|v| v.get(index))
                };
                if let Some(field) = member.bit_field {
                    let mut data = data;
                    while let Some(GlobalVariableData::Arr(v)) = data {
                        data = v.first();
                    }
                    let value = if let Some(GlobalVariableData::Elem(value)) = data {
                        value.parse::<i64>().unwrap() as u64
                    } else {
                        0
                    };
                    for bit in 0..field.width {
                        let position =
                            (offset + member.offset) * 8 + usize::from(field.shift + bit);
                        let bit_mask = 1 << (position % 8);
                        bytes[position / 8] = (bytes[position / 8] & !bit_mask)
                            | (((value >> bit) as u8 & 1) << (position % 8));
                    }
                } else {
                    initializer_bytes(&member.ty, data, offset + member.offset, bytes, addresses);
                }
            }
        }
        Type::Arr(child, size) => {
            let elements = if let Some(GlobalVariableData::Arr(v)) = data {
                Some(v)
            } else {
                None
            };
            for index in 0..size.unwrap_or(0) {
                initializer_bytes(
                    child,
                    elements.and_then(|v| v.get(index)),
                    offset + index * child.size_of(),
                    bytes,
                    addresses,
                );
            }
        }
        _ => {
            let mut data = data;
            while let Some(GlobalVariableData::Arr(v)) = data {
                data = v.first();
            }
            match data {
                Some(GlobalVariableData::Address {
                    symbol,
                    offset: delta,
                }) => {
                    addresses.insert(offset, (symbol.clone(), *delta));
                }
                Some(GlobalVariableData::Elem(value)) if value.starts_with('@') => {
                    addresses.insert(offset, (value[1..].to_string(), 0));
                }
                Some(GlobalVariableData::Elem(value)) => {
                    let word = if ty.float_bits() == Some(80) {
                        value.parse::<u128>().unwrap()
                    } else {
                        value.parse::<i64>().unwrap() as u64 as u128
                    }
                    .to_le_bytes();
                    bytes[offset..offset + ty.size_of()].copy_from_slice(&word[..ty.size_of()]);
                }
                _ => {}
            }
        }
    }
}

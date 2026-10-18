use super::AsmGenerator;
use crate::ctype::Type;
use crate::generator::Os;
use crate::generator::x86_64::assembly::Assembly;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::token::Token;

impl AsmGenerator<'_> {
    pub(super) fn gen_string_literals(&self, string_literals: &[Token]) -> Assembly {
        if string_literals.is_empty() {
            vec![]
        } else {
            vec![
                if let Os::MacOS = self.target_os {
                    ".section __TEXT,__const".into()
                } else {
                    ".section .data".into()
                },
                string_literals
                    .iter()
                    .enumerate()
                    .map(|(i, token)| {
                        let bytes = token.encoded_string();
                        vec![
                            format!("  .balign {}", token.string_encoding.unit_size()).into(),
                            format!("L_.str.{}:", i).into(),
                            format!(
                                "  .byte {}",
                                bytes
                                    .into_iter()
                                    .map(|byte| byte.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                            .into(),
                        ]
                        .into()
                    })
                    .collect::<Vec<Assembly>>()
                    .into(),
            ]
        }
        .into()
    }

    pub(super) fn weak_reference(&self, name: &str) -> Assembly {
        let directive = match self.target_os {
            Os::Linux => ".weak",
            Os::MacOS => ".weak_reference",
        };
        format!("{directive} {}", self.with_prefix(name)).into()
    }

    pub(super) fn gen_global_variable(&self, name: &str, gv: &GlobalVariable) -> Assembly {
        if gv.declaration_only {
            return if gv.weak_import {
                self.weak_reference(name)
            } else {
                vec![].into()
            };
        }
        if gv.common && !gv.internal && !gv.thread_local && gv.data.is_none() {
            let alignment = gv.ty.align_of().max(gv.alignment).max(1);
            let alignment = match self.target_os {
                Os::Linux => alignment,
                Os::MacOS => alignment.trailing_zeros() as usize,
            };
            return format!(
                ".comm {},{},{}",
                self.with_prefix(name),
                gv.ty.size_of(),
                alignment
            )
            .into();
        }
        vec![
            if let Os::MacOS = self.target_os {
                ".section __DATA,__data".into()
            } else {
                ".section .data".into()
            },
            if gv.internal {
                vec![].into()
            } else {
                format!(".globl {}", self.with_prefix(name)).into()
            },
            format!("  .balign {}", gv.ty.align_of().max(gv.alignment)).into(),
            format!("{}:", self.with_prefix(name)).into(),
            self.gen_initializer_element(&gv.ty, gv.data.as_ref()),
        ]
        .into()
    }

    pub(super) fn gen_initializer_element(
        &self,
        ty: &Type,
        data: Option<&GlobalVariableData>,
    ) -> Assembly {
        let data = if !matches!(ty, Type::Arr(..) | Type::Struct(_)) {
            let mut value = data;
            while let Some(GlobalVariableData::Arr(elements)) = value {
                value = elements.first();
            }
            value
        } else {
            data
        };
        match ty {
            Type::Bool | Type::Integer { .. } | Type::LongLong { .. } => format!(
                "  {} {}",
                if ty.size_of() == 1 {
                    ".byte".to_string()
                } else {
                    format!(".{}byte", ty.size_of())
                },
                if let Some(GlobalVariableData::Address { symbol, offset }) = data {
                    format!("{}{offset:+}", self.address_symbol(symbol))
                } else if let Some(GlobalVariableData::Elem(value)) = data {
                    ty.convert_constant(value.parse().unwrap()).to_string()
                } else {
                    "0".into()
                }
            )
            .into(),
            Type::I8 | Type::SignedChar => format!(
                "  .byte {}",
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            )
            .into(),
            Type::I32 => format!(
                "  .4byte {}",
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            )
            .into(),
            Type::Integer128 { .. } => "  .zero 16".into(),
            Type::Enum(value) => self.gen_initializer_element(&value.underlying.borrow(), data),
            Type::Float16 => "  .zero 2".into(),
            Type::LongDouble(bits) if *bits != 64 => {
                let value = if let Some(GlobalVariableData::Elem(value)) = data {
                    value.parse::<u128>().unwrap()
                } else {
                    0
                };
                format!(
                    "  .8byte {}\n  .8byte {}",
                    value as u64,
                    (value >> 64) as u64
                )
                .into()
            }
            Type::Float(_) | Type::LongDouble(_) => format!(
                "  .{}byte {}",
                ty.size_of(),
                if let Some(GlobalVariableData::Elem(value)) = data {
                    value.as_str()
                } else {
                    "0"
                }
            )
            .into(),
            Type::Arr(children_ty, size) => {
                let size = size.unwrap_or(0);
                if let Some(GlobalVariableData::Arr(v)) = data {
                    vec![
                        v.iter()
                            .enumerate()
                            .filter(|(i, _)| *i < size)
                            .map(|(_, d)| {
                                self.gen_initializer_element(children_ty.as_ref(), Some(d))
                            })
                            .collect::<Vec<Assembly>>()
                            .into(),
                        format!(
                            "  .zero {}",
                            children_ty.size_of()
                                * v.iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i < size)
                                    .fold(size, |_, _| { size.saturating_sub(v.len()) })
                        )
                        .into(),
                    ]
                    .into()
                } else {
                    format!("  .zero {}", children_ty.size_of() * size).into()
                }
            }
            Type::Vector(..) => format!("  .zero {}", ty.size_of()).into(),
            Type::Struct(struct_type)
                if struct_type.has_bit_fields() || struct_type.definition().borrow().is_union =>
            {
                let mut bytes = vec![0; ty.size_of()];
                let mut addresses = std::collections::HashMap::new();
                crate::global::initializer_bytes(ty, data, 0, &mut bytes, &mut addresses);
                let mut lines = vec![];
                let mut index = 0;
                while index < bytes.len() {
                    if let Some((symbol, delta)) = addresses.get(&index) {
                        let symbol = self.address_symbol(symbol);
                        lines.push(format!("  .8byte {symbol}{delta:+}").into());
                        index += 8;
                    } else {
                        lines.push(format!("  .byte {}", bytes[index]).into());
                        index += 1;
                    }
                }
                lines.into()
            }
            Type::Struct(struct_type) => {
                let elements = if let Some(GlobalVariableData::Arr(elements)) = data {
                    Some(elements)
                } else {
                    None
                };
                let mut lines = vec![];
                let mut end = 0;
                for (i, member) in struct_type
                    .members()
                    .iter()
                    .take(if struct_type.definition().borrow().is_union {
                        1
                    } else {
                        usize::MAX
                    })
                    .enumerate()
                {
                    if member.offset > end {
                        lines.push(format!("  .zero {}", member.offset - end).into());
                    }
                    lines.push(self.gen_initializer_element(
                        &member.ty,
                        elements.and_then(|elements| elements.get(i)),
                    ));
                    end = member.offset + member.ty.size_of();
                }
                if ty.size_of() > end {
                    lines.push(format!("  .zero {}", ty.size_of() - end).into());
                }
                lines.into()
            }
            Type::Aligned(child, _) => self.gen_initializer_element(child, data),
            _ => format!(
                "  .8byte {}",
                if let Some(GlobalVariableData::Address { symbol, offset }) = data {
                    let symbol = self.address_symbol(symbol);
                    format!("{symbol}{offset:+}")
                } else if let Some(GlobalVariableData::Elem(s)) = data {
                    if s.starts_with("@.str.") {
                        s.replace('@', "L_")
                    } else if let Some(name) = s.strip_prefix('@') {
                        self.address_symbol(name)
                    } else {
                        s.clone()
                    }
                } else {
                    "0".to_string()
                }
            )
            .into(),
        }
    }
}

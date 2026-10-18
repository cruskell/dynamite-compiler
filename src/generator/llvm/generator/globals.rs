use super::IrGenerator;
use crate::ctype::Type;
use crate::global::{GlobalVariable, GlobalVariableData};
use crate::token::Token;
use std::collections::HashMap;

impl IrGenerator<'_> {
    pub(super) fn gen_string_literals(&self, string_literals: &[Token]) -> Vec<String> {
        string_literals
            .iter()
            .enumerate()
            .map(|(i, token)| {
                let bytes = token.encoded_string();
                let width = token.string_encoding.unit_size();
                let element_type = token.string_encoding.element_type();
                let initializer = if width == 1 {
                    format!(
                        "c\"{}\"",
                        bytes.iter().map(|byte| format!("\\{:02X}", byte)).collect::<String>()
                    )
                } else {
                    let elements = bytes.chunks_exact(width).map(|chunk| {
                        let mut value = [0u8; 4];
                        value[..width].copy_from_slice(chunk);
                        format!("{element_type} {}", u32::from_le_bytes(value))
                    }).collect::<Vec<_>>().join(", ");
                    format!("[{elements}]")
                };
                format!(
                    "@.str.{i} = private unnamed_addr constant [{} x {element_type}] {initializer}, align {width}",
                    bytes.len() / width,
                )
            })
            .collect::<Vec<String>>()
    }

    pub(super) fn gen_global_variable(&self, name: &str, gv: &GlobalVariable) -> String {
        if gv.declaration_only {
            // Opaque pointers can reference an external record before its layout is known.
            let incomplete_record = matches!(gv.ty.unqualified(), Type::Struct(record) if !record.definition().borrow().complete);
            let ty = if incomplete_record {
                "i8".into()
            } else {
                Self::gen_type(&gv.ty)
            };
            return format!(
                "{} = {} {}{}global {}, align {}",
                self.symbol_name(name),
                if gv.weak_import {
                    "extern_weak"
                } else {
                    "external"
                },
                if gv.hidden && !gv.internal {
                    "hidden "
                } else {
                    ""
                },
                if gv.thread_local { "thread_local " } else { "" },
                ty,
                Self::align(&gv.ty).max(gv.alignment).max(1)
            );
        }
        format!(
            "{} = {}{}{}global {}, align {}",
            self.symbol_name(name),
            if gv.internal {
                "internal "
            } else if gv.common && !gv.thread_local && gv.data.is_none() {
                "common "
            } else {
                ""
            },
            if gv.hidden && !gv.internal {
                "hidden "
            } else {
                ""
            },
            if gv.thread_local { "thread_local " } else { "" },
            if Self::contains_packed_record(&gv.ty) {
                self.packed_global_initializer(&gv.ty, gv.data.as_ref())
            } else {
                self.gen_initializer_element(&gv.ty, gv.data.as_ref())
            },
            Self::align(&gv.ty).max(gv.alignment),
        )
    }

    pub(super) fn contains_packed_record(ty: &Type) -> bool {
        match ty.unqualified() {
            Type::Arr(child, _) => Self::contains_packed_record(child),
            Type::Struct(record) => {
                record.definition().borrow().packing.is_some()
                    || ty.contains_typedef_alignment()
                    || record
                        .definition()
                        .borrow()
                        .members
                        .iter()
                        .any(|member| Self::contains_packed_record(&member.ty))
            }
            _ => false,
        }
    }

    pub(super) fn packed_global_initializer(
        &self,
        ty: &Type,
        data: Option<&GlobalVariableData>,
    ) -> String {
        let mut bytes = vec![0; ty.size_of()];
        let mut addresses = HashMap::new();
        crate::global::initializer_bytes(ty, data, 0, &mut bytes, &mut addresses);
        let mut addresses = addresses.into_iter().collect::<Vec<_>>();
        addresses.sort_by_key(|(offset, _)| *offset);
        let mut types = vec![];
        let mut values = vec![];
        let mut cursor = 0;
        for (offset, (symbol, delta)) in addresses {
            if offset > cursor {
                let count = offset - cursor;
                types.push(format!("[{count} x i8]"));
                values.push(format!(
                    "[{count} x i8] c\"{}\"",
                    bytes[cursor..offset]
                        .iter()
                        .map(|byte| format!("\\{byte:02X}"))
                        .collect::<String>()
                ));
            }
            types.push("ptr".into());
            let pointer = if delta == 0 {
                self.symbol_name(&symbol)
            } else {
                format!(
                    "getelementptr (i8, ptr {}, i64 {delta})",
                    self.symbol_name(&symbol)
                )
            };
            values.push(format!("ptr {pointer}"));
            cursor = offset + 8;
        }
        if cursor < bytes.len() {
            let count = bytes.len() - cursor;
            types.push(format!("[{count} x i8]"));
            values.push(format!(
                "[{count} x i8] c\"{}\"",
                bytes[cursor..]
                    .iter()
                    .map(|byte| format!("\\{byte:02X}"))
                    .collect::<String>()
            ));
        }
        // Opaque pointers allow the definition to use an exact packed storage
        // type, including relocations at offsets below pointer alignment.
        format!("<{{ {} }}> <{{ {} }}>", types.join(", "), values.join(", "))
    }

    pub(super) fn gen_initializer_element(
        &self,
        ty: &Type,
        data: Option<&GlobalVariableData>,
    ) -> String {
        let data = if !matches!(ty, Type::Arr(..) | Type::Struct(_)) {
            let mut value = data;
            while let Some(GlobalVariableData::Arr(elements)) = value {
                value = elements.first();
            }
            value
        } else {
            data
        };
        if ty.integer_info().is_some_and(|(bits, _)| bits == 64) {
            if let Some(GlobalVariableData::Address { symbol, offset }) = data {
                let pointer = if *offset == 0 {
                    self.symbol_name(symbol)
                } else {
                    format!(
                        "getelementptr (i8, ptr {}, i64 {offset})",
                        self.symbol_name(symbol)
                    )
                };
                return format!("i64 ptrtoint (ptr {pointer} to i64)");
            }
        }
        match ty {
            Type::Void | Type::Func(_, _) | Type::VariadicFunc(_, _) => {
                format!("{} zeroinitializer", Self::gen_type(ty))
            }
            Type::Bool
            | Type::I8
            | Type::SignedChar
            | Type::I32
            | Type::Integer { .. }
            | Type::LongLong { .. } => format!(
                "{} {}",
                Self::gen_type(ty),
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            ),
            Type::Integer128 { .. } => "<2 x i64> zeroinitializer".into(),
            Type::Enum(value) => self.gen_initializer_element(&value.underlying.borrow(), data),
            Type::Float16 => "half zeroinitializer".into(),
            Type::LongDouble(80) => {
                let bits = if let Some(GlobalVariableData::Elem(value)) = data {
                    value.parse::<u128>().unwrap()
                } else {
                    0
                };
                format!("x86_fp80 {}", crate::floating::llvm_literal(bits))
            }
            Type::LongDouble(128) => {
                format!("{} zeroinitializer", Self::gen_type(ty))
            }
            Type::Float(_) | Type::LongDouble(_) => {
                let bits = if let Some(GlobalVariableData::Elem(value)) = data {
                    value.parse::<i64>().unwrap() as usize
                } else {
                    0
                };
                format!("{} {}", Self::gen_type(ty), Self::float_literal(ty, bits))
            }
            Type::Ptr(_) => format!(
                "ptr {}",
                match data {
                    Some(GlobalVariableData::Address { symbol, offset }) => {
                        if *offset == 0 {
                            self.symbol_name(symbol)
                        } else {
                            format!(
                                "getelementptr (i8, ptr {}, i64 {offset})",
                                self.symbol_name(symbol)
                            )
                        }
                    }
                    Some(GlobalVariableData::Elem(value)) if value.starts_with('@') =>
                        value.clone(),
                    Some(GlobalVariableData::Elem(value)) if value != "0" =>
                        format!("inttoptr (i64 {} to ptr)", value),
                    _ => "null".to_string(),
                }
            ),
            Type::Arr(children_ty, size) => {
                if let Some(GlobalVariableData::Arr(v)) = data {
                    format!(
                        "{} [{}]",
                        Self::gen_type(ty),
                        (0..size.unwrap_or(0))
                            .map(|i| self.gen_initializer_element(children_ty, v.get(i)))
                            .collect::<Vec<String>>()
                            .join(", "),
                    )
                } else {
                    format!("{} zeroinitializer", Self::gen_type(ty))
                }
            }
            Type::Vla(..) => unreachable!("variable length arrays have automatic storage"),
            Type::Vector(..) => format!("{} zeroinitializer", Self::gen_type(ty)),
            Type::Struct(struct_type)
                if struct_type.definition().borrow().is_union || struct_type.has_bit_fields() =>
            {
                let mut bytes = vec![0u8; ty.size_of()];
                let mut addresses = HashMap::new();
                crate::global::initializer_bytes(ty, data, 0, &mut bytes, &mut addresses);
                let width = ty.align_of().min(8);
                let elements = bytes
                    .chunks(width)
                    .enumerate()
                    .map(|(index, chunk)| {
                        let mut word = [0u8; 8];
                        word[..chunk.len()].copy_from_slice(chunk);
                        if let Some((symbol, offset)) = addresses.get(&(index * width)) {
                            let pointer = if *offset == 0 {
                                self.symbol_name(symbol)
                            } else {
                                format!(
                                    "getelementptr (i8, ptr {}, i64 {offset})",
                                    self.symbol_name(symbol)
                                )
                            };
                            format!("i64 ptrtoint (ptr {pointer} to i64)")
                        } else {
                            format!("i{} {}", width * 8, u64::from_le_bytes(word))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if ty.align_of() > width {
                    format!(
                        "{} {{ [{} x i{}] [{}], [0 x {}] zeroinitializer }}",
                        Self::gen_type(ty),
                        ty.size_of() / width,
                        width * 8,
                        elements,
                        Self::alignment_type(ty.align_of()),
                    )
                } else {
                    format!("{} [{}]", Self::gen_type(ty), elements)
                }
            }
            Type::Struct(struct_type) => {
                if let Some(GlobalVariableData::Arr(elements)) = data {
                    let mut fields = vec![];
                    let mut offset = 0usize;
                    for (index, member) in
                        struct_type.definition().borrow().members.iter().enumerate()
                    {
                        let aligned_offset =
                            offset.div_ceil(member.ty.align_of()) * member.ty.align_of();
                        if member.offset > aligned_offset {
                            let padding = format!("[{} x i8]", member.offset - aligned_offset);
                            fields.push(format!("{padding} zeroinitializer"));
                        }
                        fields.push(self.gen_initializer_element(&member.ty, elements.get(index)));
                        offset = member.offset + member.ty.size_of();
                    }
                    if ty.size_of() > offset {
                        let padding = format!("[{} x i8]", ty.size_of() - offset);
                        fields.push(format!("{padding} zeroinitializer"));
                    }
                    if let Some(anchor) = Self::record_alignment_member(struct_type) {
                        fields.push(format!("{anchor} zeroinitializer"));
                    }
                    format!("{} {{ {} }}", Self::gen_type(ty), fields.join(", "))
                } else {
                    format!("{} zeroinitializer", Self::gen_type(ty))
                }
            }
            Type::Qualified(child, _) => self.gen_initializer_element(child, data),
            Type::Aligned(child, _) => self.gen_initializer_element(child, data),
        }
    }
}

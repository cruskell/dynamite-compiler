use std::{collections::HashMap, vec};

use crate::{
    ast::{Identifier, ProgramAst, reserved_functions},
    ctype::Type,
    error,
    func::Func,
    global::{GlobalVariable, GlobalVariableData},
    node::{AtomicOperation, AtomicRmwOperation, Node, NodeType, render_inline_assembly},
    token::Token,
};

#[derive(Debug)]
pub struct Options<'a> {
    register_number: &'a mut usize,
    register_map: &'a mut HashMap<usize, usize>,
    register_queue: &'a mut Vec<String>,
    label_number: usize,
    terminated: bool,
    return_ty: Type,
    return_pointer: Option<String>,
    break_labels: Vec<String>,
    expression_slots: HashMap<usize, usize>,
    argument_slots: HashMap<(usize, usize), usize>,
}

impl Options<'_> {
    fn new_label(&mut self) -> String {
        self.label_number += 1;
        format!("block.{}", self.label_number)
    }

    /// Allocate an SSA register without putting a value on the expression stack.
    fn new_temporary(&mut self) -> usize {
        *self.register_number += 1;
        *self.register_number
    }

    fn new_register(&mut self) -> usize {
        let register = self.new_temporary();
        self.register_queue.push(format!("%{register}"));
        register
    }
    fn register_from_offset(&mut self, offset: usize) -> usize {
        let entry = self.register_map.entry(offset);
        let register = *entry.or_insert_with(|| *self.register_number + 1);
        if register > *self.register_number {
            *self.register_number = register;
        }
        register
    }
}

pub struct IrGenerator<'a> {
    error_logger: &'a dyn error::ErrorLogger,
    symbol_names: HashMap<String, String>,
}

impl<'a> IrGenerator<'a> {
    pub fn new(error_logger: &'a dyn error::ErrorLogger) -> Self {
        Self {
            error_logger,
            symbol_names: HashMap::new(),
        }
    }
    pub fn generate(&self, ast: ProgramAst) -> String {
        let generator = Self {
            error_logger: self.error_logger,
            symbol_names: ast.symbol_names.clone(),
        };
        [
            vec![
                "declare void @llvm.va_start(ptr)".into(),
                "declare void @llvm.va_end(ptr)".into(),
                "declare void @llvm.prefetch.p0(ptr, i32, i32, i32)".into(),
            ],
            generator.gen_string_literals(ast.string_literals),
            ast.global_variables
                .iter()
                .map(|(s, gv)| generator.gen_global_variable(s, gv))
                .collect::<Vec<String>>(),
            ast.functions
                .iter()
                .map(|(name, f)| generator.gen_func(name, f) + "\n")
                .collect::<Vec<String>>(),
            reserved_functions()
                .iter()
                .filter_map(|(name, ident)| {
                    if ast.functions.contains_key(name) {
                        return None;
                    }
                    let Identifier::Static(ty) = ident else {
                        unreachable!()
                    };
                    let (args, result, variadic) = ty.function_signature().unwrap();
                    let mut args = args
                        .iter()
                        .map(|ty| Self::abi_parameter_type(ty.clone()))
                        .collect::<Vec<_>>();
                    if variadic {
                        args.push("...".to_string());
                    }
                    Some(format!(
                        "declare {} {}({})",
                        Self::abi_result_type(result.clone()),
                        generator.symbol_name(name),
                        args.join(", ")
                    ))
                })
                .collect(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn symbol_name(&self, name: &str) -> String {
        if let Some(label) = self.symbol_names.get(name) {
            // LLVM's leading byte 1 suppresses target-specific symbol prefixes.
            format!("@\"\\01{label}\"")
        } else {
            format!("@{name}")
        }
    }

    fn gen_string_literals(&self, string_literals: &[Token]) -> Vec<String> {
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

    fn align(ty: &Type) -> usize {
        ty.align_of()
    }

    fn gen_global_variable(&self, name: &str, gv: &GlobalVariable) -> String {
        if gv.declaration_only {
            // Opaque pointers can reference an external record before its layout is known.
            let incomplete_record = matches!(gv.ty.unqualified(), Type::Struct(record) if !record.definition().borrow().complete);
            let ty = if incomplete_record {
                "i8".into()
            } else {
                Self::gen_type(gv.ty.clone())
            };
            return format!(
                "{} = {} {}global {}, align {}",
                self.symbol_name(name),
                if gv.weak_import {
                    "extern_weak"
                } else {
                    "external"
                },
                if gv.thread_local { "thread_local " } else { "" },
                ty,
                Self::align(&gv.ty).max(gv.alignment).max(1)
            );
        }
        format!(
            "{} = {}{}global {}, align {}",
            self.symbol_name(name),
            if gv.internal {
                "internal "
            } else if gv.common && !gv.thread_local && gv.data.is_none() {
                "common "
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

    fn contains_packed_record(ty: &Type) -> bool {
        match ty.unqualified() {
            Type::Struct(record) => {
                record.definition().borrow().packing.is_some()
                    || ty.contains_reduced_alignment()
                    || record
                        .members()
                        .iter()
                        .any(|member| Self::contains_packed_record(&member.ty))
            }
            Type::Arr(child, _) => Self::contains_packed_record(child),
            _ => false,
        }
    }

    fn packed_global_initializer(&self, ty: &Type, data: Option<&GlobalVariableData>) -> String {
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

    fn access_alignment(node: &Node) -> usize {
        if matches!(node.nt, NodeType::Deref) {
            1
        } else {
            node.object_type()
                .unwrap_or_else(|| node.resolve_type().unwrap())
                .align_of()
        }
    }

    fn float_literal(ty: &Type, bits: usize) -> String {
        format!("0x{:016X}", ty.float_value(bits).to_bits())
    }
    fn alignment_type(alignment: usize) -> String {
        if alignment >= 16 {
            format!("<{alignment} x i8>")
        } else {
            format!("i{}", alignment * 8)
        }
    }

    fn record_alignment_member(ty: &crate::ctype::StructType) -> Option<String> {
        let natural = ty
            .members()
            .iter()
            .map(|member| member.ty.align_of())
            .max()
            .unwrap_or(1);
        let alignment = ty.definition().borrow().align;
        (alignment > natural).then(|| format!("[0 x {}]", Self::alignment_type(alignment)))
    }

    fn abi_extension(ty: &Type) -> Option<&'static str> {
        if !cfg!(any(
            target_arch = "x86_64",
            all(target_arch = "aarch64", target_os = "macos")
        )) {
            return None;
        }
        ty.integer_info().and_then(|(bits, signed)| {
            (bits < 32).then_some(if signed { "signext" } else { "zeroext" })
        })
    }

    fn abi_parameter_type(ty: Type) -> String {
        let extension = Self::abi_extension(&ty);
        let ty = Self::gen_type(ty);
        match extension {
            Some(extension) => format!("{ty} {extension}"),
            None => ty,
        }
    }

    fn abi_result_type(ty: Type) -> String {
        let extension = Self::abi_extension(&ty);
        let ty = Self::gen_type(ty);
        match extension {
            Some(extension) => format!("{extension} {ty}"),
            None => ty,
        }
    }

    fn gen_type(ty: Type) -> String {
        match ty {
            Type::Qualified(child, _) | Type::Aligned(child, _) => Self::gen_type(*child),
            Type::Void => "void".to_string(),
            // LLVM 15 gives i128 only 8-byte alignment on some targets.
            Type::Integer128 { .. } => "<2 x i64>".to_string(),
            Type::Float16 => "half".to_string(),
            Type::Vector(element, lanes) => format!("<{lanes} x {}>", Self::gen_type(*element)),
            Type::Float(bits) | Type::LongDouble(bits) => {
                if bits == 32 { "float" } else { "double" }.to_string()
            }
            Type::Struct(ty)
                if ty.definition().borrow().is_union
                    || ty.has_bit_fields()
                    || ty.definition().borrow().packing.is_some()
                    || Type::Struct(ty.clone()).contains_reduced_alignment() =>
            {
                let ty = Type::Struct(ty);
                let width = ty.align_of().min(8);
                let storage = format!("[{} x i{}]", ty.size_of() / width, width * 8);
                if ty.align_of() > width {
                    format!(
                        "{{ {storage}, [0 x {}] }}",
                        Self::alignment_type(ty.align_of())
                    )
                } else {
                    storage
                }
            }
            Type::Struct(ty) => {
                let aggregate = Type::Struct(ty.clone());
                let mut fields = vec![];
                let mut offset = 0usize;
                for member in ty.members() {
                    let aligned_offset =
                        offset.div_ceil(member.ty.align_of()) * member.ty.align_of();
                    if member.offset > aligned_offset {
                        fields.push(format!("[{} x i8]", member.offset - aligned_offset));
                    }
                    fields.push(Self::gen_type(member.ty.clone()));
                    offset = member.offset + member.ty.size_of();
                }
                if aggregate.size_of() > offset {
                    fields.push(format!("[{} x i8]", aggregate.size_of() - offset));
                }
                if let Some(anchor) = Self::record_alignment_member(&ty) {
                    fields.push(anchor);
                }
                format!("{{ {} }}", fields.join(", "))
            }
            Type::Enum(value) => Self::gen_type(value.underlying.borrow().clone()),
            Type::I8 | Type::SignedChar => "i8".to_string(),
            Type::LongLong { .. } => "i64".to_string(),
            Type::I32 => "i32".to_string(),
            Type::Integer { bits, .. } => format!("i{bits}"),
            Type::Bool => "i8".to_string(),
            Type::Ptr(_) => "ptr".to_string(),
            Type::Arr(child, size) => {
                format!("[{} x {}]", size.unwrap_or(0), Self::gen_type(*child))
            }
            Type::Vla(child, _) => format!("[0 x {}]", Self::gen_type(*child)),
            Type::VariadicFunc(args, result) => format!(
                "{} ({}, ...)",
                Self::gen_type(*result),
                args.into_iter()
                    .map(Self::gen_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::Func(args, result) => format!(
                "{} ({})",
                Self::gen_type(*result),
                args.into_iter()
                    .map(Self::gen_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn gen_type_with_opaque_ptr(ty: Type) -> String {
        Self::gen_type(ty)
    }

    fn gen_initializer_element(&self, ty: &Type, data: Option<&GlobalVariableData>) -> String {
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
            Type::Aligned(child, _) => self.gen_initializer_element(child, data),
            Type::Vla(..) => unreachable!("variable length arrays have automatic storage"),
            Type::Integer128 { .. } => "<2 x i64> zeroinitializer".into(),
            Type::Float16 => "half zeroinitializer".into(),
            Type::Vector(..) => format!("{} zeroinitializer", Self::gen_type(ty.clone())),
            Type::Float(_) | Type::LongDouble(_) => {
                let bits = if let Some(GlobalVariableData::Elem(value)) = data {
                    value.parse::<i64>().unwrap() as usize
                } else {
                    0
                };
                format!(
                    "{} {}",
                    Self::gen_type(ty.clone()),
                    Self::float_literal(ty, bits)
                )
            }

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
                        Self::gen_type(ty.clone()),
                        ty.size_of() / width,
                        width * 8,
                        elements,
                        Self::alignment_type(ty.align_of()),
                    )
                } else {
                    format!("{} [{}]", Self::gen_type(ty.clone()), elements)
                }
            }
            Type::Qualified(child, _) => self.gen_initializer_element(child, data),
            Type::Struct(struct_type) => {
                if let Some(GlobalVariableData::Arr(elements)) = data {
                    let mut fields = vec![];
                    let mut offset = 0usize;
                    for (index, member) in struct_type.members().iter().enumerate() {
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
                    format!("{} {{ {} }}", Self::gen_type(ty.clone()), fields.join(", "))
                } else {
                    format!("{} zeroinitializer", Self::gen_type(ty.clone()))
                }
            }
            Type::Arr(children_ty, size) => {
                if let Some(GlobalVariableData::Arr(v)) = data {
                    format!(
                        "{} [{}]",
                        Self::gen_type(ty.clone()),
                        (0..size.unwrap_or(0))
                            .map(|i| self.gen_initializer_element(children_ty, v.get(i)))
                            .collect::<Vec<String>>()
                            .join(", "),
                    )
                } else {
                    format!("{} zeroinitializer", Self::gen_type(ty.clone()))
                }
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
            Type::Enum(value) => self.gen_initializer_element(&value.underlying.borrow(), data),
            Type::I8
            | Type::SignedChar
            | Type::I32
            | Type::LongLong { .. }
            | Type::Integer { .. }
            | Type::Bool => format!(
                "{} {}",
                Self::gen_type(ty.clone()),
                if let Some(GlobalVariableData::Elem(s)) = data {
                    s
                } else {
                    "0"
                }
            ),
            Type::Void | Type::Func(_, _) | Type::VariadicFunc(_, _) => {
                format!("{} zeroinitializer", Self::gen_type(ty.clone()))
            }
        }
    }

    fn target_arch() -> crate::generator::Arch {
        if cfg!(target_arch = "aarch64") {
            crate::generator::Arch::Aarch64
        } else {
            crate::generator::Arch::X86_64
        }
    }
    fn target_os() -> crate::generator::Os {
        if cfg!(target_os = "macos") {
            crate::generator::Os::MacOS
        } else {
            crate::generator::Os::Linux
        }
    }
    fn aggregate_result_type(ty: &Type) -> String {
        let parts = crate::generator::abi::aggregate_parts(ty, &Self::target_arch());
        if parts.is_empty() {
            "void".into()
        } else if parts.len() == 1 {
            Self::gen_type(parts[0].ty.clone())
        } else {
            format!(
                "{{ {} }}",
                parts
                    .iter()
                    .map(|part| Self::gen_type(part.ty.clone()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
    fn aggregate_signature(
        types: &[Type],
        result: &Type,
        fixed: usize,
        variadic: bool,
    ) -> crate::generator::abi::Signature {
        use crate::generator::abi;
        let mut signature = abi::signature(
            types,
            result,
            Self::target_arch(),
            Self::target_os(),
            fixed,
            variadic,
        );
        if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            for (ty, argument) in types.iter().zip(&mut signature.arguments) {
                if let Some(offset) = argument.memory {
                    let parts = abi::aggregate_parts(ty, &Self::target_arch());
                    if !parts.is_empty() {
                        // AArch64 lowers direct aggregates using the same coercion
                        // whether they occupy registers or stack slots. LLVM byval
                        // has different stack-slot accounting on Darwin.
                        argument.parts = parts
                            .into_iter()
                            .map(|part| (part, abi::Location::Stack(offset)))
                            .collect();
                        argument.memory = None;
                    }
                }
            }
        }
        signature
    }
    fn aggregate_parameter_types(
        ty: &Type,
        argument: &crate::generator::abi::Argument,
    ) -> Vec<String> {
        if argument.memory.is_some() {
            return vec![format!(
                "ptr byval({}) align {}",
                Self::gen_type(ty.clone()),
                argument.stack_alignment
            )];
        }
        if argument.indirect {
            return vec!["ptr".into()];
        }
        if cfg!(target_arch = "aarch64")
            && matches!(ty.unqualified(), Type::Struct(_))
            && (if cfg!(target_os = "linux") {
                ty.natural_align_of()
            } else {
                ty.align_of()
            }) >= 16
            && argument.parts.len() == 2
            && argument
                .parts
                .iter()
                .all(|(part, _)| part.ty.float_bits().is_none())
        {
            return vec!["i128".into()];
        }
        let parts = argument
            .parts
            .iter()
            .map(|(part, _)| Self::gen_type(part.ty.clone()))
            .collect::<Vec<_>>();
        if cfg!(target_arch = "aarch64") && matches!(ty, Type::Struct(_)) && parts.len() > 1 {
            vec![format!("[{} x {}]", parts.len(), parts[0])]
        } else {
            parts
        }
    }
    fn aggregate_pointer(
        base: &str,
        offset: usize,
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        if offset == 0 {
            return base.into();
        }
        let register = options.new_temporary();
        lines.push(format!(
            "  %{register} = getelementptr i8, ptr {base}, i64 {offset}"
        ));
        format!("%{register}")
    }
    fn aggregate_load_part(
        part: &crate::generator::abi::Part,
        size: usize,
        base: &str,
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        let pointer = Self::aggregate_pointer(base, part.offset, options, lines);
        let register = options.new_temporary();
        let remaining = size - part.offset;
        let loaded_type = if remaining < part.ty.size_of() {
            format!("i{}", remaining * 8)
        } else {
            Self::gen_type(part.ty.clone())
        };
        lines.push(format!(
            "  %{register} = load {loaded_type}, ptr {pointer}, align 1"
        ));
        if remaining < part.ty.size_of() {
            let extended = options.new_temporary();
            lines.push(format!(
                "  %{extended} = zext {loaded_type} %{register} to {}",
                Self::gen_type(part.ty.clone())
            ));
            format!("%{extended}")
        } else {
            format!("%{register}")
        }
    }

    fn aggregate_function(&self, name: &str, func: &Func) -> String {
        let (types, result, variadic) = func.cty.function_signature().unwrap();
        let signature = Self::aggregate_signature(&types, &result, types.len(), variadic);
        let return_type = if matches!(result, Type::Struct(_)) {
            Self::aggregate_result_type(&result)
        } else {
            Self::gen_type(result.clone())
        };
        let return_abi = if matches!(result, Type::Struct(_)) {
            return_type.clone()
        } else {
            Self::abi_result_type(result.clone())
        };
        let mut parameters = vec![];
        if signature.indirect_result {
            parameters.push(format!(
                "ptr sret({}) align {} %abi.result",
                Self::gen_type(result.clone()),
                result.align_of()
            ));
        }
        for (index, argument) in signature.arguments.iter().enumerate() {
            for (part, ty) in Self::aggregate_parameter_types(&types[index], argument)
                .into_iter()
                .enumerate()
            {
                let ty = match Self::abi_extension(&types[index]) {
                    Some(extension) => format!("{ty} {extension}"),
                    None => ty,
                };
                parameters.push(if func.body.is_some() {
                    format!("{ty} %abi.arg.{index}.{part}")
                } else {
                    ty
                });
            }
        }
        if variadic {
            parameters.push("...".into());
        }
        if func.body.is_none() || func.inline_only() {
            return format!(
                "declare {}{return_abi} {}({}){}",
                if func.weak_import { "extern_weak " } else { "" },
                self.symbol_name(name),
                parameters.join(", "),
                if func.noinline { " noinline" } else { "" }
            );
        }
        let mut number = 0;
        let mut registers = HashMap::new();
        let mut queue = vec![];
        let mut options = Options {
            register_number: &mut number,
            register_map: &mut registers,
            register_queue: &mut queue,
            label_number: 0,
            terminated: false,
            return_ty: result.clone(),
            return_pointer: signature.indirect_result.then(|| "%abi.result".into()),
            break_labels: vec![],
            expression_slots: HashMap::new(),
            argument_slots: HashMap::new(),
        };
        let mut lines = vec![format!(
            "define {}{return_abi} {}({}){} uwtable \"frame-pointer\"=\"all\" {{",
            if func.internal { "internal " } else { "" },
            self.symbol_name(name),
            parameters.join(", "),
            if func.noinline { " noinline" } else { "" }
        )];
        for (index, argument) in signature.arguments.iter().enumerate() {
            let ty = &types[index];
            let register = options.new_temporary();
            options
                .register_map
                .insert(func.args[index].offset.unwrap(), register);
            lines.push(format!(
                "  %{register} = alloca [{} x i8], align {}",
                ty.size_of().div_ceil(8) * 8,
                ty.align_of()
            ));
            if argument.memory.is_some() || argument.indirect {
                let value = options.new_temporary();
                lines.push(format!(
                    "  %{value} = load {}, ptr %abi.arg.{index}.0, align 1",
                    Self::gen_type(ty.clone())
                ));
                lines.push(format!(
                    "  store {} %{value}, ptr %{register}, align {}",
                    Self::gen_type(ty.clone()),
                    ty.align_of()
                ));
            } else {
                let grouped = cfg!(target_arch = "aarch64")
                    && matches!(ty, Type::Struct(_))
                    && argument.parts.len() > 1;
                if argument.parts.is_empty() {
                    continue;
                }
                let group_type = Self::aggregate_parameter_types(ty, argument)[0].clone();
                if group_type == "i128" {
                    lines.push(format!(
                        "  store i128 %abi.arg.{index}.0, ptr %{register}, align 1"
                    ));
                    continue;
                }
                for (part_index, (part, _)) in argument.parts.iter().enumerate() {
                    let value = if grouped {
                        let value = options.new_temporary();
                        lines.push(format!("  %{value} = extractvalue {group_type} %abi.arg.{index}.0, {part_index}"));
                        format!("%{value}")
                    } else {
                        format!("%abi.arg.{index}.{part_index}")
                    };
                    let pointer = Self::aggregate_pointer(
                        &format!("%{register}"),
                        part.offset,
                        &mut options,
                        &mut lines,
                    );
                    lines.push(format!(
                        "  store {} {value}, ptr {pointer}, align 1",
                        Self::gen_type(part.ty.clone())
                    ));
                }
            }
        }
        Self::allocate_locals(func.body.as_ref().unwrap(), &mut options, &mut lines);
        lines.extend(self.gen_node(func.body.as_ref().unwrap(), &mut options));
        if !options.terminated {
            lines.push(if return_type == "void" {
                "  ret void".into()
            } else {
                format!("  ret {return_type} zeroinitializer")
            });
        }
        lines.push("}".into());
        lines.join("\n")
    }
    fn aggregate_call(&self, node: &Node, options: &mut Options) -> Vec<String> {
        let types = node
            .args
            .iter()
            .rev()
            .map(|arg| arg.resolve_type().unwrap())
            .collect::<Vec<_>>();
        let result = node.resolve_type().unwrap();
        let signature =
            Self::aggregate_signature(&types, &result, node.parameter_types.len(), node.variadic);
        let mut lines = vec![];
        let callee = if let Some(callee) = &node.lhs {
            lines.extend(self.gen_node(callee, options));
            options.register_queue.pop().unwrap()
        } else {
            self.symbol_name(&node.global_name)
        };
        let mut arguments = vec![];
        for (index, arg) in node.args.iter().rev().enumerate() {
            let argument = &signature.arguments[index];
            let ty = &types[index];
            let aggregate = matches!(ty, Type::Struct(_));
            let value = if aggregate {
                lines.extend(self.gen_addr(arg, options));
                options.register_queue.pop().unwrap()
            } else {
                lines.extend(self.gen_node(arg, options));
                options.register_queue.pop().unwrap()
            };
            let parameter_types = Self::aggregate_parameter_types(ty, argument);
            if parameter_types.is_empty() {
                continue;
            }
            let aligned_variadic = cfg!(all(target_arch = "aarch64", target_os = "macos"))
                && node.variadic
                && index >= node.parameter_types.len()
                && aggregate
                && !argument.indirect
                && argument.stack_alignment == 16;
            if aligned_variadic {
                // Unnamed Darwin arguments occupy stack slots. Scalar i128 chunks
                // preserve sixteen-byte alignment, including larger HFAs.
                for offset in (0..ty.size_of()).step_by(16) {
                    let pointer = Self::aggregate_pointer(&value, offset, options, &mut lines);
                    let register = options.new_temporary();
                    lines.push(format!("  %{register} = load i128, ptr {pointer}, align 1"));
                    arguments.push(format!("i128 %{register}"));
                }
                continue;
            }
            if argument.memory.is_some() || argument.indirect {
                let slot = options.argument_slots[&(node.token.as_ref().unwrap().pos, index)];
                let register = options.new_temporary();
                let alignment = ty.align_of().max(argument.stack_alignment);
                lines.push(format!(
                    "  %{register} = load {}, ptr {value}, align 1",
                    Self::gen_type(ty.clone())
                ));
                lines.push(format!(
                    "  store {} %{register}, ptr %{slot}, align {alignment}",
                    Self::gen_type(ty.clone())
                ));
                arguments.push(format!("{} %{slot}", parameter_types[0]));
            } else if aggregate && (parameter_types[0] == "i128") {
                let register = options.new_temporary();
                lines.push(format!(
                    "  %{register} = load {}, ptr {value}, align 1",
                    parameter_types[0]
                ));
                arguments.push(format!("{} %{register}", parameter_types[0]));
            } else {
                let mut values = vec![];
                for (part, _) in &argument.parts {
                    let value = if aggregate {
                        Self::aggregate_load_part(part, ty.size_of(), &value, options, &mut lines)
                    } else {
                        value.clone()
                    };
                    values.push(value);
                }
                if parameter_types.len() == 1 && values.len() > 1 {
                    let mut group = "poison".to_string();
                    for (index, value) in values.iter().enumerate() {
                        let register = options.new_temporary();
                        lines.push(format!(
                            "  %{register} = insertvalue {} {group}, {} {value}, {index}",
                            parameter_types[0],
                            Self::gen_type(argument.parts[index].0.ty.clone())
                        ));
                        group = format!("%{register}");
                    }
                    arguments.push(format!("{} {group}", parameter_types[0]));
                } else {
                    arguments.extend(parameter_types.into_iter().zip(values).map(
                        |(part_type, value)| {
                            let parameter_type = match Self::abi_extension(ty) {
                                Some(extension) => format!("{part_type} {extension}"),
                                None => part_type,
                            };
                            format!("{parameter_type} {value}")
                        },
                    ));
                }
            }
        }
        let result_slot = node
            .offset
            .map(|offset| format!("%{}", options.register_map[&offset]));
        if signature.indirect_result {
            arguments.insert(
                0,
                format!(
                    "ptr sret({}) align {} {}",
                    Self::gen_type(result.clone()),
                    result.align_of(),
                    result_slot.as_ref().unwrap()
                ),
            );
        }
        let return_type = if matches!(result, Type::Struct(_)) {
            Self::aggregate_result_type(&result)
        } else {
            Self::gen_type(result.clone())
        };
        let return_abi = if matches!(result, Type::Struct(_)) {
            return_type.clone()
        } else {
            Self::abi_result_type(result.clone())
        };
        let variadic_signature = if node.variadic {
            let fixed = Self::aggregate_signature(
                &node.parameter_types,
                &result,
                node.parameter_types.len(),
                true,
            );
            let mut parameters = vec![];
            if fixed.indirect_result {
                parameters.push("ptr".into());
            }
            for (index, argument) in fixed.arguments.iter().enumerate() {
                parameters.extend(
                    Self::aggregate_parameter_types(&node.parameter_types[index], argument)
                        .into_iter()
                        .map(|ty| ty.split(" byval").next().unwrap().to_string()),
                );
            }
            parameters.push("...".into());
            format!(" ({})", parameters.join(", "))
        } else {
            String::new()
        };
        let value = if return_type == "void" {
            lines.push(format!(
                "  call void{variadic_signature} {callee}({})",
                arguments.join(", ")
            ));
            "0".into()
        } else {
            let register = options.new_temporary();
            lines.push(format!(
                "  %{register} = call {return_abi}{variadic_signature} {callee}({})",
                arguments.join(", ")
            ));
            format!("%{register}")
        };
        if matches!(result, Type::Struct(_)) {
            let slot = result_slot.unwrap();
            if !signature.indirect_result {
                for (index, part) in signature.result.iter().enumerate() {
                    let value = if signature.result.len() > 1 {
                        let register = options.new_temporary();
                        lines.push(format!(
                            "  %{register} = extractvalue {return_type} {value}, {index}"
                        ));
                        format!("%{register}")
                    } else {
                        value.clone()
                    };
                    let pointer = Self::aggregate_pointer(&slot, part.offset, options, &mut lines);
                    let remaining = result.size_of() - part.offset;
                    let mut stored_type = Self::gen_type(part.ty.clone());
                    let value = if remaining < part.ty.size_of() {
                        let narrowed = options.new_temporary();
                        let narrow_type = format!("i{}", remaining * 8);
                        lines.push(format!(
                            "  %{narrowed} = trunc {stored_type} {value} to {narrow_type}"
                        ));
                        stored_type = narrow_type;
                        format!("%{narrowed}")
                    } else {
                        value
                    };
                    lines.push(format!(
                        "  store {stored_type} {value}, ptr {pointer}, align 1"
                    ));
                }
            }
            let register = options.new_register();
            lines.push(format!(
                "  %{register} = load {}, ptr {slot}, align {}",
                Self::gen_type(result.clone()),
                result.align_of()
            ));
        } else {
            options.register_queue.push(value);
        }
        lines
    }
    fn aggregate_return(&self, node: &Node, options: &mut Options) -> Vec<String> {
        let ty = options.return_ty.clone();
        let mut lines = self.gen_addr(node.lhs.as_ref().unwrap(), options);
        let address = options.register_queue.pop().unwrap();
        if ty.size_of() == 0 {
            lines.push("  ret void".into());
            options.terminated = true;
            return lines;
        }
        let parts = crate::generator::abi::aggregate_parts(&ty, &Self::target_arch());
        if let Some(pointer) = options.return_pointer.clone() {
            let register = options.new_temporary();
            lines.push(format!(
                "  %{register} = load {}, ptr {address}, align 1",
                Self::gen_type(ty.clone())
            ));
            lines.push(format!(
                "  store {} %{register}, ptr {pointer}, align {}",
                Self::gen_type(ty.clone()),
                ty.align_of()
            ));
            lines.push("  ret void".into());
        } else {
            let return_type = Self::aggregate_result_type(&ty);
            let mut result = "poison".to_string();
            for (index, part) in parts.iter().enumerate() {
                let value =
                    Self::aggregate_load_part(part, ty.size_of(), &address, options, &mut lines);
                if parts.len() == 1 {
                    result = value;
                } else {
                    let combined = options.new_temporary();
                    lines.push(format!(
                        "  %{combined} = insertvalue {return_type} {result}, {} {value}, {index}",
                        Self::gen_type(part.ty.clone())
                    ));
                    result = format!("%{combined}");
                }
            }
            lines.push(format!("  ret {return_type} {result}"));
        }
        options.terminated = true;
        lines
    }
    fn gen_func(&self, name: &str, func: &Func) -> String {
        let (types, result, _) = func.cty.function_signature().unwrap();
        if matches!(result, Type::Struct(_)) || types.iter().any(|ty| matches!(ty, Type::Struct(_)))
        {
            return self.aggregate_function(name, func);
        }
        if func.body.is_none() || func.inline_only() {
            let (args, result, variadic) = func.cty.function_signature().unwrap();
            let mut arguments = args
                .iter()
                .map(|ty| Self::abi_parameter_type(ty.clone()))
                .collect::<Vec<_>>();
            if variadic {
                arguments.push("...".to_string());
            }
            return format!(
                "declare {}{} {}({}){}",
                if func.weak_import { "extern_weak " } else { "" },
                Self::abi_result_type(result),
                self.symbol_name(name),
                arguments.join(", "),
                if func.noinline { " noinline" } else { "" }
            );
        }
        let (_, result, variadic) = func.cty.function_signature().unwrap();
        let return_ty = &Box::new(result);
        let options = &mut Options {
            register_number: &mut func.args.len(),
            register_map: &mut HashMap::<usize, usize>::new(),
            register_queue: &mut vec![],
            label_number: 0,
            terminated: false,
            return_ty: *return_ty.clone(),
            return_pointer: None,
            break_labels: vec![],
            expression_slots: HashMap::new(),
            argument_slots: HashMap::new(),
        };
        [
            vec![format!(
                "define {}{} {}({}){} uwtable \"frame-pointer\"=\"all\" {{",
                if func.internal { "internal " } else { "" },
                Self::abi_result_type(*return_ty.clone()),
                self.symbol_name(name),
                func.args
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| format!(
                        "{} noundef %{}",
                        Self::abi_parameter_type(arg.resolve_type().unwrap()),
                        i
                    ))
                    .chain(variadic.then_some("...".to_string()))
                    .collect::<Vec<_>>()
                    .join(", "),
                if func.noinline { " noinline" } else { "" }
            )],
            func.args
                .iter()
                .enumerate()
                .map(|(i, node)| {
                    let register = options.new_register();
                    options.register_map.insert(node.offset.unwrap(), register);
                    format!(
                        "  %{} = alloca {}, align {}",
                        register,
                        Self::gen_type(node.resolve_type().unwrap()),
                        node.resolve_type().unwrap().size_of(),
                    ) + "\n"
                        + &format!(
                            "  store {} %{}, ptr %{}, align {}",
                            Self::gen_type(node.resolve_type().unwrap()),
                            i,
                            register,
                            node.resolve_type().unwrap().size_of(),
                        )
                })
                .collect::<Vec<String>>(),
            {
                let mut lines = vec![];
                Self::allocate_locals(func.body.as_ref().unwrap(), options, &mut lines);
                lines
            },
            self.gen_node(func.body.as_ref().unwrap(), options),
            if options.terminated {
                vec!["}".to_string()]
            } else {
                vec![
                    if **return_ty == Type::Void {
                        "  ret void".to_string()
                    } else {
                        format!(
                            "  ret {} {}",
                            Self::gen_type(*return_ty.clone()),
                            if matches!(return_ty.as_ref(), Type::Ptr(_)) {
                                "null"
                            } else if return_ty.float_bits().is_some() {
                                "0.0"
                            } else {
                                "0"
                            }
                        )
                    },
                    "}".to_string(),
                ]
            },
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn allocate_locals(node: &Node, options: &mut Options, lines: &mut Vec<String>) {
        if matches!(node.nt, NodeType::LocalVar)
            || (node.nt == NodeType::CallFunc
                && matches!(node.resolve_type(), Some(Type::Struct(_))))
        {
            let offset = node.offset.unwrap();
            if !options.register_map.contains_key(&offset) {
                let register = options.register_from_offset(offset);
                let ty = node.resolve_type().unwrap();
                lines.push(format!(
                    "  %{} = alloca {}, align {}",
                    register,
                    if matches!(ty, Type::Struct(_)) {
                        format!("[{} x i8]", ty.size_of().div_ceil(8) * 8)
                    } else {
                        Self::gen_type_with_opaque_ptr(ty.clone())
                    },
                    Self::align(&ty).max(node.alignment)
                ));
            }
        }
        if node.nt == NodeType::CallFunc {
            let types = node
                .args
                .iter()
                .rev()
                .map(|arg| arg.resolve_type().unwrap())
                .collect::<Vec<_>>();
            let signature = Self::aggregate_signature(
                &types,
                &node.resolve_type().unwrap(),
                node.parameter_types.len(),
                node.variadic,
            );
            for (index, (ty, argument)) in types.iter().zip(&signature.arguments).enumerate() {
                if argument.memory.is_some() || argument.indirect {
                    let register = options.new_temporary();
                    options
                        .argument_slots
                        .insert((node.token.as_ref().unwrap().pos, index), register);
                    lines.push(format!(
                        "  %{register} = alloca {}, align {}",
                        Self::gen_type(ty.clone()),
                        ty.align_of().max(argument.stack_alignment)
                    ));
                }
            }
        }
        if matches!(
            node.nt,
            NodeType::LogicalAnd | NodeType::LogicalOr | NodeType::Conditional
        ) {
            let ty = node.resolve_type().unwrap();
            if ty != Type::Void {
                let register = options.new_temporary();
                options
                    .expression_slots
                    .insert(node.token.as_ref().unwrap().pos, register);
                lines.push(format!(
                    "  %{} = alloca {}, align {}",
                    register,
                    Self::gen_type(ty.clone()),
                    Self::align(&ty)
                ));
            }
        }
        for child in [
            &node.lhs, &node.rhs, &node.cond, &node.then, &node.els, &node.ini, &node.upd,
        ]
        .into_iter()
        .flatten()
        {
            Self::allocate_locals(child, options, lines);
        }
        for child in node.children.iter().chain(&node.args) {
            Self::allocate_locals(child, options, lines);
        }
    }

    fn convert_integer(
        value: String,
        source: &Type,
        target: &Type,
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        if source == target
            || (source.float_bits().is_some() && source.float_bits() == target.float_bits())
        {
            return value;
        }
        if source.float_bits().is_some() || target.float_bits().is_some() {
            let register = options.new_temporary();
            if *target == Type::Bool {
                lines.push(format!(
                    "  %{register} = fcmp une {} {value}, 0.0",
                    Self::gen_type(source.clone())
                ));
                let result = options.new_temporary();
                lines.push(format!("  %{result} = zext i1 %{register} to i8"));
                return format!("%{result}");
            }
            let operation = match (source.float_bits(), target.float_bits()) {
                (Some(sb), Some(tb)) => {
                    if sb < tb {
                        "fpext"
                    } else {
                        "fptrunc"
                    }
                }
                (None, Some(_)) => {
                    if source.integer_info().unwrap().1 {
                        "sitofp"
                    } else {
                        "uitofp"
                    }
                }
                (Some(_), None) => {
                    if target.integer_info().unwrap().1 {
                        "fptosi"
                    } else {
                        "fptoui"
                    }
                }
                _ => unreachable!(),
            };
            lines.push(format!(
                "  %{register} = {operation} {} {value} to {}",
                Self::gen_type(source.clone()),
                Self::gen_type(target.clone())
            ));
            return format!("%{register}");
        }
        if matches!(target, Type::Ptr(_)) && source.integer_info().is_some() {
            if value == "0" {
                return "null".to_string();
            }
            let wide_value = Self::convert_integer(
                value,
                source,
                &Type::integer(64, source.integer_info().unwrap().1),
                options,
                lines,
            );
            let register = options.new_temporary();
            lines.push(format!("  %{register} = inttoptr i64 {wide_value} to ptr"));
            return format!("%{register}");
        }
        if *target == Type::Bool && source.is_pointer_value() {
            let predicate = options.new_temporary();
            lines.push(format!("  %{predicate} = icmp ne ptr {value}, null"));
            let result = options.new_temporary();
            lines.push(format!("  %{result} = zext i1 %{predicate} to i8"));
            return format!("%{result}");
        }
        if source.is_pointer_value() && target.integer_info().is_some() {
            let register = options.new_temporary();
            lines.push(format!(
                "  %{} = ptrtoint ptr {} to {}",
                register,
                value,
                Self::gen_type(target.clone())
            ));
            return format!("%{}", register);
        }
        if *target == Type::Bool {
            let register = options.new_temporary();
            lines.push(format!(
                "  %{register} = icmp ne {} {value}, 0",
                Self::gen_type(source.clone())
            ));
            let result = options.new_temporary();
            lines.push(format!("  %{result} = zext i1 %{register} to i8"));
            return format!("%{result}");
        }
        let operation = match (source.integer_info(), target.integer_info()) {
            (Some((sb, signed)), Some((tb, _))) if sb < tb => {
                if signed {
                    "sext"
                } else {
                    "zext"
                }
            }
            (Some((sb, _)), Some((tb, _))) if sb > tb => "trunc",
            _ => return value,
        };
        let register = options.new_temporary();
        lines.push(format!(
            "  %{} = {} {} {} to {}",
            register,
            operation,
            Self::gen_type(source.clone()),
            value,
            Self::gen_type(target.clone())
        ));
        format!("%{}", register)
    }

    fn gen_predicate(&self, node: &Node, options: &mut Options, lines: &mut Vec<String>) -> String {
        lines.extend(self.gen_node(node, options));
        let value = options.register_queue.pop().unwrap();
        let ty = node.resolve_type().unwrap();
        let pointer = ty.is_pointer_value();
        let register = options.new_temporary();
        lines.push(format!(
            "  %{} = {} {} {}, {}",
            register,
            if ty.float_bits().is_some() {
                "fcmp une"
            } else {
                "icmp ne"
            },
            if pointer {
                "ptr".to_string()
            } else {
                Self::gen_type(ty)
            },
            value,
            if pointer {
                "null"
            } else if node.resolve_type().unwrap().float_bits().is_some() {
                "0.0"
            } else {
                "0"
            }
        ));
        format!("%{}", register)
    }

    fn gen_addr(&self, node: &Node, options: &mut Options) -> Vec<String> {
        match node.nt {
            NodeType::CallFunc => {
                let lines = self.gen_node(node, options);
                options.register_queue.pop();
                options
                    .register_queue
                    .push(format!("%{}", options.register_map[&node.offset.unwrap()]));
                lines
            }
            NodeType::Conditional => {
                let lines = self.gen_node(node, options);
                options.register_queue.pop();
                let slot = options.expression_slots[&node.token.as_ref().unwrap().pos];
                options.register_queue.push(format!("%{}", slot));
                lines
            }

            NodeType::Sequence => {
                let mut lines = vec![];
                let depth = options.register_queue.len();
                for child in &node.children[..node.children.len() - 1] {
                    lines.extend(self.gen_node(child, options));
                    options.register_queue.truncate(depth);
                }
                lines.extend(self.gen_addr(node.children.last().unwrap(), options));
                lines
            }

            NodeType::LocalVar => {
                let register = options.register_from_offset(node.offset.unwrap());
                options.register_queue.push(format!("%{}", register));
                vec![]
            }
            NodeType::GlobalVar => {
                let symbol = if node.dest.is_empty() {
                    self.symbol_name(&node.global_name)
                } else {
                    node.dest.clone()
                };
                // LLVM resolves TLS globals in the current thread, including on LLVM 15.
                options.register_queue.push(symbol);
                vec![]
            }
            NodeType::Deref => self.gen_node(node.lhs.as_ref().unwrap(), options),
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "expression is not addressable",
                );
                unreachable!()
            }
        }
    }

    fn atomic_update(
        node: &Node,
        operation: AtomicRmwOperation,
        return_new: bool,
        values: &[String],
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        let object = node.args[0].dest_type().unwrap();
        let storage = Type::integer((object.size_of() * 8) as u8, false);
        let scalar = Self::gen_type(storage.clone());
        let value = Self::convert_integer(
            values[1].clone(),
            &node.args[1].resolve_type().unwrap(),
            &storage,
            options,
            lines,
        );
        let mnemonic = match operation {
            AtomicRmwOperation::Add => "add",
            AtomicRmwOperation::Sub => "sub",
            AtomicRmwOperation::And => "and",
            AtomicRmwOperation::Or => "or",
            AtomicRmwOperation::Xor => "xor",
            AtomicRmwOperation::Nand => "nand",
            AtomicRmwOperation::Exchange => "xchg",
        };
        let volatile = if node.volatile_access {
            "volatile "
        } else {
            ""
        };
        let old = options.new_temporary();
        lines.push(format!(
            "  %{old} = atomicrmw {volatile}{mnemonic} ptr {}, {scalar} {value} seq_cst, align {}",
            values[0],
            object.align_of()
        ));
        let result = if return_new {
            let result = options.new_temporary();
            if operation == AtomicRmwOperation::Nand {
                lines.push(format!("  %{result} = and {scalar} %{old}, {value}"));
                let inverted = options.new_temporary();
                lines.push(format!("  %{inverted} = xor {scalar} %{result}, -1"));
                format!("%{inverted}")
            } else {
                lines.push(format!("  %{result} = {mnemonic} {scalar} %{old}, {value}"));
                format!("%{result}")
            }
        } else {
            format!("%{old}")
        };
        Self::convert_integer(result, &storage, &object, options, lines)
    }

    fn atomic_compare_exchange(
        node: &Node,
        expected_volatile: bool,
        values: &[String],
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        let object = node.args[0].dest_type().unwrap();
        let storage = Type::integer((object.size_of() * 8) as u8, false);
        let scalar = Self::gen_type(storage.clone());
        let desired = Self::convert_integer(
            values[2].clone(),
            &node.args[2].resolve_type().unwrap(),
            &storage,
            options,
            lines,
        );
        let expected = options.new_temporary();
        let expected_volatile = if expected_volatile { "volatile " } else { "" };
        lines.push(format!(
            "  %{expected} = load {expected_volatile}{scalar}, ptr {}, align {}",
            values[1],
            object.align_of()
        ));
        let exchanged = options.new_temporary();
        let volatile = if node.volatile_access {
            "volatile "
        } else {
            ""
        };
        lines.push(format!("  %{exchanged} = cmpxchg {volatile}ptr {}, {scalar} %{expected}, {scalar} {desired} seq_cst seq_cst, align {}", values[0], object.align_of()));
        let old = options.new_temporary();
        lines.push(format!(
            "  %{old} = extractvalue {{ {scalar}, i1 }} %{exchanged}, 0"
        ));
        let success = options.new_temporary();
        lines.push(format!(
            "  %{success} = extractvalue {{ {scalar}, i1 }} %{exchanged}, 1"
        ));
        let failure = options.new_label();
        let done = options.new_label();
        lines.push(format!(
            "  br i1 %{success}, label %{done}, label %{failure}"
        ));
        lines.push(format!("{failure}:"));
        lines.push(format!(
            "  store {expected_volatile}{scalar} %{old}, ptr {}, align {}",
            values[1],
            object.align_of()
        ));
        lines.push(format!("  br label %{done}"));
        lines.push(format!("{done}:"));
        let result = options.new_temporary();
        lines.push(format!("  %{result} = zext i1 %{success} to i8"));
        format!("%{result}")
    }

    fn gen_atomic(
        &self,
        node: &Node,
        operation: AtomicOperation,
        options: &mut Options,
    ) -> Vec<String> {
        let mut lines = vec![];
        let mut values = vec![];
        for argument in &node.args {
            lines.extend(self.gen_node(argument, options));
            values.push(options.register_queue.pop().unwrap());
        }
        if let AtomicOperation::CompareExchange(expected_volatile) = operation {
            let result = Self::atomic_compare_exchange(
                node,
                expected_volatile,
                &values,
                options,
                &mut lines,
            );
            options.register_queue.push(result);
            return lines;
        }
        let ty = node.args[0].dest_type().unwrap();
        if let AtomicOperation::Update(op, return_new) = operation {
            let value = Self::atomic_update(node, op, return_new, &values, options, &mut lines);
            options.register_queue.push(value);
            return lines;
        }
        let scalar = Self::gen_type(ty.clone());
        let volatile = if node.volatile_access {
            "volatile "
        } else {
            ""
        };
        if operation == AtomicOperation::Load {
            lines.push(format!(
                "  %{} = load atomic {volatile}{scalar}, ptr {} seq_cst, align {}",
                options.new_register(),
                values[0],
                ty.align_of()
            ));
        } else {
            lines.push(format!(
                "  store atomic {volatile}{scalar} {}, ptr {} seq_cst, align {}",
                values[1],
                values[0],
                ty.align_of()
            ));
            options.register_queue.push("0".into());
        }
        lines
    }

    fn gen_prefetch(
        &self,
        node: &Node,
        write: bool,
        locality: u8,
        options: &mut Options,
    ) -> Vec<String> {
        let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
        let pointer = options.register_queue.pop().unwrap();
        lines.push(format!(
            "  call void @llvm.prefetch.p0(ptr {pointer}, i32 {}, i32 {locality}, i32 1)",
            u8::from(write)
        ));
        options.register_queue.push("0".into());
        lines
    }

    fn gen_inline_assembly(
        &self,
        node: &Node,
        volatile: bool,
        memory: bool,
        options: &mut Options,
    ) -> Vec<String> {
        let mut lines = vec![];
        let mut matching = vec![];
        for (index, output) in node.children.iter().enumerate() {
            if node.value.unwrap() & (1 << index) != 0 {
                lines.extend(self.gen_node(output, options));
                let value = options.register_queue.pop().unwrap();
                matching.push((
                    index,
                    format!("{} {value}", Self::gen_type(output.resolve_type().unwrap())),
                ));
            }
        }
        let mut arguments = vec![];
        for input in &node.args {
            lines.extend(self.gen_node(input, options));
            let value = options.register_queue.pop().unwrap();
            arguments.push(format!(
                "{} {value}",
                Self::gen_type(input.resolve_type().unwrap())
            ));
        }
        let template = render_inline_assembly(&node.dest.replace('$', "$$"), |index, modifier| {
            let arch = if cfg!(target_arch = "aarch64") {
                crate::generator::Arch::Aarch64
            } else {
                crate::generator::Arch::X86_64
            };
            let operand = node.children.iter().chain(&node.args).nth(index)?;
            crate::generator::asm_register_width(
                arch,
                modifier,
                operand.resolve_type()?.size_of(),
            )?;
            Some(if let Some(modifier) = modifier {
                format!("${{{index}:{modifier}}}")
            } else {
                format!("${index}")
            })
        })
        .unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "unsupported asm register modifier for target",
            );
            unreachable!()
        });
        let template = template
            .bytes()
            .map(|byte| format!("\\{byte:02X}"))
            .collect::<String>();
        let mut constraints = vec!["=r".to_string(); node.children.len()];
        constraints.extend(vec!["r".to_string(); node.args.len()]);
        for (index, value) in matching {
            constraints.push(index.to_string());
            arguments.push(value);
        }
        constraints.extend(
            if cfg!(target_arch = "x86_64") {
                vec!["~{dirflag}", "~{fpsr}", "~{flags}"]
            } else {
                vec!["~{cc}"]
            }
            .into_iter()
            .map(String::from),
        );
        if memory {
            constraints.push("~{memory}".to_string());
        }
        let types = node
            .children
            .iter()
            .map(|output| Self::gen_type(output.resolve_type().unwrap()))
            .collect::<Vec<_>>();
        let result_type = if types.is_empty() {
            "void".to_string()
        } else if types.len() == 1 {
            types[0].clone()
        } else {
            format!("{{ {} }}", types.join(", "))
        };
        let result = if types.is_empty() {
            String::new()
        } else {
            let register = format!("%{}", options.new_temporary());
            register
        };
        let assignment = if types.is_empty() {
            String::new()
        } else {
            format!("{result} = ")
        };
        lines.push(format!(
            "  {assignment}call {result_type} asm {}\"{template}\", \"{}\"({})",
            if volatile { "sideeffect " } else { "" },
            constraints.join(","),
            arguments.join(", ")
        ));
        for (index, output) in node.children.iter().enumerate() {
            let value = if types.len() == 1 {
                result.clone()
            } else {
                let value = format!("%{}", options.new_temporary());
                lines.push(format!(
                    "  {value} = extractvalue {result_type} {result}, {index}"
                ));
                value
            };
            lines.extend(self.gen_addr(output, options));
            let address = options.register_queue.pop().unwrap();
            lines.push(format!(
                "  store {}{} {value}, ptr {address}, align {}",
                if output.volatile_access {
                    "volatile "
                } else {
                    ""
                },
                types[index],
                Self::access_alignment(output)
            ));
        }
        options.register_queue.push("0".to_string());
        lines
    }

    fn gen_node(&self, node: &Node, options: &mut Options) -> Vec<String> {
        if options.terminated
            && !matches!(node.nt, NodeType::Block | NodeType::Case | NodeType::Label)
        {
            if !node.contains_case() {
                return vec![];
            }
            // A switch can enter a label inside an otherwise unreachable statement.
            let label = options.new_label();
            options.terminated = false;
            let mut lines = vec![format!("{}:", label)];
            lines.extend(self.gen_node(node, options));
            return lines;
        }
        match node.nt {
            NodeType::Prefetch(write, locality) => {
                return self.gen_prefetch(node, write, locality, options);
            }
            NodeType::InlineAsm(volatile, memory) => {
                return self.gen_inline_assembly(node, volatile, memory, options);
            }
            NodeType::Atomic(operation) => {
                return self.gen_atomic(node, operation, options);
            }
            NodeType::VaStart | NodeType::VaEnd => {
                let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
                let pointer = options.register_queue.pop().unwrap();
                let intrinsic = if node.nt == NodeType::VaStart {
                    "va_start"
                } else {
                    "va_end"
                };
                lines.push(format!("  call void @llvm.{intrinsic}(ptr {pointer})"));
                options.register_queue.push("0".into());
                return lines;
            }

            NodeType::FloatAbs => {
                let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                let ty = node.resolve_type().unwrap();
                let bits = ty.float_bits().unwrap();
                let float = Self::gen_type(ty);
                let raw = options.new_temporary();
                let masked = options.new_temporary();
                let result = options.new_register();
                let mask = (1u64 << (bits - 1)) - 1;
                lines.extend([
                    format!("  %{raw} = bitcast {float} {value} to i{bits}"),
                    format!("  %{masked} = and i{bits} %{raw}, {mask}"),
                    format!("  %{result} = bitcast i{bits} %{masked} to {float}"),
                ]);
                return lines;
            }
            NodeType::Cast => {
                let operand = node.lhs.as_ref().unwrap();
                let mut lines = self.gen_node(operand, options);
                let value = options.register_queue.pop().unwrap();
                let target = node.cty.as_ref().unwrap();
                if *target == Type::Void {
                    options.register_queue.push("0".to_string());
                } else {
                    let value = Self::convert_integer(
                        value,
                        &operand.resolve_type().unwrap(),
                        target,
                        options,
                        &mut lines,
                    );
                    options.register_queue.push(value);
                }
                return lines;
            }
            NodeType::Sequence => {
                let mut lines = vec![];
                let depth = options.register_queue.len();
                for (index, child) in node.children.iter().enumerate() {
                    lines.extend(self.gen_node(child, options));
                    if index + 1 < node.children.len() {
                        options.register_queue.truncate(depth);
                    }
                }
                return lines;
            }
            NodeType::DefVar => {
                return node
                    .children
                    .iter()
                    .flat_map(|node| self.gen_node(node, options))
                    .collect();
            }
            NodeType::CallFunc => {
                if matches!(node.resolve_type(), Some(Type::Struct(_)))
                    || node
                        .args
                        .iter()
                        .any(|arg| matches!(arg.resolve_type(), Some(Type::Struct(_))))
                {
                    return self.aggregate_call(node, options);
                }
                let parameters = node.parameter_types.clone();
                let result = node.cty.clone().unwrap();
                let variadic = node.variadic;
                let mut lines = vec![];
                let callee = if let Some(callee) = &node.lhs {
                    lines.extend(self.gen_node(callee, options));
                    options.register_queue.pop().unwrap()
                } else {
                    self.symbol_name(&node.global_name)
                };
                let mut arguments = vec![];
                for (i, arg) in node.args.iter().enumerate() {
                    lines.extend(self.gen_node(arg, options));
                    let value = options.register_queue.pop().unwrap();
                    let source = arg.resolve_type().unwrap();
                    let source = match source {
                        Type::Arr(child, _) | Type::Vla(child, _) => Type::Ptr(child),
                        Type::Func(..) | Type::VariadicFunc(..) => Type::Ptr(Box::new(source)),
                        _ => source,
                    };
                    let ty = parameters
                        .get(node.args.len() - i - 1)
                        .cloned()
                        .unwrap_or_else(|| {
                            if source.integer_info().is_some() {
                                source.promoted()
                            } else {
                                source.clone()
                            }
                        });
                    let value = Self::convert_integer(value, &source, &ty, options, &mut lines);
                    arguments.push(format!("{} {}", Self::abi_parameter_type(ty), value));
                }
                arguments.reverse();
                let signature = if variadic {
                    format!(
                        " ({}, ...)",
                        parameters
                            .iter()
                            .map(|ty| Self::gen_type(ty.clone()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    String::new()
                };
                if result == Type::Void {
                    lines.push(format!(
                        "  call void{} {}({})",
                        signature,
                        callee,
                        arguments.join(", ")
                    ));
                    options.register_queue.push("0".to_string());
                    return lines;
                }
                lines.push(format!(
                    "  %{} = call {}{} {}({})",
                    options.new_register(),
                    Self::abi_result_type(result),
                    signature,
                    callee,
                    arguments.join(", ")
                ));
                return lines;
            }
            NodeType::Block => {
                return node
                    .children
                    .iter()
                    .flat_map(|node| self.gen_node(node, options))
                    .collect();
            }
            NodeType::Return => {
                if matches!(options.return_ty, Type::Struct(_)) {
                    return self.aggregate_return(node, options);
                }
                if options.return_ty == Type::Void {
                    let mut lines = if let Some(operand) = &node.lhs {
                        self.gen_node(operand, options)
                    } else {
                        vec![]
                    };
                    if node.lhs.is_some() {
                        options.register_queue.pop();
                    }
                    lines.push("  ret void".to_string());
                    options.terminated = true;
                    return lines;
                }
                if node.lhs.is_none() {
                    options.terminated = true;
                    return vec!["  ret void".to_string()];
                }
                let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                let source_ty = node.resolve_type().unwrap();
                let target_ty = options.return_ty.clone();
                let value =
                    Self::convert_integer(value, &source_ty, &target_ty, options, &mut lines);
                lines.push(format!(
                    "  ret {} {}",
                    Self::gen_type_with_opaque_ptr(target_ty),
                    value
                ));
                options.terminated = true;
                return lines;
            }
            NodeType::Num => {
                let ty = node.resolve_type().unwrap();
                options.register_queue.push(if ty.float_bits().is_some() {
                    Self::float_literal(&ty, node.value.unwrap())
                } else {
                    node.value.unwrap().to_string()
                });
                return vec![];
            }
            NodeType::Assign => {
                let mut lines = self.gen_node(node.rhs.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                lines.extend(self.gen_addr(node.lhs.as_ref().unwrap(), options));
                let address = options.register_queue.pop().unwrap();
                let ty = node.lhs.as_ref().unwrap().resolve_type().unwrap();
                let source_ty = node.rhs.as_ref().unwrap().resolve_type().unwrap();
                let value = Self::convert_integer(value, &source_ty, &ty, options, &mut lines);
                lines.push(format!(
                    "  store {}{} {}, ptr {}, align {}",
                    if node.lhs.as_ref().is_some_and(|lhs| lhs.volatile_access) {
                        "volatile "
                    } else {
                        ""
                    },
                    Self::gen_type_with_opaque_ptr(ty.clone()),
                    value,
                    address,
                    Self::access_alignment(node.lhs.as_ref().unwrap())
                ));
                options.register_queue.push(value);
                return lines;
            }
            NodeType::Addr => {
                return self.gen_addr(node.lhs.as_ref().unwrap(), options);
            }
            NodeType::Deref | NodeType::LocalVar | NodeType::GlobalVar => {
                let mut lines = self.gen_addr(node, options);
                let ty = node.resolve_type().unwrap();
                if !matches!(
                    ty,
                    Type::Arr(..) | Type::Vla(..) | Type::Func(..) | Type::VariadicFunc(..)
                ) {
                    let address = options.register_queue.pop().unwrap();
                    lines.push(format!(
                        "  %{} = load {}{}, ptr {}, align {}",
                        options.new_register(),
                        if node.volatile_access {
                            "volatile "
                        } else {
                            ""
                        },
                        Self::gen_type_with_opaque_ptr(ty.clone()),
                        address,
                        Self::access_alignment(node)
                    ));
                }
                return lines;
            }
            NodeType::Sub
                if node.lhs.as_ref().unwrap().dest_type().is_some()
                    && node.rhs.as_ref().unwrap().dest_type().is_some() =>
            {
                let mut lines = self.gen_node(node.rhs.as_ref().unwrap(), options);
                let rhs = options.register_queue.pop().unwrap();
                lines.extend(self.gen_node(node.lhs.as_ref().unwrap(), options));
                let lhs = options.register_queue.pop().unwrap();
                let left = options.new_temporary();
                lines.push(format!("  %{} = ptrtoint ptr {} to i64", left, lhs));
                let right = options.new_temporary();
                lines.push(format!("  %{} = ptrtoint ptr {} to i64", right, rhs));
                let difference = options.new_temporary();
                lines.push(format!("  %{} = sub i64 %{}, %{}", difference, left, right));
                let count = options.new_temporary();
                lines.push(format!(
                    "  %{} = sdiv i64 %{}, {}",
                    count,
                    difference,
                    node.lhs.as_ref().unwrap().dest_type().unwrap().size_of()
                ));
                lines.push(format!(
                    "  %{} = trunc i64 %{} to i32",
                    options.new_register(),
                    count
                ));
                return lines;
            }
            NodeType::Add | NodeType::Sub if node.lhs.as_ref().unwrap().dest_type().is_some() => {
                let base = node.lhs.as_ref().unwrap();
                let index = node.rhs.as_ref().unwrap();
                let mut lines = self.gen_node(index, options);
                let mut value = options.register_queue.pop().unwrap();
                // GEP sign-extends its indices, so preserve C signedness explicitly.
                value = Self::convert_integer(
                    value,
                    &index.resolve_type().unwrap(),
                    &Type::integer(64, true),
                    options,
                    &mut lines,
                );
                let index_ty = "i64";
                if matches!(node.nt, NodeType::Sub) {
                    let register = options.new_temporary();
                    lines.push(format!("  %{} = sub {} 0, {}", register, index_ty, value));
                    value = format!("%{}", register);
                }
                lines.extend(self.gen_node(base, options));
                let address = options.register_queue.pop().unwrap();
                lines.push(format!(
                    "  %{} = getelementptr inbounds {}, ptr {}, {} {}",
                    options.new_register(),
                    Self::gen_type_with_opaque_ptr(base.dest_type().unwrap()),
                    address,
                    index_ty,
                    value
                ));
                return lines;
            }
            NodeType::BitNot => {
                let operand = node.lhs.as_ref().unwrap();
                let mut lines = self.gen_node(operand, options);
                let value = options.register_queue.pop().unwrap();
                let value = Self::convert_integer(
                    value,
                    &operand.resolve_type().unwrap(),
                    &node.resolve_type().unwrap(),
                    options,
                    &mut lines,
                );
                lines.push(format!(
                    "  %{} = xor {} {}, -1",
                    options.new_register(),
                    Self::gen_type(node.resolve_type().unwrap()),
                    value
                ));
                return lines;
            }
            NodeType::Switch => {
                let mut lines = self.gen_node(node.cond.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                let selector_ty = node
                    .cond
                    .as_ref()
                    .unwrap()
                    .resolve_type()
                    .unwrap()
                    .promoted();
                let value = Self::convert_integer(
                    value,
                    &node.cond.as_ref().unwrap().resolve_type().unwrap(),
                    &selector_ty,
                    options,
                    &mut lines,
                );
                let end = options.new_label();
                let default = node
                    .children
                    .iter()
                    .find(|case| case.value.is_none())
                    .map(|case| format!("case.{}", case.token.as_ref().unwrap().pos))
                    .unwrap_or(end.clone());
                lines.push(format!(
                    "  switch {} {}, label %{} [",
                    Self::gen_type(selector_ty.clone()),
                    value,
                    default
                ));
                for case in &node.children {
                    if let Some(value) = case.value {
                        lines.push(format!(
                            "    {} {}, label %case.{}",
                            Self::gen_type(selector_ty.clone()),
                            value as i64,
                            case.token.as_ref().unwrap().pos
                        ));
                    }
                }
                lines.push("  ]".to_string());
                options.terminated = true;
                options.break_labels.push(end.clone());
                lines.extend(self.gen_node(node.then.as_ref().unwrap(), options));
                options.break_labels.pop();
                if !options.terminated {
                    lines.push(format!("  br label %{}", end));
                }
                lines.push(format!("{}:", end));
                options.terminated = false;
                return lines;
            }
            NodeType::Goto => {
                options.terminated = true;
                return vec![format!("  br label %goto.{}", node.value.unwrap())];
            }
            NodeType::Label => {
                let label = format!("goto.{}", node.value.unwrap());
                let mut lines = vec![];
                if !options.terminated {
                    lines.push(format!("  br label %{label}"));
                }
                lines.push(format!("{label}:"));
                options.terminated = false;
                lines.extend(self.gen_node(node.lhs.as_ref().unwrap(), options));
                return lines;
            }
            NodeType::Case => {
                let label = format!("case.{}", node.token.as_ref().unwrap().pos);
                let mut lines = vec![];
                if !options.terminated {
                    lines.push(format!("  br label %{}", label));
                }
                lines.push(format!("{}:", label));
                options.terminated = false;
                lines.extend(self.gen_node(node.lhs.as_ref().unwrap(), options));
                return lines;
            }
            NodeType::DoWhile => {
                let body_label = options.new_label();
                let condition_label = format!("continue.{}", node.token.as_ref().unwrap().pos);
                let end_label = options.new_label();
                let mut lines = vec![
                    format!("  br label %{}", body_label),
                    format!("{}:", body_label),
                ];
                options.break_labels.push(end_label.clone());
                lines.extend(self.gen_node(node.then.as_ref().unwrap(), options));
                options.break_labels.pop();
                if !options.terminated {
                    lines.push(format!("  br label %{}", condition_label));
                }
                options.terminated = false;
                lines.push(format!("{}:", condition_label));
                let predicate =
                    self.gen_predicate(node.cond.as_ref().unwrap(), options, &mut lines);
                lines.push(format!(
                    "  br i1 {}, label %{}, label %{}",
                    predicate, body_label, end_label
                ));
                lines.push(format!("{}:", end_label));
                return lines;
            }
            NodeType::While | NodeType::For => {
                let mut lines = vec![];
                if let Some(initializer) = &node.ini {
                    lines.extend(self.gen_node(initializer, options));
                }
                let condition_label = options.new_label();
                let body_label = options.new_label();
                let update_label = format!("continue.{}", node.token.as_ref().unwrap().pos);
                let end_label = options.new_label();
                lines.push(format!("  br label %{}", condition_label));
                lines.push(format!("{}:", condition_label));
                let predicate = if let Some(condition) = &node.cond {
                    self.gen_predicate(condition, options, &mut lines)
                } else {
                    "true".to_string()
                };
                lines.push(format!(
                    "  br i1 {}, label %{}, label %{}",
                    predicate, body_label, end_label
                ));
                lines.push(format!("{}:", body_label));
                options.break_labels.push(end_label.clone());
                lines.extend(self.gen_node(node.then.as_ref().unwrap(), options));
                options.break_labels.pop();
                if !options.terminated {
                    lines.push(format!("  br label %{}", update_label));
                }
                options.terminated = false;
                lines.push(format!("{}:", update_label));
                if let Some(update) = &node.upd {
                    lines.extend(self.gen_node(update, options));
                }
                lines.push(format!("  br label %{}", condition_label));
                lines.push(format!("{}:", end_label));
                return lines;
            }
            NodeType::Continue => {
                options.terminated = true;
                return vec![format!("  br label %continue.{}", node.value.unwrap())];
            }
            NodeType::Break => {
                let Some(label) = options.break_labels.last() else {
                    self.error_logger.print_error_position(
                        node.token.as_ref().unwrap().pos,
                        "unexpected break found",
                    );
                    unreachable!()
                };
                options.terminated = true;
                return vec![format!("  br label %{}", label)];
            }
            NodeType::LogicalAnd | NodeType::LogicalOr => {
                let mut lines = vec![];
                let predicate = self.gen_predicate(node.lhs.as_ref().unwrap(), options, &mut lines);
                let slot = options.expression_slots[&node.token.as_ref().unwrap().pos];
                let rhs_label = options.new_label();
                let short_label = options.new_label();
                let end_label = options.new_label();
                let is_and = matches!(node.nt, NodeType::LogicalAnd);
                lines.push(format!(
                    "  br i1 {}, label %{}, label %{}",
                    predicate,
                    if is_and { &rhs_label } else { &short_label },
                    if is_and { &short_label } else { &rhs_label }
                ));
                lines.push(format!("{}:", short_label));
                lines.push(format!(
                    "  store i32 {}, ptr %{}, align 4",
                    if is_and { 0 } else { 1 },
                    slot
                ));
                lines.push(format!("  br label %{}", end_label));
                lines.push(format!("{}:", rhs_label));
                let predicate = self.gen_predicate(node.rhs.as_ref().unwrap(), options, &mut lines);
                let value = options.new_temporary();
                lines.push(format!("  %{} = zext i1 {} to i32", value, predicate));
                lines.push(format!("  store i32 %{}, ptr %{}, align 4", value, slot));
                lines.push(format!("  br label %{}", end_label));
                lines.push(format!("{}:", end_label));
                lines.push(format!(
                    "  %{} = load i32, ptr %{}, align 4",
                    options.new_register(),
                    slot
                ));
                return lines;
            }
            NodeType::Conditional => {
                if node.resolve_type() == Some(Type::Void) {
                    let mut lines = vec![];
                    let predicate =
                        self.gen_predicate(node.cond.as_ref().unwrap(), options, &mut lines);
                    let then_label = options.new_label();
                    let else_label = options.new_label();
                    let end_label = options.new_label();
                    lines.push(format!(
                        "  br i1 {}, label %{}, label %{}",
                        predicate, then_label, else_label
                    ));
                    for (label, branch) in [
                        (&then_label, node.then.as_ref().unwrap()),
                        (&else_label, node.els.as_ref().unwrap()),
                    ] {
                        lines.push(format!("{}:", label));
                        lines.extend(self.gen_node(branch, options));
                        options.register_queue.pop();
                        lines.push(format!("  br label %{}", end_label));
                    }
                    lines.push(format!("{}:", end_label));
                    options.register_queue.push("0".to_string());
                    return lines;
                }

                let mut lines = vec![];
                let ty = node.resolve_type().unwrap();
                let slot = options.expression_slots[&node.token.as_ref().unwrap().pos];
                let predicate =
                    self.gen_predicate(node.cond.as_ref().unwrap(), options, &mut lines);
                let then_label = options.new_label();
                let else_label = options.new_label();
                let end_label = options.new_label();
                lines.push(format!(
                    "  br i1 {}, label %{}, label %{}",
                    predicate, then_label, else_label
                ));
                for (label, branch) in [
                    (&then_label, node.then.as_ref().unwrap()),
                    (&else_label, node.els.as_ref().unwrap()),
                ] {
                    lines.push(format!("{}:", label));
                    lines.extend(self.gen_node(branch, options));
                    let value = options.register_queue.pop().unwrap();
                    let value = Self::convert_integer(
                        value,
                        &branch.resolve_type().unwrap(),
                        &ty,
                        options,
                        &mut lines,
                    );
                    lines.push(format!(
                        "  store {} {}, ptr %{}, align {}",
                        Self::gen_type(ty.clone()),
                        value,
                        slot,
                        Self::align(&ty)
                    ));
                    lines.push(format!("  br label %{}", end_label));
                }
                lines.push(format!("{}:", end_label));
                lines.push(format!(
                    "  %{} = load {}, ptr %{}, align {}",
                    options.new_register(),
                    Self::gen_type(ty.clone()),
                    slot,
                    Self::align(&ty)
                ));
                return lines;
            }
            NodeType::SuffixIncr | NodeType::SuffixDecr => {
                let operand = node.lhs.as_ref().unwrap();
                let mut lines = self.gen_addr(operand, options);
                let address = options.register_queue.pop().unwrap();
                let ty = operand.resolve_type().unwrap();
                let old = options.new_temporary();
                lines.push(format!(
                    "  %{} = load {}{}, ptr {}, align {}",
                    old,
                    if node.volatile_access {
                        "volatile "
                    } else {
                        ""
                    },
                    Self::gen_type(ty.clone()),
                    address,
                    Self::access_alignment(operand)
                ));
                let new = options.new_temporary();
                if let Type::Ptr(child) = &ty {
                    lines.push(format!(
                        "  %{} = getelementptr inbounds {}, ptr %{}, i32 {}",
                        new,
                        Self::gen_type(*child.clone()),
                        old,
                        if matches!(node.nt, NodeType::SuffixIncr) {
                            1
                        } else {
                            -1
                        }
                    ));
                } else {
                    lines.push(format!(
                        "  %{} = {} {} %{}, {}",
                        new,
                        if matches!(node.nt, NodeType::SuffixIncr) {
                            if ty.float_bits().is_some() {
                                "fadd"
                            } else {
                                "add"
                            }
                        } else if ty.float_bits().is_some() {
                            "fsub"
                        } else {
                            "sub"
                        },
                        Self::gen_type(ty.clone()),
                        old,
                        if ty.float_bits().is_some() {
                            "1.0"
                        } else {
                            "1"
                        }
                    ));
                }
                let new = if ty == Type::Bool {
                    let predicate = options.new_temporary();
                    lines.push(format!("  %{predicate} = icmp ne i8 %{new}, 0"));
                    let result = options.new_temporary();
                    lines.push(format!("  %{result} = zext i1 %{predicate} to i8"));
                    result
                } else {
                    new
                };
                lines.push(format!(
                    "  store {}{} %{}, ptr {}, align {}",
                    if node.volatile_access {
                        "volatile "
                    } else {
                        ""
                    },
                    Self::gen_type(ty.clone()),
                    new,
                    address,
                    Self::access_alignment(operand)
                ));
                options.register_queue.push(format!("%{}", old));
                return lines;
            }
            NodeType::If => {
                let mut lines = vec![];
                let predicate =
                    self.gen_predicate(node.cond.as_ref().unwrap(), options, &mut lines);
                let then_label = options.new_label();
                let else_label = options.new_label();
                let end_label = options.new_label();
                lines.push(format!(
                    "  br i1 {}, label %{}, label %{}",
                    predicate, then_label, else_label,
                ));

                lines.push(format!("{}:", then_label));
                options.terminated = false;
                lines.extend(self.gen_node(node.then.as_ref().unwrap(), options));
                let then_terminated = options.terminated;
                if !then_terminated {
                    lines.push(format!("  br label %{}", end_label));
                }

                lines.push(format!("{}:", else_label));
                options.terminated = false;
                if let Some(els) = &node.els {
                    lines.extend(self.gen_node(els, options));
                }
                let else_terminated = options.terminated;
                if !else_terminated {
                    lines.push(format!("  br label %{}", end_label));
                }

                options.terminated = then_terminated && else_terminated;
                if !options.terminated {
                    lines.push(format!("{}:", end_label));
                }
                return lines;
            }
            _ => {}
        }

        let left_first = node.nt.left_operand_first();
        let (first, second) = if left_first {
            (node.lhs.as_ref().unwrap(), node.rhs.as_ref().unwrap())
        } else {
            (node.rhs.as_ref().unwrap(), node.lhs.as_ref().unwrap())
        };
        let lhs = self.gen_node(first, options);
        let rhs = self.gen_node(second, options);

        let operation = match node.nt {
            NodeType::Add => "add nsw",
            NodeType::Sub => "sub nsw",
            NodeType::Mul => "mul nsw",
            NodeType::Div => "sdiv",
            NodeType::Mod => "srem",
            NodeType::Eq => "icmp eq",
            NodeType::Ne => "icmp ne",
            NodeType::Lt => "icmp slt",
            NodeType::Le => "icmp sle",
            NodeType::BitAnd => "and",
            NodeType::BitXor => "xor",
            NodeType::BitOr => "or",
            NodeType::BitLeft => "shl",
            NodeType::BitRight => "ashr",
            _ => "unknown",
        };
        let mut rhs_register = options.register_queue.pop().unwrap();
        let mut lhs_register = options.register_queue.pop().unwrap();
        if left_first {
            std::mem::swap(&mut lhs_register, &mut rhs_register);
        }
        let left_ty = node.lhs.as_ref().unwrap().resolve_type().unwrap();
        let right_ty = node.rhs.as_ref().unwrap().resolve_type().unwrap();
        let operand_ty = if left_ty.is_arithmetic() && right_ty.is_arithmetic() {
            if matches!(node.nt, NodeType::BitLeft | NodeType::BitRight) {
                left_ty.promoted()
            } else {
                Type::arithmetic_type(&left_ty, &right_ty)
            }
        } else if left_ty.is_pointer_value() {
            left_ty.decayed()
        } else if right_ty.is_pointer_value() && matches!(node.nt, NodeType::Eq | NodeType::Ne) {
            right_ty.decayed()
        } else {
            left_ty.clone()
        };
        let operation = if operand_ty.float_bits().is_some() {
            match node.nt {
                NodeType::Add => "fadd",
                NodeType::Sub => "fsub",
                NodeType::Mul => "fmul",
                NodeType::Div => "fdiv",
                NodeType::Eq => "fcmp oeq",
                NodeType::Ne => "fcmp une",
                NodeType::Lt => "fcmp olt",
                NodeType::Le => "fcmp ole",
                _ => unreachable!(),
            }
        } else if operand_ty.integer_info().is_some_and(|(_, signed)| !signed) {
            match node.nt {
                NodeType::Div => "udiv",
                NodeType::Mod => "urem",
                NodeType::Lt => "icmp ult",
                NodeType::Le => "icmp ule",
                NodeType::BitRight => "lshr",
                NodeType::Add => "add",
                NodeType::Sub => "sub",
                NodeType::Mul => "mul",
                _ => operation,
            }
        } else {
            operation
        };
        let mut conversions = vec![];
        rhs_register = Self::convert_integer(
            rhs_register,
            &left_ty,
            &operand_ty,
            options,
            &mut conversions,
        );
        lhs_register = Self::convert_integer(
            lhs_register,
            &right_ty,
            &operand_ty,
            options,
            &mut conversions,
        );
        let comparison = matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        );
        let mut lines = [
            lhs,
            rhs,
            conversions,
            vec![match node.nt {
                NodeType::Add
                | NodeType::Sub
                | NodeType::Mul
                | NodeType::Div
                | NodeType::Mod
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le
                | NodeType::BitAnd
                | NodeType::BitXor
                | NodeType::BitOr
                | NodeType::BitLeft
                | NodeType::BitRight => {
                    format!(
                        "  %{} = {} {} {}, {}",
                        options.new_register(),
                        operation,
                        Self::gen_type(operand_ty),
                        rhs_register,
                        lhs_register,
                    )
                }

                _ => {
                    self.error_logger.print_error_position(
                        node.token.as_ref().unwrap().pos,
                        &format!("unexpected node: {:?}", node.nt),
                    );
                    unreachable!();
                }
            }],
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if comparison {
            let value = options.register_queue.pop().unwrap();
            let register = options.new_register();
            lines.push(format!("  %{} = zext i1 {} to i32", register, value));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_array_initializers_fill_missing_elements() {
        let logger = error::ErrorPrinter::new("");
        let generator = IrGenerator::new(&logger);
        let ty = Type::Arr(Box::new(Type::I32), Some(3));
        assert_eq!(
            generator.gen_initializer_element(&ty, None),
            "[3 x i32] zeroinitializer"
        );
        let data = GlobalVariableData::Arr(vec![GlobalVariableData::Elem("7".to_string())]);
        assert_eq!(
            generator.gen_initializer_element(&ty, Some(&data)),
            "[3 x i32] [i32 7, i32 0, i32 0]"
        );
        let nested = Type::Arr(Box::new(ty), Some(2));
        let data = GlobalVariableData::Arr(vec![data]);
        assert_eq!(
            generator.gen_initializer_element(&nested, Some(&data)),
            "[2 x [3 x i32]] [[3 x i32] [i32 7, i32 0, i32 0], [3 x i32] zeroinitializer]"
        );
    }
}

mod aggregate;
mod atomic;
mod calls;
mod control_flow;
mod globals;
mod inline_asm;

use std::{borrow::Cow, collections::HashMap, vec};

use crate::{
    ast::{Identifier, ProgramAst, reserved_functions},
    ctype::Type,
    error,
    func::Func,
    node::{Node, NodeType},
};

#[derive(Debug)]
pub struct Options<'a> {
    return_ty: Type,
    return_pointer: Option<String>,

    register_number: &'a mut usize,
    register_map: &'a mut HashMap<usize, usize>,
    register_queue: &'a mut Vec<String>,
    expression_slots: HashMap<usize, usize>,
    argument_slots: HashMap<(usize, usize), usize>,

    label_number: usize,
    break_labels: Vec<String>,
    terminated: bool,
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
        let references = crate::generator::referenced_symbols(&ast);
        let generator = Self {
            error_logger: self.error_logger,
            symbol_names: ast.symbol_names.clone(),
        };
        [
            vec![
                "declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)".into(),
                "declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)".into(),
                "declare void @llvm.va_start(ptr)".into(),
                "declare void @llvm.va_end(ptr)".into(),
                "declare void @llvm.prefetch.p0(ptr, i32, i32, i32)".into(),
                "declare i32 @llvm.ctlz.i32(i32, i1)".into(),
                "declare i64 @llvm.ctlz.i64(i64, i1)".into(),
                "declare i32 @llvm.cttz.i32(i32, i1)".into(),
                "declare i64 @llvm.cttz.i64(i64, i1)".into(),
            ],
            generator.gen_string_literals(ast.string_literals),
            ast.global_variables
                .iter()
                .filter(|(name, object)| {
                    !object.hidden || !object.declaration_only || references.contains(name.as_str())
                })
                .map(|(s, gv)| generator.gen_global_variable(s, gv))
                .collect::<Vec<String>>(),
            ast.functions
                .iter()
                .filter(|(name, function)| {
                    !(function.internal && function.inline_definition)
                        || references.contains(name.as_str())
                })
                .filter(|(name, function)| {
                    !function.hidden
                        || (function.body.is_some() && !function.inline_only())
                        || references.contains(name.as_str())
                })
                // Preserve ABI diagnostics when omitting an unused declaration.
                .inspect(|(_, function)| generator.validate_function_abi(function))
                .filter(|(name, function)| {
                    (function.body.is_some() && !function.inline_only())
                        || references.contains(name.as_str())
                })
                .map(|(name, f)| generator.gen_func(name, f) + "\n")
                .collect::<Vec<String>>(),
            reserved_functions()
                .iter()
                .filter_map(|(name, ident)| {
                    if ast.functions.contains_key(name) || !references.contains(name.as_str()) {
                        return None;
                    }
                    let Identifier::Static(ty) = ident else {
                        unreachable!()
                    };
                    let (args, result, variadic) = ty.function_signature_ref().unwrap();
                    let mut args = args
                        .iter()
                        .map(Self::abi_parameter_type)
                        .collect::<Vec<_>>();
                    if variadic {
                        args.push("...".to_string());
                    }
                    Some(format!(
                        "declare {} {}({})",
                        Self::abi_result_type(result),
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

    fn align(ty: &Type) -> usize {
        ty.align_of()
    }

    fn access_alignment(node: &Node) -> usize {
        if matches!(node.nt, NodeType::Deref) {
            1
        } else {
            node.object_type_view()
                .unwrap_or_else(|| node.resolve_type_view().unwrap())
                .align_of()
        }
    }

    fn float_literal(ty: &Type, bits: usize) -> String {
        if ty.float_bits() == Some(80) {
            use rustc_apfloat::Float;
            return crate::floating::llvm_literal(
                crate::floating::from_double(f64::from_bits(bits as u64)).to_bits(),
            );
        }
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
            .definition()
            .borrow()
            .members
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

    fn abi_parameter_type(ty: &Type) -> String {
        let extension = Self::abi_extension(ty);
        let ty = Self::gen_type(ty);
        match extension {
            Some(extension) => format!("{ty} {extension}"),
            None => ty,
        }
    }

    fn abi_result_type(ty: &Type) -> String {
        let extension = Self::abi_extension(ty);
        let ty = Self::gen_type(ty);
        match extension {
            Some(extension) => format!("{extension} {ty}"),
            None => ty,
        }
    }

    fn gen_type(ty: &Type) -> String {
        match ty {
            Type::Void => "void".to_string(),
            Type::Bool => "i8".to_string(),
            Type::I8 | Type::SignedChar => "i8".to_string(),
            Type::I32 => "i32".to_string(),
            Type::Integer { bits, .. } => format!("i{bits}"),
            Type::LongLong { .. } => "i64".to_string(),
            // LLVM 15 gives i128 only 8-byte alignment on some targets.
            Type::Integer128 { .. } => "<2 x i64>".to_string(),
            Type::Enum(value) => Self::gen_type(&value.underlying.borrow()),
            Type::Float16 => "half".to_string(),
            Type::LongDouble(80) => "x86_fp80".to_string(),
            Type::LongDouble(128) => "fp128".to_string(),
            Type::Float(bits) | Type::LongDouble(bits) => {
                if *bits == 32 { "float" } else { "double" }.to_string()
            }
            Type::Ptr(_) => "ptr".to_string(),
            Type::Arr(child, size) => {
                format!("[{} x {}]", size.unwrap_or(0), Self::gen_type(child))
            }
            Type::Vla(child, _) => format!("[0 x {}]", Self::gen_type(child)),
            Type::Vector(element, lanes) => format!("<{lanes} x {}>", Self::gen_type(element)),
            Type::Struct(record)
                if record.definition().borrow().is_union
                    || record.has_bit_fields()
                    || record.definition().borrow().packing.is_some()
                    || ty.contains_typedef_alignment() =>
            {
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
            Type::Struct(record) => {
                let mut fields = vec![];
                let mut offset = 0usize;
                for member in &record.definition().borrow().members {
                    let aligned_offset =
                        offset.div_ceil(member.ty.align_of()) * member.ty.align_of();
                    if member.offset > aligned_offset {
                        fields.push(format!("[{} x i8]", member.offset - aligned_offset));
                    }
                    fields.push(Self::gen_type(&member.ty));
                    offset = member.offset + member.ty.size_of();
                }
                if ty.size_of() > offset {
                    fields.push(format!("[{} x i8]", ty.size_of() - offset));
                }
                if let Some(anchor) = Self::record_alignment_member(record) {
                    fields.push(anchor);
                }
                format!("{{ {} }}", fields.join(", "))
            }
            Type::Func(args, result) => format!(
                "{} ({})",
                Self::gen_type(result),
                args.iter()
                    .map(Self::gen_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::VariadicFunc(args, result) => format!(
                "{} ({}, ...)",
                Self::gen_type(result),
                args.iter()
                    .map(Self::gen_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::Qualified(child, _) | Type::Aligned(child, _) => Self::gen_type(child),
        }
    }

    fn gen_type_with_opaque_ptr(ty: &Type) -> String {
        Self::gen_type(ty)
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

    fn gen_func(&self, name: &str, func: &Func) -> String {
        let (types, result, variadic) = func.cty.function_signature_ref().unwrap();
        if matches!(result, Type::Struct(_)) || types.iter().any(|ty| matches!(ty, Type::Struct(_)))
        {
            return self.aggregate_function(name, func);
        }
        if func.body.is_none() || func.inline_only() {
            let mut arguments = types
                .iter()
                .map(Self::abi_parameter_type)
                .collect::<Vec<_>>();
            if variadic {
                arguments.push("...".to_string());
            }
            return format!(
                "declare {}{}{} {}({}){}",
                if func.weak_import { "extern_weak " } else { "" },
                if func.hidden && !func.internal {
                    "hidden "
                } else {
                    ""
                },
                Self::abi_result_type(result),
                self.symbol_name(name),
                arguments.join(", "),
                if func.noinline { " noinline" } else { "" }
            );
        }
        let return_ty = result;
        let options = &mut Options {
            return_ty: return_ty.clone(),
            return_pointer: None,
            register_number: &mut func.args.len(),
            register_map: &mut HashMap::<usize, usize>::new(),
            register_queue: &mut vec![],
            expression_slots: HashMap::new(),
            argument_slots: HashMap::new(),
            label_number: 0,
            break_labels: vec![],
            terminated: false,
        };
        [
            vec![format!(
                "define {}{}{} {}({}){} uwtable \"frame-pointer\"=\"all\" {{",
                if func.internal { "internal " } else { "" },
                if func.hidden && !func.internal {
                    "hidden "
                } else {
                    ""
                },
                Self::abi_result_type(return_ty),
                self.symbol_name(name),
                func.args
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| format!(
                        "{} noundef %{}",
                        Self::abi_parameter_type(&arg.resolve_type_view().unwrap()),
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
                        Self::gen_type(&node.resolve_type_view().unwrap()),
                        node.resolve_type_view().unwrap().size_of(),
                    ) + "\n"
                        + &format!(
                            "  store {} %{}, ptr %{}, align {}",
                            Self::gen_type(&node.resolve_type_view().unwrap()),
                            i,
                            register,
                            node.resolve_type_view().unwrap().size_of(),
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
                    if *return_ty == Type::Void {
                        "  ret void".to_string()
                    } else {
                        format!(
                            "  ret {} {}",
                            Self::gen_type(return_ty),
                            if matches!(return_ty, Type::Ptr(_)) {
                                "null".to_string()
                            } else if return_ty.float_bits().is_some() {
                                Self::float_literal(return_ty, 0)
                            } else {
                                "0".to_string()
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
        let result = if matches!(node.nt, NodeType::LocalVar | NodeType::CallFunc) {
            node.resolve_type_view()
        } else {
            None
        };
        if matches!(node.nt, NodeType::LocalVar)
            || (node.nt == NodeType::CallFunc && matches!(result.as_deref(), Some(Type::Struct(_))))
        {
            let offset = node.offset.unwrap();
            if !options.register_map.contains_key(&offset) {
                let register = options.register_from_offset(offset);
                let ty = result.as_deref().unwrap();
                lines.push(format!(
                    "  %{} = alloca {}, align {}",
                    register,
                    if matches!(ty, Type::Struct(_)) {
                        format!("[{} x i8]", ty.size_of().div_ceil(8) * 8)
                    } else {
                        Self::gen_type_with_opaque_ptr(ty)
                    },
                    Self::align(ty).max(node.alignment)
                ));
            }
        }
        if node.nt == NodeType::CallFunc {
            let types = node
                .args
                .iter()
                .rev()
                .map(|arg| arg.resolve_type_view().unwrap())
                .collect::<Vec<_>>();
            let signature = Self::aggregate_signature(
                &types,
                result.as_deref().unwrap(),
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
                        Self::gen_type(ty),
                        ty.align_of().max(argument.stack_alignment)
                    ));
                }
            }
        }
        if matches!(
            node.nt,
            NodeType::LogicalAnd | NodeType::LogicalOr | NodeType::Conditional
        ) {
            let ty = node.resolve_type_view().unwrap();
            if *ty != Type::Void {
                let register = options.new_temporary();
                options
                    .expression_slots
                    .insert(node.token.as_ref().unwrap().pos, register);
                lines.push(format!(
                    "  %{} = alloca {}, align {}",
                    register,
                    Self::gen_type(&ty),
                    Self::align(&ty)
                ));
            }
        }
        for child in node.child_nodes() {
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
                    "  %{register} = fcmp une {} {value}, {}",
                    Self::gen_type(source),
                    Self::float_literal(source, 0)
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
                Self::gen_type(source),
                Self::gen_type(target)
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
                Self::gen_type(target)
            ));
            return format!("%{}", register);
        }
        if *target == Type::Bool {
            let register = options.new_temporary();
            lines.push(format!(
                "  %{register} = icmp ne {} {value}, 0",
                Self::gen_type(source)
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
            Self::gen_type(source),
            value,
            Self::gen_type(target)
        ));
        format!("%{}", register)
    }

    fn gen_predicate(&self, node: &Node, options: &mut Options, lines: &mut Vec<String>) -> String {
        lines.extend(self.gen_node(node, options));
        let value = options.register_queue.pop().unwrap();
        let ty = node.resolve_type_view().unwrap();
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
                Self::gen_type(&ty)
            },
            value,
            if pointer {
                "null".to_string()
            } else if ty.float_bits().is_some() {
                Self::float_literal(&ty, 0)
            } else {
                "0".to_string()
            }
        ));
        format!("%{}", register)
    }

    fn gen_addr(&self, node: &Node, options: &mut Options) -> Vec<String> {
        match node.nt {
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
            NodeType::Conditional => {
                let lines = self.gen_node(node, options);
                options.register_queue.pop();
                let slot = options.expression_slots[&node.token.as_ref().unwrap().pos];
                options.register_queue.push(format!("%{}", slot));
                lines
            }
            NodeType::CallFunc => {
                let lines = self.gen_node(node, options);
                options.register_queue.pop();
                options
                    .register_queue
                    .push(format!("%{}", options.register_map[&node.offset.unwrap()]));
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
            NodeType::CopyBytes { .. } => self.gen_node(node, options),
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "expression is not addressable",
                );
                unreachable!()
            }
        }
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
            NodeType::Num => {
                let ty = node.resolve_type_view().unwrap();
                options.register_queue.push(if ty.float_bits().is_some() {
                    node.extended_float.as_ref().map_or_else(
                        || Self::float_literal(&ty, node.value.unwrap()),
                        |bits| crate::floating::llvm_literal(**bits),
                    )
                } else {
                    node.value.unwrap().to_string()
                });
                return vec![];
            }
            NodeType::LocalVar | NodeType::GlobalVar | NodeType::Deref => {
                let mut lines = self.gen_addr(node, options);
                let ty = node.resolve_type_view().unwrap();
                if !matches!(
                    *ty,
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
                        Self::gen_type_with_opaque_ptr(&ty),
                        address,
                        Self::access_alignment(node)
                    ));
                }
                return lines;
            }
            NodeType::Addr => {
                return self.gen_addr(node.lhs.as_ref().unwrap(), options);
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
                        &operand.resolve_type_view().unwrap(),
                        target,
                        options,
                        &mut lines,
                    );
                    options.register_queue.push(value);
                }
                return lines;
            }
            NodeType::Sub
                if node.lhs.as_ref().unwrap().dest_type_view().is_some()
                    && node.rhs.as_ref().unwrap().dest_type_view().is_some() =>
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
                    node.lhs
                        .as_ref()
                        .unwrap()
                        .dest_type_view()
                        .unwrap()
                        .size_of()
                ));
                lines.push(format!(
                    "  %{} = trunc i64 %{} to i32",
                    options.new_register(),
                    count
                ));
                return lines;
            }
            NodeType::Add | NodeType::Sub
                if node.lhs.as_ref().unwrap().dest_type_view().is_some() =>
            {
                let base = node.lhs.as_ref().unwrap();
                let index = node.rhs.as_ref().unwrap();
                let mut lines = self.gen_node(index, options);
                let mut value = options.register_queue.pop().unwrap();
                // GEP sign-extends its indices, so preserve C signedness explicitly.
                value = Self::convert_integer(
                    value,
                    &index.resolve_type_view().unwrap(),
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
                    Self::gen_type_with_opaque_ptr(&base.dest_type_view().unwrap()),
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
                    &operand.resolve_type_view().unwrap(),
                    &node.resolve_type_view().unwrap(),
                    options,
                    &mut lines,
                );
                lines.push(format!(
                    "  %{} = xor {} {}, -1",
                    options.new_register(),
                    Self::gen_type(&node.resolve_type_view().unwrap()),
                    value
                ));
                return lines;
            }
            NodeType::LogicalAnd
            | NodeType::LogicalOr
            | NodeType::Conditional
            | NodeType::Block
            | NodeType::Return
            | NodeType::If
            | NodeType::While
            | NodeType::For
            | NodeType::DoWhile
            | NodeType::Switch
            | NodeType::Case
            | NodeType::Break
            | NodeType::Continue
            | NodeType::Goto
            | NodeType::Label => {
                return self.gen_control_flow(node, options);
            }

            NodeType::Assign => {
                let mut lines = self.gen_node(node.rhs.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                lines.extend(self.gen_addr(node.lhs.as_ref().unwrap(), options));
                let address = options.register_queue.pop().unwrap();
                let ty = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
                let source_ty = node.rhs.as_ref().unwrap().resolve_type_view().unwrap();
                let value = Self::convert_integer(value, &source_ty, &ty, options, &mut lines);
                lines.push(format!(
                    "  store {}{} {}, ptr {}, align {}",
                    if node.lhs.as_ref().is_some_and(|lhs| lhs.volatile_access) {
                        "volatile "
                    } else {
                        ""
                    },
                    Self::gen_type_with_opaque_ptr(&ty),
                    value,
                    address,
                    Self::access_alignment(node.lhs.as_ref().unwrap())
                ));
                options.register_queue.push(value);
                return lines;
            }
            NodeType::SuffixIncr | NodeType::SuffixDecr => {
                let operand = node.lhs.as_ref().unwrap();
                let mut lines = self.gen_addr(operand, options);
                let address = options.register_queue.pop().unwrap();
                let ty = operand.resolve_type_view().unwrap();
                let old = options.new_temporary();
                lines.push(format!(
                    "  %{} = load {}{}, ptr {}, align {}",
                    old,
                    if node.volatile_access {
                        "volatile "
                    } else {
                        ""
                    },
                    Self::gen_type(&ty),
                    address,
                    Self::access_alignment(operand)
                ));
                let new = options.new_temporary();
                if let Type::Ptr(child) = ty.as_ref() {
                    lines.push(format!(
                        "  %{} = getelementptr inbounds {}, ptr %{}, i32 {}",
                        new,
                        Self::gen_type(child),
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
                        Self::gen_type(&ty),
                        old,
                        if ty.float_bits().is_some() {
                            Self::float_literal(&ty, ty.float_constant(1.0))
                        } else {
                            "1".to_string()
                        }
                    ));
                }
                let new = if *ty == Type::Bool {
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
                    Self::gen_type(&ty),
                    new,
                    address,
                    Self::access_alignment(operand)
                ));
                options.register_queue.push(format!("%{}", old));
                return lines;
            }
            NodeType::CallFunc | NodeType::VaStart | NodeType::VaEnd => {
                return self.gen_calls(node, options);
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

            NodeType::FloatAbs => {
                let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                let ty = node.resolve_type_view().unwrap();
                let bits = ty.float_bits().unwrap();
                let float = Self::gen_type(&ty);
                let raw = options.new_temporary();
                let masked = options.new_temporary();
                let result = options.new_register();
                let mask = (1u128 << (bits - 1)) - 1;
                lines.extend([
                    format!("  %{raw} = bitcast {float} {value} to i{bits}"),
                    format!("  %{masked} = and i{bits} %{raw}, {mask}"),
                    format!("  %{result} = bitcast i{bits} %{masked} to {float}"),
                ]);
                return lines;
            }
            NodeType::BitScan { bits, leading } => {
                let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
                let operand = options.register_queue.pop().unwrap();
                let count = options.new_temporary();
                let intrinsic = if leading { "ctlz" } else { "cttz" };
                lines.push(format!(
                    "  %{count} = call i{bits} @llvm.{intrinsic}.i{bits}(i{bits} {operand}, i1 false)"
                ));
                if bits == 64 {
                    let result = options.new_register();
                    lines.push(format!("  %{result} = trunc i64 %{count} to i32"));
                } else {
                    options.register_queue.push(format!("%{count}"));
                }
                return lines;
            }

            NodeType::StackAlloc => {
                let mut lines = self.gen_node(node.lhs.as_ref().unwrap(), options);
                let size = options.register_queue.pop().unwrap();
                let register = options.new_register();
                lines.push(format!("  %{register} = alloca i8, i64 {size}, align 16"));
                return lines;
            }
            NodeType::Prefetch(write, locality) => {
                return self.gen_prefetch(node, write, locality, options);
            }
            NodeType::Atomic(operation) => {
                return self.gen_atomic(node, operation, options);
            }
            NodeType::InlineAsm(volatile, memory) => {
                return self.gen_inline_assembly(node, volatile, memory, options);
            }
            NodeType::ZeroFill { bytes } => {
                let mut lines = self.gen_addr(node.lhs.as_ref().unwrap(), options);
                let pointer = options.register_queue.pop().unwrap();
                lines.push(format!(
                    "  call void @llvm.memset.p0.i64(ptr align 1 {pointer}, i8 0, i64 {bytes}, i1 false)"
                ));
                options.register_queue.push("0".into());
                return lines;
            }
            NodeType::CopyBytes { bytes } => {
                let mut lines = self.gen_addr(node.lhs.as_ref().unwrap(), options);
                let destination = options.register_queue.pop().unwrap();
                lines.extend(self.gen_addr(node.rhs.as_ref().unwrap(), options));
                let source = options.register_queue.pop().unwrap();
                lines.push(format!(
                    "  call void @llvm.memcpy.p0.p0.i64(ptr align 1 {destination}, ptr align 1 {source}, i64 {bytes}, i1 false)"
                ));
                options.register_queue.push(destination);
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
            NodeType::BitLeft => "shl",
            NodeType::BitRight => "ashr",
            NodeType::BitAnd => "and",
            NodeType::BitXor => "xor",
            NodeType::BitOr => "or",
            NodeType::Eq => "icmp eq",
            NodeType::Ne => "icmp ne",
            NodeType::Lt => "icmp slt",
            NodeType::Le => "icmp sle",
            _ => "unknown",
        };
        let mut rhs_register = options.register_queue.pop().unwrap();
        let mut lhs_register = options.register_queue.pop().unwrap();
        if left_first {
            std::mem::swap(&mut lhs_register, &mut rhs_register);
        }
        let left_ty = node.lhs.as_ref().unwrap().resolve_type_view().unwrap();
        let right_ty = node.rhs.as_ref().unwrap().resolve_type_view().unwrap();
        let operand_ty = if left_ty.is_arithmetic() && right_ty.is_arithmetic() {
            if matches!(node.nt, NodeType::BitLeft | NodeType::BitRight) {
                Cow::Owned(left_ty.promoted())
            } else {
                Cow::Owned(Type::arithmetic_type(&left_ty, &right_ty))
            }
        } else if left_ty.is_pointer_value() {
            Type::decay_view(Cow::Borrowed(left_ty.as_ref()))
        } else if right_ty.is_pointer_value() && matches!(node.nt, NodeType::Eq | NodeType::Ne) {
            Type::decay_view(Cow::Borrowed(right_ty.as_ref()))
        } else {
            Cow::Borrowed(left_ty.as_ref())
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
                NodeType::Add => "add",
                NodeType::Sub => "sub",
                NodeType::Mul => "mul",
                NodeType::Div => "udiv",
                NodeType::Mod => "urem",
                NodeType::BitRight => "lshr",
                NodeType::Lt => "icmp ult",
                NodeType::Le => "icmp ule",
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
                | NodeType::BitLeft
                | NodeType::BitRight
                | NodeType::BitAnd
                | NodeType::BitXor
                | NodeType::BitOr
                | NodeType::Eq
                | NodeType::Ne
                | NodeType::Lt
                | NodeType::Le => {
                    format!(
                        "  %{} = {} {} {}, {}",
                        options.new_register(),
                        operation,
                        Self::gen_type(&operand_ty),
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
    use crate::global::GlobalVariableData;
    use std::rc::Rc;

    #[test]
    fn global_array_initializers_fill_missing_elements() {
        let logger = error::ErrorPrinter::new("");
        let generator = IrGenerator::new(&logger);
        let ty = Type::Arr(Rc::new(Type::I32), Some(3));
        assert_eq!(
            generator.gen_initializer_element(&ty, None),
            "[3 x i32] zeroinitializer"
        );
        let data = GlobalVariableData::Arr(vec![GlobalVariableData::Elem("7".to_string())]);
        assert_eq!(
            generator.gen_initializer_element(&ty, Some(&data)),
            "[3 x i32] [i32 7, i32 0, i32 0]"
        );
        let nested = Type::Arr(Rc::new(ty), Some(2));
        let data = GlobalVariableData::Arr(vec![data]);
        assert_eq!(
            generator.gen_initializer_element(&nested, Some(&data)),
            "[2 x [3 x i32]] [[3 x i32] [i32 7, i32 0, i32 0], [3 x i32] zeroinitializer]"
        );
    }
}

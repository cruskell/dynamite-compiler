use super::IrGenerator;
use super::Options;
use crate::ctype::Type;
use crate::func::Func;
use crate::node::Node;
use std::borrow::Cow;
use std::collections::HashMap;

impl IrGenerator<'_> {
    pub(super) fn aggregate_result_type(ty: &Type) -> String {
        let parts = crate::generator::abi::aggregate_parts(ty, &Self::target_arch());
        if parts.is_empty() {
            "void".into()
        } else if parts.len() == 1 {
            Self::gen_type(&parts[0].ty)
        } else {
            format!(
                "{{ {} }}",
                parts
                    .iter()
                    .map(|part| Self::gen_type(&part.ty))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    pub(super) fn aggregate_signature(
        types: &[impl std::borrow::Borrow<Type>],
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
                    let parts =
                        abi::aggregate_parts(std::borrow::Borrow::borrow(ty), &Self::target_arch());
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

    pub(super) fn aggregate_parameter_types(
        ty: &Type,
        argument: &crate::generator::abi::Argument,
    ) -> Vec<String> {
        if argument.memory.is_some() {
            return vec![format!(
                "ptr byval({}) align {}",
                Self::gen_type(ty),
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
            .map(|(part, _)| Self::gen_type(&part.ty))
            .collect::<Vec<_>>();
        if cfg!(target_arch = "aarch64") && matches!(ty, Type::Struct(_)) && parts.len() > 1 {
            vec![format!("[{} x {}]", parts.len(), parts[0])]
        } else {
            parts
        }
    }

    pub(super) fn aggregate_pointer(
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

    pub(super) fn aggregate_load_part(
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
            Self::gen_type(&part.ty)
        };
        lines.push(format!(
            "  %{register} = load {loaded_type}, ptr {pointer}, align 1"
        ));
        if remaining < part.ty.size_of() {
            let extended = options.new_temporary();
            lines.push(format!(
                "  %{extended} = zext {loaded_type} %{register} to {}",
                Self::gen_type(&part.ty)
            ));
            format!("%{extended}")
        } else {
            format!("%{register}")
        }
    }

    pub(super) fn validate_function_abi(&self, func: &Func) {
        let (types, result, _) = func.cty.function_signature_ref().unwrap();
        let aggregate = matches!(result, Type::Struct(_))
            || types.iter().any(|ty| matches!(ty, Type::Struct(_)));
        if aggregate
            && types
                .iter()
                .chain(std::iter::once(result))
                .any(Type::contains_extended_long_double)
        {
            self.error_logger.print_error_position(func.token.as_ref().map_or(0, |token|token.pos), "extended long double cannot be combined with by-value aggregate parameters or results");
        }
    }

    pub(super) fn aggregate_function(&self, name: &str, func: &Func) -> String {
        let (types, result, variadic) = func.cty.function_signature_ref().unwrap();
        let signature = Self::aggregate_signature(types, result, types.len(), variadic);
        let return_type = if matches!(result, Type::Struct(_)) {
            Self::aggregate_result_type(result)
        } else {
            Self::gen_type(result)
        };
        let return_abi = if matches!(result, Type::Struct(_)) {
            Cow::Borrowed(return_type.as_str())
        } else {
            Cow::Owned(Self::abi_result_type(result))
        };
        let mut parameters = vec![];
        if signature.indirect_result {
            parameters.push(format!(
                "ptr sret({}) align {} %abi.result",
                Self::gen_type(result),
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
                "declare {}{}{return_abi} {}({}){}",
                if func.weak_import { "extern_weak " } else { "" },
                if func.hidden && !func.internal {
                    "hidden "
                } else {
                    ""
                },
                self.symbol_name(name),
                parameters.join(", "),
                if func.noinline { " noinline" } else { "" }
            );
        }
        let mut number = 0;
        let mut registers = HashMap::new();
        let mut queue = vec![];
        let mut options = Options {
            return_ty: result.clone(),
            return_pointer: signature.indirect_result.then(|| "%abi.result".into()),
            register_number: &mut number,
            register_map: &mut registers,
            register_queue: &mut queue,
            expression_slots: HashMap::new(),
            argument_slots: HashMap::new(),
            label_number: 0,
            break_labels: vec![],
            terminated: false,
        };
        let mut lines = vec![format!(
            "define {}{}{return_abi} {}({}){} uwtable \"frame-pointer\"=\"all\" {{",
            if func.internal { "internal " } else { "" },
            if func.hidden && !func.internal {
                "hidden "
            } else {
                ""
            },
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
                    Self::gen_type(ty)
                ));
                lines.push(format!(
                    "  store {} %{value}, ptr %{register}, align {}",
                    Self::gen_type(ty),
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
                        Self::gen_type(&part.ty)
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

    pub(super) fn aggregate_call(&self, node: &Node, options: &mut Options) -> Vec<String> {
        let types = node
            .args
            .iter()
            .rev()
            .map(|arg| arg.resolve_type_view().unwrap())
            .collect::<Vec<_>>();
        let result = node.resolve_type_view().unwrap();
        if types.iter().any(|ty| ty.contains_extended_long_double())
            || result.contains_extended_long_double()
        {
            self.error_logger.print_error_position(node.token.as_ref().map_or(0,|token|token.pos), "extended long double cannot be combined with by-value aggregate arguments or results");
        }
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
            let ty = types[index].as_ref();
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
                    Self::gen_type(ty)
                ));
                lines.push(format!(
                    "  store {} %{register}, ptr %{slot}, align {alignment}",
                    Self::gen_type(ty)
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
                            Self::gen_type(&argument.parts[index].0.ty)
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
                    Self::gen_type(&result),
                    result.align_of(),
                    result_slot.as_ref().unwrap()
                ),
            );
        }
        let return_type = if matches!(*result, Type::Struct(_)) {
            Self::aggregate_result_type(&result)
        } else {
            Self::gen_type(&result)
        };
        let return_abi = if matches!(*result, Type::Struct(_)) {
            Cow::Borrowed(return_type.as_str())
        } else {
            Cow::Owned(Self::abi_result_type(&result))
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
        if matches!(*result, Type::Struct(_)) {
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
                    let mut stored_type = Self::gen_type(&part.ty);
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
                Self::gen_type(&result),
                result.align_of()
            ));
        } else {
            options.register_queue.push(value);
        }
        lines
    }

    pub(super) fn aggregate_return(&self, node: &Node, options: &mut Options) -> Vec<String> {
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
                Self::gen_type(&ty)
            ));
            lines.push(format!(
                "  store {} %{register}, ptr {pointer}, align {}",
                Self::gen_type(&ty),
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
                        Self::gen_type(&part.ty)
                    ));
                    result = format!("%{combined}");
                }
            }
            lines.push(format!("  ret {return_type} {result}"));
        }
        options.terminated = true;
        lines
    }
}

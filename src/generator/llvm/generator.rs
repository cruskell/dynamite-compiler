use std::{collections::HashMap, vec};

use crate::{
    ast::{Identifier, ProgramAst, reserved_functions},
    ctype::Type,
    error,
    func::Func,
    global::{GlobalVariable, GlobalVariableData},
    node::{Node, NodeType},
};

#[derive(Debug)]
pub struct Options<'a> {
    register_number: &'a mut usize,
    register_map: &'a mut HashMap<usize, usize>,
    register_queue: &'a mut Vec<String>,
    label_number: usize,
    terminated: bool,
    return_ty: Type,
    break_labels: Vec<String>,
    expression_slots: HashMap<usize, usize>,
}

impl Options<'_> {
    fn new_label(&mut self) -> String {
        self.label_number += 1;
        format!("block.{}", self.label_number)
    }

    fn new_register(&mut self) -> usize {
        *self.register_number += 1;
        self.register_queue
            .push(format!("%{}", self.register_number));
        *self.register_number
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
}

impl<'a> IrGenerator<'a> {
    pub fn new(error_logger: &'a dyn error::ErrorLogger) -> Self {
        Self { error_logger }
    }
    pub fn generate(&self, ast: ProgramAst) -> String {
        [
            self.gen_string_literals(ast.string_literals),
            ast.global_variables
                .iter()
                .map(|(s, gv)| self.gen_global_variable(s, gv))
                .collect::<Vec<String>>(),
            ast.functions
                .iter()
                .map(|(name, f)| self.gen_func(name, f) + "\n")
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
                        .map(|ty| Self::gen_type(ty.clone()))
                        .collect::<Vec<_>>();
                    if variadic {
                        args.push("...".to_string());
                    }
                    Some(format!(
                        "declare {} @{}({})",
                        Self::gen_type(result.clone()),
                        name,
                        args.join(", ")
                    ))
                })
                .collect(),
        ]
        .concat()
        .join("\n")
    }

    fn gen_string_literals(&self, string_literals: &[String]) -> Vec<String> {
        string_literals
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let bytes = crate::tokenizer::decode_string_literal(value).unwrap();
                format!(
                    "@.str.{} = private unnamed_addr constant [{} x i8] c\"{}\\00\", align 1",
                    i,
                    bytes.len() + 1,
                    bytes
                        .iter()
                        .map(|byte| format!("\\{:02X}", byte))
                        .collect::<String>()
                )
            })
            .collect::<Vec<String>>()
    }

    fn align(ty: &Type) -> usize {
        ty.align_of()
    }

    fn gen_global_variable(&self, name: &str, gv: &GlobalVariable) -> String {
        if gv.declaration_only {
            return format!(
                "@{} = external global {}",
                name,
                Self::gen_type(gv.ty.clone())
            );
        }
        format!(
            "@{} = {}global {}, align {}",
            name,
            if gv.internal { "internal " } else { "" },
            Self::gen_initializer_element(&gv.ty, gv.data.as_ref()),
            Self::align(&gv.ty),
        )
    }

    fn gen_type(ty: Type) -> String {
        match ty {
            Type::Qualified(child, _) => Self::gen_type(*child),
            Type::Void => "void".to_string(),
            Type::Struct(ty) if ty.definition().borrow().is_union || ty.has_bit_fields() => {
                let ty = Type::Struct(ty);
                format!(
                    "[{} x i{}]",
                    ty.size_of() / ty.align_of(),
                    ty.align_of() * 8
                )
            }
            Type::Struct(ty) => format!(
                "{{ {} }}",
                ty.members()
                    .iter()
                    .map(|member| Self::gen_type(member.ty.clone()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::I8 => "i8".to_string(),
            Type::I32 => "i32".to_string(),
            Type::Integer { bits, .. } => format!("i{bits}"),
            Type::Bool => "i8".to_string(),
            Type::Ptr(_) => "ptr".to_string(),
            Type::Arr(child, size) => format!("[{} x {}]", size, Self::gen_type(*child)),
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

    fn gen_initializer_element(ty: &Type, data: Option<&GlobalVariableData>) -> String {
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
            Type::Struct(struct_type)
                if struct_type.definition().borrow().is_union || struct_type.has_bit_fields() =>
            {
                let mut bytes = vec![0u8; ty.size_of()];
                let mut addresses = HashMap::new();
                crate::global::initializer_bytes(ty, data, 0, &mut bytes, &mut addresses);
                let width = ty.align_of();
                let elements = bytes
                    .chunks(width)
                    .enumerate()
                    .map(|(index, chunk)| {
                        let mut word = [0u8; 8];
                        word[..chunk.len()].copy_from_slice(chunk);
                        if let Some((symbol, offset)) = addresses.get(&(index * width)) {
                            let pointer = if *offset == 0 {
                                format!("@{symbol}")
                            } else {
                                format!("getelementptr (i8, ptr @{symbol}, i64 {offset})")
                            };
                            format!("i64 ptrtoint (ptr {pointer} to i64)")
                        } else {
                            format!("i{} {}", width * 8, u64::from_le_bytes(word))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} [{}]", Self::gen_type(ty.clone()), elements)
            }
            Type::Qualified(child, _) => Self::gen_initializer_element(child, data),
            Type::Struct(struct_type) => {
                if let Some(GlobalVariableData::Arr(elements)) = data {
                    format!(
                        "{} {{ {} }}",
                        Self::gen_type(ty.clone()),
                        struct_type
                            .members()
                            .iter()
                            .enumerate()
                            .map(|(i, member)| Self::gen_initializer_element(
                                &member.ty,
                                elements.get(i)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    format!("{} zeroinitializer", Self::gen_type(ty.clone()))
                }
            }
            Type::Arr(children_ty, size) => {
                if let Some(GlobalVariableData::Arr(v)) = data {
                    format!(
                        "{} [{}]",
                        Self::gen_type(ty.clone()),
                        (0..*size)
                            .map(|i| Self::gen_initializer_element(children_ty, v.get(i)))
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
                            format!("@{symbol}")
                        } else {
                            format!("getelementptr (i8, ptr @{symbol}, i64 {offset})")
                        }
                    }
                    Some(GlobalVariableData::Elem(value)) if value.starts_with('@') =>
                        value.clone(),
                    Some(GlobalVariableData::Elem(value)) if value != "0" =>
                        format!("inttoptr (i64 {} to ptr)", value),
                    _ => "null".to_string(),
                }
            ),
            Type::I8 | Type::I32 | Type::Integer { .. } | Type::Bool => format!(
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

    fn gen_func(&self, name: &str, func: &Func) -> String {
        if func.body.is_none() {
            let (args, result, variadic) = func.cty.function_signature().unwrap();
            let mut arguments = args
                .iter()
                .map(|ty| Self::gen_type(ty.clone()))
                .collect::<Vec<_>>();
            if variadic {
                arguments.push("...".to_string());
            }
            return format!(
                "declare {} @{}({})",
                Self::gen_type(result),
                name,
                arguments.join(", ")
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
            break_labels: vec![],
            expression_slots: HashMap::new(),
        };
        [
            vec![format!(
                "define {}{} @{}({}) {{",
                if func.internal { "internal " } else { "" },
                Self::gen_type(*return_ty.clone()),
                name,
                func.args
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| format!(
                        "{} noundef %{}",
                        Self::gen_type(arg.resolve_type().unwrap()),
                        i
                    ))
                    .chain(variadic.then_some("...".to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
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
            Self::allocate_locals(func.body.as_ref().unwrap(), options),
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
                            } else {
                                "0"
                            }
                        )
                    },
                    "}".to_string(),
                ]
            },
        ]
        .concat()
        .join("\n")
    }

    fn allocate_locals(node: &Node, options: &mut Options) -> Vec<String> {
        let mut lines = vec![];
        if matches!(node.nt, NodeType::LocalVar) {
            let offset = node.offset.unwrap();
            if !options.register_map.contains_key(&offset) {
                let register = options.register_from_offset(offset);
                let ty = node.resolve_type().unwrap();
                lines.push(format!(
                    "  %{} = alloca {}, align {}",
                    register,
                    Self::gen_type_with_opaque_ptr(ty.clone()),
                    Self::align(&ty)
                ));
            }
        }
        if matches!(
            node.nt,
            NodeType::LogicalAnd | NodeType::LogicalOr | NodeType::Conditional
        ) {
            let ty = node.resolve_type().unwrap();
            if ty != Type::Void {
                let register = options.new_register();
                options.register_queue.pop();
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
            lines.extend(Self::allocate_locals(child, options));
        }
        for child in node.children.iter().chain(&node.args) {
            lines.extend(Self::allocate_locals(child, options));
        }
        lines
    }

    fn convert_integer(
        value: String,
        source: &Type,
        target: &Type,
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
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
            let register = options.new_register();
            options.register_queue.pop();
            lines.push(format!("  %{register} = inttoptr i64 {wide_value} to ptr"));
            return format!("%{register}");
        }
        if *target == Type::Bool && matches!(source, Type::Ptr(_) | Type::Arr(..)) {
            let predicate = options.new_register();
            options.register_queue.pop();
            lines.push(format!("  %{predicate} = icmp ne ptr {value}, null"));
            let result = options.new_register();
            options.register_queue.pop();
            lines.push(format!("  %{result} = zext i1 %{predicate} to i8"));
            return format!("%{result}");
        }
        if matches!(source, Type::Ptr(_) | Type::Arr(..)) && target.integer_info().is_some() {
            let register = options.new_register();
            options.register_queue.pop();
            lines.push(format!(
                "  %{} = ptrtoint ptr {} to {}",
                register,
                value,
                Self::gen_type(target.clone())
            ));
            return format!("%{}", register);
        }
        if *target == Type::Bool {
            let register = options.new_register();
            options.register_queue.pop();
            lines.push(format!(
                "  %{register} = icmp ne {} {value}, 0",
                Self::gen_type(source.clone())
            ));
            let result = options.new_register();
            options.register_queue.pop();
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
        let register = options.new_register();
        options.register_queue.pop();
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
        let pointer = matches!(ty, Type::Ptr(_) | Type::Arr(..));
        let register = options.new_register();
        options.register_queue.pop();
        lines.push(format!(
            "  %{} = icmp ne {} {}, {}",
            register,
            if pointer {
                "ptr".to_string()
            } else {
                Self::gen_type(ty)
            },
            value,
            if pointer { "null" } else { "0" }
        ));
        format!("%{}", register)
    }

    fn gen_addr(&self, node: &Node, options: &mut Options) -> Vec<String> {
        match node.nt {
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
                options.register_queue.push(if node.dest.is_empty() {
                    format!("@{}", node.global_name)
                } else {
                    node.dest.clone()
                });
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
                let parameters = node.parameter_types.clone();
                let result = node.cty.clone().unwrap();
                let variadic = node.variadic;
                let mut lines = vec![];
                let callee = if let Some(callee) = &node.lhs {
                    lines.extend(self.gen_node(callee, options));
                    options.register_queue.pop().unwrap()
                } else {
                    format!("@{}", node.global_name)
                };
                let mut arguments = vec![];
                for (i, arg) in node.args.iter().enumerate() {
                    lines.extend(self.gen_node(arg, options));
                    let value = options.register_queue.pop().unwrap();
                    let source = arg.resolve_type().unwrap();
                    let source = match source {
                        Type::Arr(child, _) => Type::Ptr(child),
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
                    arguments.push(format!("{} {}", Self::gen_type(ty), value));
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
                    Self::gen_type(result),
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
                    .map(|node| self.gen_node(node, options))
                    .collect::<Vec<Vec<String>>>()
                    .concat();
            }
            NodeType::Return => {
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
                options.register_queue.push(node.value.unwrap().to_string());
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
                    Self::align(&ty)
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
                if !matches!(ty, Type::Arr(..) | Type::Func(..) | Type::VariadicFunc(..)) {
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
                        Self::align(&ty)
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
                let left = options.new_register();
                options.register_queue.pop();
                lines.push(format!("  %{} = ptrtoint ptr {} to i64", left, lhs));
                let right = options.new_register();
                options.register_queue.pop();
                lines.push(format!("  %{} = ptrtoint ptr {} to i64", right, rhs));
                let difference = options.new_register();
                options.register_queue.pop();
                lines.push(format!("  %{} = sub i64 %{}, %{}", difference, left, right));
                let count = options.new_register();
                options.register_queue.pop();
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
                let index_ty = Self::gen_type(index.resolve_type().unwrap());
                if matches!(node.nt, NodeType::Sub) {
                    let register = options.new_register();
                    options.register_queue.pop();
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
                let value = options.new_register();
                options.register_queue.pop();
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
                let old = options.new_register();
                options.register_queue.pop();
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
                    Self::align(&ty)
                ));
                let new = options.new_register();
                options.register_queue.pop();
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
                        "  %{} = {} {} %{}, 1",
                        new,
                        if matches!(node.nt, NodeType::SuffixIncr) {
                            "add"
                        } else {
                            "sub"
                        },
                        Self::gen_type(ty.clone()),
                        old
                    ));
                }
                let new = if ty == Type::Bool {
                    let predicate = options.new_register();
                    options.register_queue.pop();
                    lines.push(format!("  %{predicate} = icmp ne i8 %{new}, 0"));
                    let result = options.new_register();
                    options.register_queue.pop();
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
                    Self::align(&ty)
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

        let lhs = self.gen_node(
            node.rhs.as_ref().unwrap_or_else(|| panic!("{:?}", node)),
            options,
        );
        let rhs = self.gen_node(
            node.lhs.as_ref().unwrap_or_else(|| panic!("{:?}", node)),
            options,
        );

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
        let left_ty = node.lhs.as_ref().unwrap().resolve_type().unwrap();
        let right_ty = node.rhs.as_ref().unwrap().resolve_type().unwrap();
        let operand_ty = if left_ty.integer_info().is_some() && right_ty.integer_info().is_some() {
            if matches!(node.nt, NodeType::BitLeft | NodeType::BitRight) {
                left_ty.promoted()
            } else {
                Type::arithmetic_type(&left_ty, &right_ty)
            }
        } else if let Type::Arr(child, _) = &left_ty {
            Type::Ptr(child.clone())
        } else {
            left_ty.clone()
        };
        let operation = if operand_ty.integer_info().is_some_and(|(_, signed)| !signed) {
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
        .concat();
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
        let ty = Type::Arr(Box::new(Type::I32), 3);
        assert_eq!(
            IrGenerator::gen_initializer_element(&ty, None),
            "[3 x i32] zeroinitializer"
        );
        let data = GlobalVariableData::Arr(vec![GlobalVariableData::Elem("7".to_string())]);
        assert_eq!(
            IrGenerator::gen_initializer_element(&ty, Some(&data)),
            "[3 x i32] [i32 7, i32 0, i32 0]"
        );
        let nested = Type::Arr(Box::new(ty), 2);
        let data = GlobalVariableData::Arr(vec![data]);
        assert_eq!(
            IrGenerator::gen_initializer_element(&nested, Some(&data)),
            "[2 x [3 x i32]] [[3 x i32] [i32 7, i32 0, i32 0], [3 x i32] zeroinitializer]"
        );
    }
}

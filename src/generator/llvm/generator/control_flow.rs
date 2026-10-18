use super::{IrGenerator, Options};
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};

impl IrGenerator<'_> {
    pub(super) fn gen_control_flow(&self, node: &Node, options: &mut Options) -> Vec<String> {
        match node.nt {
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
                lines
            }
            NodeType::Conditional => {
                if node.resolve_type_view().as_deref() == Some(&Type::Void) {
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
                let ty = node.resolve_type_view().unwrap();
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
                        &branch.resolve_type_view().unwrap(),
                        &ty,
                        options,
                        &mut lines,
                    );
                    lines.push(format!(
                        "  store {} {}, ptr %{}, align {}",
                        Self::gen_type(&ty),
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
                    Self::gen_type(&ty),
                    slot,
                    Self::align(&ty)
                ));
                lines
            }
            NodeType::Block => node
                .children
                .iter()
                .flat_map(|node| self.gen_node(node, options))
                .collect(),
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
                let source_ty = node.resolve_type_view().unwrap();
                let target_ty = options.return_ty.clone();
                let value =
                    Self::convert_integer(value, &source_ty, &target_ty, options, &mut lines);
                lines.push(format!(
                    "  ret {} {}",
                    Self::gen_type_with_opaque_ptr(&target_ty),
                    value
                ));
                options.terminated = true;
                lines
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
                lines
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
                lines
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
                lines
            }
            NodeType::Switch => {
                let mut lines = self.gen_node(node.cond.as_ref().unwrap(), options);
                let value = options.register_queue.pop().unwrap();
                let selector_ty = node
                    .cond
                    .as_ref()
                    .unwrap()
                    .resolve_type_view()
                    .unwrap()
                    .promoted();
                let value = Self::convert_integer(
                    value,
                    &node.cond.as_ref().unwrap().resolve_type_view().unwrap(),
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
                    Self::gen_type(&selector_ty),
                    value,
                    default
                ));
                for case in &node.children {
                    if let Some(value) = case.value {
                        lines.push(format!(
                            "    {} {}, label %case.{}",
                            Self::gen_type(&selector_ty),
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
                lines
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
                lines
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
                vec![format!("  br label %{}", label)]
            }
            NodeType::Continue => {
                options.terminated = true;
                vec![format!("  br label %continue.{}", node.value.unwrap())]
            }
            NodeType::Goto => {
                options.terminated = true;
                vec![format!("  br label %goto.{}", node.value.unwrap())]
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
                lines
            }
            _ => unreachable!("node dispatched to the wrong generator"),
        }
    }
}

use super::{IrGenerator, Options};
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};
use std::borrow::Cow;

impl IrGenerator<'_> {
    pub(super) fn gen_calls(&self, node: &Node, options: &mut Options) -> Vec<String> {
        match node.nt {
            NodeType::CallFunc => {
                if matches!(node.resolve_type_view().as_deref(), Some(Type::Struct(_)))
                    || node.args.iter().any(|arg| {
                        matches!(arg.resolve_type_view().as_deref(), Some(Type::Struct(_)))
                    })
                {
                    return self.aggregate_call(node, options);
                }
                let parameters = &node.parameter_types;
                let result = node.cty.as_ref().unwrap();
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
                    let source = Type::decay_view(arg.resolve_type_view().unwrap());
                    let ty = parameters
                        .get(node.args.len() - i - 1)
                        .map(Cow::Borrowed)
                        .unwrap_or_else(|| {
                            if source.integer_info().is_some() {
                                Cow::Owned(source.promoted())
                            } else {
                                Cow::Borrowed(source.as_ref())
                            }
                        });
                    let value = Self::convert_integer(value, &source, &ty, options, &mut lines);
                    arguments.push(format!("{} {}", Self::abi_parameter_type(&ty), value));
                }
                arguments.reverse();
                let signature = if variadic {
                    format!(
                        " ({}, ...)",
                        parameters
                            .iter()
                            .map(Self::gen_type)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    String::new()
                };
                if *result == Type::Void {
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
                lines
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
                lines
            }
            _ => unreachable!("node dispatched to the wrong generator"),
        }
    }
}

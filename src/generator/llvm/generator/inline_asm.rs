use super::IrGenerator;
use super::Options;
use crate::node::{Node, render_inline_assembly};

impl IrGenerator<'_> {
    pub(super) fn gen_inline_assembly(
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
                    format!(
                        "{} {value}",
                        Self::gen_type(&output.resolve_type_view().unwrap())
                    ),
                ));
            }
        }
        let mut arguments = vec![];
        for input in &node.args {
            lines.extend(self.gen_node(input, options));
            let value = options.register_queue.pop().unwrap();
            arguments.push(format!(
                "{} {value}",
                Self::gen_type(&input.resolve_type_view().unwrap())
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
                operand.resolve_type_view()?.size_of(),
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
            .map(|output| Self::gen_type(&output.resolve_type_view().unwrap()))
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
}

use super::AsmGenerator;
use super::Options;
use crate::ctype::Type;
use crate::generator::aarch64::assembly::Assembly;
use crate::generator::aarch64::instruction::{
    InstOperand::{self, *},
    InstOperator::*,
    Register::*,
};
use crate::node::{Node, render_inline_assembly};

impl AsmGenerator<'_> {
    pub(super) fn gen_inline_assembly(&self, node: &Node, options: Options) -> Assembly {
        let registers = [X8, X9, X10, X11, X12, X13, X14];
        let operands = node.children.iter().chain(&node.args).collect::<Vec<_>>();
        let template = render_inline_assembly(&node.dest, |index, modifier| {
            let natural = operands.get(index)?.resolve_type()?.size_of();
            let width = crate::generator::asm_register_width(
                crate::generator::Arch::Aarch64,
                modifier,
                natural,
            )?;
            Some(format!(
                "{}{}",
                if width == 4 { "w" } else { "x" },
                8 + index
            ))
        })
        .unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "unsupported asm register modifier for AArch64",
            );
            unreachable!()
        });
        let mut lines = vec![];
        let matching = node
            .children
            .iter()
            .enumerate()
            .filter(|(index, _)| node.value.unwrap() & (1 << index) != 0)
            .collect::<Vec<_>>();
        for (_, input) in &matching {
            lines.push(self.gen_node(input, options));
        }
        for input in &node.args {
            lines.push(self.gen_node(input, options));
        }
        for (index, input) in node
            .args
            .iter()
            .enumerate()
            .rev()
            .map(|(index, input)| (index + node.children.len(), input))
            .chain(matching.into_iter().rev())
        {
            let register = registers[index];
            lines.push(Self::pop(register));
            if input.resolve_type().unwrap().size_of() == 4 {
                lines.push(Self::normalize_integer(&Type::integer(32, false), register));
            }
        }
        lines.push(Assembly::inst1(
            InlineAsm,
            InstOperand::InlineAsmTemplate(template),
        ));
        // Buffer every result before address generation can overwrite operand registers.
        for register in registers[..node.children.len()].iter().rev() {
            lines.extend([
                Assembly::inst3(SUB, X19, X19, 8),
                Assembly::inst3(AND, X20, X19, "#-16"),
                Assembly::inst2(MOV, SP, X20),
                Assembly::inst2(STR, *register, Ptr(X19, 8)),
            ]);
        }
        for output in &node.children {
            lines.push(self.gen_addr(output, options));
            lines.extend([
                Self::pop(X8),
                Self::pop(X13),
                Assembly::inst2(STR, X13, Ptr(X8, output.resolve_type().unwrap().size_of())),
            ]);
        }
        lines.push(Self::push(0));
        lines.into()
    }
}

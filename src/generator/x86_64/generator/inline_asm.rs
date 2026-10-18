use super::AsmGenerator;
use crate::ctype::Type;
use crate::generator::x86_64::assembly::Assembly;
use crate::generator::x86_64::instruction::{InstOperand::*, InstOperator::*, Register::*};
use crate::node::{Node, render_inline_assembly};

impl AsmGenerator<'_> {
    pub(super) fn gen_inline_assembly(
        &self,
        node: &Node,
        offset: usize,
        breakable_branch_num: usize,
    ) -> Assembly {
        let registers = [RAX, R10, R11, RCX, RDX, RSI, RDI];
        let names = [
            ["%al", "%ax", "%eax", "%rax"],
            ["%r10b", "%r10w", "%r10d", "%r10"],
            ["%r11b", "%r11w", "%r11d", "%r11"],
            ["%cl", "%cx", "%ecx", "%rcx"],
            ["%dl", "%dx", "%edx", "%rdx"],
            ["%sil", "%si", "%esi", "%rsi"],
            ["%dil", "%di", "%edi", "%rdi"],
        ];
        let operands = node.children.iter().chain(&node.args).collect::<Vec<_>>();
        let template = render_inline_assembly(&node.dest, |index, modifier| {
            let natural = operands.get(index)?.resolve_type()?.size_of();
            let width = crate::generator::asm_register_width(
                crate::generator::Arch::X86_64,
                modifier,
                natural,
            )?;
            Some(names.get(index)?[width.trailing_zeros() as usize].into())
        })
        .unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "unsupported asm register modifier for x86_64",
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
            lines.push(self.gen_node(input, offset, breakable_branch_num));
        }
        for input in &node.args {
            lines.push(self.gen_node(input, offset, breakable_branch_num));
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
            lines.push(Assembly::inst1(POP, register));
            if input.resolve_type().unwrap().size_of() == 4 {
                lines.push(Self::normalize_integer(&Type::integer(32, false), register));
            }
        }
        lines.push(Assembly::inst1(InlineAsm, InlineAsmTemplate(template)));
        // Buffer every result before address generation can overwrite operand registers.
        for register in registers[..node.children.len()].iter().rev() {
            lines.push(Assembly::inst1(PUSH, *register));
        }
        for output in &node.children {
            lines.push(self.gen_addr(output, offset, breakable_branch_num));
            lines.extend([
                Assembly::inst1(POP, RAX),
                Assembly::inst1(POP, RDI),
                self.operation2rdi(output.resolve_type(), MOV, RAX),
            ]);
        }
        lines.push(Assembly::inst1(PUSH, 0));
        lines.into()
    }
}

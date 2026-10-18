use super::IrGenerator;
use super::Options;
use crate::ctype::Type;
use crate::node::{AtomicOperation, AtomicRmwOperation, Node};

impl IrGenerator<'_> {
    pub(super) fn atomic_update(
        node: &Node,
        operation: AtomicRmwOperation,
        return_new: bool,
        values: &[String],
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        let object = node.args[0].dest_type_view().unwrap();
        let storage = Type::integer((object.size_of() * 8) as u8, false);
        let scalar = Self::gen_type(&storage);
        let value = Self::convert_integer(
            values[1].clone(),
            &node.args[1].resolve_type_view().unwrap(),
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

    pub(super) fn atomic_compare_exchange(
        node: &Node,
        expected_volatile: bool,
        values: &[String],
        options: &mut Options,
        lines: &mut Vec<String>,
    ) -> String {
        let object = node.args[0].dest_type_view().unwrap();
        let storage = Type::integer((object.size_of() * 8) as u8, false);
        let scalar = Self::gen_type(&storage);
        let desired = Self::convert_integer(
            values[2].clone(),
            &node.args[2].resolve_type_view().unwrap(),
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

    pub(super) fn gen_atomic(
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
        let ty = node.args[0].dest_type_view().unwrap();
        if let AtomicOperation::Update(op, return_new) = operation {
            let value = Self::atomic_update(node, op, return_new, &values, options, &mut lines);
            options.register_queue.push(value);
            return lines;
        }
        let scalar = Self::gen_type(&ty);
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
}

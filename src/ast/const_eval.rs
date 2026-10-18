//! Evaluate constant expressions and classify expressions eligible for folding.

use super::AstBuilderImpl;
use crate::{
    ctype::Type,
    node::{Node, NodeType},
};
use rustc_apfloat::Float;

impl AstBuilderImpl<'_> {
    pub(super) fn integer_constant(node: &Node) -> bool {
        match node.nt {
            NodeType::Num => node
                .resolve_type()
                .is_some_and(|ty| ty.integer_info().is_some()),
            NodeType::Cast
                if node
                    .resolve_type()
                    .is_some_and(|ty| ty.integer_info().is_some())
                    && node
                        .lhs
                        .as_ref()
                        .is_some_and(|child| child.nt == NodeType::Num) =>
            {
                true
            }
            NodeType::Cast => {
                node.resolve_type()
                    .is_some_and(|ty| ty.integer_info().is_some())
                    && node.lhs.as_deref().is_some_and(Self::integer_constant)
            }
            NodeType::BitNot => node.lhs.as_deref().is_some_and(Self::integer_constant),
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
            | NodeType::BitOr
            | NodeType::BitXor
            | NodeType::BitLeft
            | NodeType::BitRight
            | NodeType::LogicalAnd
            | NodeType::LogicalOr => {
                node.lhs
                    .as_ref()
                    .is_some_and(|child| Self::integer_constant(child))
                    && node
                        .rhs
                        .as_ref()
                        .is_some_and(|child| Self::integer_constant(child))
            }
            NodeType::Conditional => [&node.cond, &node.then, &node.els]
                .into_iter()
                .flatten()
                .all(|child| Self::integer_constant(child)),
            _ => false,
        }
    }

    pub(super) fn constant_address(&mut self, node: &Node) -> Option<(String, i64)> {
        match node.nt {
            NodeType::GlobalVar if node.thread_local => None,
            NodeType::GlobalVar
                if node.resolve_type().is_some_and(|ty| {
                    matches!(ty, Type::Arr(..) | Type::Func(..) | Type::VariadicFunc(..))
                }) =>
            {
                Some((
                    if node.dest.is_empty() {
                        node.global_name.clone()
                    } else {
                        node.dest.trim_start_matches('@').to_string()
                    },
                    0,
                ))
            }
            NodeType::Addr => {
                let child = node.lhs.as_ref().unwrap();
                if matches!(child.nt, NodeType::GlobalVar) && !child.thread_local {
                    Some((
                        if child.dest.is_empty() {
                            child.global_name.clone()
                        } else {
                            child.dest.trim_start_matches('@').to_string()
                        },
                        0,
                    ))
                } else if matches!(child.nt, NodeType::Deref) {
                    self.constant_address(child.lhs.as_ref().unwrap())
                } else {
                    None
                }
            }
            NodeType::Cast
                if node.resolve_type().is_some_and(|ty| {
                    matches!(ty.unqualified(), Type::Ptr(_))
                        || ty.integer_info().is_some_and(|(bits, _)| bits == 64)
                }) =>
            {
                self.constant_address(node.lhs.as_ref().unwrap())
            }
            NodeType::Add => {
                let lhs = node.lhs.as_ref().unwrap();
                let rhs = node.rhs.as_ref().unwrap();
                self.constant_address_offset(lhs, rhs, false)
                    .or_else(|| self.constant_address_offset(rhs, lhs, false))
            }
            NodeType::Sub => self.constant_address_offset(
                node.lhs.as_ref().unwrap(),
                node.rhs.as_ref().unwrap(),
                true,
            ),
            _ => None,
        }
    }

    fn constant_address_offset(
        &mut self,
        base: &Node,
        amount: &Node,
        subtract: bool,
    ) -> Option<(String, i64)> {
        let (symbol, offset) = self.constant_address(base)?;
        if !Self::integer_constant(amount) {
            return None;
        }
        let ty = base.resolve_type()?;
        let stride = if ty.is_pointer_value() {
            base.dest_type()?.size_of() as i64
        } else if ty.integer_info().is_some_and(|(bits, _)| bits == 64) {
            1
        } else {
            return None;
        };
        let delta = self.eval(amount).checked_mul(stride)?;
        Some((
            symbol,
            if subtract {
                offset.checked_sub(delta)?
            } else {
                offset.checked_add(delta)?
            },
        ))
    }

    pub(super) fn eval(&mut self, node: &Node) -> i64 {
        if node
            .resolve_type()
            .is_some_and(|ty| ty.float_bits().is_some())
        {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "integer constant expression required",
            );
        }
        if matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        ) && node
            .lhs
            .as_ref()
            .is_some_and(|lhs| lhs.resolve_type().unwrap().float_bits().is_some())
        {
            if node
                .lhs
                .as_ref()
                .unwrap()
                .resolve_type_view()
                .unwrap()
                .float_bits()
                == Some(80)
                || node
                    .rhs
                    .as_ref()
                    .unwrap()
                    .resolve_type_view()
                    .unwrap()
                    .float_bits()
                    == Some(80)
            {
                let lhs = self.eval_extended(node.lhs.as_ref().unwrap());
                let rhs = self.eval_extended(node.rhs.as_ref().unwrap());
                return i64::from(match node.nt {
                    NodeType::Eq => lhs == rhs,
                    NodeType::Ne => lhs != rhs,
                    NodeType::Lt => lhs < rhs,
                    NodeType::Le => lhs <= rhs,
                    _ => unreachable!(),
                });
            }
            let lhs = self.eval_float(node.lhs.as_ref().unwrap());
            let rhs = self.eval_float(node.rhs.as_ref().unwrap());
            return i64::from(match node.nt {
                NodeType::Eq => lhs == rhs,
                NodeType::Ne => lhs != rhs,
                NodeType::Lt => lhs < rhs,
                NodeType::Le => lhs <= rhs,
                _ => unreachable!(),
            });
        }
        match node.nt {
            NodeType::Num => return node.value.unwrap() as i64,
            NodeType::Cast => {
                let child = node.lhs.as_ref().unwrap();
                if child.resolve_type_view().unwrap().float_bits() == Some(80) {
                    let value = self.eval_extended(child);
                    let target = node.cty.as_ref().unwrap();
                    if *target.unqualified() == Type::Bool {
                        return i64::from(!value.is_zero());
                    }
                    if let Some((bits, signed)) = target.integer_info() {
                        return if signed {
                            value.to_i128(bits.into()).value as i64
                        } else {
                            value.to_u128(bits.into()).value as i64
                        };
                    }
                }
                let value = if child
                    .resolve_type()
                    .is_some_and(|ty| ty.float_bits().is_some())
                {
                    self.eval_float(child) as i64
                } else {
                    self.eval(child)
                };
                return match node.cty.as_ref().unwrap() {
                    ty if ty.integer_info().is_some() => ty.convert_constant(value),
                    Type::Ptr(_) => value,
                    _ => {
                        self.error_logger.print_error_position(
                            node.token.as_ref().unwrap().pos,
                            "cast is not a constant initializer",
                        );
                        unreachable!()
                    }
                };
            }

            NodeType::LogicalAnd => {
                return (self.eval_truth(node.lhs.as_ref().unwrap())
                    && self.eval_truth(node.rhs.as_ref().unwrap())) as i64;
            }
            NodeType::LogicalOr => {
                return (self.eval_truth(node.lhs.as_ref().unwrap())
                    || self.eval_truth(node.rhs.as_ref().unwrap())) as i64;
            }
            NodeType::Conditional => {
                return if self.eval_truth(node.cond.as_ref().unwrap()) {
                    self.eval(node.then.as_ref().unwrap())
                } else {
                    self.eval(node.els.as_ref().unwrap())
                };
            }
            NodeType::BitNot => {
                let value = !self.eval(node.lhs.as_ref().unwrap());
                return node.resolve_type().unwrap().convert_constant(value);
            }
            NodeType::Eq
            | NodeType::Ne
            | NodeType::Le
            | NodeType::Lt
            | NodeType::Add
            | NodeType::Sub
            | NodeType::Mul
            | NodeType::Div
            | NodeType::Mod
            | NodeType::BitAnd
            | NodeType::BitOr
            | NodeType::BitXor
            | NodeType::BitLeft
            | NodeType::BitRight => {}
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "initializer element is not a compile-time constant",
                );
                unreachable!()
            }
        }
        let lhs_node = node.lhs.as_ref().unwrap();
        let rhs_node = node.rhs.as_ref().unwrap();
        let lhs_ty = lhs_node.resolve_type().unwrap();
        let rhs_ty = rhs_node.resolve_type().unwrap();
        let ty = Type::arithmetic_type(&lhs_ty, &rhs_ty);
        let lhs = ty.convert_constant(self.eval(lhs_node));
        let rhs = ty.convert_constant(self.eval(rhs_node));
        let unsigned = ty.integer_info().is_some_and(|(_, signed)| !signed);
        if unsigned {
            let (lhs, rhs) = (lhs as u64, rhs as u64);
            let value = match node.nt {
                NodeType::Div if rhs != 0 => Some(lhs / rhs),
                NodeType::Mod if rhs != 0 => Some(lhs % rhs),
                NodeType::Lt => Some(u64::from(lhs < rhs)),
                NodeType::Le => Some(u64::from(lhs <= rhs)),
                NodeType::Add => Some(lhs.wrapping_add(rhs)),
                NodeType::Sub => Some(lhs.wrapping_sub(rhs)),
                NodeType::Mul => Some(lhs.wrapping_mul(rhs)),
                NodeType::BitRight if rhs < ty.integer_info().unwrap().0 as u64 => Some(lhs >> rhs),
                _ => None,
            };
            if let Some(value) = value {
                return ty.convert_constant(value as i64);
            }
        }
        let result = match node.nt {
            NodeType::Eq => Some((lhs == rhs) as i64),
            NodeType::Ne => Some((lhs != rhs) as i64),
            NodeType::Le => Some((lhs <= rhs) as i64),
            NodeType::Lt => Some((lhs < rhs) as i64),
            NodeType::Add => lhs.checked_add(rhs),
            NodeType::Sub => lhs.checked_sub(rhs),
            NodeType::Mul => lhs.checked_mul(rhs),
            NodeType::Div => lhs.checked_div(rhs),
            NodeType::Mod => lhs.checked_rem(rhs),
            NodeType::BitAnd => Some(lhs & rhs),
            NodeType::BitOr => Some(lhs | rhs),
            NodeType::BitXor => Some(lhs ^ rhs),
            NodeType::BitLeft
                if (0..lhs_ty.promoted().integer_info().unwrap().0 as i64).contains(&rhs) =>
            {
                lhs.checked_shl(rhs as u32)
            }
            NodeType::BitRight
                if (0..lhs_ty.promoted().integer_info().unwrap().0 as i64).contains(&rhs) =>
            {
                lhs.checked_shr(rhs as u32)
            }
            _ => None,
        };
        let value = result.unwrap_or_else(|| {
            self.error_logger.print_error_position(
                node.token.as_ref().unwrap().pos,
                "invalid arithmetic in constant initializer",
            );
            unreachable!()
        });
        if matches!(
            node.nt,
            NodeType::Eq | NodeType::Ne | NodeType::Lt | NodeType::Le
        ) {
            value
        } else {
            ty.convert_constant(value)
        }
    }

    fn eval_truth(&mut self, node: &Node) -> bool {
        match node.resolve_type_view().unwrap().float_bits() {
            Some(80) => !self.eval_extended(node).is_zero(),
            Some(_) => self.eval_float(node) != 0.0,
            None => self.eval(node) != 0,
        }
    }

    pub(super) fn eval_float(&mut self, node: &Node) -> f64 {
        let ty = node.resolve_type().unwrap();
        if ty.float_bits() == Some(80) {
            return crate::floating::to_double(self.eval_extended(node));
        }
        if node.nt == NodeType::Cast
            && ty.float_bits().is_some()
            && node
                .lhs
                .as_ref()
                .unwrap()
                .resolve_type_view()
                .unwrap()
                .float_bits()
                == Some(80)
        {
            return crate::floating::to_double(crate::floating::round(
                self.eval_extended(node.lhs.as_ref().unwrap()),
                ty.float_bits().unwrap(),
            ));
        }
        if ty.integer_info().is_some() {
            return if ty.integer_info().unwrap().1 {
                self.eval(node) as f64
            } else {
                self.eval(node) as u64 as f64
            };
        }
        let value = match node.nt {
            NodeType::Num => ty.float_value(node.value.unwrap()),
            NodeType::FloatAbs => self.eval_float_as(node.lhs.as_ref().unwrap(), &ty).abs(),
            NodeType::Cast => self.eval_float(node.lhs.as_ref().unwrap()),
            NodeType::Add => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    + self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Sub => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    - self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Mul => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    * self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Div => {
                self.eval_float_as(node.lhs.as_ref().unwrap(), &ty)
                    / self.eval_float_as(node.rhs.as_ref().unwrap(), &ty)
            }
            NodeType::Conditional => {
                if self.eval_truth(node.cond.as_ref().unwrap()) {
                    self.eval_float(node.then.as_ref().unwrap())
                } else {
                    self.eval_float(node.els.as_ref().unwrap())
                }
            }
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "floating initializer is not constant",
                );
                unreachable!()
            }
        };
        ty.float_value(ty.float_constant(value))
    }

    fn eval_float_as(&mut self, node: &Node, ty: &Type) -> f64 {
        let value = self.eval_float(node);
        ty.float_value(ty.float_constant(value))
    }

    pub(super) fn eval_extended(&mut self, node: &Node) -> crate::floating::Extended {
        use crate::floating::{Extended, from_double};
        let ty = node.resolve_type().unwrap();
        if let Some((_, signed)) = ty.integer_info() {
            let value = self.eval(node);
            return if signed {
                Extended::from_i128(value.into()).value
            } else {
                Extended::from_u128(value as u64 as u128).value
            };
        }
        if ty.float_bits() != Some(80) {
            return from_double(self.eval_float(node));
        }
        match node.nt {
            NodeType::Num => node.extended_float.as_ref().map_or_else(
                || from_double(f64::from_bits(node.value.unwrap() as u64)),
                |bits| Extended::from_bits(**bits),
            ),
            NodeType::Cast => self.eval_extended(node.lhs.as_ref().unwrap()),
            NodeType::FloatAbs => self.eval_extended(node.lhs.as_ref().unwrap()).abs(),
            NodeType::Add | NodeType::Sub | NodeType::Mul | NodeType::Div => {
                let lhs = self.eval_extended(node.lhs.as_ref().unwrap());
                let rhs = self.eval_extended(node.rhs.as_ref().unwrap());
                match node.nt {
                    NodeType::Add => (lhs + rhs).value,
                    NodeType::Sub => (lhs - rhs).value,
                    NodeType::Mul => (lhs * rhs).value,
                    NodeType::Div => (lhs / rhs).value,
                    _ => unreachable!(),
                }
            }
            NodeType::Conditional => {
                let condition = node.cond.as_ref().unwrap();
                let nonzero = if condition.resolve_type_view().unwrap().float_bits() == Some(80) {
                    !self.eval_extended(condition).is_zero()
                } else {
                    self.eval_float(condition) != 0.0
                };
                self.eval_extended(if nonzero {
                    node.then.as_ref().unwrap()
                } else {
                    node.els.as_ref().unwrap()
                })
            }
            _ => {
                self.error_logger.print_error_position(
                    node.token.as_ref().unwrap().pos,
                    "floating initializer is not constant",
                );
                unreachable!()
            }
        }
    }

    pub(super) fn foldable_integer_constant(&mut self, node: &Node) -> bool {
        if !Self::integer_constant(node) {
            return false;
        }
        if node.nt == NodeType::Cast
            && node
                .lhs
                .as_ref()
                .is_some_and(|child| child.nt == NodeType::Num)
        {
            return true;
        }
        if [&node.lhs, &node.rhs, &node.cond, &node.then, &node.els]
            .into_iter()
            .flatten()
            .any(|child| !self.foldable_integer_constant(child))
        {
            return false;
        }
        if matches!(node.nt, NodeType::Div | NodeType::Mod) {
            let right = self.eval(node.rhs.as_ref().unwrap());
            if right == 0 {
                return false;
            }
            let ty = node.resolve_type().unwrap();
            let (bits, signed) = ty.integer_info().unwrap();
            if signed
                && right == -1
                && i128::from(self.eval(node.lhs.as_ref().unwrap())) == -(1i128 << (bits - 1))
            {
                return false;
            }
        }
        if matches!(node.nt, NodeType::BitLeft | NodeType::BitRight) {
            let shift = self.eval(node.rhs.as_ref().unwrap());
            let bits = node.resolve_type().unwrap().integer_info().unwrap().0;
            if shift < 0 || shift >= i64::from(bits) {
                return false;
            }
        }
        true
    }
}

use super::{Arch, Os};
use crate::ctype::Type;
use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct Part {
    pub offset: usize,
    pub ty: Type,
}
#[derive(Clone, Copy, Debug)]
pub enum Location {
    General(usize),
    Floating(usize),
    Stack(usize),
}
#[derive(Clone, Debug)]
pub struct Argument {
    pub parts: Vec<(Part, Location)>,
    pub indirect: bool,
    pub memory: Option<usize>,
    pub stack_alignment: usize,
}
#[derive(Clone, Debug)]
pub struct Signature {
    pub arguments: Vec<Argument>,
    pub result: Vec<Part>,
    pub indirect_result: bool,
    pub stack_size: usize,
    pub floating: usize,
    pub general: usize,
}

fn floating_members(ty: &Type, offset: usize, members: &mut Vec<Part>) -> bool {
    match ty.unqualified() {
        Type::Float(_) | Type::LongDouble(_) => {
            members.push(Part {
                offset,
                ty: Type::Float(ty.float_bits().unwrap()),
            });
            true
        }
        Type::Arr(child, count) => (0..count.unwrap_or(0))
            .all(|index| floating_members(child, offset + index * child.size_of(), members)),
        Type::Struct(st) if !st.has_bit_fields() && st.definition().borrow().is_union => {
            let mut largest: Vec<Part> = vec![];
            for member in &st.definition().borrow().members {
                let mut candidate = vec![];
                if !floating_members(&member.ty, offset, &mut candidate) || candidate.is_empty() {
                    return false;
                }
                if !largest.is_empty() && candidate.iter().any(|part| part.ty != largest[0].ty) {
                    return false;
                }
                if candidate.len() > largest.len() {
                    largest = candidate;
                }
            }
            members.extend(largest);
            true
        }
        Type::Struct(st) if !st.has_bit_fields() && !st.definition().borrow().is_union => st
            .definition()
            .borrow()
            .members
            .iter()
            .all(|member| floating_members(&member.ty, offset + member.offset, members)),
        _ => false,
    }
}
fn integer_slots(ty: &Type, offset: usize, slots: &mut [Option<bool>]) {
    match ty.unqualified() {
        Type::Float(_) | Type::LongDouble(_) => {
            for slot in slots
                .iter_mut()
                .take((offset + ty.size_of()).div_ceil(8))
                .skip(offset / 8)
            {
                if slot.is_none() {
                    *slot = Some(false);
                }
            }
        }
        Type::Struct(st) if st.definition().borrow().members.is_empty() && st.has_bit_fields() => {
            slots.fill(Some(true))
        }
        Type::Struct(st) => {
            for member in &st.definition().borrow().members {
                if let Some(field) = member.bit_field {
                    let start = (offset + member.offset) * 8 + usize::from(field.shift);
                    for slot in slots
                        .iter_mut()
                        .take((start + usize::from(field.width)).div_ceil(64))
                        .skip(start / 64)
                    {
                        *slot = Some(true);
                    }
                } else {
                    integer_slots(&member.ty, offset + member.offset, slots);
                }
            }
        }
        Type::Arr(child, count) => {
            for index in 0..count.unwrap_or(0) {
                integer_slots(child, offset + index * child.size_of(), slots);
            }
        }
        _ => {
            for slot in slots
                .iter_mut()
                .take((offset + ty.size_of()).div_ceil(8))
                .skip(offset / 8)
            {
                *slot = Some(true);
            }
        }
    }
}
fn has_unaligned_member(ty: &Type, offset: usize) -> bool {
    match ty.unqualified() {
        Type::Struct(record) => record.definition().borrow().members.iter().any(|member| {
            let offset = offset + member.offset;
            member.bit_field.is_none()
                && (offset % member.ty.align_of() != 0 || has_unaligned_member(&member.ty, offset))
        }),
        Type::Arr(child, count) => (0..count.unwrap_or(0))
            .any(|index| has_unaligned_member(child, offset + index * child.size_of())),
        _ => false,
    }
}

pub fn aggregate_parts(ty: &Type, arch: &Arch) -> Vec<Part> {
    if matches!(arch, Arch::X86_64) && ty.float_bits() == Some(80) {
        return vec![];
    }
    if let Type::Arr(child, _) = ty.unqualified() {
        return vec![Part {
            offset: 0,
            ty: Type::Ptr(child.clone()),
        }];
    }
    if matches!(ty.unqualified(), Type::Func(..) | Type::VariadicFunc(..)) {
        return vec![Part {
            offset: 0,
            ty: Type::Ptr(Box::new(ty.clone())),
        }];
    }
    if !matches!(ty.unqualified(), Type::Struct(_)) {
        return vec![Part {
            offset: 0,
            ty: ty.clone(),
        }];
    }
    // Flexible tails force SysV memory classification and prevent AAPCS64 HFAs.
    if matches!(arch, Arch::X86_64) && (ty.has_flexible_array() || has_unaligned_member(ty, 0)) {
        return vec![];
    }
    if matches!(arch, Arch::Aarch64) && !ty.has_flexible_array() {
        let mut members = vec![];
        if floating_members(ty, 0, &mut members)
            && !members.is_empty()
            && members.len() <= 4
            && ty.size_of() == members.len() * members[0].ty.size_of()
            && members.iter().all(|part| part.ty == members[0].ty)
        {
            return members;
        }
    }
    if ty.size_of() > 16 {
        return vec![];
    }
    let mut integers = vec![None; ty.size_of().div_ceil(8)];
    integer_slots(ty, 0, &mut integers);
    integers
        .into_iter()
        .enumerate()
        .filter_map(|(index, integer)| {
            if integer.is_none() && matches!(arch, Arch::X86_64) {
                return None;
            }
            let size = (ty.size_of() - index * 8).min(8);
            Some(Part {
                offset: index * 8,
                ty: if integer == Some(true) || matches!(arch, Arch::Aarch64) {
                    Type::integer(
                        if matches!(arch, Arch::Aarch64) {
                            64
                        } else {
                            (size * 8) as u8
                        },
                        false,
                    )
                } else {
                    Type::Float(if size <= 4 { 32 } else { 64 })
                },
            })
        })
        .collect()
}
/// Alignment in eight-byte slots for direct general-register arguments.
pub fn general_register_alignment(ty: &Type, arch: &Arch, os: Os) -> usize {
    if matches!((arch, os), (Arch::Aarch64, Os::Linux))
        && matches!(ty.unqualified(), Type::Struct(_))
        && ty.natural_align_of() >= 16
    {
        2
    } else {
        1
    }
}

pub fn stack_argument_alignment(ty: &Type, arch: &Arch, os: Os, variadic: bool) -> usize {
    if matches!(arch, Arch::Aarch64) && matches!(ty.unqualified(), Type::Struct(_)) {
        let parts = aggregate_parts(ty, arch);
        if parts.is_empty() {
            return 8;
        }
        if matches!(os, Os::Linux) {
            return ty.natural_align_of().max(8);
        }
        if !variadic && parts[0].ty.float_bits().is_some() {
            return parts[0].ty.align_of();
        }
    }
    // Scalar typedef storage alignment does not change its by-value ABI alignment.
    let alignment = ty.unqualified().align_of();
    if matches!((arch, os), (Arch::Aarch64, Os::MacOS)) && !variadic {
        alignment
    } else {
        alignment.max(8)
    }
}

pub fn signature(
    types: &[impl std::borrow::Borrow<Type>],
    result: &Type,
    arch: Arch,
    os: Os,
    fixed: usize,
    variadic: bool,
) -> Signature {
    let result_parts = aggregate_parts(result, &arch);
    let indirect_result = matches!(result.unqualified(), Type::Struct(_))
        && result.size_of() != 0
        && result_parts.is_empty();
    let mut general = usize::from(indirect_result && matches!(arch, Arch::X86_64));
    let mut floating = 0;
    let mut stack = 0usize;
    let mut arguments = vec![];
    let general_limit = if matches!(arch, Arch::X86_64) { 6 } else { 8 };
    for (index, source) in types.iter().enumerate() {
        let source: &Type = std::borrow::Borrow::borrow(source);
        let effective = match source {
            Type::Arr(child, _) => Cow::Owned(Type::Ptr(child.clone())),
            Type::Func(..) | Type::VariadicFunc(..) => {
                Cow::Owned(Type::Ptr(Box::new(source.clone())))
            }
            _ => Cow::Borrowed(source),
        };
        let ty = effective.as_ref();
        let aggregate = matches!(ty.unqualified(), Type::Struct(_));
        if aggregate && ty.size_of() == 0 {
            // Empty GNU records have no argument payload or register/stack allocation.
            arguments.push(Argument {
                parts: vec![],
                indirect: false,
                memory: None,
                stack_alignment: ty.align_of(),
            });
            continue;
        }
        let mut parts = aggregate_parts(ty, &arch);
        let indirect = aggregate && parts.is_empty() && matches!(arch, Arch::Aarch64);
        if indirect {
            parts = vec![Part {
                offset: 0,
                ty: Type::Ptr(Box::new(ty.clone())),
            }];
        }
        let gp = parts
            .iter()
            .filter(|part| part.ty.float_bits().is_none())
            .count();
        let fp = parts.len() - gp;
        let forced_stack =
            matches!((&arch, os), (Arch::Aarch64, Os::MacOS)) && variadic && index >= fixed;
        if !indirect && gp > 0 {
            let alignment = general_register_alignment(ty, &arch, os);
            general = general.div_ceil(alignment) * alignment;
        }
        let stack_alignment = stack_argument_alignment(ty, &arch, os, variadic && index >= fixed);
        let registers = !forced_stack
            && !parts.is_empty()
            && general + gp <= general_limit
            && floating + fp <= 8;
        if registers {
            let parts = parts
                .into_iter()
                .map(|part| {
                    let location = if part.ty.float_bits().is_some() {
                        let n = floating;
                        floating += 1;
                        Location::Floating(n)
                    } else {
                        let n = general;
                        general += 1;
                        Location::General(n)
                    };
                    (part, location)
                })
                .collect();
            arguments.push(Argument {
                parts,
                indirect,
                memory: None,
                stack_alignment,
            });
        } else {
            if aggregate && matches!(arch, Arch::Aarch64) && !indirect {
                if fp > 0 {
                    floating = 8;
                } else {
                    general = general_limit;
                }
            }
            let compact = matches!((&arch, os), (Arch::Aarch64, Os::MacOS)) && index < fixed;
            let size = if indirect {
                8
            } else if aggregate || compact || ty.float_bits() == Some(80) {
                ty.size_of()
            } else {
                8
            };
            let align = stack_alignment;
            stack = stack.div_ceil(align) * align;
            let start = stack;
            stack += if compact { size } else { size.div_ceil(8) * 8 };
            arguments.push(Argument {
                parts: vec![(
                    Part {
                        offset: 0,
                        ty: if indirect {
                            Type::Ptr(Box::new(ty.clone()))
                        } else {
                            ty.clone()
                        },
                    },
                    Location::Stack(start),
                )],
                indirect,
                memory: aggregate.then_some(start).filter(|_| !indirect),
                stack_alignment,
            });
        }
    }
    Signature {
        arguments,
        result: result_parts,
        indirect_result,
        stack_size: stack,
        floating,
        general,
    }
}

pub fn variadic_state(node: &crate::node::Node, arch: Arch, os: Os) -> (usize, usize, usize) {
    let result = node.rhs.as_ref().unwrap().resolve_type().unwrap();
    let signature = signature(
        &node.parameter_types,
        &result,
        arch,
        os,
        node.parameter_types.len(),
        true,
    );
    (
        signature.general,
        signature.floating,
        signature.stack_size.div_ceil(8) * 8,
    )
}

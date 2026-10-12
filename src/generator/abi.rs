use super::{Arch, Os};
use crate::ctype::Type;

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
        Type::Float(_) => {
            members.push(Part {
                offset,
                ty: ty.clone(),
            });
            true
        }
        Type::Arr(child, count) => (0..*count)
            .all(|index| floating_members(child, offset + index * child.size_of(), members)),
        Type::Struct(st) if !st.has_bit_fields() && st.definition().borrow().is_union => {
            let mut largest: Vec<Part> = vec![];
            for member in st.members() {
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
            .members()
            .iter()
            .all(|member| floating_members(&member.ty, offset + member.offset, members)),
        _ => false,
    }
}
fn integer_slots(ty: &Type, offset: usize, slots: &mut [bool]) {
    match ty.unqualified() {
        Type::Float(_) => {}
        Type::Struct(st) if st.members().is_empty() && st.has_bit_fields() => slots.fill(true),
        Type::Struct(st) => {
            for member in st.members() {
                integer_slots(&member.ty, offset + member.offset, slots);
            }
        }
        Type::Arr(child, count) => {
            for index in 0..*count {
                integer_slots(child, offset + index * child.size_of(), slots);
            }
        }
        _ => {
            for slot in slots
                .iter_mut()
                .take((offset + ty.size_of()).div_ceil(8))
                .skip(offset / 8)
            {
                *slot = true;
            }
        }
    }
}
pub fn aggregate_parts(ty: &Type, arch: &Arch) -> Vec<Part> {
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
    if matches!(arch, Arch::Aarch64) {
        let mut members = vec![];
        if floating_members(ty, 0, &mut members)
            && !members.is_empty()
            && members.len() <= 4
            && members.iter().all(|part| part.ty == members[0].ty)
        {
            return members;
        }
    }
    if ty.size_of() > 16 {
        return vec![];
    }
    let mut integers = vec![false; ty.size_of().div_ceil(8)];
    integer_slots(ty, 0, &mut integers);
    integers
        .into_iter()
        .enumerate()
        .map(|(index, integer)| {
            let size = (ty.size_of() - index * 8).min(8);
            Part {
                offset: index * 8,
                ty: if integer || matches!(arch, Arch::Aarch64) {
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
            }
        })
        .collect()
}
pub fn signature(
    types: &[Type],
    result: &Type,
    arch: Arch,
    os: Os,
    fixed: usize,
    variadic: bool,
) -> Signature {
    let result_parts = aggregate_parts(result, &arch);
    let indirect_result =
        matches!(result.unqualified(), Type::Struct(_)) && result_parts.is_empty();
    let mut general = usize::from(indirect_result && matches!(arch, Arch::X86_64));
    let mut floating = 0;
    let mut stack = 0usize;
    let mut arguments = vec![];
    let general_limit = if matches!(arch, Arch::X86_64) { 6 } else { 8 };
    for (index, source) in types.iter().enumerate() {
        let effective = match source {
            Type::Arr(child, _) => Type::Ptr(child.clone()),
            Type::Func(..) | Type::VariadicFunc(..) => Type::Ptr(Box::new(source.clone())),
            _ => source.clone(),
        };
        let ty = &effective;
        let aggregate = matches!(ty.unqualified(), Type::Struct(_));
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
            } else if aggregate || compact {
                ty.size_of()
            } else {
                8
            };
            let align = if compact {
                ty.align_of()
            } else {
                ty.align_of().max(8)
            };
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

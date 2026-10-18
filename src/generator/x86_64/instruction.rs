use crate::generator::Os;

pub enum InstOperator {
    // Data movement and memory operations.
    MOV,
    MOVSX,
    MOVSXD,
    MOVZX,
    LEA,
    PUSH,
    POP,
    REPSTOSB,
    REPMOVSB,

    // Integer operations.
    ADD,
    SUB,
    IMUL,
    IDIV,
    DIV,
    CQO,
    AND,
    OR,
    XOR,
    NOT,
    SHL,
    SHR,
    SAR,
    BSF,
    BSR,

    // Comparisons and control flow.
    CMP,
    SETE,
    SETNE,
    SETL,
    SETLE,
    SETB,
    SETBE,
    SETNP,
    SETP,
    CMOVE,
    JMP,
    JE,
    JNE,
    JGE,
    CALL,
    RET,

    // Floating-point operations and conversions.
    MOVD,
    MOVQ,
    ADDSS,
    ADDSD,
    SUBSS,
    SUBSD,
    MULSS,
    MULSD,
    DIVSS,
    DIVSD,
    UCOMISS,
    UCOMISD,
    CVTSS2SD,
    CVTSD2SS,
    CVTSI2SS,
    CVTSI2SD,
    CVTTSS2SI,
    CVTTSD2SI,

    // Atomic operations, prefetching and inline assembly.
    CMPXCHG,
    XCHG,
    MFENCE,
    PREFETCHNTA,
    PREFETCHT0,
    PREFETCHT1,
    PREFETCHT2,
    InlineAsm,
}

use InstOperator::*;
use std::fmt::{Debug, Display, Error, Formatter};

impl InstOperator {
    pub fn to_string(&self, _target_os: Os) -> &str {
        // if matches!(target_os, Os::Linux) && matches!(self, MOVZX) {
        //     return "movzb";
        // }
        match self {
            PREFETCHNTA => "prefetchnta",
            PREFETCHT0 => "prefetcht0",
            PREFETCHT1 => "prefetcht1",
            PREFETCHT2 => "prefetcht2",
            InlineAsm => "",
            CMPXCHG => "lock cmpxchg",
            MFENCE => "mfence",
            XCHG => "xchg",
            MOVD => "movd",
            MOVQ => "movq",
            ADDSS => "addss",
            ADDSD => "addsd",
            SUBSS => "subss",
            SUBSD => "subsd",
            MULSS => "mulss",
            MULSD => "mulsd",
            DIVSS => "divss",
            DIVSD => "divsd",
            UCOMISS => "ucomiss",
            UCOMISD => "ucomisd",
            CVTSS2SD => "cvtss2sd",
            CVTSD2SS => "cvtsd2ss",
            CVTSI2SD => "cvtsi2sd",
            CVTSI2SS => "cvtsi2ss",
            CVTTSD2SI => "cvttsd2si",
            CVTTSS2SI => "cvttss2si",
            SETNP => "setnp",
            SETP => "setp",
            PUSH => "push",
            POP => "pop",
            REPSTOSB => "rep stosb",
            REPMOVSB => "rep movsb",
            BSF => "bsf",
            BSR => "bsr",
            CMOVE => "cmove",
            MOV => "mov",
            ADD => "add",
            SUB => "sub",
            RET => "ret",
            CALL => "call",
            CQO => "cqo",
            JMP => "jmp",
            JE => "je",
            JNE => "jne",
            JGE => "jge",
            IMUL => "imul",
            IDIV => "idiv",
            DIV => "div",
            SHR => "shr",
            SETB => "setb",
            SETBE => "setbe",
            SHL => "shl",
            SAR => "sar",
            NOT => "not",
            SETE => "sete",
            SETNE => "setne",
            SETL => "setl",
            SETLE => "setle",
            LEA => "lea",
            MOVSX => "movsx",
            MOVSXD => "movsxd",
            MOVZX => "movzx",
            CMP => "cmp",
            AND => "and",
            OR => "or",
            XOR => "xor",
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Register {
    // General registers and their available subregisters.
    RAX,
    EAX,
    AX,
    AL,
    RBX,
    RCX,
    ECX,
    CX,
    CL,
    RDX,
    DL,
    RSI,
    RDI,
    EDI,
    DI,
    DIL,
    R8,
    R9,
    R10,
    R11,

    // Stack, frame and instruction pointers.
    RSP,
    RBP,
    RIP,

    Xmm(usize),
}
use Register::*;

impl Display for Register {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        if let Self::Xmm(index) = self {
            return write!(f, "xmm{index}");
        }
        f.write_str(match self {
            Self::Xmm(_) => unreachable!(),
            R11 => "r11",
            DL => "dl",

            RAX => "rax",
            EAX => "eax",
            RBX => "rbx",
            RCX => "rcx",
            ECX => "ecx",
            CX => "cx",
            RDX => "rdx",
            RSI => "rsi",
            RDI => "rdi",
            RBP => "rbp",
            RSP => "rsp",
            EDI => "edi",
            DI => "di",
            DIL => "dil",
            AX => "ax",
            AL => "al",
            CL => "cl",
            RIP => "rip",
            R8 => "r8",
            R9 => "r9",
            R10 => "r10",
        })?;
        Ok(())
    }
}

pub enum InstOperand {
    Reg(Register),
    Num(usize),
    Str(&'static str),
    Label(String),

    Ptr(Register, usize),
    PtrOffset(Register, usize, usize),
    PtrAdd(Register, String),

    BeginFlag(usize),
    ElseFlag(usize),
    EndFlag(usize),
    InlineAsmTemplate(String),
}

impl Display for InstOperand {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.write_str(&match self {
            InstOperand::InlineAsmTemplate(template) => template.clone(),
            InstOperand::Reg(r) => format!("{}", r),
            InstOperand::Num(i) => format!("{}", i),
            InstOperand::Label(l) => l.clone(),
            InstOperand::Str(s) => String::from(*s),
            InstOperand::PtrOffset(r, offset, size) => format!(
                "{} ptr[{} + {}]",
                match size {
                    1 => "byte",
                    2 => "word",
                    4 => "dword",
                    8 => "qword",
                    _ => unreachable!(),
                },
                r,
                offset
            ),
            InstOperand::Ptr(r, i) => match i {
                1 => format!("byte ptr[{}]", r),
                2 => format!("word ptr[{}]", r),
                4 => format!("dword ptr[{}]", r),
                8 => format!("qword ptr[{}]", r),
                _ => unreachable!(),
            },
            InstOperand::ElseFlag(i) => format!(".Lelse{}", i),
            InstOperand::BeginFlag(i) => format!(".Lbegin{}", i),
            InstOperand::EndFlag(i) => format!(".Lend{}", i),
            InstOperand::PtrAdd(s, r) => format!("[{} + {}]", s, r),
        })
    }
}

impl From<String> for InstOperand {
    fn from(val: String) -> Self {
        InstOperand::Label(val)
    }
}

impl From<&'static str> for InstOperand {
    fn from(val: &'static str) -> Self {
        InstOperand::Str(val)
    }
}

impl From<usize> for InstOperand {
    fn from(val: usize) -> Self {
        InstOperand::Num(val)
    }
}

impl From<Register> for InstOperand {
    fn from(val: Register) -> Self {
        InstOperand::Reg(val)
    }
}

pub struct Instruction {
    pub operator: InstOperator,
    pub operand1: Option<InstOperand>,
    pub operand2: Option<InstOperand>,
}
impl Instruction {
    pub fn to_string(&self, target_os: Os) -> String {
        if let (InstOperator::InlineAsm, Some(InstOperand::InlineAsmTemplate(template))) =
            (&self.operator, &self.operand1)
        {
            return format!("  .att_syntax prefix\n{template}\n  .intel_syntax noprefix");
        }
        if self.operand2.is_some() {
            return format!(
                "  {} {}, {}",
                self.operator.to_string(target_os),
                &self.operand1.as_ref().unwrap().to_string()[..],
                &self.operand2.as_ref().unwrap().to_string()[..]
            );
        }
        if self.operand1.is_some() {
            return format!(
                "  {} {}",
                self.operator.to_string(target_os),
                &self.operand1.as_ref().unwrap().to_string()[..],
            );
        }
        format!("  {}", self.operator.to_string(target_os))
    }
}

use crate::generator::Os;

pub enum InstOperator {
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
    CVTSI2SD,
    CVTSI2SS,
    CVTTSD2SI,
    CVTTSS2SI,
    SETNP,
    SETP,

    PUSH,
    POP,
    MOV,
    ADD,
    SUB,
    RET,
    CALL,
    CQO,
    JMP,
    JE,
    JNE,
    JGE,
    IMUL,
    IDIV,
    DIV,
    SHR,
    SETB,
    SETBE,
    SHL,
    SAR,
    NOT,
    SETE,
    SETNE,
    SETL,
    SETLE,
    LEA,
    MOVSX,
    MOVSXD,
    MOVZX,
    CMP,
    AND,
    OR,
    XOR,
}

use InstOperator::*;
use std::fmt::{Debug, Display, Error, Formatter};

impl InstOperator {
    pub fn to_string(&self, _target_os: Os) -> &str {
        // if matches!(target_os, Os::Linux) && matches!(self, MOVZX) {
        //     return "movzb";
        // }
        match self {
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
    Xmm(usize),
    R11,
    DL,

    RAX,
    EAX,
    RBX,
    RCX,
    RDX,
    RSI,
    RDI,
    RBP,
    RSP,
    EDI,
    DI,
    DIL,
    AL,
    CL,
    RIP,
    R8,
    R9,
    R10,
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
            RDX => "rdx",
            RSI => "rsi",
            RDI => "rdi",
            RBP => "rbp",
            RSP => "rsp",
            EDI => "edi",
            DI => "di",
            DIL => "dil",
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
    Label(String),
    Str(&'static str),
    Ptr(Register, usize),
    PtrAdd(Register, String),
    ElseFlag(usize),
    BeginFlag(usize),
    EndFlag(usize),
}

impl Display for InstOperand {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.write_str(&match self {
            InstOperand::Reg(r) => format!("{}", r),
            InstOperand::Num(i) => format!("{}", i),
            InstOperand::Label(l) => l.clone(),
            InstOperand::Str(s) => String::from(*s),
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

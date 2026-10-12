use crate::generator::Os;

#[derive(Default)]
pub enum InstOperator {
    MOV,
    CSET,
    SXTB,
    SXTW,
    MVN,
    ADD,
    SUB,
    RET,
    BL,
    BLR,
    SVC,
    CQO,
    JMP,
    JE,
    JNE,
    MUL,
    SDIV,
    UDIV,
    SHR,
    LDRU,
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
    LDR,
    STR,
    ADRP,
    STP,
    LDP,
    MSUB,
    #[default]
    NOP,
}

use InstOperator::*;
use std::fmt::{Debug, Display, Error, Formatter};

impl InstOperator {
    pub fn to_string(&self, target_os: Os) -> &str {
        if matches!(target_os, Os::Linux) && matches!(self, MOVZX) {
            return "movzb";
        }
        match self {
            MOV => "mov",
            CSET => "cset",
            SXTB => "sxtb",
            SXTW => "sxtw",
            MVN => "mvn",
            ADD => "add",
            SUB => "sub",
            RET => "ret",
            BL => "bl",
            BLR => "blr",
            CQO => "cqo",
            SVC => "svc",
            JMP => "b",
            JE => "b.eq",
            JNE => "b.ne",
            MUL => "mul",
            SDIV => "sdiv",
            UDIV => "udiv",
            SHR => "lsr",
            LDRU => "ldr",
            MSUB => "msub",
            SHL => "lsl",
            SAR => "asr",
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
            OR => "orr",
            XOR => "eor",
            LDR => "ldr",
            STR => "str",
            ADRP => "adrp",
            STP => "stp",
            LDP => "ldp",
            NOP => "nop",
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Register {
    X0,
    W0,
    X1,
    X2,
    X3,
    X4,
    X5,
    X6,
    X7,
    X8,
    W8,
    X9,
    X10,
    X11,
    X12,
    X13,
    X14,
    X15,
    X16,
    X17,
    X19,
    X20,
    X29,
    X30,
    AL,
    CL,
    RIP,
    R8,
    R9,
    SP,
}
use Register::*;

impl Display for Register {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.write_str(match self {
            X0 => "x0",
            W0 => "w0",
            X1 => "x1",
            X2 => "x2",
            X3 => "x3",
            X4 => "x4",
            X5 => "x5",
            X6 => "x6",
            X7 => "x7",
            X8 => "x8",
            W8 => "w8",
            X9 => "x9",
            X10 => "x10",
            X11 => "x11",
            X12 => "x12",
            X13 => "x13",
            X14 => "x14",
            X15 => "x15",
            X16 => "x16",
            X17 => "x17",
            X19 => "x19",
            X20 => "x20",
            X29 => "x29",
            X30 => "x30",
            AL => "al",
            CL => "cl",
            RIP => "rip",
            R8 => "r8",
            R9 => "r9",
            SP => "sp",
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
            InstOperand::Num(i) => format!("#{}", i),
            InstOperand::Label(l) => l.clone(),
            InstOperand::Str(s) => String::from(*s),
            InstOperand::Ptr(r, _) => format!("[{}]", r),
            InstOperand::ElseFlag(i) => format!(".Lelse{}", i),
            InstOperand::BeginFlag(i) => format!(".Lbegin{}", i),
            InstOperand::EndFlag(i) => format!(".Lend{}", i),
            InstOperand::PtrAdd(s, r) => format!("[{}, {}]", s, r),
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
#[derive(Default)]
pub struct Instruction {
    pub operator: InstOperator,
    pub operand1: Option<InstOperand>,
    pub operand2: Option<InstOperand>,
    pub operand3: Option<InstOperand>,
    pub operand4: Option<InstOperand>,
}
impl Instruction {
    pub fn to_string(&self, target_os: Os) -> String {
        if let (Some(destination), Some(base), Some(InstOperand::Num(value))) =
            (&self.operand1, &self.operand2, &self.operand3)
        {
            if matches!(self.operator, ADD | SUB) && *value > 4095 {
                // The generator reserves X16 for materializing large immediates.
                let immediate = Instruction {
                    operator: MOV,
                    operand1: Some(InstOperand::Reg(X16)),
                    operand2: Some(InstOperand::Num(*value)),
                    ..Self::default()
                };
                return format!(
                    "{}\n  {} {}, {}, x16",
                    immediate.to_string(target_os),
                    self.operator.to_string(target_os),
                    destination,
                    base
                );
            }
        }
        if let (MOV, Some(InstOperand::Reg(register)), Some(InstOperand::Num(value))) =
            (&self.operator, &self.operand1, &self.operand2)
        {
            let mut lines = vec![format!("  movz {}, #{}", register, value & 0xffff)];
            for shift in [16, 32, 48] {
                let part = (value >> shift) & 0xffff;
                if part != 0 {
                    lines.push(format!("  movk {}, #{}, lsl #{}", register, part, shift));
                }
            }
            return lines.join("\n");
        }
        if let (Some(InstOperand::Reg(register)), Some(InstOperand::Ptr(base, size))) =
            (&self.operand1, &self.operand2)
        {
            if matches!(self.operator, LDR | LDRU | STR) {
                let (operator, operand) = match (&self.operator, size) {
                    (LDR, 1) => ("ldrsb", register.to_string()),
                    (LDR, 2) => ("ldrsh", register.to_string()),
                    (LDRU, 1) => ("ldrb", register.to_string().replacen('x', "w", 1)),
                    (LDRU, 2) => ("ldrh", register.to_string().replacen('x', "w", 1)),
                    (LDRU, 4) => ("ldr", register.to_string().replacen('x', "w", 1)),
                    (STR, 2) => ("strh", register.to_string().replacen('x', "w", 1)),
                    (LDR, 4) => ("ldrsw", register.to_string()),
                    (STR, 1) => ("strb", register.to_string().replacen('x', "w", 1)),
                    (STR, 4) => ("str", register.to_string().replacen('x', "w", 1)),
                    _ => (self.operator.to_string(target_os), register.to_string()),
                };
                return format!("  {} {}, [{}]", operator, operand, base);
            }
        }
        if self.operand4.is_some() {
            return format!(
                "  {} {}, {}, {}, {}",
                self.operator.to_string(target_os),
                &self.operand1.as_ref().unwrap().to_string()[..],
                &self.operand2.as_ref().unwrap().to_string()[..],
                &self.operand3.as_ref().unwrap().to_string()[..],
                &self.operand4.as_ref().unwrap().to_string()[..],
            );
        }
        if self.operand3.is_some() {
            return format!(
                "  {} {}, {}, {}",
                self.operator.to_string(target_os),
                &self.operand1.as_ref().unwrap().to_string()[..],
                &self.operand2.as_ref().unwrap().to_string()[..],
                &self.operand3.as_ref().unwrap().to_string()[..]
            );
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

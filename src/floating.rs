//! Exact x87 constants, independent of the host's floating-point ABI.

use rustc_apfloat::{
    Float, FloatConvert,
    ieee::{Double, Single, X87DoubleExtended},
};

pub(crate) type Extended = X87DoubleExtended;

pub(crate) fn from_double(value: f64) -> Extended {
    Double::from_bits(value.to_bits().into())
        .convert(&mut false)
        .value
}

pub(crate) fn to_double(value: Extended) -> f64 {
    let value: Double = value.convert(&mut false).value;
    f64::from_bits(value.to_bits() as u64)
}

pub(crate) fn round(value: Extended, bits: u8) -> Extended {
    match bits {
        32 => {
            let value: Single = value.convert(&mut false).value;
            value.convert(&mut false).value
        }
        64 => {
            let value: Double = value.convert(&mut false).value;
            value.convert(&mut false).value
        }
        _ => value,
    }
}

pub(crate) fn llvm_literal(bits: u128) -> String {
    format!("0xK{bits:020X}")
}

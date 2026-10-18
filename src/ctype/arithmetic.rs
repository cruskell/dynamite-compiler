use super::Type;

impl Type {
    pub fn is_arithmetic(&self) -> bool {
        self.integer_info().is_some() || self.float_bits().is_some()
    }

    pub fn float_value(&self, bits: usize) -> f64 {
        if self.float_bits() == Some(32) {
            f32::from_bits(bits as u32) as f64
        } else {
            f64::from_bits(bits as u64)
        }
    }

    pub fn float_constant(&self, value: f64) -> usize {
        if self.float_bits() == Some(32) {
            (value as f32).to_bits() as usize
        } else {
            value.to_bits() as usize
        }
    }

    pub fn integer(bits: u8, signed: bool) -> Self {
        match (bits, signed) {
            (8, true) => Self::I8,
            (32, true) => Self::I32,
            _ => Self::Integer { bits, signed },
        }
    }

    pub fn integer_info(&self) -> Option<(u8, bool)> {
        match self {
            Self::Bool => Some((8, false)),
            Self::I8 | Self::SignedChar => Some((8, true)),
            Self::I32 => Some((32, true)),
            Self::Integer { bits, signed } => Some((*bits, *signed)),
            Self::LongLong { signed } => Some((64, *signed)),
            Self::Enum(value) => value.underlying.borrow().integer_info(),
            Self::Qualified(child, _) | Self::Aligned(child, _) => child.integer_info(),
            _ => None,
        }
    }

    pub fn promoted(&self) -> Self {
        if let Self::Enum(value) = self.unqualified() {
            return value.underlying.borrow().promoted();
        }
        if self.integer_info().is_some_and(|(bits, _)| bits < 32) {
            Self::I32
        } else {
            self.clone()
        }
    }

    pub fn arithmetic_type(lhs: &Self, rhs: &Self) -> Self {
        if lhs.float_bits().is_some() || rhs.float_bits().is_some() {
            let bits = lhs
                .float_bits()
                .unwrap_or(0)
                .max(rhs.float_bits().unwrap_or(0));
            return if matches!(lhs.unqualified(), Self::LongDouble(_))
                || matches!(rhs.unqualified(), Self::LongDouble(_))
            {
                Self::LongDouble(bits)
            } else {
                Self::Float(bits)
            };
        }
        let lhs = lhs.promoted();
        let rhs = rhs.promoted();
        let (Some((lb, ls)), Some((rb, rs))) = (lhs.integer_info(), rhs.integer_info()) else {
            return lhs;
        };
        let bits = lb.max(rb);
        let signed = match lb.cmp(&rb) {
            std::cmp::Ordering::Equal => ls && rs,
            std::cmp::Ordering::Greater => ls,
            std::cmp::Ordering::Less => rs,
        };
        if matches!(lhs.unqualified(), Self::LongLong { .. })
            || matches!(rhs.unqualified(), Self::LongLong { .. })
        {
            Self::LongLong { signed }
        } else {
            Self::integer(bits, signed)
        }
    }

    pub fn convert_constant(&self, value: i64) -> i64 {
        if let Self::Enum(ty) = self.unqualified() {
            return ty.underlying.borrow().convert_constant(value);
        }
        if *self == Self::Bool {
            return i64::from(value != 0);
        }
        match self.integer_info() {
            Some((bits, signed)) if bits < 64 => {
                let shift = 64 - bits;
                if signed {
                    (value << shift) >> shift
                } else {
                    (value as u64 & ((1u64 << bits) - 1)) as i64
                }
            }
            _ => value,
        }
    }
}

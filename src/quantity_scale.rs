//! Dependency-free exact rational arithmetic for the nominal quantity preview.
//! Nine limbs cover 256x256-bit products and a further signed-i64 magnitude.
//! All stored scales are positive, reduced and at most 256 bits per component.
use std::cmp::Ordering;

const LIMBS: usize = 9;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Wide([u64; LIMBS]);
impl Wide {
    const ZERO: Self = Self([0; LIMBS]);
    const ONE: Self = Self::from_u64(1);
    const fn from_u64(value: u64) -> Self {
        let mut limbs = [0; LIMBS];
        limbs[0] = value;
        Self(limbs)
    }
    fn bits(self) -> usize {
        for i in (0..LIMBS).rev() {
            if self.0[i] != 0 {
                return 64 * i + (64 - self.0[i].leading_zeros() as usize);
            }
        }
        0
    }
    fn compare(self, rhs: Self) -> Ordering {
        for i in (0..LIMBS).rev() {
            let order = self.0[i].cmp(&rhs.0[i]);
            if order != Ordering::Equal { return order; }
        }
        Ordering::Equal
    }
    fn sub(self, rhs: Self) -> Self {
        debug_assert!(self.compare(rhs) != Ordering::Less);
        let mut out = Self::ZERO;
        let mut borrow = false;
        for i in 0..LIMBS {
            let (v, a) = self.0[i].overflowing_sub(rhs.0[i]);
            let (v, b) = v.overflowing_sub(u64::from(borrow));
            out.0[i] = v;
            borrow = a || b;
        }
        debug_assert!(!borrow);
        out
    }
    fn mul(self, rhs: Self) -> Option<Self> {
        let mut out = Self::ZERO;
        for i in 0..self.bits().div_ceil(64) {
            let mut carry = 0u128;
            for j in 0..rhs.bits().div_ceil(64) {
                let k = i + j;
                let product = u128::from(self.0[i]) * u128::from(rhs.0[j]) + carry;
                if k >= LIMBS {
                    if product != 0 { return None; }
                    continue;
                }
                let value = product + u128::from(out.0[k]);
                out.0[k] = value as u64;
                carry = value >> 64;
            }
            let mut k = i + rhs.bits().div_ceil(64);
            while carry != 0 {
                if k >= LIMBS { return None; }
                let value = u128::from(out.0[k]) + carry;
                out.0[k] = value as u64;
                carry = value >> 64;
                k += 1;
            }
        }
        Some(out)
    }
    fn div_rem(self, divisor: Self) -> (Self, Self) {
        debug_assert!(divisor != Self::ZERO);
        let mut q = Self::ZERO;
        let mut r = Self::ZERO;
        for i in (0..self.bits()).rev() {
            let mut carry = (self.0[i / 64] >> (i % 64)) & 1;
            for word in &mut r.0 {
                let next = *word >> 63;
                *word = (*word << 1) | carry;
                carry = next;
            }
            // All callers use a divisor of at most 512 bits, so r < 2*divisor
            // fits the 576-bit work buffer. Never silently truncate a carry.
            assert_eq!(carry, 0);
            if r.compare(divisor) != Ordering::Less {
                r = r.sub(divisor);
                q.0[i / 64] |= 1u64 << (i % 64);
            }
        }
        (q, r)
    }
    fn gcd(mut self, mut other: Self) -> Self {
        while other != Self::ZERO {
            let remainder = self.div_rem(other).1;
            self = other;
            other = remainder;
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Scale { num: Wide, den: Wide }
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConversionError { Inexact, Range }
impl Scale {
    pub(crate) const ONE: Self = Self { num: Wide::ONE, den: Wide::ONE };
    pub(crate) fn positive(value: u64) -> Option<Self> {
        if value == 0 { return None; }
        Some(Self { num: Wide::from_u64(value), den: Wide::ONE })
    }
    pub(crate) fn inv(self) -> Self { Self { num: self.den, den: self.num } }
    pub(crate) fn mul(self, rhs: Self) -> Option<Self> {
        let n = self.num.mul(rhs.num)?;
        let d = self.den.mul(rhs.den)?;
        let gcd = n.gcd(d);
        let num = n.div_rem(gcd).0;
        let den = d.div_rem(gcd).0;
        if num.bits() > 256 || den.bits() > 256 { return None; }
        Some(Self { num, den })
    }
    pub(crate) fn convert(self, value: i64, target: Self) -> Result<i64, ConversionError> {
        // Do not apply the 256-bit stored-scale limit to conversion intermediates.
        let numerator = self.num.mul(target.den)
            .and_then(|n| n.mul(Wide::from_u64(value.unsigned_abs())))
            .ok_or(ConversionError::Range)?;
        let denominator = self.den.mul(target.num).ok_or(ConversionError::Range)?;
        let (q, rem) = numerator.div_rem(denominator);
        if rem != Wide::ZERO { return Err(ConversionError::Inexact); }
        if q.bits() > 64 { return Err(ConversionError::Range); }
        let magnitude = q.0[0];
        if value < 0 && magnitude == 1u64 << 63 { return Ok(i64::MIN); }
        let signed = i64::try_from(magnitude).map_err(|_| ConversionError::Range)?;
        Ok(if value < 0 { -signed } else { signed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stored_scale_limit_and_reduced_products() {
        let two = Scale::positive(2).unwrap();
        let mut s = Scale::ONE;
        for _ in 0..255 { s = s.mul(two).unwrap(); }
        assert_eq!(s.num.bits(), 256);
        assert!(s.mul(two).is_none());
        assert_eq!(s.mul(s.inv()), Some(Scale::ONE));
        assert_eq!(s.convert(0, s.inv()), Ok(0));
        assert_eq!(s.convert(1, s.inv()), Err(ConversionError::Range));
        assert_eq!(s.inv().convert(1, s), Err(ConversionError::Inexact));
    }
    #[test]
    fn exact_conversion_signed_boundaries() {
        let eight = Scale::positive(8).unwrap();
        assert_eq!(eight.convert(i64::MIN / 8, Scale::ONE), Ok(i64::MIN));
        assert_eq!(Scale::ONE.convert(i64::MIN, eight), Ok(i64::MIN / 8));
        assert_eq!(Scale::ONE.convert(-9, eight), Err(ConversionError::Inexact));
        assert_eq!(eight.convert(i64::MAX, Scale::ONE), Err(ConversionError::Range));
    }
    #[test]
    fn small_rational_products_match_integer_arithmetic() {
        for a in 1..=12 { for b in 1..=12 { for c in 1..=12 {
            let x = Scale::positive(a).unwrap().mul(Scale::positive(b).unwrap().inv()).unwrap();
            let y = Scale::positive(c).unwrap();
            let expected = a*c;
            let actual = x.convert(c as i64, Scale::ONE);
            if expected % b == 0 { assert_eq!(actual, Ok((expected/b) as i64)); }
            else { assert_eq!(actual, Err(ConversionError::Inexact)); }
            assert_eq!(x.mul(y).unwrap().mul(y.inv()), Some(x));
        }}}
    }
}

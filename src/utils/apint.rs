//! Aribitrary precision integer implementation.
//! This is similar in functionality to LLVM's APInt class.

use crate::{arg_error_noloc, result::Result};
use awint::{Awi, SerdeError};
use std::num::NonZero;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct APInt {
    value: Awi,
}

impl From<SerdeError> for crate::result::Error {
    fn from(value: SerdeError) -> Self {
        arg_error_noloc!("APInt error: {}", value)
    }
}

pub use awint::bw;

impl APInt {
    /// Get the bitwidth of the APInt.
    pub fn bw(&self) -> usize {
        self.value.bw()
    }

    /// Get zero valued APInt.
    pub fn zero(width: NonZero<usize>) -> APInt {
        APInt {
            value: Awi::zero(width),
        }
    }

    /// Is this value zero?
    pub fn is_zero(&self) -> bool {
        self.value.is_zero()
    }

    /// Add `self` and `rhs`. They must have the same bitwidth.
    pub fn add(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::add: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let mut value = self.value.clone();
        value
            .add_(&rhs.value)
            .expect("APInt::add: bitwidth mismatch");
        APInt { value }
    }

    /// Subtract `rhs` from `self`. They must have the same bitwidth.
    pub fn sub(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::sub: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let mut value = self.value.clone();
        value
            .sub_(&rhs.value)
            .expect("APInt::sub: bitwidth mismatch");
        APInt { value }
    }

    /// Get unsigned max value
    pub fn umax(width: NonZero<usize>) -> APInt {
        APInt {
            value: Awi::umax(width),
        }
    }

    /// Get signed max value
    pub fn imax(width: NonZero<usize>) -> APInt {
        APInt {
            value: Awi::imax(width),
        }
    }

    /// Get signed min value
    pub fn imin(width: NonZero<usize>) -> APInt {
        APInt {
            value: Awi::imin(width),
        }
    }

    /// Get unsigned one value
    pub fn uone(width: NonZero<usize>) -> APInt {
        APInt {
            value: Awi::uone(width),
        }
    }

    /// Parse a string into an APInt.
    pub fn from_str(value: &str, width: usize, radix: u8) -> Result<APInt> {
        let sign_opt = value.chars().next().ok_or(SerdeError::Empty)?;
        let neg = sign_opt == '-';
        let value = if neg || sign_opt == '+' {
            &value[1..]
        } else {
            value
        };

        let sign = if neg { Some(true) } else { None };
        let value = Awi::from_str_radix(
            sign,
            value,
            radix,
            NonZero::new(width).ok_or(SerdeError::ZeroBitwidth)?,
        )?;

        Ok(APInt { value })
    }

    /// Convert APInt to string, interpreting it as a signed or unsigned integer.
    pub fn to_string(&self, radix: u8, signed: bool) -> String {
        match Awi::bits_to_string_radix(&self.value, signed, radix, false, 1) {
            Ok(mut s) => {
                if signed && self.value.msb() {
                    s.insert(0, '-');
                }
                s
            }
            Err(e) => {
                panic!("APInt error: {e}");
            }
        }
    }

    /// Convert APInt to a decimal string
    pub fn to_string_decimal(&self, signed: bool) -> String {
        self.to_string(10, signed)
    }

    /// Convert APInt to string, interpreting it as a signed integer.
    pub fn to_string_signed(&self, radix: u8) -> String {
        self.to_string(radix, true)
    }

    /// Convert APInt to string, interpreting it as an unsigned integer.
    pub fn to_string_unsigned(&self, radix: u8) -> String {
        self.to_string(radix, false)
    }

    /// Convert APInt to decimal string, interpreting it as a signed integer.
    pub fn to_string_signed_decimal(&self) -> String {
        self.to_string_signed(10)
    }

    /// Convert APInt to decimal string, interpreting it as an unsigned integer.
    pub fn to_string_unsigned_decimal(&self) -> String {
        self.to_string_unsigned(10)
    }

    /// Build APInt from u8.
    /// Zero extends value if width > 8.
    /// Truncates value if width < 8.
    pub fn from_u8(value: u8, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.u8_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to u8.
    /// Truncates value if width > 8.
    /// Zero extends value if width < 8.
    pub fn to_u8(&self) -> u8 {
        self.value.to_u8()
    }

    /// Build APInt from u16.
    /// Zero extends value if width > 16.
    /// Truncates value if width < 16.
    pub fn from_u16(value: u16, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.u16_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to u16.
    /// Truncates value if width > 16.
    /// Zero extends value if width < 16.
    pub fn to_u16(&self) -> u16 {
        self.value.to_u16()
    }

    /// Build APInt from u32.
    /// Zero extends value if width > 32.
    /// Truncates value if width < 32.
    pub fn from_u32(value: u32, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.u32_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to u32.
    /// Truncates value if width > 32.
    /// Zero extends value if width < 32.
    pub fn to_u32(&self) -> u32 {
        self.value.to_u32()
    }

    /// Build APInt from u64.
    /// Zero extends value if width > 64.
    /// Truncates value if width < 64.
    pub fn from_u64(value: u64, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.u64_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to u64.
    /// Truncates value if width > 64.
    /// Zero extends value if width < 64.
    pub fn to_u64(&self) -> u64 {
        self.value.to_u64()
    }

    /// Build APInt from usize.
    /// Zero extends value if width > usize.
    /// Truncates value if width < usize.
    pub fn from_usize(value: usize, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.usize_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to usize.
    /// Truncates value if width > usize.
    /// Zero extends value if width < usize.
    pub fn to_usize(&self) -> usize {
        self.value.to_usize()
    }

    /// Build APInt from u128.
    /// Zero extends value if width > 128.
    /// Truncates value if width < 128.
    pub fn from_u128(value: u128, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.u128_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to u128.
    /// Truncates value if width > 128.
    /// Zero extends value if width < 128.
    pub fn to_u128(&self) -> u128 {
        self.value.to_u128()
    }

    /// Build APInt from i8.
    /// Sign extends value if width > 8.
    /// Truncates value if width < 8.
    pub fn from_i8(value: i8, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.i8_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to i8.
    /// Truncates value if width > 8.
    /// Sign extends value if width < 8.
    pub fn to_i8(&self) -> i8 {
        self.value.to_i8()
    }

    /// Build APInt from i16.
    /// Sign extends value if width > 16.
    /// Truncates value if width < 16.
    pub fn from_i16(value: i16, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.i16_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to i16.
    /// Truncates value if width > 16.
    /// Sign extends value if width < 16.
    pub fn to_i16(&self) -> i16 {
        self.value.to_i16()
    }

    /// Build APInt from i32.
    /// Sign extends value if width > 32.
    /// Truncates value if width < 32.
    pub fn from_i32(value: i32, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.i32_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to i32.
    /// Truncates value if width > 32.
    /// Sign extends value if width < 32.
    pub fn to_i32(&self) -> i32 {
        self.value.to_i32()
    }

    /// Build APInt from i64.
    /// Sign extends value if width > 64.
    /// Truncates value if width < 64.
    pub fn from_i64(value: i64, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.i64_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to i64.
    /// Truncates value if width > 64.
    /// Sign extends value if width < 64.
    pub fn to_i64(&self) -> i64 {
        self.value.to_i64()
    }

    /// Build APInt from i128.
    /// Sign extends value if width > 128.
    /// Truncates value if width < 128.
    pub fn from_i128(value: i128, width: NonZero<usize>) -> APInt {
        let mut awi_value = Awi::zero_with_capacity(width, width);
        awi_value.i128_(value);
        APInt { value: awi_value }
    }

    /// Convert APInt to i128.
    /// Truncates value if width > 128.
    /// Sign extends value if width < 128.
    pub fn to_i128(&self) -> i128 {
        self.value.to_i128()
    }
}
#[cfg(test)]
mod tests {
    use expect_test::expect;

    use super::*;

    #[test]
    fn test_zero() {
        let width = bw(4);
        let apint = APInt::zero(width);
        assert!(apint.is_zero());
    }

    #[test]
    fn test_limits() {
        let width = bw(4);

        let umax = APInt::umax(width);
        assert_eq!(umax.to_u8(), 15);
        assert_eq!(umax.to_i8(), -1);

        let imax = APInt::imax(width);
        assert_eq!(imax.to_i8(), 7);
        assert_eq!(imax.to_u8(), 7);

        let imin = APInt::imin(width);
        assert_eq!(imin.to_i8(), -8);
        assert_eq!(imin.to_u8(), 8);
    }

    #[test]
    fn test_from_str() {
        let width = 4;
        let apint = APInt::from_str("7", width, 10).unwrap();
        assert_eq!(apint.to_u8(), 7);

        let apint = APInt::from_str("-8", width, 10).unwrap();
        assert_eq!(apint.to_i8(), -8);
        assert_eq!(apint.to_string(10, true), "-8");

        let apint = APInt::from_str("+15", width, 10).unwrap();
        assert_eq!(apint.to_i8(), -1);
        assert_eq!(apint.to_u8(), 15);
        assert_eq!(apint.to_string(10, true), "-1");
        assert_eq!(apint.to_string(10, false), "15");

        let apint = APInt::from_str("-2", width, 10).unwrap();
        assert_eq!(apint.to_i8(), -2);
        assert_eq!(apint.to_u8(), 14);
        assert_eq!(apint.to_string(10, true), "-2");
        assert_eq!(apint.to_string(10, false), "14");
    }

    #[test]
    fn test_from_str_failure() {
        let width = 4;
        let result = APInt::from_str("invalid", width, 10);
        expect![[r#"
            Compilation error: invalid argument.
            APInt error: InvalidChar"#]]
        .assert_eq(&result.unwrap_err().to_string());
        let result = APInt::from_str("", width, 10);
        expect![[r#"
            Compilation error: invalid argument.
            APInt error: Empty"#]]
        .assert_eq(&result.unwrap_err().to_string());
        let result = APInt::from_str("16", width, 10);
        expect![[r#"
            Compilation error: invalid argument.
            APInt error: Overflow"#]]
        .assert_eq(&result.unwrap_err().to_string());
    }

    #[test]
    fn test_from_u8() {
        let width = bw(4);
        for i in 0..16 {
            let apint = APInt::from_u8(i, width);
            assert_eq!(apint.to_u8(), i);
        }
    }

    #[test]
    fn test_from_u16() {
        let width = bw(4);
        for i in 0..16 {
            let apint = APInt::from_u16(i, width);
            assert_eq!(apint.to_u16(), i);
        }
    }

    #[test]
    fn test_from_u32() {
        let width = bw(4);
        for i in 0..16 {
            let apint = APInt::from_u32(i, width);
            assert_eq!(apint.to_u32(), i);
        }
    }

    #[test]
    fn test_from_u64() {
        let width = bw(4);
        for i in 0..16 {
            let apint = APInt::from_u64(i, width);
            assert_eq!(apint.to_u64(), i);
        }
    }

    #[test]
    fn test_from_u128() {
        let width = bw(4);
        for i in 0..16 {
            let apint = APInt::from_u128(i, width);
            assert_eq!(apint.to_u128(), i);
        }
    }

    #[test]
    fn test_from_i8() {
        let width = bw(4);
        for i in -8..8 {
            let apint = APInt::from_i8(i, width);
            assert_eq!(apint.to_i8(), i);
        }
    }

    #[test]
    fn test_from_i16() {
        let width = bw(4);
        for i in -8..8 {
            let apint = APInt::from_i16(i, width);
            assert_eq!(apint.to_i16(), i);
        }
    }

    #[test]
    fn test_from_i32() {
        let width = bw(4);
        for i in -8..8 {
            let apint = APInt::from_i32(i, width);
            assert_eq!(apint.to_i32(), i);
        }
    }

    #[test]
    fn test_from_i64() {
        let width = bw(4);
        for i in -8..8 {
            let apint = APInt::from_i64(i, width);
            assert_eq!(apint.to_i64(), i);
        }
    }

    #[test]
    fn test_from_i128() {
        let width = bw(4);
        for i in -8..8 {
            let apint = APInt::from_i128(i, width);
            assert_eq!(apint.to_i128(), i);
        }
    }
}

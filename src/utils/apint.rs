//! Aribitrary precision integer implementation.
//! This is similar in functionality to LLVM's APInt class.

use crate::{arg_error_noloc, result::Result};
use alloc::string::String;
use awint::{Awi, Bits, SerdeError};
use core::num::NonZero;

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

    /// Add `self` and `rhs`, reporting whether the (truncating) result overflowed.
    /// They must have the same bitwidth.
    ///
    /// Returns `(result, unsigned_overflow_occured, signed_overflow_occured)`
    pub fn add_overflow(&self, rhs: &APInt) -> (APInt, bool, bool) {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::add_overflow: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let mut value = Awi::zero(NonZero::new(self.bw()).expect("self has zero bitwidth"));
        let (unsigned_overflow, signed_overflow) = value
            .cin_sum_(false, &self.value, &rhs.value)
            .expect("APInt::add_overflow: bitwidth mismatch");
        (APInt { value }, unsigned_overflow, signed_overflow)
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

    /// Subtract `rhs` from `self`, reporting whether the (truncating) result
    /// overflowed. They must have the same bitwidth.
    ///
    /// Returns `(result, unsigned_overflow_occured, signed_overflow_occured)`.
    pub fn sub_overflow(&self, rhs: &APInt) -> (APInt, bool, bool) {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::sub_overflow: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        // Below, I use `n` to denote the bitwidth.
        // Two's complement: `self - rhs ≡ self + (~rhs) + 1  (mod 2^n)`.
        let mut not_rhs = rhs.value.clone();
        not_rhs.not_();
        let mut value = Awi::zero(NonZero::new(self.bw()).expect("self has zero bitwidth"));
        let (carry_out, signed_overflow) = value
            .cin_sum_(true, &self.value, &not_rhs)
            .expect("APInt::sub_overflow: bitwidth mismatch");
        // Signed overflow occurs when the true mathematical value of `self - rhs``
        // is not in `[−2^(n−1), 2^(n−1)−1]`. This matches addition's signed overflow.
        //
        // Unsigned overflow occurs when `self < rhs`.
        // Computed as an integer, `self + (~rhs) + 1 = (self - rhs) + 2^n`, so
        // `carry_out == true` iff `(self - rhs) + 2^n >= 2^n`` iff `self >= rhs`.
        // Hence unsigned overflow is !carry_out.
        (APInt { value }, !carry_out, signed_overflow)
    }

    /// Multiply `self` and `rhs`. They must have the same bitwidth.
    pub fn mul(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::mul: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let mut value = Awi::zero(NonZero::new(self.bw()).expect("self has zero bitwidth"));
        value
            .mul_add_(&self.value, &rhs.value)
            .expect("APInt::mul: bitwidth mismatch");
        APInt { value }
    }

    /// Multiply `self` and `rhs`, reporting whether the (truncating) result
    /// overflowed. They must have the same bitwidth.
    ///
    /// Returns `(result, unsigned_overflow_occured, signed_overflow_occured)`.
    pub fn mul_overflow(&self, rhs: &APInt) -> (APInt, bool, bool) {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::mul_overflow: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let bw = NonZero::new(self.bw()).expect("self has zero bitwidth");
        let dbw = NonZero::new(self.bw() * 2).expect("self has zero bitwidth");

        let mut ulhs = Awi::zero(dbw);
        ulhs.zero_resize_(&self.value);
        let mut urhs = Awi::zero(dbw);
        urhs.zero_resize_(&rhs.value);
        let mut uprod = Awi::zero(dbw);
        uprod.arb_umul_add_(&ulhs, &urhs);
        let mut utrunc = Awi::zero(bw);
        let unsigned_overflow = utrunc.zero_resize_(&uprod);

        let mut slhs = Awi::zero(dbw);
        slhs.sign_resize_(&self.value);
        let mut srhs = Awi::zero(dbw);
        srhs.sign_resize_(&rhs.value);
        let mut sprod = Awi::zero(dbw);
        sprod.arb_imul_add_(&mut slhs, &mut srhs);
        let mut strunc = Awi::zero(bw);
        let signed_overflow = strunc.sign_resize_(&sprod);

        (self.mul(rhs), unsigned_overflow, signed_overflow)
    }

    /// Left-shift `self` by `rhs` bits. They must have the same bitwidth.
    /// If the shift amount is greater than or equal to the bitwidth, the
    /// result is zero.
    pub fn shl(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::shl: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let shamt = rhs.to_usize();
        let mut value = self.value.clone();
        if value.shl_(shamt).is_none() {
            // Shift amount >= bitwidth: every bit is shifted out.
            value.zero_();
        }
        APInt { value }
    }

    /// Logically (unsigned) right-shift `self` by `rhs` bits. They must have the
    /// same bitwidth. If the shift amount is greater than or equal to the
    /// bitwidth, the result is zero.
    pub fn lshr(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::lshr: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let shamt = rhs.to_usize();
        let mut value = self.value.clone();
        if value.lshr_(shamt).is_none() {
            // Shift amount >= bitwidth: every bit is shifted out.
            value.zero_();
        }
        APInt { value }
    }

    /// Arithmetically (signed) right-shift `self` by `rhs` bits. They must have
    /// the same bitwidth. If the shift amount is greater than or equal to the
    /// bitwidth, the result is the sign bit replicated across all bits.
    pub fn ashr(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::ashr: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let shamt = rhs.to_usize().min(self.bw() - 1);
        let mut value = self.value.clone();
        value
            .ashr_(shamt)
            .expect("shift amount clamped below bitwidth above");
        APInt { value }
    }

    /// Sign-extend (or truncate) `self` to `width` bits. When `width` is larger
    /// than the current bitwidth, the sign bit is replicated into the new high
    /// bits.
    pub fn sext(&self, width: NonZero<usize>) -> APInt {
        let mut value = Awi::zero(width);
        value.sign_resize_(&self.value);
        APInt { value }
    }

    /// Zero-extend (or truncate) `self` to `width` bits. When `width` is larger
    /// than the current bitwidth, the new high bits are filled with zeros.
    pub fn zext(&self, width: NonZero<usize>) -> APInt {
        let mut value = Awi::zero(width);
        value.zero_resize_(&self.value);
        APInt { value }
    }

    /// Left-shift `self` by `rhs` bits, reporting whether the result
    /// overflowed. They must have the same bitwidth, and the shift amount `rhs`
    /// must be less than the bitwidth (a shift amount `>=` the bitwidth is
    /// undefined for `shl` and must be ruled out by the caller).
    ///
    /// Returns `(result, unsigned_overflow_occured, signed_overflow_occured)`,
    /// where the result is the shifted value, `unsigned_overflow_occured` is true
    /// if any bit shifted off the top was set (so the shift is not invertible by a
    /// logical right shift), and `signed_overflow_occured` is true if the bits
    /// shifted off the top together with the result's new sign bit are not all
    /// equal to the original sign bit (so the shift is not invertible by an
    /// arithmetic right shift). These match LLVM's `nuw` and `nsw` poison
    /// conditions for `shl`, respectively.
    pub fn shl_overflow(&self, rhs: &APInt) -> (APInt, bool, bool) {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::shl_overflow: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        let shamt = rhs.to_usize();
        assert!(
            shamt < self.bw(),
            "APInt::shl_overflow: shift amount {} >= bitwidth {}",
            shamt,
            self.bw()
        );
        let result = self.shl(rhs);

        let mut ushifted_back = result.value.clone();
        ushifted_back
            .lshr_(shamt)
            .expect("shift amount checked against bitwidth above");
        let unsigned_overflow = ushifted_back != self.value;

        let mut sshifted_back = result.value.clone();
        sshifted_back
            .ashr_(shamt)
            .expect("shift amount checked against bitwidth above");
        let signed_overflow = sshifted_back != self.value;

        (result, unsigned_overflow, signed_overflow)
    }

    /// Unsigned-divide `self` by `rhs`. They must have the same bitwidth.
    pub fn udiv(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::udiv: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        assert!(!rhs.is_zero(), "APInt::udiv: division by zero");
        let width = NonZero::new(self.bw()).expect("self has zero bitwidth");
        let mut quo = Awi::zero(width);
        let mut rem = Awi::zero(width);
        Bits::udivide(&mut quo, &mut rem, &self.value, &rhs.value).unwrap();
        APInt { value: quo }
    }

    /// Signed-divide `self` by `rhs`. They must have the same bitwidth.
    pub fn sdiv(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::sdiv: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        assert!(!rhs.is_zero(), "APInt::sdiv: division by zero");
        let width = NonZero::new(self.bw()).expect("self has zero bitwidth");
        let mut quo = Awi::zero(width);
        let mut rem = Awi::zero(width);
        let mut duo = self.value.clone();
        let mut div = rhs.value.clone();
        Bits::idivide(&mut quo, &mut rem, &mut duo, &mut div).unwrap();
        APInt { value: quo }
    }

    /// Unsigned remainder of `self` divided by `rhs`. They must have the same
    /// bitwidth.
    pub fn urem(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::urem: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        assert!(!rhs.is_zero(), "APInt::urem: division by zero");
        let width = NonZero::new(self.bw()).expect("self has zero bitwidth");
        let mut quo = Awi::zero(width);
        let mut rem = Awi::zero(width);
        Bits::udivide(&mut quo, &mut rem, &self.value, &rhs.value).unwrap();
        APInt { value: rem }
    }

    /// Signed remainder of `self` divided by `rhs`. They must have the same
    /// bitwidth. The sign of the result follows the dividend.
    pub fn srem(&self, rhs: &APInt) -> APInt {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::srem: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        assert!(!rhs.is_zero(), "APInt::srem: division by zero");
        let width = NonZero::new(self.bw()).expect("self has zero bitwidth");
        let mut quo = Awi::zero(width);
        let mut rem = Awi::zero(width);
        let mut duo = self.value.clone();
        let mut div = rhs.value.clone();
        Bits::idivide(&mut quo, &mut rem, &mut duo, &mut div).unwrap();
        APInt { value: rem }
    }

    /// Bitwise AND of `self` and `rhs`. They must have the same bitwidth.
    pub fn and(&self, rhs: &APInt) -> APInt {
        let mut value = self.value.clone();
        value
            .and_(&rhs.value)
            .expect("APInt::and: bitwidth mismatch");
        APInt { value }
    }

    /// Bitwise OR of `self` and `rhs`. They must have the same bitwidth.
    pub fn or(&self, rhs: &APInt) -> APInt {
        let mut value = self.value.clone();
        value.or_(&rhs.value).expect("APInt::or: bitwidth mismatch");
        APInt { value }
    }

    /// Bitwise XOR of `self` and `rhs`. They must have the same bitwidth.
    pub fn xor(&self, rhs: &APInt) -> APInt {
        let mut value = self.value.clone();
        value
            .xor_(&rhs.value)
            .expect("APInt::xor: bitwidth mismatch");
        APInt { value }
    }

    /// Unsigned less-than comparison. They must have the same bitwidth.
    pub fn ult(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::ult: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .ult(&rhs.value)
            .expect("APInt::ult: bitwidth mismatch")
    }

    /// Unsigned less-than-or-equal comparison. They must have the same bitwidth.
    pub fn ule(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::ule: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .ule(&rhs.value)
            .expect("APInt::ule: bitwidth mismatch")
    }

    /// Unsigned greater-than comparison. They must have the same bitwidth.
    pub fn ugt(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::ugt: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .ugt(&rhs.value)
            .expect("APInt::ugt: bitwidth mismatch")
    }

    /// Unsigned greater-than-or-equal comparison. They must have the same
    /// bitwidth.
    pub fn uge(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::uge: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .uge(&rhs.value)
            .expect("APInt::uge: bitwidth mismatch")
    }

    /// Signed less-than comparison. They must have the same bitwidth.
    pub fn slt(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::slt: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .ilt(&rhs.value)
            .expect("APInt::slt: bitwidth mismatch")
    }

    /// Signed less-than-or-equal comparison. They must have the same bitwidth.
    pub fn sle(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::sle: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .ile(&rhs.value)
            .expect("APInt::sle: bitwidth mismatch")
    }

    /// Signed greater-than comparison. They must have the same bitwidth.
    pub fn sgt(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::sgt: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .igt(&rhs.value)
            .expect("APInt::sgt: bitwidth mismatch")
    }

    /// Signed greater-than-or-equal comparison. They must have the same
    /// bitwidth.
    pub fn sge(&self, rhs: &APInt) -> bool {
        assert_eq!(
            self.bw(),
            rhs.bw(),
            "APInt::sge: bitwidth mismatch ({} vs {})",
            self.bw(),
            rhs.bw()
        );
        self.value
            .ige(&rhs.value)
            .expect("APInt::sge: bitwidth mismatch")
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
    use alloc::string::ToString;
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

    #[test]
    fn test_add() {
        let width = bw(4);

        // Basic addition.
        let sum = APInt::from_u8(3, width).add(&APInt::from_u8(4, width));
        assert_eq!(sum.to_u8(), 7);

        // Wraps (truncates) on overflow: 15 + 1 == 0 (mod 16).
        let sum = APInt::from_u8(15, width).add(&APInt::from_u8(1, width));
        assert_eq!(sum.to_u8(), 0);

        // Signed view of wrapping: (-1) + 1 == 0.
        let sum = APInt::from_i8(-1, width).add(&APInt::from_i8(1, width));
        assert_eq!(sum.to_i8(), 0);

        // Adding zero is the identity.
        let sum = APInt::from_u8(9, width).add(&APInt::zero(width));
        assert_eq!(sum.to_u8(), 9);
    }

    #[test]
    fn test_sub() {
        let width = bw(4);

        // Basic subtraction.
        let diff = APInt::from_u8(7, width).sub(&APInt::from_u8(4, width));
        assert_eq!(diff.to_u8(), 3);

        // Wraps (truncates) on borrow: 0 - 1 == 15 (mod 16), i.e. -1.
        let diff = APInt::from_u8(0, width).sub(&APInt::from_u8(1, width));
        assert_eq!(diff.to_u8(), 15);
        assert_eq!(diff.to_i8(), -1);

        // Subtracting from itself yields zero.
        let diff = APInt::from_u8(5, width).sub(&APInt::from_u8(5, width));
        assert!(diff.is_zero());
    }

    #[test]
    fn test_mul() {
        let width = bw(4);

        // Basic multiplication.
        let prod = APInt::from_u8(3, width).mul(&APInt::from_u8(4, width));
        assert_eq!(prod.to_u8(), 12);

        // Wraps (truncates) on overflow: 3 * 6 == 18 == 2 (mod 16).
        let prod = APInt::from_u8(3, width).mul(&APInt::from_u8(6, width));
        assert_eq!(prod.to_u8(), 2);

        // (-2) * 3 == -6, whose 4-bit representation is 10 unsigned.
        let prod = APInt::from_i8(-2, width).mul(&APInt::from_i8(3, width));
        assert_eq!(prod.to_i8(), -6);
        assert_eq!(prod.to_u8(), 10);

        // Multiplying by zero yields zero.
        let prod = APInt::from_u8(7, width).mul(&APInt::zero(width));
        assert!(prod.is_zero());
    }

    #[test]
    fn test_shl() {
        let width = bw(4);

        // Basic shift: 1 << 2 == 4.
        let res = APInt::from_u8(1, width).shl(&APInt::from_u8(2, width));
        assert_eq!(res.to_u8(), 4);

        // Shifting by zero is the identity.
        let res = APInt::from_u8(5, width).shl(&APInt::zero(width));
        assert_eq!(res.to_u8(), 5);

        // Bits shifted past the top are truncated: 3 << 3 == 24 == 8 (mod 16).
        let res = APInt::from_u8(3, width).shl(&APInt::from_u8(3, width));
        assert_eq!(res.to_u8(), 8);

        // Shift amount equal to the bitwidth shifts every bit out, yielding 0.
        let res = APInt::from_u8(0xf, width).shl(&APInt::from_u8(4, width));
        assert!(res.is_zero());

        // Shift amount greater than the bitwidth also yields 0.
        let res = APInt::from_u8(0xf, width).shl(&APInt::from_u8(7, width));
        assert!(res.is_zero());
    }

    #[test]
    fn test_lshr() {
        let width = bw(4);

        let res = APInt::from_u8(8, width).lshr(&APInt::from_u8(2, width));
        assert_eq!(res.to_u8(), 2);

        let res = APInt::from_u8(5, width).lshr(&APInt::zero(width));
        assert_eq!(res.to_u8(), 5);

        let res = APInt::from_u8(0b1000, width).lshr(&APInt::from_u8(1, width));
        assert_eq!(res.to_u8(), 0b0100);

        let res = APInt::from_u8(0xf, width).lshr(&APInt::from_u8(4, width));
        assert!(res.is_zero());

        let res = APInt::from_u8(0xf, width).lshr(&APInt::from_u8(7, width));
        assert!(res.is_zero());
    }

    #[test]
    fn test_ashr() {
        let width = bw(4);

        let res = APInt::from_i8(6, width).ashr(&APInt::from_u8(1, width));
        assert_eq!(res.to_i8(), 3);

        let res = APInt::from_i8(-3, width).ashr(&APInt::zero(width));
        assert_eq!(res.to_i8(), -3);

        let res = APInt::from_i8(-8, width).ashr(&APInt::from_u8(1, width));
        assert_eq!(res.to_i8(), -4);

        // Shift amount >= bitwidth replicates the sign bit across all bits:
        // a negative value yields all-ones (-1), ...
        let res = APInt::from_i8(-8, width).ashr(&APInt::from_u8(4, width));
        assert_eq!(res.to_i8(), -1);

        // ... and a non-negative value yields 0.
        let res = APInt::from_i8(7, width).ashr(&APInt::from_u8(7, width));
        assert!(res.is_zero());
    }

    #[test]
    fn test_sext() {
        // A non-negative value extends with zeros: 5 (i4) -> 5 (i8).
        let res = APInt::from_i8(5, bw(4)).sext(bw(8));
        assert_eq!(res.bw(), 8);
        assert_eq!(res.to_i8(), 5);

        // A negative value replicates the sign bit: -1 (i4) -> -1 (i8) == 0xff.
        let res = APInt::from_i8(-1, bw(4)).sext(bw(8));
        assert_eq!(res.bw(), 8);
        assert_eq!(res.to_i8(), -1);
        assert_eq!(res.to_u8(), 0xff);

        // -3 (i4, 0b1101) sign-extends to -3 (i8, 0b11111101).
        let res = APInt::from_i8(-3, bw(4)).sext(bw(8));
        assert_eq!(res.to_i8(), -3);
    }

    #[test]
    fn test_zext() {
        // A non-negative value extends with zeros: 5 (i4) -> 5 (i8).
        let res = APInt::from_i8(5, bw(4)).zext(bw(8));
        assert_eq!(res.bw(), 8);
        assert_eq!(res.to_u8(), 5);

        // The sign bit is not replicated: -1 (i4, 0xf) -> 15 (i8), not -1.
        let res = APInt::from_i8(-1, bw(4)).zext(bw(8));
        assert_eq!(res.bw(), 8);
        assert_eq!(res.to_u8(), 15);

        // -3 (i4, 0b1101 == 13 unsigned) zero-extends to 13 (i8).
        let res = APInt::from_i8(-3, bw(4)).zext(bw(8));
        assert_eq!(res.to_u8(), 13);
    }

    #[test]
    fn test_udiv() {
        let width = bw(4);

        let res = APInt::from_u8(12, width).udiv(&APInt::from_u8(4, width));
        assert_eq!(res.to_u8(), 3);

        let res = APInt::from_u8(13, width).udiv(&APInt::from_u8(4, width));
        assert_eq!(res.to_u8(), 3);

        let res = APInt::from_u8(0xf, width).udiv(&APInt::from_u8(2, width));
        assert_eq!(res.to_u8(), 7);

        let res = APInt::from_u8(9, width).udiv(&APInt::uone(width));
        assert_eq!(res.to_u8(), 9);

        let res = APInt::from_u8(3, width).udiv(&APInt::from_u8(5, width));
        assert!(res.is_zero());
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn test_udiv_by_zero_panics() {
        let width = bw(4);
        let _ = APInt::from_u8(7, width).udiv(&APInt::zero(width));
    }

    #[test]
    fn test_unsigned_cmp() {
        let width = bw(4);
        let one = APInt::from_u8(1, width);
        let four = APInt::from_u8(4, width);
        let four2 = APInt::from_u8(4, width);

        assert!(one.ult(&four) && !four.ult(&four2) && !four.ult(&one));
        assert!(one.ule(&four) && four.ule(&four2) && !four.ule(&one));
        assert!(four.ugt(&one) && !one.ugt(&four) && !four.ugt(&four2));
        assert!(four.uge(&one) && four.uge(&four2) && !one.uge(&four));

        // 0xf == 15 is the largest under the unsigned interpretation.
        let high = APInt::from_u8(0xf, width);
        assert!(high.ugt(&four) && high.uge(&four) && !high.ule(&four));
    }

    #[test]
    fn test_signed_cmp() {
        let width = bw(4);
        let neg_one = APInt::from_i8(-1, width);
        let one = APInt::from_i8(1, width);
        let neg_one2 = APInt::from_i8(-1, width);

        assert!(neg_one.slt(&one) && !one.slt(&neg_one));
        assert!(neg_one.sle(&one) && neg_one.sle(&neg_one2) && !one.sle(&neg_one));
        assert!(one.sgt(&neg_one) && !neg_one.sgt(&one));
        assert!(one.sge(&neg_one) && neg_one.sge(&neg_one2) && !neg_one.sge(&one));

        // The same bit pattern compares oppositely signed vs unsigned: -1 is the
        // smallest signed value but the largest unsigned (0xf).
        assert!(neg_one.slt(&one) && neg_one.ugt(&one));
    }

    #[test]
    fn test_sdiv() {
        let width = bw(4);

        // Positive / positive.
        assert_eq!(
            APInt::from_i8(6, width)
                .sdiv(&APInt::from_i8(2, width))
                .to_i8(),
            3
        );

        // Negative dividend: truncates toward zero, so -7 / 2 == -3.
        assert_eq!(
            APInt::from_i8(-7, width)
                .sdiv(&APInt::from_i8(2, width))
                .to_i8(),
            -3
        );

        // Negative divisor.
        assert_eq!(
            APInt::from_i8(7, width)
                .sdiv(&APInt::from_i8(-2, width))
                .to_i8(),
            -3
        );

        // Negative / negative is positive.
        assert_eq!(
            APInt::from_i8(-6, width)
                .sdiv(&APInt::from_i8(-3, width))
                .to_i8(),
            2
        );

        // Dividing by one is the identity.
        assert_eq!(
            APInt::from_i8(-5, width).sdiv(&APInt::uone(width)).to_i8(),
            -5
        );
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn test_sdiv_by_zero_panics() {
        let width = bw(4);
        let _ = APInt::from_i8(7, width).sdiv(&APInt::zero(width));
    }

    #[test]
    fn test_urem() {
        let width = bw(4);

        // Basic remainder.
        assert_eq!(
            APInt::from_u8(13, width)
                .urem(&APInt::from_u8(4, width))
                .to_u8(),
            1
        );

        // Exact division leaves no remainder.
        assert_eq!(
            APInt::from_u8(12, width)
                .urem(&APInt::from_u8(4, width))
                .to_u8(),
            0
        );

        // Dividend smaller than divisor.
        assert_eq!(
            APInt::from_u8(3, width)
                .urem(&APInt::from_u8(5, width))
                .to_u8(),
            3
        );

        // Unsigned: 0xf == 15, so 15 % 4 == 3 (not -1 % 4).
        assert_eq!(
            APInt::from_u8(0xf, width)
                .urem(&APInt::from_u8(4, width))
                .to_u8(),
            3
        );
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn test_urem_by_zero_panics() {
        let width = bw(4);
        let _ = APInt::from_u8(7, width).urem(&APInt::zero(width));
    }

    #[test]
    fn test_srem() {
        let width = bw(4);

        // The result's sign follows the dividend.
        assert_eq!(
            APInt::from_i8(7, width)
                .srem(&APInt::from_i8(3, width))
                .to_i8(),
            1
        );
        assert_eq!(
            APInt::from_i8(-7, width)
                .srem(&APInt::from_i8(3, width))
                .to_i8(),
            -1
        );
        assert_eq!(
            APInt::from_i8(7, width)
                .srem(&APInt::from_i8(-3, width))
                .to_i8(),
            1
        );
        assert_eq!(
            APInt::from_i8(-7, width)
                .srem(&APInt::from_i8(-3, width))
                .to_i8(),
            -1
        );

        // Exact division leaves no remainder.
        assert_eq!(
            APInt::from_i8(-6, width)
                .srem(&APInt::from_i8(3, width))
                .to_i8(),
            0
        );
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn test_srem_by_zero_panics() {
        let width = bw(4);
        let _ = APInt::from_i8(7, width).srem(&APInt::zero(width));
    }

    #[test]
    fn test_and() {
        let width = bw(4);
        assert_eq!(
            APInt::from_u8(0b1100, width)
                .and(&APInt::from_u8(0b1010, width))
                .to_u8(),
            0b1000
        );
        assert_eq!(
            APInt::from_u8(0b1011, width)
                .and(&APInt::umax(width))
                .to_u8(),
            0b1011
        );
        assert!(
            APInt::from_u8(0b1011, width)
                .and(&APInt::zero(width))
                .is_zero()
        );
    }

    #[test]
    fn test_or() {
        let width = bw(4);
        assert_eq!(
            APInt::from_u8(0b1100, width)
                .or(&APInt::from_u8(0b1010, width))
                .to_u8(),
            0b1110
        );
        assert_eq!(
            APInt::from_u8(0b1011, width)
                .or(&APInt::zero(width))
                .to_u8(),
            0b1011
        );
        assert_eq!(
            APInt::from_u8(0b1011, width)
                .or(&APInt::umax(width))
                .to_u8(),
            0b1111
        );
    }

    #[test]
    fn test_xor() {
        let width = bw(4);
        assert_eq!(
            APInt::from_u8(0b1100, width)
                .xor(&APInt::from_u8(0b1010, width))
                .to_u8(),
            0b0110
        );
        assert!(
            APInt::from_u8(0b1011, width)
                .xor(&APInt::from_u8(0b1011, width))
                .is_zero()
        );
        assert_eq!(
            APInt::from_u8(0b1011, width)
                .xor(&APInt::umax(width))
                .to_u8(),
            0b0100
        );
    }
}

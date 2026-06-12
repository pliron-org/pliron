//! Arbitrary precision float implementation.
//! This is similar in functionality to LLVM's APFloat class.
//!
//! Unlike LLVM's `APFloat`, [rustc_apfloat] does not provide a
//! single `APFloat` type with dynamic semantics. Instead, the semantics
//! are determined by the [Semantics](rustc_apfloat::ieee::Semantics) trait
//! and its associated constants.
//!
//! This module provides two main functionalities:
//! 1. [Printable](crate::printable::Printable) and [Parsable]
//!    implementations for various float types in [rustc_apfloat]
//! 2. An object safe (dyn compatible) trait [DynFloat] that allows for operations
//!    on various [Float] types without knowing the specific type at compile time.
//!    Internally, this just downcasts the [DynFloat] to the specific type
//!    and performs the operation provided by [Float].
//!    *Note* This cannot be part of an [AttrObj](crate::attribute::AttrObj)
//!    because it does not provide a [Parsable] implementation. Providing a
//!    [Parsable] implementation requires knowing the concrete type, and the
//!    concrete types already implement [Parsable].

use core::{
    cmp::Ordering,
    fmt::Display,
    ops::{Add, Div, Mul, Neg, Rem, Sub},
};

use alloc::{
    boxed::Box,
    string::{String, ToString},
};

use downcast_rs::{Downcast, impl_downcast};
use rustc_apfloat::ieee::{
    BFloatS, DoubleS, Float8E4M3FNS, Float8E5M2S, HalfS, NonfiniteBehavior, QuadS, SingleS,
    X87DoubleExtendedS,
};
use thiserror::Error;

pub use rustc_apfloat::{
    Category, ExpInt, Float, Round, StatusAnd,
    ieee::{BFloat, Double, Float8E4M3FN, Float8E5M2, Half, Quad, Single, X87DoubleExtended},
};

use crate::{
    arg_error_noloc,
    combine::{Parser, parser::char},
    impl_printable_for_display, input_error,
    location::Located,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    result::Result,
};

impl_printable_for_display!(BFloat);
impl_printable_for_display!(Double);
impl_printable_for_display!(Float8E4M3FN);
impl_printable_for_display!(Float8E5M2);
impl_printable_for_display!(Half);
impl_printable_for_display!(Quad);
impl_printable_for_display!(Single);
impl_printable_for_display!(X87DoubleExtended);

/// Convert from [rustc_apfloat]'s [Single] to Rust [f32].
pub fn single_to_f32(value: Single) -> f32 {
    // clippy_utils::consts::Constant::parse_f16 (from rust-lang/rust) does this
    f32::from_bits(value.to_bits().try_into().unwrap())
}

/// Convert from Rust [f32] to [rustc_apfloat]'s [Single].
pub fn f32_to_single(value: f32) -> Single {
    // rustc_mir_build::builder::parse_float_into_scalar (from rust-lang/rust) does this
    Single::from_bits(value.to_bits().into())
}

/// Convert from [rustc_apfloat]'s [Double] to Rust [f64].
pub fn double_to_f64(value: Double) -> f64 {
    f64::from_bits(value.to_bits().try_into().unwrap())
}

/// Convert from Rust [f64] to [rustc_apfloat]'s [Double].
pub fn f64_to_double(value: f64) -> Double {
    Double::from_bits(value.to_bits().into())
}

#[derive(Debug, Error)]
pub enum FloatErr {
    #[error("Invalid float literal: {0}")]
    InvalidFloatLiteral(String),
    #[error("Failed to parse float from string {0}: {1}")]
    ParseError(String, String),
}

/// Parse a specified [Float] of type `T`.
pub fn float_parse<'a, T: Float>(
    state_stream: &mut StateStream<'a>,
    _arg: (),
) -> ParseResult<'a, T> {
    // Parse characters allowed in a float literal.
    // Digits, decimal point, exponent, and sign.

    let loc = state_stream.loc();

    let mut allowed_chars = combine::many1::<String, _, _>(
        char::digit()
            .or(char::char('E'))
            .or(char::char('e'))
            .or(char::char('+'))
            .or(char::char('-'))
            .or(char::char('.')),
    );

    let (f_str, _) = allowed_chars
        .parse(state_stream)
        .map_err(|e| input_error!(loc.clone(), FloatErr::InvalidFloatLiteral(e.to_string())))?;

    T::from_str(&f_str)
        .map_err(|e| input_error!(loc, FloatErr::ParseError(f_str, e.0.to_string())))
        .into_parse_result()
}

/// A parser combinator to parse a [Float] of type `T`.
pub fn float_parser<'a, T: Float + 'a>(_arg: ()) -> impl Parser<StateStream<'a>, Output = T> + 'a {
    combine::parser(move |parsable_state: &mut StateStream<'a>| float_parse(parsable_state, ()))
        .boxed()
}

impl Parsable for BFloat {
    type Arg = ();
    type Parsed = BFloat;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for Double {
    type Arg = ();
    type Parsed = Double;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for Float8E4M3FN {
    type Arg = ();
    type Parsed = Float8E4M3FN;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for Float8E5M2 {
    type Arg = ();
    type Parsed = Float8E5M2;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for Half {
    type Arg = ();
    type Parsed = Half;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for Quad {
    type Arg = ();
    type Parsed = Quad;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for Single {
    type Arg = ();
    type Parsed = Single;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

impl Parsable for X87DoubleExtended {
    type Arg = ();
    type Parsed = X87DoubleExtended;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        float_parse(state_stream, ())
    }
}

/// A struct to hold the [semantics](rustc_apfloat::ieee::Semantics) of a floating point type.
pub struct Semantics {
    pub bits: usize,
    pub exp_bits: usize,
    pub precision: usize,
    pub nonfinite_behavior: NonfiniteBehavior,
    pub max_exp: ExpInt,
    pub ieee_max_exp: ExpInt,
    pub min_exp: ExpInt,
    pub ieee_min_exp: ExpInt,
    pub nan_significand_base: u128,
    pub nan_payload_mask: u128,
    pub qnan_significand: u128,
}

pub trait GetSemantics {
    fn get_semantics() -> Semantics
    where
        Self: Sized;
}

macro_rules! impl_get_semantics_for_float {
    ($ty_name:ty, $struct_name:ty) => {
        impl GetSemantics for $ty_name {
            fn get_semantics() -> Semantics {
                use rustc_apfloat::ieee::Semantics;
                crate::utils::apfloat::Semantics {
                    bits: <$struct_name>::BITS,
                    exp_bits: <$struct_name>::EXP_BITS,
                    precision: <$struct_name>::PRECISION,
                    nonfinite_behavior: <$struct_name>::NONFINITE_BEHAVIOR,
                    max_exp: <$struct_name>::MAX_EXP,
                    ieee_max_exp: <$struct_name>::IEEE_MAX_EXP,
                    min_exp: <$struct_name>::MIN_EXP,
                    ieee_min_exp: <$struct_name>::IEEE_MIN_EXP,
                    nan_significand_base: <$struct_name>::NAN_SIGNIFICAND_BASE,
                    nan_payload_mask: <$struct_name>::NAN_PAYLOAD_MASK,
                    qnan_significand: <$struct_name>::QNAN_SIGNIFICAND,
                }
            }
        }
    };
}

impl_get_semantics_for_float!(BFloat, BFloatS);
impl_get_semantics_for_float!(Double, DoubleS);
impl_get_semantics_for_float!(Float8E4M3FN, Float8E4M3FNS);
impl_get_semantics_for_float!(Float8E5M2, Float8E5M2S);
impl_get_semantics_for_float!(Half, HalfS);
impl_get_semantics_for_float!(Quad, QuadS);
impl_get_semantics_for_float!(Single, SingleS);
impl_get_semantics_for_float!(X87DoubleExtended, X87DoubleExtendedS);

/// This is an object safe version of the [Float] trait.
/// *Panics* if operands to an operation are of different float types.
///
/// Some functions that could've been static (Self: Sized) are not,
/// intentionally, to enable building / constructing [DynFloat] objects
/// of the same type as `&self`, but when the concrete type is not known.
/// These functions have a `build_*` prefix.
/// When the concrete type is known, functions in [Float] can be used.
pub trait DynFloat: Downcast + core::fmt::Debug {
    // Function that could've been static (i.e., Self: Sized),
    // but are not for convenience. The static versions are anyway
    // available as methods on the [Float] trait.

    /// [Float::qnan], `self` is ignored
    fn build_qnan(&self, payload: Option<u128>) -> Box<dyn DynFloat>;
    /// [Float::snan], `self` is ignored
    fn build_snan(&self, payload: Option<u128>) -> Box<dyn DynFloat>;
    /// [Float::largest], `self` is ignored
    fn build_largest(&self) -> Box<dyn DynFloat>;
    /// [Float::smallest_normalized], `self` is ignored
    fn build_smallest_normalized(&self) -> Box<dyn DynFloat>;
    /// [Float::from_bits], `self` is ignored
    fn build_from_bits(&self, bits: u128) -> Box<dyn DynFloat>;
    /// [Float::from_u128_r], `self` is ignored
    fn build_from_u128_r(&self, value: u128, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::from_str_r], `self` is ignored
    fn build_from_str_r(&self, s: &str, round: Round) -> Result<StatusAnd<Box<dyn DynFloat>>>;
    /// [Float::from_i128_r], `self` is ignored
    fn build_from_i128_r(&self, value: i128, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::from_i128], `self` is ignored
    fn build_from_i128(&self, value: i128) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::from_u128], `self` is ignored
    fn build_from_u128(&self, value: u128) -> StatusAnd<Box<dyn DynFloat>>;

    // Rest of the non-static (dyn compatible) functions

    /// [core::fmt::Display::fmt]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result;

    /// [Neg::neg]
    fn neg(&self) -> Box<dyn DynFloat>;
    /// [Add::add]
    fn add(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Sub::sub]
    fn sub(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Mul::mul]
    fn mul(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Div::div]
    fn div(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Rem::rem]
    fn rem(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::add_r]
    fn add_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::mul_r]
    fn mul_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::mul_add_r]
    fn mul_add_r(
        &self,
        rhs: &dyn DynFloat,
        addend: &dyn DynFloat,
        round: Round,
    ) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::div_r]
    fn div_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::ieee_rem]
    fn ieee_rem(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::c_fmod]
    fn c_fmod(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::round_to_integral]
    fn round_to_integral(&self, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::next_up]
    fn next_up(&self) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::to_bits]
    fn to_bits(&self) -> u128;
    /// [Float::to_u128_r]
    fn to_u128_r(&self, width: usize, round: Round, is_exact: &mut bool) -> StatusAnd<u128>;
    /// [Float::cmp_abs_normal]
    fn cmp_abs_normal(&self, other: &dyn DynFloat) -> Ordering;
    /// [Float::bitwise_eq]
    fn bitwise_eq(&self, other: &dyn DynFloat) -> bool;
    /// [Float::is_negative]
    fn is_negative(&self) -> bool;
    /// [Float::is_denormal]
    fn is_denormal(&self) -> bool;
    /// [Float::is_signaling]
    fn is_signaling(&self) -> bool;
    /// [Float::category]
    fn category(&self) -> Category;
    /// [Float::get_exact_inverse]
    fn get_exact_inverse(&self) -> Option<Box<dyn DynFloat>>;
    /// [Float::ilogb]
    fn ilogb(&self) -> ExpInt;
    /// [Float::scalbn_r]
    fn scalbn_r(&self, n: ExpInt, round: Round) -> Box<dyn DynFloat>;
    /// [Float::frexp_r]
    fn frexp_r(&self, exp: &mut ExpInt, round: Round) -> Box<dyn DynFloat>;

    /// [Float::sub_r]
    fn sub_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::mul_add]
    fn mul_add(
        &self,
        multiplicand: &dyn DynFloat,
        addend: &dyn DynFloat,
    ) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::next_down]
    fn next_down(&self) -> StatusAnd<Box<dyn DynFloat>>;
    /// [Float::abs]
    fn abs(&self) -> Box<dyn DynFloat>;
    /// [Float::copy_sign]
    fn copy_sign(&self, other: &dyn DynFloat) -> Box<dyn DynFloat>;
    /// [Float::to_i128_r]
    fn to_i128_r(&self, width: usize, round: Round, is_exact: &mut bool) -> StatusAnd<i128>;
    /// [Float::to_i128]
    fn to_i128(&self, width: usize) -> StatusAnd<i128>;
    /// [Float::to_u128]
    fn to_u128(&self, width: usize) -> StatusAnd<u128>;
    /// [Float::min]
    fn min(&self, other: &dyn DynFloat) -> Box<dyn DynFloat>;
    /// [Float::max]
    fn max(&self, other: &dyn DynFloat) -> Box<dyn DynFloat>;
    /// [Float::minimum]
    fn minimum(&self, other: &dyn DynFloat) -> Box<dyn DynFloat>;
    /// [Float::maximum]
    fn maximum(&self, other: &dyn DynFloat) -> Box<dyn DynFloat>;
    /// [Float::is_normal]
    fn is_normal(&self) -> bool;
    /// [Float::is_finite]
    fn is_finite(&self) -> bool;
    /// [Float::is_zero]
    fn is_zero(&self) -> bool;
    /// [Float::is_infinite]
    fn is_infinite(&self) -> bool;
    /// [Float::is_nan]
    fn is_nan(&self) -> bool;
    /// [Float::is_non_zero]
    fn is_non_zero(&self) -> bool;
    /// [Float::is_finite_non_zero]
    fn is_finite_non_zero(&self) -> bool;
    /// [Float::is_pos_zero]
    fn is_pos_zero(&self) -> bool;
    /// [Float::is_neg_zero]
    fn is_neg_zero(&self) -> bool;
    /// [Float::is_pos_infinity]
    fn is_pos_infinity(&self) -> bool;
    /// [Float::is_neg_infinity]
    fn is_neg_infinity(&self) -> bool;
    /// [Float::is_smallest]
    fn is_smallest(&self) -> bool;
    /// [Float::is_smallest_normalized]
    fn is_smallest_normalized(&self) -> bool;
    /// [Float::is_largest]
    fn is_largest(&self) -> bool;
    /// [Float::is_integer]
    fn is_integer(&self) -> bool;
    /// [Float::scalbn]
    fn scalbn(&self, n: ExpInt) -> Box<dyn DynFloat>;
    /// [Float::frexp]
    fn frexp(&self, exp: &mut ExpInt) -> Box<dyn DynFloat>;

    /// Get [Semantics]
    fn semantics(&self) -> Semantics;
    /// Get [Semantics], static version
    fn semantics_static() -> Semantics
    where
        Self: Sized;
}
impl_downcast!(DynFloat);

impl<T: Float + GetSemantics + core::fmt::Debug + 'static> DynFloat for T {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Use the Display trait to format the float value
        <Self as core::fmt::Display>::fmt(self, f)
    }

    fn neg(&self) -> Box<dyn DynFloat> {
        Box::new(<Self as Neg>::neg(*self))
    }
    fn add(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::add");
        <Self as Add>::add(*self, *rhs_float).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn sub(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::sub");
        <Self as Sub>::sub(*self, *rhs_float).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn mul(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::mul");
        <Self as Mul>::mul(*self, *rhs_float).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn div(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::div");
        <Self as Div>::div(*self, *rhs_float).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn rem(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::rem");
        <Self as Rem>::rem(*self, *rhs_float).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }

    fn build_qnan(&self, payload: Option<u128>) -> Box<dyn DynFloat> {
        Box::new(<T as Float>::qnan(payload))
    }
    fn build_snan(&self, payload: Option<u128>) -> Box<dyn DynFloat> {
        Box::new(<T as Float>::snan(payload))
    }
    fn build_largest(&self) -> Box<dyn DynFloat> {
        Box::new(<T as Float>::largest())
    }
    fn build_smallest_normalized(&self) -> Box<dyn DynFloat> {
        Box::new(<T as Float>::smallest_normalized())
    }
    fn add_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::add_r");
        <Self as Float>::add_r(*self, *rhs_float, round)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn mul_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::mul_r");
        <Self as Float>::mul_r(*self, *rhs_float, round)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn mul_add_r(
        &self,
        rhs: &dyn DynFloat,
        addend: &dyn DynFloat,
        round: Round,
    ) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::mul_add_r");
        let addend_float = addend
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::mul_add_r");
        <Self as Float>::mul_add_r(*self, *rhs_float, *addend_float, round)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn div_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::div_r");
        <Self as Float>::div_r(*self, *rhs_float, round)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn ieee_rem(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::ieee_rem");
        <Self as Float>::ieee_rem(*self, *rhs_float)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn c_fmod(&self, rhs: &dyn DynFloat) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::c_fmod");
        <Self as Float>::c_fmod(*self, *rhs_float)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn round_to_integral(&self, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        <Self as Float>::round_to_integral(*self, round)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn next_up(&self) -> StatusAnd<Box<dyn DynFloat>> {
        <Self as Float>::next_up(*self).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn build_from_bits(&self, bits: u128) -> Box<dyn DynFloat> {
        let float_value = <T as Float>::from_bits(bits);
        Box::new(float_value) as Box<dyn DynFloat>
    }
    fn build_from_u128_r(&self, value: u128, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        <T as Float>::from_u128_r(value, round).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn build_from_str_r(&self, s: &str, round: Round) -> Result<StatusAnd<Box<dyn DynFloat>>> {
        <T as Float>::from_str_r(s, round)
            .map_err(|e| {
                // Convert the error to a Result type
                arg_error_noloc!(FloatErr::ParseError(s.to_string(), e.0.to_string()))
            })
            .map(|status_and| status_and.map(|float| Box::new(float) as Box<dyn DynFloat>))
    }
    fn to_bits(&self) -> u128 {
        <Self as Float>::to_bits(*self)
    }
    fn to_u128_r(&self, width: usize, round: Round, is_exact: &mut bool) -> StatusAnd<u128> {
        <Self as Float>::to_u128_r(*self, width, round, is_exact)
    }
    fn cmp_abs_normal(&self, other: &dyn DynFloat) -> Ordering {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::cmp_abs_normal");
        <Self as Float>::cmp_abs_normal(*self, *other_float)
    }
    fn bitwise_eq(&self, other: &dyn DynFloat) -> bool {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::bitwise_eq");
        <Self as Float>::bitwise_eq(*self, *other_float)
    }
    fn is_negative(&self) -> bool {
        <Self as Float>::is_negative(*self)
    }
    fn is_denormal(&self) -> bool {
        <Self as Float>::is_denormal(*self)
    }
    fn is_signaling(&self) -> bool {
        <Self as Float>::is_signaling(*self)
    }
    fn category(&self) -> Category {
        <Self as Float>::category(*self)
    }
    fn get_exact_inverse(&self) -> Option<Box<dyn DynFloat>> {
        <Self as Float>::get_exact_inverse(*self)
            .map(|inverse| Box::new(inverse) as Box<dyn DynFloat>)
    }
    fn ilogb(&self) -> ExpInt {
        <Self as Float>::ilogb(*self)
    }
    fn scalbn_r(&self, n: ExpInt, round: Round) -> Box<dyn DynFloat> {
        Box::new(<Self as Float>::scalbn_r(*self, n, round))
    }
    fn frexp_r(&self, exp: &mut ExpInt, round: Round) -> Box<dyn DynFloat> {
        Box::new(<Self as Float>::frexp_r(*self, exp, round))
    }

    fn sub_r(&self, rhs: &dyn DynFloat, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        let rhs_float = rhs
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::sub_r");
        <Self as Float>::sub_r(*self, *rhs_float, round)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn mul_add(
        &self,
        multiplicand: &dyn DynFloat,
        addend: &dyn DynFloat,
    ) -> StatusAnd<Box<dyn DynFloat>> {
        let multiplicand_float = multiplicand
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::mul_add");
        let addend_float = addend
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::mul_add");
        <Self as Float>::mul_add(*self, *multiplicand_float, *addend_float)
            .map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn next_down(&self) -> StatusAnd<Box<dyn DynFloat>> {
        <Self as Float>::next_down(*self).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn abs(&self) -> Box<dyn DynFloat> {
        Box::new(<Self as Float>::abs(*self))
    }
    fn copy_sign(&self, other: &dyn DynFloat) -> Box<dyn DynFloat> {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::copy_sign");
        Box::new(<Self as Float>::copy_sign(*self, *other_float))
    }
    fn build_from_i128_r(&self, value: i128, round: Round) -> StatusAnd<Box<dyn DynFloat>> {
        <T as Float>::from_i128_r(value, round).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn build_from_i128(&self, value: i128) -> StatusAnd<Box<dyn DynFloat>> {
        <T as Float>::from_i128(value).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn build_from_u128(&self, value: u128) -> StatusAnd<Box<dyn DynFloat>> {
        <T as Float>::from_u128(value).map(|result| Box::new(result) as Box<dyn DynFloat>)
    }
    fn to_i128_r(&self, width: usize, round: Round, is_exact: &mut bool) -> StatusAnd<i128> {
        <Self as Float>::to_i128_r(*self, width, round, is_exact)
    }
    fn to_i128(&self, width: usize) -> StatusAnd<i128> {
        <Self as Float>::to_i128(*self, width)
    }
    fn to_u128(&self, width: usize) -> StatusAnd<u128> {
        <Self as Float>::to_u128(*self, width)
    }
    fn minimum(&self, other: &dyn DynFloat) -> Box<dyn DynFloat> {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::minimum");
        Box::new(<Self as Float>::minimum(*self, *other_float))
    }
    fn maximum(&self, other: &dyn DynFloat) -> Box<dyn DynFloat> {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::maximum");
        Box::new(<Self as Float>::maximum(*self, *other_float))
    }
    fn min(&self, other: &dyn DynFloat) -> Box<dyn DynFloat> {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::min");
        Box::new(<Self as Float>::min(*self, *other_float))
    }
    fn max(&self, other: &dyn DynFloat) -> Box<dyn DynFloat> {
        let other_float = other
            .downcast_ref::<T>()
            .expect("Type mismatch in DynFloat::max");
        Box::new(<Self as Float>::max(*self, *other_float))
    }
    fn is_normal(&self) -> bool {
        <Self as Float>::is_normal(*self)
    }
    fn is_finite(&self) -> bool {
        <Self as Float>::is_finite(*self)
    }
    fn is_zero(&self) -> bool {
        <Self as Float>::is_zero(*self)
    }
    fn is_infinite(&self) -> bool {
        <Self as Float>::is_infinite(*self)
    }
    fn is_nan(&self) -> bool {
        <Self as Float>::is_nan(*self)
    }
    fn is_non_zero(&self) -> bool {
        <Self as Float>::is_non_zero(*self)
    }
    fn is_finite_non_zero(&self) -> bool {
        <Self as Float>::is_finite_non_zero(*self)
    }
    fn is_pos_zero(&self) -> bool {
        <Self as Float>::is_pos_zero(*self)
    }
    fn is_neg_zero(&self) -> bool {
        <Self as Float>::is_neg_zero(*self)
    }
    fn is_pos_infinity(&self) -> bool {
        <Self as Float>::is_pos_infinity(*self)
    }
    fn is_neg_infinity(&self) -> bool {
        <Self as Float>::is_neg_infinity(*self)
    }
    fn is_smallest_normalized(&self) -> bool {
        <Self as Float>::is_smallest_normalized(*self)
    }
    fn is_smallest(&self) -> bool {
        <Self as Float>::is_smallest(*self)
    }
    fn is_largest(&self) -> bool {
        <Self as Float>::is_largest(*self)
    }
    fn is_integer(&self) -> bool {
        <Self as Float>::is_integer(*self)
    }
    fn scalbn(&self, n: ExpInt) -> Box<dyn DynFloat> {
        Box::new(<Self as Float>::scalbn(*self, n))
    }
    fn frexp(&self, exp: &mut ExpInt) -> Box<dyn DynFloat> {
        Box::new(<Self as Float>::frexp(*self, exp))
    }

    fn semantics(&self) -> Semantics {
        Self::semantics_static()
    }
    fn semantics_static() -> Semantics
    where
        Self: Sized,
    {
        <Self as GetSemantics>::get_semantics()
    }
}

impl Display for dyn DynFloat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <Self as DynFloat>::fmt(self, f)
    }
}

impl_printable_for_display!(dyn DynFloat);

impl<T: Float + GetSemantics + core::fmt::Debug + 'static> From<T> for Box<dyn DynFloat> {
    fn from(value: T) -> Self {
        Box::new(value)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{str::FromStr, string::ToString};

    use rustc_apfloat::ParseError;

    use crate::{
        context::Context,
        location,
        parsable::{self, state_stream_from_iterator},
        printable::Printable,
        utils::apfloat::{double_to_f64, f32_to_single, f64_to_double, single_to_f32},
    };

    use super::{
        BFloat, Double, Float8E4M3FN, Float8E5M2, Half, Parsable, Quad, Single, X87DoubleExtended,
    };

    fn print_and_parse<T>(value: T, ctx: &mut Context)
    where
        T: Printable + Parsable<Arg = (), Parsed = T> + PartialEq,
    {
        let parsed = {
            let s = value.disp(ctx).to_string();
            let mut state_stream = state_stream_from_iterator(
                s.chars(),
                parsable::State::new(ctx, location::Source::InMemory),
            );
            match T::parse(&mut state_stream, ()) {
                Ok((parsed_res, _)) => parsed_res,
                Err(err) => {
                    panic!("{}\nError parsing {}", err.into_inner().error, s);
                }
            }
        };
        assert!(value == parsed, "Failed for value: {}", value.disp(ctx));
    }

    #[test]
    fn test_bfloat_print_parse() -> Result<(), ParseError> {
        let values = [
            BFloat::from_str("0.0")?,
            BFloat::from_str("1.5")?,
            BFloat::from_str("-2.25")?,
            BFloat::from_str("12345.0")?,
            BFloat::from_str("-0.0001")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_double_print_parse() -> Result<(), ParseError> {
        let values = [
            Double::from_str("0.0")?,
            Double::from_str("1.5")?,
            Double::from_str("-2.25")?,
            Double::from_str("12345.0")?,
            Double::from_str("-0.0001")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_float8e4m3fn_print_parse() -> Result<(), ParseError> {
        let values = [
            Float8E4M3FN::from_str("0.0")?,
            Float8E4M3FN::from_str("1.5")?,
            Float8E4M3FN::from_str("-2.25")?,
            Float8E4M3FN::from_str("12.0")?,
            Float8E4M3FN::from_str("-0.125")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_float8e5m2_print_parse() -> Result<(), ParseError> {
        let values = [
            Float8E5M2::from_str("0.0")?,
            Float8E5M2::from_str("1.5")?,
            Float8E5M2::from_str("-2.25")?,
            Float8E5M2::from_str("12.0")?,
            Float8E5M2::from_str("-0.125")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_half_print_parse() -> Result<(), ParseError> {
        let values = [
            Half::from_str("0.0")?,
            Half::from_str("1.5")?,
            Half::from_str("-2.25")?,
            Half::from_str("12345.0")?,
            Half::from_str("-0.0001")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_quad_print_parse() -> Result<(), ParseError> {
        let values = [
            Quad::from_str("0.0")?,
            Quad::from_str("1.5")?,
            Quad::from_str("-2.25")?,
            Quad::from_str("12345.0")?,
            Quad::from_str("-0.0001")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_single_print_parse() -> Result<(), ParseError> {
        let values = [
            Single::from_str("0.0")?,
            Single::from_str("1.5")?,
            Single::from_str("-2.25")?,
            Single::from_str("12345.0")?,
            Single::from_str("-0.0001")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_x87doubleextended_print_parse() -> Result<(), ParseError> {
        let values = [
            X87DoubleExtended::from_str("0.0")?,
            X87DoubleExtended::from_str("1.5")?,
            X87DoubleExtended::from_str("-2.25")?,
            X87DoubleExtended::from_str("12345.0")?,
            X87DoubleExtended::from_str("-0.0001")?,
        ];
        for v in values {
            print_and_parse(v, &mut Context::default())
        }
        Ok(())
    }

    #[test]
    fn test_single_to_f32_and_f32_to_single() {
        let cases = [
            0.0f32,
            -0.0f32,
            1.0f32,
            -1.0f32,
            f32::MIN,
            f32::MAX,
            f32::EPSILON,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];

        for &val in &cases {
            let single = f32_to_single(val);
            let back = single_to_f32(single);
            if val.is_nan() {
                assert!(back.is_nan(), "Expected NaN for input {}", val);
            } else {
                assert_eq!(val, back, "Failed for value: {}", val);
            }
        }
    }

    #[test]
    fn test_double_to_f64_and_f64_to_double() {
        let cases = [
            0.0f64,
            -0.0f64,
            1.0f64,
            -1.0f64,
            f64::MIN,
            f64::MAX,
            f64::EPSILON,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];

        for &val in &cases {
            let double = f64_to_double(val);
            let back = double_to_f64(double);
            if val.is_nan() {
                assert!(back.is_nan(), "Expected NaN for input {}", val);
            } else {
                assert_eq!(val, back, "Failed for value: {}", val);
            }
        }
    }

    #[test]
    fn test_single_to_f32_subnormals_and_extremes() {
        // Smallest positive subnormal
        let val = f32::from_bits(0x00000001);
        let single = f32_to_single(val);
        let back = single_to_f32(single);
        assert_eq!(val, back);

        // Largest subnormal
        let val = f32::from_bits(0x007FFFFF);
        let single = f32_to_single(val);
        let back = single_to_f32(single);
        assert_eq!(val, back);

        // Smallest positive normal
        let val = f32::from_bits(0x00800000);
        let single = f32_to_single(val);
        let back = single_to_f32(single);
        assert_eq!(val, back);
    }

    #[test]
    fn test_double_to_f64_subnormals_and_extremes() {
        // Smallest positive subnormal
        let val = f64::from_bits(0x0000000000000001);
        let double = f64_to_double(val);
        let back = double_to_f64(double);
        assert_eq!(val, back);

        // Largest subnormal
        let val = f64::from_bits(0x000FFFFFFFFFFFFF);
        let double = f64_to_double(val);
        let back = double_to_f64(double);
        assert_eq!(val, back);

        // Smallest positive normal
        let val = f64::from_bits(0x0010000000000000);
        let double = f64_to_double(val);
        let back = double_to_f64(double);
        assert_eq!(val, back);
    }
}

use core::cmp::Ordering;

use crate::{
    attribute::Attribute,
    context::{Context, Ptr},
    operation::Operation,
    result::Result,
    storage_uniquer::TypeValueHash,
    r#type::TypeObj,
    utils::apfloat::{Category, DynFloat, ExpInt, Round, Semantics, StatusAnd},
};
use alloc::boxed::Box;
use pliron::derive::attr_interface;

/// [Attribute]s that have an associated [Type](crate::type::Type).
/// This serves the same purpose as MLIR's `TypedAttrInterface`.
#[attr_interface]
pub trait TypedAttrInterface {
    /// Get this attribute's type.
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj>;

    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// [Attribute]s that should be printed after the top level [Operation](crate::operation::Operation)
/// is printed. An [Op](crate::op::Op) may choose to print such an attribute as part of its
/// syntax specification. This will be unknown to the outline attributes printer and will be
/// printed nevertheless while printing all outline attributes.
#[attr_interface]
pub trait OutlinedAttr {
    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// [Attribute]s that should be printed only once, and some form of "reference" to be
/// used for repeated printing. These must be outlined attributes.
#[attr_interface]
pub trait PrintOnceAttr: OutlinedAttr {
    /// Compute and get the hash for this instance of Self.
    /// Hash collisions can be a possibility.
    ///
    /// A typical implementation involves deriving `Hash` for `Self`
    /// and calling `TypeValueHash::new(self)` inside this method.
    fn hash_attr(&self) -> TypeValueHash;

    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// [Attribute]s that represent a floating point value.
///
/// This trait provides operations on floating point values
/// from the [DynFloat] trait.
///
/// The `build_*` methods can be used to construct a new [FloatAttr]
/// with the same type as the argument. This can be useful when doing
/// folding optimizations, where we have some dyn instance and want to build
/// another one of the same type but with a different value.
/// The `self` argument is ignored in these methods. In the absense of
/// of an object (but when the concrete type is known), then the static (`Self: Sized`)
/// methods on the [Float](rustc_apfloat::Float) trait can be used instead.
#[attr_interface]
pub trait FloatAttr: TypedAttrInterface {
    /// Get the underlying floating point value as a [DynFloat] trait object.
    fn get_inner(&self) -> &dyn DynFloat;
    /// Build a [FloatAttr] with the same concrete type as `Self`, from the given trait object.
    fn build_from(&self, df: Box<dyn DynFloat>) -> Box<dyn FloatAttr>;
    /// Get semantics of the underlying floating point type.
    fn get_semantics(&self) -> Semantics;
    /// Static version of `get_semantics`.
    fn get_semantics_static() -> Semantics
    where
        Self: Sized;

    /// [Float::qnan](rustc_apfloat::Float::qnan), `self` is ignored
    fn build_qnan(&self, payload: Option<u128>) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let qnan = df.build_qnan(payload);
        self.build_from(qnan)
    }
    /// [Float::snan](rustc_apfloat::Float::snan), `self` is ignored
    fn build_snan(&self, payload: Option<u128>) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let snan = df.build_snan(payload);
        self.build_from(snan)
    }
    /// [Float::largest](rustc_apfloat::Float::largest), `self` is ignored
    fn build_largest(&self) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let largest = df.build_largest();
        self.build_from(largest)
    }
    /// [Float::smallest_normalized](rustc_apfloat::Float::smallest_normalized), `self` is ignored
    fn build_smallest_normalized(&self) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let smallest_normalized = df.build_smallest_normalized();
        self.build_from(smallest_normalized)
    }
    /// [Float::from_bits](rustc_apfloat::Float::from_bits), `self` is ignored
    fn build_from_bits(&self, bits: u128) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let from_bits = df.build_from_bits(bits);
        self.build_from(from_bits)
    }
    /// [Float::from_u128_r](rustc_apfloat::Float::from_u128_r), `self` is ignored
    fn build_from_u128_r(&self, value: u128, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.build_from_u128_r(value, round)
            .map(|df| self.build_from(df))
    }
    /// [Float::from_str_r](rustc_apfloat::Float::from_str_r), `self` is ignored
    fn build_from_str_r(&self, s: &str, round: Round) -> Result<StatusAnd<Box<dyn FloatAttr>>> {
        let df = self.get_inner();
        let res = df.build_from_str_r(s, round)?;
        Ok(res.map(|df| self.build_from(df)))
    }
    /// [Float::from_i128_r](rustc_apfloat::Float::from_i128_r), `self` is ignored
    fn build_from_i128_r(&self, value: i128, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.build_from_i128_r(value, round)
            .map(|from_i128_r| self.build_from(from_i128_r))
    }
    /// [Float::from_i128](rustc_apfloat::Float::from_i128), `self` is ignored
    fn build_from_i128(&self, value: i128) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.build_from_i128(value)
            .map(|from_i128| self.build_from(from_i128))
    }
    /// [Float::from_u128](rustc_apfloat::Float::from_u128), `self` is ignored
    fn build_from_u128(&self, value: u128) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.build_from_u128(value)
            .map(|from_u128| self.build_from(from_u128))
    }

    /// [Neg::neg](core::ops::Neg::neg)
    fn neg(&self) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let negated = df.neg();
        self.build_from(negated)
    }
    /// [Add::add](core::ops::Add::add)
    fn add(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.add(rhs_df).map(|add| self.build_from(add))
    }
    /// [Sub::sub](core::ops::Sub::sub)
    fn sub(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.sub(rhs_df).map(|sub| self.build_from(sub))
    }
    /// [Mul::mul](core::ops::Mul::mul)
    fn mul(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.mul(rhs_df).map(|mul| self.build_from(mul))
    }
    /// [Div::div](core::ops::Div::div)
    fn div(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.div(rhs_df).map(|div| self.build_from(div))
    }
    /// [Rem::rem](core::ops::Rem::rem)
    fn rem(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.rem(rhs_df).map(|rem| self.build_from(rem))
    }
    /// [Float::add_r](rustc_apfloat::Float::add_r)
    fn add_r(&self, rhs: &dyn FloatAttr, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.add_r(rhs_df, round).map(|add_r| self.build_from(add_r))
    }
    /// [Float::mul_r](rustc_apfloat::Float::mul_r)
    fn mul_r(&self, rhs: &dyn FloatAttr, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.mul_r(rhs_df, round).map(|mul_r| self.build_from(mul_r))
    }
    /// [Float::mul_add_r](rustc_apfloat::Float::mul_add_r)
    fn mul_add_r(
        &self,
        rhs: &dyn FloatAttr,
        addend: &dyn FloatAttr,
        round: Round,
    ) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        let addend_df = addend.get_inner();
        df.mul_add_r(rhs_df, addend_df, round)
            .map(|mul_add_r| self.build_from(mul_add_r))
    }
    /// [Float::div_r](rustc_apfloat::Float::div_r)
    fn div_r(&self, rhs: &dyn FloatAttr, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.div_r(rhs_df, round).map(|div_r| self.build_from(div_r))
    }
    /// [Float::ieee_rem](rustc_apfloat::Float::ieee_rem)
    fn ieee_rem(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.ieee_rem(rhs_df)
            .map(|ieee_rem| self.build_from(ieee_rem))
    }
    /// [Float::c_fmod](rustc_apfloat::Float::c_fmod)
    fn c_fmod(&self, rhs: &dyn FloatAttr) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.c_fmod(rhs_df).map(|c_fmod| self.build_from(c_fmod))
    }
    /// [Float::round_to_integral](rustc_apfloat::Float::round_to_integral)
    fn round_to_integral(&self, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.round_to_integral(round)
            .map(|round_to_integral| self.build_from(round_to_integral))
    }
    /// [Float::next_up](rustc_apfloat::Float::next_up)
    fn next_up(&self) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.next_up().map(|next_up| self.build_from(next_up))
    }
    /// [Float::to_bits](rustc_apfloat::Float::to_bits)
    fn to_bits(&self) -> u128 {
        let df = self.get_inner();
        df.to_bits()
    }
    /// [Float::to_u128_r](rustc_apfloat::Float::to_u128_r)
    fn to_u128_r(&self, width: usize, round: Round, is_exact: &mut bool) -> StatusAnd<u128> {
        let df = self.get_inner();
        df.to_u128_r(width, round, is_exact)
    }
    /// [Float::cmp_abs_normal](rustc_apfloat::Float::cmp_abs_normal)
    fn cmp_abs_normal(&self, other: &dyn FloatAttr) -> Ordering {
        let df = self.get_inner();
        df.cmp_abs_normal(other.get_inner())
    }
    /// [Float::bitwise_eq](rustc_apfloat::Float::bitwise_eq)
    fn bitwise_eq(&self, other: &dyn FloatAttr) -> bool {
        let df = self.get_inner();
        df.bitwise_eq(other.get_inner())
    }
    /// [Float::is_negative](rustc_apfloat::Float::is_negative)
    fn is_negative(&self) -> bool {
        let df = self.get_inner();
        df.is_negative()
    }
    /// [Float::is_denormal](rustc_apfloat::Float::is_denormal)
    fn is_denormal(&self) -> bool {
        let df = self.get_inner();
        df.is_denormal()
    }
    /// [Float::is_signaling](rustc_apfloat::Float::is_signaling)
    fn is_signaling(&self) -> bool {
        let df = self.get_inner();
        df.is_signaling()
    }
    /// [Float::category](rustc_apfloat::Float::category)
    fn category(&self) -> Category {
        let df = self.get_inner();
        df.category()
    }
    /// [Float::get_exact_inverse](rustc_apfloat::Float::get_exact_inverse)
    fn get_exact_inverse(&self) -> Option<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.get_exact_inverse()
            .map(|inverse| self.build_from(inverse))
    }
    /// [Float::ilogb](rustc_apfloat::Float::ilogb)
    fn ilogb(&self) -> ExpInt {
        let df = self.get_inner();
        df.ilogb()
    }
    /// [Float::scalbn_r](rustc_apfloat::Float::scalbn_r)
    fn scalbn_r(&self, n: ExpInt, round: Round) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        self.build_from(df.scalbn_r(n, round))
    }
    /// [Float::frexp_r](rustc_apfloat::Float::frexp_r)
    fn frexp_r(&self, exp: &mut ExpInt, round: Round) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let frexp_r = df.frexp_r(exp, round);
        self.build_from(frexp_r)
    }
    /// [Float::sub_r](rustc_apfloat::Float::sub_r)
    fn sub_r(&self, rhs: &dyn FloatAttr, round: Round) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let rhs_df = rhs.get_inner();
        df.sub_r(rhs_df, round).map(|sub_r| self.build_from(sub_r))
    }
    /// [Float::mul_add](rustc_apfloat::Float::mul_add)
    fn mul_add(
        &self,
        multiplicand: &dyn FloatAttr,
        addend: &dyn FloatAttr,
    ) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        let multiplicand_df = multiplicand.get_inner();
        let addend_df = addend.get_inner();
        df.mul_add(multiplicand_df, addend_df)
            .map(|mul_add| self.build_from(mul_add))
    }
    /// [Float::next_down](rustc_apfloat::Float::next_down)
    fn next_down(&self) -> StatusAnd<Box<dyn FloatAttr>> {
        let df = self.get_inner();
        df.next_down().map(|next_down| self.build_from(next_down))
    }
    /// [Float::abs](rustc_apfloat::Float::abs)
    fn abs(&self) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let abs = df.abs();
        self.build_from(abs)
    }
    /// [Float::copy_sign](rustc_apfloat::Float::copy_sign)
    fn copy_sign(&self, other: &dyn FloatAttr) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let other_df = other.get_inner();
        self.build_from(df.copy_sign(other_df))
    }
    /// [Float::to_i128_r](rustc_apfloat::Float::to_i128_r)
    fn to_i128_r(&self, width: usize, round: Round, is_exact: &mut bool) -> StatusAnd<i128> {
        let df = self.get_inner();
        df.to_i128_r(width, round, is_exact)
    }
    /// [Float::to_i128](rustc_apfloat::Float::to_i128)
    fn to_i128(&self, width: usize) -> StatusAnd<i128> {
        let df = self.get_inner();
        df.to_i128(width)
    }
    /// [Float::to_u128](rustc_apfloat::Float::to_u128)
    fn to_u128(&self, width: usize) -> StatusAnd<u128> {
        let df = self.get_inner();
        df.to_u128(width)
    }
    /// [Float::min](rustc_apfloat::Float::min)
    fn min(&self, other: &dyn FloatAttr) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let other_df = other.get_inner();
        self.build_from(df.min(other_df))
    }
    /// [Float::max](rustc_apfloat::Float::max)
    fn max(&self, other: &dyn FloatAttr) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let other_df = other.get_inner();
        self.build_from(df.max(other_df))
    }
    /// [Float::minimum](rustc_apfloat::Float::minimum)
    fn minimum(&self, other: &dyn FloatAttr) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let other_df = other.get_inner();
        self.build_from(df.minimum(other_df))
    }
    /// [Float::maximum](rustc_apfloat::Float::maximum)
    fn maximum(&self, other: &dyn FloatAttr) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let other_df = other.get_inner();
        self.build_from(df.maximum(other_df))
    }
    /// [Float::is_normal](rustc_apfloat::Float::is_normal)
    fn is_normal(&self) -> bool {
        let df = self.get_inner();
        df.is_normal()
    }
    /// [Float::is_finite](rustc_apfloat::Float::is_finite)
    fn is_finite(&self) -> bool {
        let df = self.get_inner();
        df.is_finite()
    }
    /// [Float::is_zero](rustc_apfloat::Float::is_zero)
    fn is_zero(&self) -> bool {
        let df = self.get_inner();
        df.is_zero()
    }
    /// [Float::is_infinite](rustc_apfloat::Float::is_infinite)
    fn is_infinite(&self) -> bool {
        let df = self.get_inner();
        df.is_infinite()
    }
    /// [Float::is_nan](rustc_apfloat::Float::is_nan)
    fn is_nan(&self) -> bool {
        let df = self.get_inner();
        df.is_nan()
    }
    /// [Float::is_non_zero](rustc_apfloat::Float::is_non_zero)
    fn is_non_zero(&self) -> bool {
        let df = self.get_inner();
        df.is_non_zero()
    }
    /// [Float::is_finite_non_zero](rustc_apfloat::Float::is_finite_non_zero)
    fn is_finite_non_zero(&self) -> bool {
        let df = self.get_inner();
        df.is_finite_non_zero()
    }
    /// [Float::is_pos_zero](rustc_apfloat::Float::is_pos_zero)
    fn is_pos_zero(&self) -> bool {
        let df = self.get_inner();
        df.is_pos_zero()
    }
    /// [Float::is_neg_zero](rustc_apfloat::Float::is_neg_zero)
    fn is_neg_zero(&self) -> bool {
        let df = self.get_inner();
        df.is_neg_zero()
    }
    /// [Float::is_pos_infinity](rustc_apfloat::Float::is_pos_infinity)
    fn is_pos_infinity(&self) -> bool {
        let df = self.get_inner();
        df.is_pos_infinity()
    }
    /// [Float::is_neg_infinity](rustc_apfloat::Float::is_neg_infinity)
    fn is_neg_infinity(&self) -> bool {
        let df = self.get_inner();
        df.is_neg_infinity()
    }
    /// [Float::is_smallest](rustc_apfloat::Float::is_smallest)
    fn is_smallest(&self) -> bool {
        let df = self.get_inner();
        df.is_smallest()
    }
    /// [Float::is_smallest_normalized](rustc_apfloat::Float::is_smallest_normalized)
    fn is_smallest_normalized(&self) -> bool {
        let df = self.get_inner();
        df.is_smallest_normalized()
    }
    /// [Float::is_largest](rustc_apfloat::Float::is_largest)
    fn is_largest(&self) -> bool {
        let df = self.get_inner();
        df.is_largest()
    }
    /// [Float::is_integer](rustc_apfloat::Float::is_integer)
    fn is_integer(&self) -> bool {
        let df = self.get_inner();
        df.is_integer()
    }
    /// [Float::scalbn](rustc_apfloat::Float::scalbn)
    fn scalbn(&self, n: ExpInt) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let scalbn = df.scalbn(n);
        self.build_from(scalbn)
    }
    /// [Float::frexp](rustc_apfloat::Float::frexp)
    fn frexp(&self, exp: &mut ExpInt) -> Box<dyn FloatAttr> {
        let df = self.get_inner();
        let frexp_r = df.frexp(exp);
        self.build_from(frexp_r)
    }

    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// [Attribute]s for which we can generate pliron code to store the attribute value
/// into an pliron value
#[attr_interface]
pub trait MaterializableAttr: TypedAttrInterface {
    /// Returns an operation that assigns a materialization of `self` to some result
    fn materialize(&self, ctx: &mut Context) -> Ptr<Operation>;

    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rustc_apfloat::ieee::Single;

    use super::*;
    use crate::builtin::attributes::FPSingleAttr;

    #[test]
    fn test_float_attr_give_build_qnan_neg() {
        let attr = FPSingleAttr(Single::from_str("1.0").unwrap());

        let qnan = attr.build_qnan(Some(42));
        assert!(qnan.get_inner().is_nan());

        let neg = attr.neg();
        assert!(
            (&*neg as &dyn Attribute)
                .downcast_ref::<FPSingleAttr>()
                .unwrap()
                != &attr
        );
        let neg_neg = neg.neg();
        assert!(
            (&*neg_neg as &dyn Attribute)
                .downcast_ref::<FPSingleAttr>()
                .is_some_and(|n| n == &attr)
        );
    }
}

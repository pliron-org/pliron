// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Attributes belonging to the LLVM dialect.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::{
    fmt::Display,
    hash::{Hash, Hasher},
};
use thiserror::Error;

use pliron::{
    attribute::{AttrObj, attr_type},
    builtin::{
        attr_interfaces::TypedAttrInterface,
        attributes::{IntegerAttr, StringAttr},
        ops::ModuleOp,
        types::{IntegerType, Signedness},
    },
    combine::{self, Parser, choice, parser::char::spaces},
    common_traits::Verify,
    context::Context,
    derive::{attr_interface_impl, format, pliron_attr},
    dict_key,
    identifier::Identifier,
    impl_printable_for_display, input_error,
    location::Located,
    op::Op,
    parsable::{IntoParseResult, Parsable},
    printable::Printable,
    result::Result,
    r#type::{TypeHandle, TypedHandle},
    verify_err_noloc,
};

use crate::types::{ArrayType, PointerType, StructType, VectorType};

use bitflags::bitflags;

/// Integer overflow flags for arithmetic operations.
/// The description below is from LLVM's
/// [release notes](https://releases.llvm.org/2.6/docs/ReleaseNotes.html)
/// that added the flags.
/// "nsw" and "nuw" bits indicate that the operation is guaranteed to not overflow
/// (in the signed or unsigned case, respectively). This gives the optimizer more information
///  and can be used for things like C signed integer values, which are undefined on overflow.
#[pliron_attr(name = "llvm.integer_overlflow_flags", format, verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Default, Hash)]
pub struct IntegerOverflowFlagsAttr {
    pub nsw: bool,
    pub nuw: bool,
}

bitflags! {
    /// Fast math flags for floating point operations.
    #[derive(PartialEq, Eq, Clone, Debug, Hash, Copy)]
    pub struct FastmathFlags: u8 {
        const NNAN = 1;
        const NINF = 2;
        const NSZ = 4;
        const ARCP = 8;
        const CONTRACT = 16;
        const AFN = 32;
        const REASSOC = 64;
        const FAST = 127;
    }
}

#[pliron_attr(name = "llvm.fast_math_flags", verifier = "succ")]
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct FastmathFlagsAttr(pub FastmathFlags);

impl Default for FastmathFlagsAttr {
    fn default() -> Self {
        FastmathFlagsAttr(FastmathFlags::empty())
    }
}

impl From<FastmathFlags> for FastmathFlagsAttr {
    fn from(value: FastmathFlags) -> Self {
        FastmathFlagsAttr(value)
    }
}

impl From<FastmathFlagsAttr> for FastmathFlags {
    fn from(attr: FastmathFlagsAttr) -> Self {
        attr.0
    }
}

impl Display for FastmathFlagsAttr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "<")?;
        bitflags::parser::to_writer(&self.0, &mut *f)?;
        write!(f, ">")
    }
}

impl_printable_for_display!(FastmathFlagsAttr);

#[derive(Debug, Error)]
#[error("Error parsing fastmath flags: {0}")]
pub struct FastmathFlagParseErr(pub bitflags::parser::ParseError);

impl Parsable for FastmathFlagsAttr {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut pliron::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> pliron::parsable::ParseResult<'a, Self::Parsed> {
        let pos = state_stream.loc();
        let allowed_chars = combine::choice!(
            combine::parser::char::space().map(|c| c.to_string()),
            combine::parser::char::alpha_num().map(|c| c.to_string()),
            combine::parser::char::char('|').map(|c: char| c.to_string())
        );

        let (parsed, _): (Vec<String>, _) = combine::between(
            combine::parser::char::char('<').with(spaces()),
            spaces().with(combine::parser::char::char('>')),
            combine::many(allowed_chars),
        )
        .parse_stream(state_stream)
        .into_result()?;
        let parsed_string = parsed.concat();

        let (fast_math_flags, _) = bitflags::parser::from_str::<FastmathFlags>(&parsed_string)
            .map_err(|e| input_error!(pos.clone(), FastmathFlagParseErr(e)))
            .into_parse_result()?;

        Ok(FastmathFlagsAttr(fast_math_flags)).into_parse_result()
    }
}

bitflags! {
    /// No-wrap flags for getelementptr operations.
    #[derive(PartialEq, Eq, Clone, Debug, Hash, Copy)]
    pub struct GepNoWrapFlags: u8 {
        const INBOUNDS = 1;
        const NUSW = 2;
        const NUW = 4;
    }
}

impl GepNoWrapFlags {
    /// Normalize flags to LLVM semantics: `inbounds` implies `nusw`.
    pub fn normalized(self) -> Self {
        if self.contains(Self::INBOUNDS) {
            self | Self::NUSW
        } else {
            self
        }
    }
}

#[pliron_attr(name = "llvm.gep_no_wrap_flags", verifier = "succ")]
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct GepNoWrapFlagsAttr(pub GepNoWrapFlags);

impl Default for GepNoWrapFlagsAttr {
    fn default() -> Self {
        Self(GepNoWrapFlags::empty())
    }
}

impl From<GepNoWrapFlags> for GepNoWrapFlagsAttr {
    fn from(value: GepNoWrapFlags) -> Self {
        Self(value.normalized())
    }
}

impl From<GepNoWrapFlagsAttr> for GepNoWrapFlags {
    fn from(attr: GepNoWrapFlagsAttr) -> Self {
        attr.0
    }
}

impl Display for GepNoWrapFlagsAttr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut flags = self.0.normalized();

        // `inbounds` implies `nusw`, but LLVM prints the implied `nusw`
        // implicitly rather than redundantly.
        if flags.contains(GepNoWrapFlags::INBOUNDS) {
            flags.remove(GepNoWrapFlags::NUSW);
        }

        write!(f, "<")?;
        bitflags::parser::to_writer(&flags, &mut *f)?;
        write!(f, ">")
    }
}

impl_printable_for_display!(GepNoWrapFlagsAttr);

#[derive(Debug, Error)]
#[error("Error parsing GEP no-wrap flags: {0}")]
pub struct GepNoWrapFlagParseErr(pub bitflags::parser::ParseError);

impl Parsable for GepNoWrapFlagsAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut pliron::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> pliron::parsable::ParseResult<'a, Self::Parsed> {
        let pos = state_stream.loc();
        let allowed_chars = combine::choice!(
            combine::parser::char::space().map(|c| c.to_string()),
            combine::parser::char::alpha_num().map(|c| c.to_string()),
            combine::parser::char::char('|').map(|c: char| c.to_string())
        );

        let (parsed, _): (Vec<String>, _) = combine::between(
            combine::parser::char::char('<').with(spaces()),
            spaces().with(combine::parser::char::char('>')),
            combine::many(allowed_chars),
        )
        .parse_stream(state_stream)
        .into_result()?;
        let parsed_string = parsed.concat();

        let (flags, _) = bitflags::parser::from_str::<GepNoWrapFlags>(&parsed_string)
            .map_err(|e| input_error!(pos.clone(), GepNoWrapFlagParseErr(e)))
            .into_parse_result()?;

        Ok(GepNoWrapFlagsAttr(flags.normalized())).into_parse_result()
    }
}

#[pliron_attr(name = "llvm.icmp_predicate", verifier = "succ", format)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum ICmpPredicateAttr {
    EQ,
    NE,
    SLT,
    SLE,
    SGT,
    SGE,
    ULT,
    ULE,
    UGT,
    UGE,
}

#[pliron_attr(name = "llvm.fcmp_predicate", format, verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum FCmpPredicateAttr {
    False,
    OEQ,
    OGT,
    OGE,
    OLT,
    OLE,
    ONE,
    ORD,
    UEQ,
    UGT,
    UGE,
    ULT,
    ULE,
    UNE,
    UNO,
    True,
}

/// An index for a GEP can be either a constant or an SSA operand.
/// Contrary to its name, this isn't an [Attribute][pliron::attribute::Attribute].
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
#[format]
pub enum GepIndexAttr {
    /// This GEP index is a raw u32 compile time constant
    Constant(u32),
    /// This GEP Index is the SSA value in the containing
    /// [Operation](pliron::operation::Operation)s `operands[idx]`
    OperandIdx(usize),
}

#[pliron_attr(
    name = "llvm.gep_indices",
    format = "`[` vec($0, CharSpace(`,`)) `]`",
    verifier = "succ"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct GepIndicesAttr(pub Vec<GepIndexAttr>);

/// An attribute that contains a list of case values for a switch operation.
#[pliron_attr(name = "llvm.case_values", format = "`[` vec($0, CharSpace(`,`)) `]`")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct CaseValuesAttr(pub Vec<IntegerAttr>);

#[derive(Debug, Error)]
#[error("Case values must be of the same type, but found different types: {0} and {1}")]
pub struct CaseValuesAttrVerifyErr(pub String, pub String);

impl Verify for CaseValuesAttr {
    fn verify(&self, ctx: &Context) -> Result<()> {
        self.0.windows(2).try_for_each(|pair| {
            pair[0].verify(ctx)?;
            if pair[0].get_type() != pair[1].get_type() {
                verify_err_noloc!(CaseValuesAttrVerifyErr(
                    pair[0].get_type().disp(ctx).to_string(),
                    pair[1].get_type().disp(ctx).to_string()
                ))
            } else {
                Ok(())
            }
        })
    }
}

#[pliron_attr(name = "llvm.linkage", format, verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum LinkageAttr {
    ExternalLinkage,
    AvailableExternallyLinkage,
    LinkOnceAnyLinkage,
    LinkOnceODRLinkage,
    LinkOnceODRAutoHideLinkage,
    WeakAnyLinkage,
    WeakODRLinkage,
    AppendingLinkage,
    InternalLinkage,
    PrivateLinkage,
    DLLImportLinkage,
    DLLExportLinkage,
    ExternalWeakLinkage,
    GhostLinkage,
    CommonLinkage,
    LinkerPrivateLinkage,
    LinkerPrivateWeakLinkage,
}

#[pliron_attr(
    name = "llvm.insert_extract_value_indices",
    format = "`[` vec($0, CharSpace(`,`)) `]`",
    verifier = "succ"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct InsertExtractValueIndicesAttr(pub Vec<u32>);

#[pliron_attr(name = "llvm.align", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct AlignmentAttr(pub u32);

/// Address space of a pointer or global, corresponding to LLVM's `addrspace(N)`.
#[pliron_attr(name = "llvm.addrspace", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct AddressSpaceAttr(pub u32);

/// The "zero" value of a type: a null pointer, or an all-zero-bits aggregate.
#[pliron_attr(name = "llvm.zero", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct ZeroAttr(pub TypeHandle);

#[attr_interface_impl]
impl TypedAttrInterface for ZeroAttr {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        self.0
    }
}

/// The `undef` value of a type.
#[pliron_attr(name = "llvm.undef", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct UndefAttr(pub TypeHandle);

#[attr_interface_impl]
impl TypedAttrInterface for UndefAttr {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        self.0
    }
}

/// The `poison` value of a type.
#[pliron_attr(name = "llvm.poison", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct PoisonAttr(pub TypeHandle);

#[attr_interface_impl]
impl TypedAttrInterface for PoisonAttr {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        self.0
    }
}

/// An attribute containing a sequence of bytes: LLVM's `ConstantDataArray` of `i8`s.
#[pliron_attr(
    name = "llvm.bytes",
    format = "`[` vec($0, CharSpace(`,`)) `]`",
    verifier = "succ"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct BytesAttr(Vec<u8>);

impl BytesAttr {
    /// Create a new [BytesAttr].
    pub fn new(bytes: Vec<u8>) -> Self {
        BytesAttr(bytes)
    }
}

impl From<BytesAttr> for Vec<u8> {
    fn from(value: BytesAttr) -> Self {
        value.0
    }
}

impl From<Vec<u8>> for BytesAttr {
    fn from(value: Vec<u8>) -> Self {
        BytesAttr::new(value)
    }
}

impl AsRef<Vec<u8>> for BytesAttr {
    fn as_ref(&self) -> &Vec<u8> {
        &self.0
    }
}

/// The type of [BytesAttr] is `[N x i8]`.
#[attr_interface_impl]
impl TypedAttrInterface for BytesAttr {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        let i8_ty = IntegerType::get(ctx, 8, Signedness::Signless);
        ArrayType::get(ctx, i8_ty.into(), self.0.len() as u64).into()
    }
}

/// A vector constant all of whose elements are `element`: LLVM's `splat (...)`
#[pliron_attr(name = "llvm.splat", format = "`<` $element ` : ` $ty `>`")]
#[derive(Clone, Debug)]
pub struct SplatAttr {
    element: AttrObj,
    ty: TypedHandle<VectorType>,
}

impl PartialEq for SplatAttr {
    fn eq(&self, other: &Self) -> bool {
        self.ty == other.ty && PartialEq::eq(&self.element, &other.element)
    }
}

impl Eq for SplatAttr {}

impl Hash for SplatAttr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.element.hash(state);
        self.ty.hash(state);
    }
}

impl SplatAttr {
    /// A vector constant of type `ty`, every element of which is `element`.
    pub fn new(element: AttrObj, ty: TypedHandle<VectorType>) -> Self {
        SplatAttr { element, ty }
    }

    /// The element that this splat repeats.
    pub fn element(&self) -> &AttrObj {
        &self.element
    }

    /// The vector type of this splat.
    pub fn ty(&self) -> TypedHandle<VectorType> {
        self.ty
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for SplatAttr {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        self.ty.into()
    }
}

/// Verify that `element`, the `idx`'th element of an aggregate or splat, is of type `expected`.
fn verify_element(
    ctx: &Context,
    idx: usize,
    element: &AttrObj,
    expected: TypeHandle,
) -> Result<()> {
    element.verify(ctx)?;
    if let Some(ty) = attr_type(&**element, ctx)
        && ty != expected
    {
        verify_err_noloc!(ConstAggregateVerifyErr::ElementType(
            idx,
            ty.disp(ctx).to_string(),
            expected.disp(ctx).to_string()
        ))?
    }
    Ok(())
}

impl Verify for SplatAttr {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // That the type is a vector is the [TypedHandle]'s to guarantee.
        let elem_ty = self.ty.deref(ctx).elem_type();
        verify_element(ctx, 0, &self.element, elem_ty)
    }
}

/// The address of a global variable or a function, LLVM's `ptr @symbol`.
#[pliron_attr(
    name = "llvm.symbol_addr",
    format = "`<@` $symbol ` : ` $ty `>`",
    verifier = "succ"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct SymbolAddrAttr {
    symbol: Identifier,
    ty: TypedHandle<PointerType>,
}

impl SymbolAddrAttr {
    /// The address, of pointer type `ty`, of `symbol` — a global or a function of
    /// the module.
    pub fn new(symbol: Identifier, ty: TypedHandle<PointerType>) -> Self {
        SymbolAddrAttr { symbol, ty }
    }

    /// The symbol whose address this is.
    pub fn symbol(&self) -> &Identifier {
        &self.symbol
    }

    /// The pointer type of this address.
    pub fn ty(&self) -> TypedHandle<PointerType> {
        self.ty
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for SymbolAddrAttr {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        self.ty.into()
    }
}

/// A constant aggregate, with a constant attribute per element:
/// LLVM's `ConstantArray`, `ConstantStruct` or `ConstantVector`
/// (and their `Data` variants)
#[pliron_attr(
    name = "llvm.aggregate",
    format = "`<[` vec($elements, CharSpace(`,`)) `] : ` $ty `>`"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct AggregateAttr {
    elements: Vec<AttrObj>,
    ty: TypeHandle,
}

impl AggregateAttr {
    /// A constant aggregate of type `ty`, with one constant attribute per element.
    pub fn new(elements: Vec<AttrObj>, ty: TypeHandle) -> Self {
        AggregateAttr { elements, ty }
    }

    /// The elements of this aggregate.
    pub fn elements(&self) -> &[AttrObj] {
        &self.elements
    }

    /// The type of this aggregate.
    pub fn ty(&self) -> TypeHandle {
        self.ty
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for AggregateAttr {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        self.ty
    }
}

#[derive(Debug, Error)]
pub enum ConstAggregateVerifyErr {
    #[error("{0} is not an array, struct or vector type")]
    NotAnAggregate(String),
    #[error("A constant of the scalable vector type {0} must use llvm.splat")]
    ScalableAggregate(String),
    #[error("Type {0} has {1} element(s), but {2} were provided")]
    NumElements(String, u64, usize),
    #[error("Element {0} is of type {1}, but {2} was expected")]
    ElementType(usize, String, String),
}

impl Verify for AggregateAttr {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let ty = self.ty.deref(ctx);
        if let Some(array_ty) = ty.downcast_ref::<ArrayType>() {
            if array_ty.size() != self.elements.len() as u64 {
                verify_err_noloc!(ConstAggregateVerifyErr::NumElements(
                    self.ty.disp(ctx).to_string(),
                    array_ty.size(),
                    self.elements.len()
                ))?
            }
            let elem_ty = array_ty.elem_type();
            for (idx, element) in self.elements.iter().enumerate() {
                verify_element(ctx, idx, element, elem_ty)?;
            }
        } else if let Some(struct_ty) = ty.downcast_ref::<StructType>() {
            if struct_ty.is_opaque() || struct_ty.num_fields() != self.elements.len() {
                verify_err_noloc!(ConstAggregateVerifyErr::NumElements(
                    self.ty.disp(ctx).to_string(),
                    if struct_ty.is_opaque() {
                        0
                    } else {
                        struct_ty.num_fields() as u64
                    },
                    self.elements.len()
                ))?
            }
            for (idx, element) in self.elements.iter().enumerate() {
                verify_element(ctx, idx, element, struct_ty.field_type(idx))?;
            }
        } else if let Some(vector_ty) = ty.downcast_ref::<VectorType>() {
            if vector_ty.is_scalable() {
                verify_err_noloc!(ConstAggregateVerifyErr::ScalableAggregate(
                    self.ty.disp(ctx).to_string()
                ))?
            }
            if vector_ty.num_elements() as usize != self.elements.len() {
                verify_err_noloc!(ConstAggregateVerifyErr::NumElements(
                    self.ty.disp(ctx).to_string(),
                    vector_ty.num_elements() as u64,
                    self.elements.len()
                ))?
            }
            let elem_ty = vector_ty.elem_type();
            for (idx, element) in self.elements.iter().enumerate() {
                verify_element(ctx, idx, element, elem_ty)?;
            }
        } else {
            verify_err_noloc!(ConstAggregateVerifyErr::NotAnAggregate(
                self.ty.disp(ctx).to_string()
            ))?
        }
        Ok(())
    }
}

/// Memory ordering for an atomic operation
#[pliron_attr(name = "llvm.atomic_ordering", verifier = "succ", format)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum AtomicOrderingAttr {
    Monotonic,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

/// The kind of an LLVM `atomicrmw` operation.
#[pliron_attr(name = "llvm.atomic_rmw_kind", verifier = "succ", format)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum AtomicRmwKindAttr {
    Xchg,
    Add,
    Sub,
    And,
    Nand,
    Or,
    Xor,
    Max,
    Min,
    UMax,
    UMin,
    FAdd,
    FSub,
    FMax,
    FMin,
}

/// Synchronization scope of an atomic operation
#[pliron_attr(name = "llvm.sync_scope", verifier = "succ", format)]
#[derive(PartialEq, Eq, Clone, Debug, Default, Hash)]
pub enum SyncScopeAttr {
    /// Synchronizes with all other threads in the system.
    #[default]
    System,
    /// Synchronizes only with other atomic operations in the same thread.
    SingleThread,
    /// A target specific scope, named in LLVM-IR.
    NamedScope(StringAttr),
}

impl SyncScopeAttr {
    /// The LLVM-IR name of this scope. The system scope is unnamed in LLVM-IR
    /// and hence maps to the empty string (as expected by `LLVMGetSyncScopeID`).
    pub fn to_name(&self) -> String {
        match self {
            SyncScopeAttr::SingleThread => "singlethread".to_string(),
            SyncScopeAttr::System => String::new(),
            SyncScopeAttr::NamedScope(name) => name.as_str().to_string(),
        }
    }
}

#[pliron_attr(
    name = "llvm.shuffle_vector_mask",
    format = "`[` vec($0, CharSpace(`,`)) `]`",
    verifier = "succ"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct ShuffleVectorMaskAttr(pub Vec<i32>);

#[cfg(test)]
mod tests {
    use expect_test::expect;
    use pliron::{parsable::parse_from_str, result::ExpectOk};

    use super::*;

    #[test]
    fn test_fastmath_flags_attr_empty() {
        let flags = FastmathFlags::empty();
        assert_eq!(flags.bits(), 0);

        let ctx = &mut Context::default();
        let flags_attr: FastmathFlagsAttr = flags.into();
        expect!["<>"].assert_eq(&flags_attr.disp(ctx).to_string());

        let parsed = parse_from_str(FastmathFlagsAttr::parser(()), ctx, "<>").expect_ok(ctx);
        assert_eq!(parsed, flags_attr);
    }

    #[test]
    fn test_fastmath_flags_attr_set_flags() {
        let mut flags = FastmathFlags::empty();
        flags |= FastmathFlags::NNAN | FastmathFlags::NINF;
        assert!(flags.contains(FastmathFlags::NNAN));
        assert!(flags.contains(FastmathFlags::NINF));
        assert!(!flags.contains(FastmathFlags::NSZ));
    }

    #[test]
    fn test_fastmath_flags_attr_fmt() {
        let ctx = &Context::default();
        let flags: FastmathFlagsAttr = (FastmathFlags::NNAN | FastmathFlags::ARCP).into();
        expect!["<NNAN | ARCP>"].assert_eq(&flags.disp(ctx).to_string());
    }

    #[test]
    fn test_fastmath_flags_attr_fmt_fast() {
        let ctx = &Context::default();
        let flags: FastmathFlagsAttr = FastmathFlags::FAST.into();
        expect!["<NNAN | NINF | NSZ | ARCP | CONTRACT | AFN | REASSOC>"]
            .assert_eq(&flags.disp(ctx).to_string());
    }

    #[test]
    fn test_fastmath_flags_attr_parse_valid() {
        let ctx = &mut Context::default();

        let parsed =
            parse_from_str(FastmathFlagsAttr::parser(()), ctx, "<NNAN | ARCP>").expect_ok(ctx);
        assert!(parsed.0.contains(FastmathFlags::NNAN));
        assert!(parsed.0.contains(FastmathFlags::ARCP));
    }

    // Test input with FAST flag set
    #[test]
    fn test_fastmath_flags_attr_parse_fast() {
        let ctx = &mut Context::default();

        let parsed = parse_from_str(FastmathFlagsAttr::parser(()), ctx, "<FAST>").expect_ok(ctx);
        assert!(parsed.0.contains(FastmathFlags::FAST));

        // FAST also means all the other flags.
        assert!(parsed.0.contains(FastmathFlags::NNAN));
        assert!(parsed.0.contains(FastmathFlags::NINF));
        assert!(parsed.0.contains(FastmathFlags::NSZ));
        assert!(parsed.0.contains(FastmathFlags::ARCP));
        assert!(parsed.0.contains(FastmathFlags::CONTRACT));
        assert!(parsed.0.contains(FastmathFlags::REASSOC));
    }

    #[test]
    fn test_fastmath_flags_attr_parse_invalid() {
        let ctx = &mut Context::default();
        let input = "<INVALIDFLAG>";
        match parse_from_str(FastmathFlagsAttr::parser(()), ctx, input) {
            Ok(parsed) => {
                panic!("Expected error, but got: {}", parsed);
            }
            Err(e) => {
                expect![[r#"
                    Compilation error: invalid input program.
                    Parse error at line: 1, column: 1
                    Error parsing fastmath flags: unrecognized named flag `INVALIDFLAG`
                "#]]
                .assert_eq(&e.to_string());
            }
        }
    }

    #[test]
    fn test_gep_no_wrap_flags_attr_fmt() {
        let ctx = &Context::default();

        let flags: GepNoWrapFlagsAttr = (GepNoWrapFlags::NUSW | GepNoWrapFlags::NUW).into();
        expect!["<NUSW | NUW>"].assert_eq(&flags.disp(ctx).to_string());

        let flags: GepNoWrapFlagsAttr = (GepNoWrapFlags::INBOUNDS | GepNoWrapFlags::NUW).into();
        expect!["<INBOUNDS | NUW>"].assert_eq(&flags.disp(ctx).to_string());
    }

    #[test]
    fn test_gep_no_wrap_flags_inbounds_implies_nusw() {
        let flags: GepNoWrapFlagsAttr = GepNoWrapFlags::INBOUNDS.into();

        assert!(flags.0.contains(GepNoWrapFlags::INBOUNDS));
        assert!(flags.0.contains(GepNoWrapFlags::NUSW));
    }

    #[test]
    fn test_gep_no_wrap_flags_attr_parse_valid() {
        let ctx = &mut Context::default();

        let parsed =
            parse_from_str(GepNoWrapFlagsAttr::parser(()), ctx, "<INBOUNDS | NUW>").expect_ok(ctx);
        assert!(parsed.0.contains(GepNoWrapFlags::INBOUNDS));
        assert!(parsed.0.contains(GepNoWrapFlags::NUSW));
        assert!(parsed.0.contains(GepNoWrapFlags::NUW));
    }

    #[test]
    fn test_gep_no_wrap_flags_attr_parse_invalid() {
        let ctx = &mut Context::default();
        let input = "<INVALIDFLAG>";

        let err = parse_from_str(GepNoWrapFlagsAttr::parser(()), ctx, input)
            .expect_err("invalid GEP no-wrap flag must fail to parse");
        let parse_errors = err
            .err
            .downcast_ref::<pliron::combine::easy::Errors<
                char,
                char,
                pliron::combine::stream::position::SourcePosition,
            >>()
            .expect("expected combine parser errors");

        let parse_err = parse_errors
            .errors
            .iter()
            .find_map(|err| match err {
                pliron::combine::easy::Error::Other(err) => {
                    err.downcast_ref::<GepNoWrapFlagParseErr>()
                }
                _ => None,
            })
            .expect("expected GepNoWrapFlagParseErr");

        expect!["Error parsing GEP no-wrap flags: unrecognized named flag `INVALIDFLAG`"]
            .assert_eq(&parse_err.to_string());
    }

    fn assert_attr_roundtrips<A>(ctx: &mut Context, attr: A)
    where
        A: Parsable<Arg = (), Parsed = A> + Printable + PartialEq + core::fmt::Debug,
    {
        let printed = attr.disp(ctx).to_string();
        let parsed = parse_from_str(A::parser(()), ctx, &printed).expect_ok(ctx);
        assert_eq!(parsed, attr, "round-trip mismatch for `{printed}`");
    }

    #[test]
    fn test_atomic_ordering_attr_roundtrip() {
        let ctx = &mut Context::default();
        for ordering in [
            AtomicOrderingAttr::Monotonic,
            AtomicOrderingAttr::Acquire,
            AtomicOrderingAttr::Release,
            AtomicOrderingAttr::AcqRel,
            AtomicOrderingAttr::SeqCst,
        ] {
            assert_attr_roundtrips(ctx, ordering);
        }
    }

    #[test]
    fn test_atomic_rmw_kind_attr_roundtrip() {
        let ctx = &mut Context::default();
        for kind in [
            AtomicRmwKindAttr::Xchg,
            AtomicRmwKindAttr::Add,
            AtomicRmwKindAttr::Sub,
            AtomicRmwKindAttr::And,
            AtomicRmwKindAttr::Nand,
            AtomicRmwKindAttr::Or,
            AtomicRmwKindAttr::Xor,
            AtomicRmwKindAttr::Max,
            AtomicRmwKindAttr::Min,
            AtomicRmwKindAttr::UMax,
            AtomicRmwKindAttr::UMin,
            AtomicRmwKindAttr::FAdd,
            AtomicRmwKindAttr::FSub,
            AtomicRmwKindAttr::FMax,
            AtomicRmwKindAttr::FMin,
        ] {
            assert_attr_roundtrips(ctx, kind);
        }
    }

    #[test]
    fn test_sync_scope_attr_roundtrip() {
        let ctx = &mut Context::default();
        for scope in [
            SyncScopeAttr::SingleThread,
            SyncScopeAttr::System,
            SyncScopeAttr::NamedScope(StringAttr::new("device".to_string())),
        ] {
            assert_attr_roundtrips(ctx, scope);
        }
    }

    #[test]
    fn test_address_space_attr_roundtrip() {
        let ctx = &mut Context::default();
        for n in [0u32, 1, 3, 5, 7] {
            assert_attr_roundtrips(ctx, AddressSpaceAttr(n));
        }
    }

    #[test]
    fn test_fp_half_attr_roundtrip() {
        use pliron::{builtin::attributes::FPHalfAttr, utils::apfloat::Half};
        let ctx = &mut Context::default();
        for s in ["0.0", "1.5", "-2.25"] {
            let value: Half = s.parse().expect("valid half literal");
            assert_attr_roundtrips(ctx, FPHalfAttr(value));
        }
    }
}

dict_key!(
    /// Attribute key for the LLVM data layout string of a [ModuleOp].
    ATTR_KEY_LLVM_DATA_LAYOUT,
    "llvm_data_layout"
);

dict_key!(
    /// Attribute key for the LLVM target triple of a [ModuleOp].
    ATTR_KEY_LLVM_TARGET_TRIPLE,
    "llvm_target_triple"
);

/// Get the LLVM data layout of `module`, if set.
pub fn get_data_layout(ctx: &Context, module: ModuleOp) -> Option<String> {
    module
        .get_operation()
        .deref(ctx)
        .attributes
        .get::<StringAttr>(&ATTR_KEY_LLVM_DATA_LAYOUT)
        .map(|attr| attr.clone().into())
}

/// Set the LLVM data layout of `module`.
pub fn set_data_layout(ctx: &Context, module: ModuleOp, data_layout: String) {
    module.get_operation().deref_mut(ctx).attributes.set(
        ATTR_KEY_LLVM_DATA_LAYOUT.clone(),
        StringAttr::new(data_layout),
    );
}

/// Get the LLVM target triple of `module`, if set.
pub fn get_target_triple(ctx: &Context, module: ModuleOp) -> Option<String> {
    module
        .get_operation()
        .deref(ctx)
        .attributes
        .get::<StringAttr>(&ATTR_KEY_LLVM_TARGET_TRIPLE)
        .map(|attr| attr.clone().into())
}

/// Set the LLVM target triple of `module`.
pub fn set_target_triple(ctx: &Context, module: ModuleOp, target_triple: String) {
    module.get_operation().deref_mut(ctx).attributes.set(
        ATTR_KEY_LLVM_TARGET_TRIPLE.clone(),
        StringAttr::new(target_triple),
    );
}

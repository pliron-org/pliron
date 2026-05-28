//! Attributes belonging to the LLVM dialect.

use std::fmt::Display;

use pliron::combine::{self, Parser, choice, parser::char::spaces};
use thiserror::Error;

use pliron::builtin::attributes::IntegerAttr;
use pliron::common_traits::Verify;
use pliron::context::Context;
use pliron::derive::{format, pliron_attr};
use pliron::location::Located;
use pliron::parsable::{IntoParseResult, Parsable};
use pliron::printable::Printable;
use pliron::result::Result;
use pliron::{impl_printable_for_display, input_error, verify_err_noloc};

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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    use pliron::{
        location,
        parsable::{self, state_stream_from_iterator},
    };

    use super::*;

    #[test]
    fn test_fastmath_flags_attr_empty() {
        let flags = FastmathFlags::empty();
        assert_eq!(flags.bits(), 0);

        let ctx = &mut Context::default();
        let flags_attr: FastmathFlagsAttr = flags.into();
        expect!["<>"].assert_eq(&flags_attr.disp(ctx).to_string());

        let input = "<>";
        let mut state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        let (parsed, _) = FastmathFlagsAttr::parse(&mut state_stream, ()).unwrap();
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

        let input = "<NNAN | ARCP>";
        let mut state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        let (parsed, _) = FastmathFlagsAttr::parse(&mut state_stream, ()).unwrap();
        assert!(parsed.0.contains(FastmathFlags::NNAN));
        assert!(parsed.0.contains(FastmathFlags::ARCP));
    }

    // Test input with FAST flag set
    #[test]
    fn test_fastmath_flags_attr_parse_fast() {
        let ctx = &mut Context::default();

        let input = "<FAST>";
        let mut state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        let (parsed, _) = FastmathFlagsAttr::parse(&mut state_stream, ()).unwrap();
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
        let state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        match FastmathFlagsAttr::parser(()).parse(state_stream) {
            Ok((parsed, _)) => {
                panic!("Expected error, but got: {}", parsed);
            }
            Err(e) => {
                expect![[r#"
                    Parse error at line: 1, column: 1
                    Error parsing fastmath flags: unrecognized named flag `INVALIDFLAG`
                "#]]
                .assert_eq(&e.to_string());
            }
        }
    }
}

//! Utilities for parsing.

use core::{num::ParseIntError, str::FromStr};

use crate::{
    arg_err,
    attribute::AttrObj,
    basic_block::BasicBlock,
    combine::{
        Parser, Stream, any, between, many, many1, none_of,
        parser::char::{digit, spaces},
        sep_by, token,
    },
    context::Ptr,
    debug_info::set_operation_result_name,
    identifier::Identifier,
    location::{Located, Location},
    operation::Operation,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    result::Result,
    r#type::TypeObj,
    value::Value,
};

use alloc::{boxed::Box, string::String, vec::Vec};

/// Parse from `parser`, ignoring whitespace(s) before and after.
/// > **Warning**: Do not use this inside inside [combine::optional] or
/// >   inside repeating combiners, such as [combine::many].
/// >   After successfully parsing one instance, if spaces are consumed to parse
/// >   the next one, but the next one doesn't exist, it is treated as a failure
/// >   that consumed some input. This messes things up. So spaces must be consumed
/// >   after a successfull parse, and not prior to an upcoming one.
/// >   A possibly right way to, for example, parse a comma separated list of [Identifier]s:
///
///```
///     # use pliron::combine::{parser::char::spaces, Parser};
///     # use pliron::parsable::Parsable;
///     let ids = spaces().with
///               (combine::sep_by::<Vec<_>, _, _, _>
///                 (pliron::identifier::Identifier::parser(()).skip(spaces()),
///                 combine::token(',').skip(spaces())));
///```
/// >    Similarly, if you want to use this inside [combine::optional], you should
/// >    use [combine::attempt] to ensure that no input is consumed if the parser fails.
pub fn spaced<Input: Stream<Token = char>, Output>(
    parser: impl Parser<Input, Output = Output>,
) -> impl Parser<Input, Output = Output> {
    combine::between(spaces(), spaces(), parser)
}

/// A parser that returns the current [Location] and does nothing else
pub fn location<'a>() -> Box<dyn Parser<StateStream<'a>, Output = Location, PartialState = ()> + 'a>
{
    combine::parser(|parsable_state: &mut StateStream<'a>| {
        combine::ParseResult::PeekOk(parsable_state.loc()).into()
    })
    .boxed()
}

/// A parser combinator to parse [TypeId](crate::type::TypeId) followed by the type's contents.
pub fn type_parser<'a>()
-> Box<dyn Parser<StateStream<'a>, Output = Ptr<TypeObj>, PartialState = ()> + 'a> {
    Ptr::<TypeObj>::parser(())
}

/// A parser to parse any Rust integer type.
pub fn int_parse<'a, IntT>(state_stream: &mut StateStream<'a>, _arg: ()) -> ParseResult<'a, IntT>
where
    IntT: FromStr,
    IntT::Err: core::error::Error + Send + Sync + 'static,
{
    many1::<String, _, _>(digit())
        .and_then(|digits| digits.parse::<IntT>())
        .parse_stream(state_stream)
        .into()
}

/// Get a parser combinator that can parse any Rust integer type.
pub fn int_parser<'a, IntT>()
-> Box<dyn Parser<StateStream<'a>, Output = IntT, PartialState = ()> + 'a>
where
    IntT: FromStr,
    IntT::Err: core::error::Error + Send + Sync + 'static,
{
    combine::parser(move |parsable_state: &mut StateStream<'a>| int_parse(parsable_state, ()))
        .boxed()
}

/// A trait for parsing an integer from a string with a given radix.
pub trait FromStrRadix: Sized {
    fn from_str_radix(src: &str, radix: u32) -> core::result::Result<Self, ParseIntError>;
}

macro_rules! impl_from_str_radix_for_int {
    ($($ty:ty),*) => {
        $(
            impl FromStrRadix for $ty {
                fn from_str_radix(src: &str, radix: u32) -> core::result::Result<Self, ParseIntError> {
                    <$ty>::from_str_radix(src, radix)
                }
            }
        )*
    };
}
impl_from_str_radix_for_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

/// Parser to parse a hexadecimal integer, which is a sequence of hexadecimal digits prefixed with `0x`.
pub fn hex_int_parse<'a, IntT>(
    state_stream: &mut StateStream<'a>,
    _arg: (),
) -> ParseResult<'a, IntT>
where
    IntT: FromStrRadix,
{
    combine::parser::char::string("0x")
        .with(many1::<String, _, _>(combine::parser::char::hex_digit()))
        .and_then(|digits| IntT::from_str_radix(&digits, 16))
        .parse_stream(state_stream)
        .into()
}

/// Get a parser combinator to parse a hexadecimal integer, which is a sequence of hexadecimal digits prefixed with `0x`.
pub fn hex_int_parser<'a, IntT>()
-> Box<dyn Parser<StateStream<'a>, Output = IntT, PartialState = ()> + 'a>
where
    IntT: FromStrRadix,
{
    combine::parser(move |parsable_state: &mut StateStream<'a>| hex_int_parse(parsable_state, ()))
        .boxed()
}

/// Parse a quoted string, which is a double-quoted string that may contain escaped characters.
pub fn quoted_string_parse<'a>(
    state_stream: &mut StateStream<'a>,
    _arg: (),
) -> ParseResult<'a, String> {
    // An escaped charater is one that is preceded by a backslash.
    let escaped_char = combine::parser(move |parsable_state: &mut StateStream<'a>| {
        // This combine::parser() is so that we can get a location before the parsing begins.
        let loc = parsable_state.loc();
        let mut escaped_char = token('\\').with(any()).then(move |c: char| {
            let loc = loc.clone();
            // This combine::parser() is so that we can return an error of the right type.
            // I can't get the right error type with `and_then`
            combine::parser(move |_parsable_state: &mut StateStream<'a>| {
                // Filter out the escaped characters that we handle.
                let result = match c {
                    '\\' => Ok('\\'),
                    '\"' => Ok('\"'),
                    _ => arg_err!(loc.clone(), "Unexpected escaped character \\{}", c),
                };
                result.into_parse_result()
            })
        });
        escaped_char.parse_stream(parsable_state).into()
    });

    // We want to scan a double quote deliminted string with possibly escaped characters in between.
    let quoted_string = between(
        token('"'),
        token('"'),
        many(escaped_char.or(none_of("\"".chars()))),
    );

    quoted_string
        .map(|chars: Vec<char>| {
            // Convert the characters to a string.
            chars.into_iter().collect::<String>()
        })
        .parse_stream(state_stream)
        .into()
}

/// A parser combinator to parse a quoted string, which is a double-quoted string that may contain escaped characters.
pub fn quoted_string_parser<'a>()
-> Box<dyn Parser<StateStream<'a>, Output = String, PartialState = ()> + 'a> {
    combine::parser(move |parsable_state: &mut StateStream<'a>| {
        quoted_string_parse(parsable_state, ())
    })
    .boxed()
}

/// A parser combinator to parse [AttrId](crate::attribute::AttrId) followed by the attribute's contents.
pub fn attr_parser<'a>()
-> Box<dyn Parser<StateStream<'a>, Output = AttrObj, PartialState = ()> + 'a> {
    AttrObj::parser(())
}

/// Parse a delimitted list of objects.
pub fn delimited_list_parser<Input: Stream<Token = char>, Output>(
    open: char,
    close: char,
    sep: char,
    parser: impl Parser<Input, Output = Output>,
) -> impl Parser<Input, Output = Vec<Output>> {
    between(
        token(open).skip(spaces()),
        spaces().with(token(close)),
        list_parser(sep, parser),
    )
}

/// Parse a list of objects.
pub fn list_parser<Input: Stream<Token = char>, Output>(
    sep: char,
    parser: impl Parser<Input, Output = Output>,
) -> impl Parser<Input, Output = Vec<Output>> {
    sep_by::<Vec<_>, _, _, _>(parser.skip(spaces()), token(sep).skip(spaces()))
}

/// Parse zero-or-more occurrences (ignoring spaces) of `parser`.
pub fn zero_or_more_parser<Input: Stream<Token = char>, Output>(
    parser: impl Parser<Input, Output = Output>,
) -> impl Parser<Input, Output = Vec<Output>> {
    many::<Vec<_>, _, _>(spaces().with(parser.skip(spaces())))
}

/// Parse an identifier into an SSA [Value]. Typically called to parse
/// the SSA operands of an [Operation]. If the SSA value hasn't been defined yet,
/// a [forward reference](crate::builtin::ops::ForwardRefOp) is returned.
pub fn ssa_opd_parse<'a>(state_stream: &mut StateStream<'a>, _arg: ()) -> ParseResult<'a, Value> {
    Identifier::parser(())
        .parse_stream(state_stream)
        .map(|opd| {
            state_stream
                .state
                .name_tracker
                .ssa_use(state_stream.state.ctx, &opd)
        })
        .into()
}

/// A parser to parse an identifier into an SSA [Value]. Typically called to parse
/// the SSA operands of an [Operation]. If the SSA value hasn't been defined yet,
/// a [forward reference](crate::builtin::ops::ForwardRefOp) is returned.
pub fn ssa_opd_parser<'a>()
-> Box<dyn Parser<StateStream<'a>, Output = Value, PartialState = ()> + 'a> {
    combine::parser(move |parsable_state: &mut StateStream<'a>| ssa_opd_parse(parsable_state, ()))
        .boxed()
}

/// Parse a block label into a [`Ptr<BasicBlock>`]. Typically called to parse
/// the block operands of an [Operation]. If the block doesn't exist, it's created,
pub fn block_opd_parse<'a>(
    state_stream: &mut StateStream<'a>,
    _arg: (),
) -> ParseResult<'a, Ptr<BasicBlock>> {
    token('^')
        .with(Identifier::parser(()))
        .parse_stream(state_stream)
        .map(|opd| {
            state_stream
                .state
                .name_tracker
                .block_use(state_stream.state.ctx, &opd)
        })
        .into()
}

/// A parser to parse a block label into a [`Ptr<BasicBlock>`]. Typically called to parse
/// the block operands of an [Operation]. If the block doesn't exist, it's created,
pub fn block_opd_parser<'a>()
-> Box<dyn Parser<StateStream<'a>, Output = Ptr<BasicBlock>, PartialState = ()> + 'a> {
    combine::parser(move |parsable_state: &mut StateStream<'a>| block_opd_parse(parsable_state, ()))
        .boxed()
}

/// After an [Operation] is fully parsed, for each result,
/// set its name and register it as an SSA definition.
pub fn process_parsed_ssa_defs(
    state_stream: &mut StateStream,
    results: &[(Identifier, Location)],
    op: Ptr<Operation>,
) -> Result<()> {
    let ctx = &mut state_stream.state.ctx;
    assert!(
        results.len() == op.deref(ctx).get_num_results(),
        "Error processing parsed SSA definitions. Result count mismatch"
    );

    let name_tracker = &mut state_stream.state.name_tracker;
    for (idx, name_loc) in results.iter().enumerate() {
        let res = op.deref(ctx).get_result(idx);
        name_tracker.ssa_def(ctx, name_loc, res)?;
        set_operation_result_name(ctx, op, idx, Some(name_loc.0.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use alloc::{format, string::ToString};

    use expect_test::expect;

    use crate::{
        context::Context,
        location,
        parsable::{self, state_stream_from_iterator},
        printable::Printable,
    };

    #[test]
    fn test_parse_type() {
        let mut ctx = Context::new();

        let state_stream = state_stream_from_iterator(
            "builtin.some".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let res = type_parser().parse(state_stream);
        let err_msg = format!("{}", res.err().unwrap());

        let expected_err_msg = expect![[r#"
            Parse error at line: 1, column: 1
            Unregistered type builtin.some
        "#]];
        expected_err_msg.assert_eq(&err_msg);

        let state_stream = state_stream_from_iterator(
            "builtin.integer a".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let res = type_parser().parse(state_stream);
        let err_msg = format!("{}", res.err().unwrap());

        let expected_err_msg = expect![[r#"
            Parse error at line: 1, column: 17
            Unexpected `a`
            Expected whitespaces, si, ui or i
        "#]];
        expected_err_msg.assert_eq(&err_msg);

        let state_stream = state_stream_from_iterator(
            "builtin.integer si32".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let parsed = type_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed.disp(&ctx).to_string(), "builtin.integer si32");
    }

    #[test]
    fn test_hex_int_parser() {
        use crate::{
            context::Context,
            location,
            parsable::{self, state_stream_from_iterator},
        };

        let mut ctx = Context::new();

        // Valid hex integer
        let state_stream = state_stream_from_iterator(
            "0xff".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let parsed: u64 = hex_int_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed, 0xff);

        // Valid hex integer with uppercase digits
        let state_stream = state_stream_from_iterator(
            "0xDEAD".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let parsed: u64 = hex_int_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed, 0xDEAD);

        // u32 type
        let state_stream = state_stream_from_iterator(
            "0xCAFE".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let parsed: u32 = hex_int_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed, 0xCAFEu32);

        // u8 type
        let state_stream = state_stream_from_iterator(
            "0x7f".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let parsed: u8 = hex_int_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed, 0x7fu8);

        // i64 type
        let state_stream = state_stream_from_iterator(
            "0x1234".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let parsed: i64 = hex_int_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed, 0x1234i64);

        // usize type
        let state_stream = state_stream_from_iterator(
            "0xABCDEF".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let parsed: usize = hex_int_parser().parse(state_stream).unwrap().0;
        assert_eq!(parsed, 0xABCDEFusize);

        // Value too large for u8 (0x100 = 256) should fail
        {
            let state_stream = state_stream_from_iterator(
                "0x100".chars(),
                parsable::State::new(&mut ctx, location::Source::InMemory),
            );
            let res = hex_int_parser::<u8>().parse(state_stream);
            assert!(res.is_err());
        }

        // Value too large for u16 (0x10000 = 65536) should fail
        {
            let state_stream = state_stream_from_iterator(
                "0x10000".chars(),
                parsable::State::new(&mut ctx, location::Source::InMemory),
            );
            let res = hex_int_parser::<u16>().parse(state_stream);
            assert!(res.is_err());
        }

        // Missing 0x prefix should fail
        {
            let state_stream = state_stream_from_iterator(
                "ff".chars(),
                parsable::State::new(&mut ctx, location::Source::InMemory),
            );
            let res = hex_int_parser::<u64>().parse(state_stream);
            assert!(res.is_err());
        }

        // No digits after 0x should fail
        {
            let state_stream = state_stream_from_iterator(
                "0x".chars(),
                parsable::State::new(&mut ctx, location::Source::InMemory),
            );
            let res = hex_int_parser::<u64>().parse(state_stream);
            assert!(res.is_err());
        }
    }
}

//! Source location for different IR entities

use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt::Debug;
use rustc_hash::FxHashSet;

use crate::{
    attribute::AttrObj,
    combine::{
        Parser, between, optional,
        parser::char::{spaces, string},
        stream::position::SourcePosition,
        token,
    },
    context::Context,
    deps::io::PathBuf,
    impl_printable_for_display,
    irfmt::{
        parsers::{delimited_list_parser, quoted_string_parser, spaced},
        printers::list_with_sep,
    },
    parsable::Parsable,
    printable::{self, Printable},
    uniqued_any::{self, UniquedKey},
};

/// Where is the source program?
#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub enum Source {
    /// Program being read from a file.
    File(UniquedKey<PathBuf>),
    /// Program is in memory.
    // TODO: Maybe have an `Rc` to the buffer?
    InMemory,
}

impl Source {
    /// Get a [Source] handle to the source specified by `path`.
    pub fn new_from_file(ctx: &mut Context, path: impl Into<PathBuf>) -> Self {
        Self::File(uniqued_any::save(ctx, path.into()))
    }
}

impl Printable for Source {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            Source::File(path_key) => {
                write!(f, "\"{}\"", uniqued_any::get(ctx, *path_key).display())
            }
            Source::InMemory => write!(f, "<in-memory>"),
        }
    }
}

impl Parsable for Source {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut crate::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> crate::parsable::ParseResult<'a, Self::Parsed> {
        enum Source {
            Filename(String),
            InMemory,
        }

        let mut parser = quoted_string_parser()
            .map(Source::Filename)
            .or(string("<in-memory>").map(|_| Source::InMemory));

        parser
            .parse_stream(state_stream)
            .map(|source| match source {
                Source::Filename(s) => Self::new_from_file(state_stream.state.ctx, s),
                Source::InMemory => Self::InMemory,
            })
            .into_result()
    }
}

/// Represents a (combination of) program source locations.
/// This captures more or less the functionality of MLIR's
/// [BuiltinLocationAttributes](https://mlir.llvm.org/docs/Dialects/Builtin/#location-attributes).
/// For simplicity, unlike in MLIR, [Location] is not extensible.
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Location {
    /// A [Source] along with a [position](SourcePosition) within it.
    /// This is same as MLIR's [FileLineColLoc](https://mlir.llvm.org/docs/Dialects/Builtin/#filelinecolloc).
    SrcPos { src: Source, pos: SourcePosition },
    /// A collection of other source locations.
    /// This is same as MLIR's [FusedLoc](https://mlir.llvm.org/docs/Dialects/Builtin/#fusedloc).
    Fused {
        metadata: Option<AttrObj>,
        locations: Vec<Location>,
    },
    /// Location with a name.
    /// See [NameLoc](https://mlir.llvm.org/docs/Dialects/Builtin/#nameloc).
    Named {
        name: String,
        child_loc: Box<Location>,
    },
    /// Connects the location of a callee with the location of the caller.
    /// See [CallSiteLoc](https://mlir.llvm.org/docs/Dialects/Builtin/#callsiteloc).
    CallSite {
        callee: Box<Location>,
        caller: Box<Location>,
    },
    /// Location unknown.
    /// See [UnknownLoc](https://mlir.llvm.org/docs/Dialects/Builtin/#unknownloc).
    Unknown,
}

impl Location {
    /// If the location is from exactly one source, get that source.
    pub fn source(&self) -> Option<Source> {
        let sources = self.sources();
        if sources.len() == 1 {
            sources.first().cloned()
        } else {
            None
        }
    }

    /// Get all sources that this location is associated with.
    pub fn sources(&self) -> Vec<Source> {
        let mut res = FxHashSet::default();
        fn sources(loc: &Location, res: &mut FxHashSet<Source>) {
            match loc {
                Location::SrcPos { src, pos: _ } => {
                    res.insert(*src);
                }
                Location::Fused {
                    metadata: _,
                    locations,
                } => {
                    for loc in locations {
                        sources(loc, res);
                    }
                }
                Location::Named { name: _, child_loc } => {
                    sources(child_loc, res);
                }
                Location::CallSite { callee, caller } => {
                    sources(callee, res);
                    sources(caller, res);
                }
                Location::Unknown => (),
            }
        }
        sources(self, &mut res);
        res.into_iter().collect()
    }

    /// Is this unknown?
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl Printable for Location {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            Self::SrcPos { src, pos } => {
                write!(f, "{}: {}", src.disp(ctx), pos)
            }
            Self::Fused {
                metadata,
                locations,
            } => {
                write!(f, "fused")?;
                if let Some(metadata) = metadata {
                    write!(f, "<{}>", metadata.disp(ctx))?;
                }
                write!(
                    f,
                    "[{}]",
                    list_with_sep(locations, printable::ListSeparator::CharSpace(',')).disp(ctx),
                )
            }
            Self::Named { name, child_loc } => {
                write!(f, "name: \"{}\", loc: ({})", name, child_loc.disp(ctx))
            }
            Self::CallSite { callee, caller } => {
                write!(f, "callsite({} at {})", callee.disp(ctx), caller.disp(ctx))
            }
            Self::Unknown => write!(f, "?"),
        }
    }
}

impl Parsable for SourcePosition {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut crate::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> crate::parsable::ParseResult<'a, Self::Parsed> {
        (
            string("line").skip(spaced(token(':'))),
            i32::parser(()).skip(spaced(token(','))),
            string("column").skip(spaced(token(':'))),
            i32::parser(()),
        )
            .parse_stream(state_stream)
            .map(|(_, line, _, column)| SourcePosition { line, column })
            .into_result()
    }
}

impl_printable_for_display!(SourcePosition);

impl Parsable for Location {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut crate::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> crate::parsable::ParseResult<'a, Self::Parsed> {
        let src_pos_parser = (
            Source::parser(()).skip(spaced(token(':'))),
            SourcePosition::parser(()),
        )
            .map(|(src, pos)| Location::SrcPos { src, pos });

        let fused_parser = string("fused").skip(spaces()).with(
            (
                optional(between(token('<'), token('>'), spaced(AttrObj::parser(())))),
                delimited_list_parser('[', ']', ',', Location::parser(())),
            )
                .map(|(metadata, locations)| Location::Fused {
                    metadata,
                    locations,
                }),
        );

        let named_parser = (
            string("name").skip(spaced(token(':'))),
            quoted_string_parser().skip(spaced(token(','))),
            string("loc").skip(spaced(token(':'))),
            between(token('('), token(')'), Location::parser(())),
        )
            .map(|(_, name, _, child_loc)| Location::Named {
                name,
                child_loc: Box::new(child_loc),
            });

        let callsite_parser = string("callsite")
            .with(between(
                spaced(token('(')),
                spaced(token(')')),
                (
                    Location::parser(()).skip(spaced(string("at"))),
                    Location::parser(()),
                ),
            ))
            .map(|(callee, caller)| Location::CallSite {
                callee: Box::new(callee),
                caller: Box::new(caller),
            });

        let unknown_parser = token('?').map(|_| Location::Unknown);

        let mut parser = src_pos_parser
            .or(fused_parser)
            .or(named_parser)
            .or(callsite_parser)
            .or(unknown_parser);

        parser.parse_stream(state_stream).into_result()
    }
}

/// Any object that has an associated location.
pub trait Located {
    fn loc(&self) -> Location;
    fn set_loc(&mut self, loc: Location);
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::ToString, vec};
    use expect_test::expect;

    use crate::{
        builtin::attributes::StringAttr,
        combine::ParseError,
        input_err,
        location::Source,
        parsable::{self, state_stream_from_iterator},
    };

    use super::*;

    fn parse_location(input: &str, ctx: &mut Context) -> Location {
        let state_stream =
            state_stream_from_iterator(input.chars(), parsable::State::new(ctx, Source::InMemory));

        let res = match Location::parser(()).parse(state_stream) {
            Ok(loc) => Ok(loc.0),
            Err(err) => {
                let loc = Location::SrcPos {
                    src: Source::InMemory,
                    pos: err.position(),
                };
                input_err!(loc, "Failed to parse location: {}", err)
            }
        };

        match res {
            Ok(loc) => loc,
            Err(e) => {
                panic!("{}", e.disp(ctx))
            }
        }
    }

    fn print_location(loc: &Location, ctx: &Context) -> String {
        format!("{}", loc.disp(ctx))
    }

    #[test]
    fn test_print_and_parse_srcpos_location() {
        let mut ctx = Context::default();
        let path = PathBuf::from("foo.mlir");
        let src = Source::new_from_file(&mut ctx, path.clone());
        let pos = SourcePosition {
            line: 42,
            column: 7,
        };
        let loc = Location::SrcPos { src, pos };

        let printed = print_location(&loc, &ctx);
        expect![[r#""foo.mlir": line: 42, column: 7"#]].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, loc);
    }

    #[test]
    fn test_print_and_parse_inmemory_srcpos_location() {
        let mut ctx = Context::default();
        let src = Source::InMemory;
        let pos = SourcePosition { line: 1, column: 2 };
        let loc = Location::SrcPos { src, pos };

        let printed = print_location(&loc, &ctx);
        expect![[r#"<in-memory>: line: 1, column: 2"#]].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, loc);
    }

    #[test]
    fn test_print_and_parse_fused_location() {
        let mut ctx = Context::default();
        let src = Source::InMemory;
        let pos = SourcePosition { line: 3, column: 4 };
        let loc1 = Location::SrcPos { src, pos };
        let loc2 = Location::Unknown;
        let fused = Location::Fused {
            metadata: None,
            locations: vec![loc1.clone(), loc2.clone()],
        };

        let printed = print_location(&fused, &ctx);
        expect![[r#"fused[<in-memory>: line: 3, column: 4, ?]"#]].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, fused);
    }

    #[test]
    fn test_print_and_parse_named_location() {
        let mut ctx = Context::default();
        let child = Location::Unknown;
        let named = Location::Named {
            name: "foo".to_string(),
            child_loc: Box::new(child.clone()),
        };

        let printed = print_location(&named, &ctx);
        expect![[r#"name: "foo", loc: (?)"#]].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, named);
    }

    #[test]
    fn test_print_and_parse_callsite_location() {
        let mut ctx = Context::default();
        let callee = Location::Unknown;
        let caller = Location::Unknown;
        let callsite = Location::CallSite {
            callee: Box::new(callee.clone()),
            caller: Box::new(caller.clone()),
        };

        let printed = print_location(&callsite, &ctx);
        expect![r#"callsite(? at ?)"#].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, callsite);
    }

    #[test]
    fn test_print_and_parse_unknown_location() {
        let mut ctx = Context::default();
        let loc = Location::Unknown;

        let printed = print_location(&loc, &ctx);
        expect![r#"?"#].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, loc);
    }

    #[test]
    fn test_fused_with_metadata_print_and_parse() {
        let mut ctx = Context::default();

        let attr = StringAttr::new("testattr".to_string());
        let loc = Location::Fused {
            metadata: Some(Box::new(attr)),
            locations: vec![Location::Unknown],
        };

        let printed = print_location(&loc, &ctx);
        expect![[r#"fused<builtin.string "testattr">[?]"#]].assert_eq(&printed);

        let parsed = parse_location(&printed, &mut ctx);
        assert_eq!(parsed, loc);
    }
}

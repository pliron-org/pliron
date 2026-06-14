//! IR objects that can be parsed from their text representation.

use alloc::{boxed::Box, string::String, vec, vec::Vec};
use core::any::Any;
use thiserror::Error;

use crate::{
    basic_block::BasicBlock,
    builtin::{
        op_interfaces::{IsolatedFromAboveInterface, OneResultInterface},
        ops::ForwardRefOp,
    },
    combine::{
        Parser, Positioned, StreamOnce, choice,
        easy::{self, Errors, ParseError},
        error::{StdParseResult2, Tracked},
        parser::char::string,
        stream::{
            self, IteratorStream, buffered,
            position::{self, SourcePosition},
            state::Stream,
        },
    },
    context::{Context, Ptr},
    deps::hash::{FxHashMap, hash_map::Entry},
    identifier::Identifier,
    input_err,
    irfmt::parsers::{hex_int_parser, int_parser, quoted_string_parser},
    location::{self, Located, Location, Source},
    op::op_impls,
    operation::Operation,
    result::{self, Result},
    value::{DefiningEntity, Value},
};

/// State during parsing of any [Parsable] object.
/// Every parser implemented using [Parsable] will be passed
/// a mutable reference (wrapped with [StateStream]) to this state.
pub struct State<'a> {
    /// The [Context] in which the parsing is being done.
    pub ctx: &'a mut Context,
    /// The [NameTracker] for this parsing session.
    pub(crate) name_tracker: NameTracker,
    /// The [Source] from which the input is being read.
    pub src: Source,
    /// Aribtrary state data that different parsers may want to use.
    pub aux_data: FxHashMap<Identifier, Box<dyn Any>>,
}

impl<'a> State<'a> {
    /// Create a new empty [State].
    pub fn new(ctx: &'a mut Context, src: Source) -> State<'a> {
        State {
            ctx,
            name_tracker: NameTracker::default(),
            src,
            aux_data: FxHashMap::default(),
        }
    }
}

/// A wrapper around any [char] [Iterator] object.
/// Buffering and positioning are automatically handled hereafter.
pub struct CharIterator<'a>(Box<dyn Iterator<Item = char> + 'a>);

impl Iterator for CharIterator<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

/// A [State]ful [Stream]. Every [Parsable::parser] gets this as input,
/// allowing for the parser to have access to a state.
pub type StateStream<'a> = Stream<
    buffered::Stream<
        easy::Stream<
            stream::position::Stream<stream::IteratorStream<CharIterator<'a>>, SourcePosition>,
        >,
    >,
    State<'a>,
>;

impl Located for StateStream<'_> {
    fn loc(&self) -> location::Location {
        location::Location::SrcPos {
            src: self.state.src,
            pos: self.position(),
        }
    }

    fn set_loc(&mut self, _loc: Location) {
        panic!("Cannot set location of parser");
    }
}

pub type ParseResult<'a, T> = StdParseResult2<T, <StateStream<'a> as StreamOnce>::Error>;

/// Any object that can be parsed from its [Printable](crate::printable::Printable) text.
///
/// Implement [parse](Parsable::parse) and call [parser](Parsable::parser)
/// to get a parser combinator that can be combined with any other parser
/// from the [combine] library.
///
/// The [parse](Parsable::parse) function may take arguments whose type is
/// specified and constrained by the associated type [Parsable::Arg].
/// Example:
/// ```
/// use pliron::combine::{
///     Parser, Stream, StreamOnce, easy, stream::position,
///     parser::char::digit, many1
/// };
/// use pliron::{context::Context, parsable::
///     { state_stream_from_iterator, StateStream, Parsable, State, ParseResult},
///     location::Source,
/// };
/// #[derive(PartialEq, Eq)]
/// struct Number { n: u64 }
/// impl Parsable for Number {
///     type Arg = ();
///     type Parsed = Number;
///     fn parse<'a>(
///         state_stream: &mut StateStream<'a>,
///         arg: Self::Arg,
///     ) -> ParseResult<'a, Self::Parsed> {
///         many1::<String, _, _>(digit())
///         .map(|digits| {
///             let _ : &mut Context = state_stream.state.ctx;
///             Number { n: digits.parse::<u64>().unwrap() }
///         })
///         .parse_stream(&mut state_stream.stream).into()
///     }
/// }
/// let mut ctx = Context::new();
/// let state_stream = state_stream_from_iterator("100".chars(), State::new(&mut ctx, Source::InMemory));
/// assert!(Number::parser(()).parse(state_stream).unwrap().0 == Number { n: 100 });
///
/// ```
pub trait Parsable {
    /// Type of the argument that must be passed to the parser.
    type Arg: Clone + 'static;
    /// The type of the parsed entity.
    type Parsed;

    /// Define a parser using existing combinators and call
    /// `into` on [Parser::parse_stream] to get the final [ParseResult].
    /// Use [state_stream.state](StateStream::state) as necessary.
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>;

    /// Get a parser combinator that can work on [StateStream] as its input.
    fn parser<'a>(
        arg: Self::Arg,
    ) -> Box<dyn Parser<StateStream<'a>, Output = Self::Parsed, PartialState = ()> + 'a> {
        combine::parser(move |parsable_state: &mut StateStream<'a>| {
            Self::parse(parsable_state, arg.clone())
        })
        .boxed()
    }
}

/// Build a [StateStream] from an iterator, for use with [Parsable].
///
/// Examples:
///
/// 1. Creating a [StateStream] from a string input:
/// ```
/// use pliron::{
///     combine::Parser,
///     context::{Context, Ptr},
///     location::Source,
///     operation::Operation,
///     parsable::{state_stream_from_iterator, State},
/// };
///
/// let mut ctx = Context::new();
/// let source = Source::InMemory;
/// let input = "builtin.module @m {}";
/// let state_stream =
///     state_stream_from_iterator(input.chars(), State::new(&mut ctx, source));
/// let (_parsed_res, _): (Ptr<Operation>, _) =
///     Operation::top_level_parser().parse(state_stream).unwrap();
/// ```
///
/// 2. Creating a [StateStream] from a file input:
/// ```no_run
/// use pliron::{
///     combine::Parser,
///     context::{Context, Ptr},
///     location::Source,
///     operation::Operation,
///     parsable::{state_stream_from_iterator, State},
/// };
///
/// fn main() {
///     let mut ctx = Context::new();
///     let plir_path = std::path::PathBuf::from("input.plir");
///     let plir_file = std::fs::File::open(&plir_path).unwrap();
///     let mut plir_file = std::io::BufReader::new(plir_file);
///
///     // Use the `BufReadCharsExt` trait to read chars from the file.
///     use utf8_chars::BufReadCharsExt;
///     let chars_iter = plir_file.chars().map(|c| {
///         c.inspect_err(|e| eprint!("Error reading chars from file: {e}"))
///             .unwrap()
///     });
///
///     let source = Source::new_from_file(&mut ctx, plir_path);
///     let state_stream = state_stream_from_iterator(chars_iter, State::new(&mut ctx, source));
///     let (_parsed_res, _) : (Ptr<Operation>, _) =
///         Operation::top_level_parser().parse(state_stream).unwrap();
/// }
/// ```
pub fn state_stream_from_iterator<'a, T: Iterator<Item = char> + 'a>(
    input: T,
    state: State<'a>,
) -> StateStream<'a> {
    StateStream {
        stream: buffered::Stream::new(
            easy::Stream::from(position::Stream::with_positioner(
                IteratorStream::new(CharIterator(Box::new(input))),
                SourcePosition::default(),
            )),
            100,
        ),
        state,
    }
}

/// Convert [Result] into [StdParseResult2].
/// Enables using `?` on [Result] during parsing.
pub trait IntoParseResult<'a, T> {
    fn into_parse_result(self) -> StdParseResult2<T, <StateStream<'a> as StreamOnce>::Error>;
}

impl<'a, T> IntoParseResult<'a, T> for Result<T> {
    fn into_parse_result(self) -> StdParseResult2<T, <StateStream<'a> as StreamOnce>::Error> {
        match self {
            Ok(t) => combine::ParseResult::CommitOk(t),
            Err(e) => combine::ParseResult::CommitErr(e.into()),
        }
        .into()
    }
}

impl From<result::Error> for ParseError<StateStream<'_>> {
    fn from(value: result::Error) -> Self {
        let position = if let Location::SrcPos { pos, .. } = value.loc {
            pos
        } else {
            SourcePosition::default()
        };
        easy::Errors::from_errors(position, vec![easy::Error::Other(value.err)])
    }
}

impl From<result::Error> for combine::error::Commit<Tracked<Errors<char, char, SourcePosition>>> {
    fn from(value: result::Error) -> Self {
        let res: StdParseResult2<(), ParseError<StateStream<'_>>> =
            combine::ParseResult::CommitErr(value.into()).into();
        res.err().unwrap()
    }
}

/// Serves a similar purpose to what [ForwardRefOp] serves for SSA names.
enum LabelRef {
    ForwardRef(Ptr<BasicBlock>),
    Defined(Ptr<BasicBlock>),
}

impl LabelRef {
    fn get_label(&self) -> Ptr<BasicBlock> {
        match self {
            LabelRef::ForwardRef(label) => *label,
            LabelRef::Defined(label) => *label,
        }
    }
}

/// Utility for parsing SSA names and block labels.
#[derive(Default)]
pub(crate) struct NameTracker {
    ssa_name_scope: Vec<FxHashMap<Identifier, Value>>,
    block_label_scope: Vec<FxHashMap<Identifier, LabelRef>>,
}

#[derive(Error, Debug)]
#[error("Identifier {0} was not resolved to any definition in the scope")]
pub struct UnresolvedReference(Identifier);

#[derive(Error, Debug)]
pub enum ParserNameTrackerError {
    #[error("Identifier {0} defined more than once in the scope")]
    MultipleDefinitions(Identifier),
    #[error("Regions in a top-level operation must be IsolatedFromAbove")]
    TopLevelOpRegionNotIsolatedFromAbove,
}

impl NameTracker {
    /// An SSA use is seen. Get its [definition value][Value]
    /// or return a [forward reference][ForwardRefOp] that will
    /// be updated when the actual definition is seen.
    pub(crate) fn ssa_use(&mut self, ctx: &mut Context, id: &Identifier) -> Value {
        let scope = self
            .ssa_name_scope
            .last_mut()
            .expect("NameTracker doesn't have an active scope.");
        match scope.entry(id.clone()) {
            Entry::Occupied(occ) => *occ.get(),
            Entry::Vacant(vac) => {
                // Insert a forward reference.
                let forward_def = ForwardRefOp::new(ctx).get_result(ctx);
                vac.insert(forward_def);
                forward_def
            }
        }
    }

    /// Register an SSA definition. If `id` is already associated with a
    /// [forward reference][ForwardRefOp], update and replace all uses with `def`.
    pub(crate) fn ssa_def(
        &mut self,
        ctx: &mut Context,
        id: &(Identifier, Location),
        def: Value,
    ) -> Result<()> {
        let scope = self
            .ssa_name_scope
            .last_mut()
            .expect("NameTracker doesn't have an active scope.");

        match scope.entry(id.0.clone()) {
            Entry::Occupied(mut occ) => match occ.get_mut().defining_entity() {
                DefiningEntity::Op(op) => {
                    let fref_opt =
                        Operation::get_op::<ForwardRefOp>(op, ctx).map(|op| op.get_result(ctx));
                    if let Some(fref) = fref_opt {
                        // If there's already a def and its a forward ref, replace that.
                        fref.replace_some_uses_with(ctx, |_, _| true, &def);
                        Operation::erase(op, ctx);
                        occ.insert(def);
                    } else {
                        // There's another def and it isn't a forward ref.
                        input_err!(
                            id.1.clone(),
                            ParserNameTrackerError::MultipleDefinitions(id.0.clone())
                        )?
                    }
                }
                DefiningEntity::Block(_) => {
                    // There's another def and it isn't a forward ref.
                    input_err!(
                        id.1.clone(),
                        ParserNameTrackerError::MultipleDefinitions(id.0.clone())
                    )?
                }
            },
            Entry::Vacant(vac) => {
                vac.insert(def);
            }
        }
        Ok(())
    }

    /// A [BasicBlock] use is seen. Get the actual block it refers to.
    /// If the block wasn't seen yet, create one now. When exiting the scope,
    /// if the block ends up not being seen, an undefined reference error is returned.
    pub(crate) fn block_use(&mut self, ctx: &mut Context, id: &Identifier) -> Ptr<BasicBlock> {
        let scope = self
            .block_label_scope
            .last_mut()
            .expect("NameTracker doesn't have an active scope.");
        match scope.entry(id.clone()) {
            Entry::Occupied(occ) => occ.get().get_label(),
            Entry::Vacant(vac) => {
                // Insert a forward reference.
                let block_forward = BasicBlock::new(ctx, Some(id.clone()), vec![]);
                vac.insert(LabelRef::ForwardRef(block_forward));
                block_forward
            }
        }
    }

    /// A [BasicBlock] def is seen. If refs was seen earlier,
    /// they are all updated now to refer to the provided block instead.
    pub(crate) fn block_def(
        &mut self,
        ctx: &mut Context,
        id: &(Identifier, Location),
        block: Ptr<BasicBlock>,
    ) -> Result<()> {
        let scope = self
            .block_label_scope
            .last_mut()
            .expect("NameTracker doesn't have an active scope.");
        match scope.entry(id.0.clone()) {
            Entry::Occupied(mut occ) => match occ.get_mut() {
                LabelRef::ForwardRef(fref) => {
                    fref.retarget_some_preds_to(ctx, |_, _| true, block);
                    BasicBlock::erase(*fref, ctx);
                    occ.insert(LabelRef::Defined(block));
                }
                LabelRef::Defined(_) => input_err!(
                    id.1.clone(),
                    ParserNameTrackerError::MultipleDefinitions(id.0.clone())
                )?,
            },
            Entry::Vacant(vac) => {
                vac.insert(LabelRef::Defined(block));
            }
        }
        Ok(())
    }

    /// Enter a new region.
    /// - If the parent op is [IsolatedFromAboveInterface],
    ///   then a new independent SSA name scope is created.
    /// - A new independent block label scope is always created.
    pub(crate) fn enter_region(&mut self, ctx: &Context, parent_op: Ptr<Operation>) -> Result<()> {
        if op_impls::<dyn IsolatedFromAboveInterface>(
            Operation::get_op_dyn(parent_op, ctx).as_ref(),
        ) {
            self.ssa_name_scope.push(FxHashMap::default());
        } else if self.ssa_name_scope.is_empty() {
            input_err!(
                parent_op.deref(ctx).loc(),
                ParserNameTrackerError::TopLevelOpRegionNotIsolatedFromAbove
            )?
        }
        self.block_label_scope.push(FxHashMap::default());
        Ok(())
    }

    /// Exit a region.
    /// - If the parent op is [IsolatedFromAboveInterface], then the top SSA name scope is popped.
    /// - The top block label scope is popped.
    pub(crate) fn exit_region(
        &mut self,
        ctx: &Context,
        parent_op: Ptr<Operation>,
        loc: Location,
    ) -> Result<()> {
        if op_impls::<dyn IsolatedFromAboveInterface>(
            Operation::get_op_dyn(parent_op, ctx).as_ref(),
        ) {
            // Check if there are any [ForwardRefOp].
            let ssa_scope = self
                .ssa_name_scope
                .pop()
                .expect("Exiting an isolated-from-above region which wasn't entered into.");
            for (id, value) in ssa_scope {
                if let DefiningEntity::Op(op) = value.defining_entity()
                    && Operation::is_op::<ForwardRefOp>(op, ctx)
                {
                    input_err!(loc.clone(), UnresolvedReference(id.clone()))?
                }
            }
        }

        let label_scope = self
            .block_label_scope
            .pop()
            .expect("Exiting an isolated-from-above region which wasn't entered into.");

        // Check if there are any unresolved forward label references.
        for (id, op) in label_scope {
            if matches!(op, LabelRef::ForwardRef(_)) {
                input_err!(loc.clone(), UnresolvedReference(id.clone()))?
            }
        }

        Ok(())
    }
}

impl Parsable for usize {
    type Arg = ();
    type Parsed = usize;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<usize>().parse_stream(state_stream).into()
    }
}
impl Parsable for u64 {
    type Arg = ();
    type Parsed = u64;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<u64>().parse_stream(state_stream).into()
    }
}

impl Parsable for u32 {
    type Arg = ();
    type Parsed = u32;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<u32>().parse_stream(state_stream).into()
    }
}

impl Parsable for u16 {
    type Arg = ();
    type Parsed = u16;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<u16>().parse_stream(state_stream).into()
    }
}

impl Parsable for u8 {
    type Arg = ();
    type Parsed = u8;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<u8>().parse_stream(state_stream).into()
    }
}

impl Parsable for i8 {
    type Arg = ();
    type Parsed = i8;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<i8>().parse_stream(state_stream).into()
    }
}

impl Parsable for i16 {
    type Arg = ();
    type Parsed = i16;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<i16>().parse_stream(state_stream).into()
    }
}

impl Parsable for i32 {
    type Arg = ();
    type Parsed = i32;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<i32>().parse_stream(state_stream).into()
    }
}

impl Parsable for i64 {
    type Arg = ();
    type Parsed = i64;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        int_parser::<i64>().parse_stream(state_stream).into()
    }
}

impl Parsable for bool {
    type Arg = ();
    type Parsed = bool;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        // Choose b/w true/false
        let mut bool_parser =
            choice((string("true").map(|_| true), string("false").map(|_| false)));

        bool_parser.parse_stream(state_stream).into()
    }
}

impl Parsable for String {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        quoted_string_parser().parse_stream(state_stream).into()
    }
}

impl Parsable for *const () {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        hex_int_parser::<usize>()
            .parse_stream(state_stream)
            .map(|n| n as *const ())
            .into()
    }
}

impl Parsable for *mut () {
    type Arg = ();

    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        hex_int_parser::<usize>()
            .parse_stream(state_stream)
            .map(|n| n as *mut ())
            .into()
    }
}
